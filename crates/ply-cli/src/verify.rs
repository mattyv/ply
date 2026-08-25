//! `cargo ply verify` orchestration: reads `ply.yaml`, generates and runs
//! the engine(s) each declared check needs, renders a cex test on a genuine
//! violation (Kani or proptest -- one renderer, `contract_rt`, per D7's own
//! design), runs `mutate` against whatever `test`/`fuzz` tests exist as its
//! kill signal (D12), and assembles the §8 JSON envelope. This wiring lives
//! in ply-cli (not ply-core) per the M3 brief's module restriction on
//! ply-core, which M4 keeps.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ply_core::callgraph::{CalleeStatus, DeclaredContract, Resolver};
use ply_core::config::{self, Check, FnClaim};
use ply_core::contract_rt::{self, RenderedTest};
use ply_core::diag::{Assumption, Counterexample, Diagnostic, Envelope, Evidence, Fix, Node};
use ply_core::engines::fuzz as fuzz_engine;
use ply_core::engines::kani::{self, KaniOutcome, KaniRunConfig};
use ply_core::engines::mutants::{self, MutantsRunConfig, MutantsRunOutcome};
use ply_core::harness::{self, ContractFn, StubSpec};
use ply_core::harness_crate;

pub const PLY_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct VerifyOptions {
    /// `None` means "use the shape-aware default" (Task 0: a flat default
    /// cannot fit every §5.4b-supported shape -- see
    /// `default_engine_timeout_secs`). An explicit value is always honored
    /// as-is, for every check kind.
    pub engine_timeout_secs: Option<u32>,
    /// `--seed`: the proptest RNG seed every `fuzz(n)` check in this run
    /// uses, replaying a recorded run exactly. `None` derives one per fn
    /// from its own name and contract text (`fuzz_gen::derive_seed`), which
    /// is still deterministic for identical source -- the property vetting
    /// 004's finding 4 showed missing.
    pub seed: Option<[u8; 32]>,
}

/// The engine-timeout default (Task 0 of the M4 brief): §6 used to say a
/// flat 60s for every `bounded` check, but a `bounded(8)` proof over an
/// 8-element `Vec` -- a shape §5.4b lists as *supported* -- measured at
/// 62-63s at that default in the M3 review (docs/m3-slice-findings.md),
/// i.e. it timed out on its own supported case 3/3 times. A `timeout` that
/// fires on the tool's own advertised-supported shapes is not an edge case;
/// it is the shape of ordinary use, which is exactly how a status meant to
/// mean "engine exhausted" turns into noise a user learns to skip.
///
/// The part of the fix that *is* derived is the shape split: within the
/// §5.4b subset actually implemented, `Vec` is the only shape whose CBMC
/// unwind cost grows with the bound, so it is the only shape scaled at all.
/// The constants are chosen, not derived, and the 2026-08-24 M4 review was
/// right to say so (O1): `30 + 15k` is picked to reproduce the M3 e2e
/// suite's own working value for `bounded(8)` exactly (150s) while keeping
/// `bounded(2)` at the unchanged 60s. `150 = base + rate * 8` is one
/// equation in two unknowns -- `0 + 18.75k` and `60 + 11.25k` fit it just as
/// well -- and the 150 itself is that fixture's generous constant, not a
/// measured requirement: docs/m3-slice-findings.md finding 3 measured the
/// *identical* harness anywhere from ~1s to ~107s across runs, variance that
/// dominates any k-linear model. So: a shape-aware budget fitted to the one
/// working data point we have, which is honest, and not a claim that the
/// coefficients mean anything on their own.
///
/// A scalar-only (no `Vec` parameter) harness keeps the original 60s default
/// unchanged: nothing in the M3 findings shows that budget insufficient for
/// any scalar-only fixture, and widening it without evidence would be
/// exactly the "bigger magic number" Task 0 warns against.
///
/// Not observed in real use by any test: every e2e passes `--engine-timeout`
/// explicitly, so only the unit test below exercises the formula (recorded
/// in TODO.md, not left to be discovered).
pub fn default_engine_timeout_secs(has_vec_param: bool, bound_k: u32) -> u32 {
    if has_vec_param { 30 + 15 * bound_k } else { 60 }
}

/// The `fuzz`/`test`/`mutate` engines never carry Kani's `Vec`-unwind cost
/// profile -- proptest's own strategies and plain `cargo test` do not blow
/// up the way CBMC's symbolic `Vec` construction does, so a single flat
/// default suffices (still explicitly overridable via `--engine-timeout`).
fn default_secondary_engine_timeout_secs() -> u32 {
    60
}

/// Runs `verify` over every fn claim declared in `<crate_dir>/ply.yaml`,
/// against the source at `<crate_dir>/src/lib.rs`. Returns the §8 envelope.
pub fn verify_crate(crate_dir: &Path, opts: &VerifyOptions) -> Result<Envelope> {
    let yaml_path = crate_dir.join("ply.yaml");
    let file = config::load(&yaml_path)?;
    let src_dir = crate_dir.join("src");
    let lib_path = src_dir.join("lib.rs");

    // Pass 1: discover every fn, resolve its effective checks list
    // (explicit, or the shape-aware default -- the routing decision M4
    // makes real), and validate D12's E0504 up front. Anything that can't
    // even get this far (unresolvable anchor) is finished right here.
    struct Plan<'a> {
        node_id: String,
        fn_name: &'a str,
        claim: &'a FnClaim,
        cf: ContractFn,
        checks: Vec<Check>,
        boundary: BoundaryPlan,
        seed: [u8; 32],
    }
    let mut plans: Vec<Plan> = Vec::new();
    let mut early_nodes_by_component: BTreeMap<&str, Vec<Node>> = BTreeMap::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // `anchor:` is finally consumed (vetting 004 finding 7: it was parsed
    // and ignored, so *every* component's fns were looked for in this
    // crate's own `src/lib.rs` -- which is why a claim written against a
    // dependency died with a misleading `E0301`). A component whose anchor
    // names another crate is a **boundary component**: Ply does not verify
    // its fns from here, it reads the contracts they declare (§5.5).
    let local_anchors = local_anchor_names(crate_dir);
    let is_local = |anchor: &str| -> bool {
        local_anchors.is_empty() || local_anchors.contains(&anchor.replace('-', "_"))
    };

    // §5.4's external-spec route, read for the first time on the verify
    // path: a `requires:`/`ensures:` entry declares a contract for a fn,
    // keyed by the path a caller writes.
    let mut declared: BTreeMap<String, DeclaredContract> = BTreeMap::new();
    for comp in file.components.values() {
        for (fn_key, claim) in &comp.fns {
            if claim.requires.is_empty() && claim.ensures.is_empty() {
                continue;
            }
            let path = if is_local(&comp.anchor) {
                fn_key.clone()
            } else {
                format!("{}::{}", comp.anchor, fn_key)
            };
            declared.insert(
                path.clone(),
                DeclaredContract {
                    path,
                    requires: claim.requires.clone(),
                    ensures: claim.ensures.clone(),
                },
            );
        }
    }
    let lib_src = std::fs::read_to_string(&lib_path).unwrap_or_default();
    let mut resolver = Resolver::new(&lib_src, crate_dir, declared)?;

    for (comp_name, comp) in &file.components {
        for (fn_name, claim) in &comp.fns {
            let node_id = format!("{comp_name}::{fn_name}");
            if !is_local(&comp.anchor) {
                // A boundary component. Its contracts are already in
                // `declared`; its `checks` cannot run from here, and saying
                // so is the honest report (`verify` is single-crate).
                if !claim.checks.is_empty() {
                    diagnostics.push(cross_crate_claim_diag(&node_id, fn_name, &comp.anchor));
                }
                continue;
            }
            let cf = match harness::discover_fn(&lib_path, fn_name) {
                Ok(cf) => cf,
                Err(e) => {
                    diagnostics.push(unresolved_anchor_diag(
                        &node_id,
                        fn_name,
                        "none",
                        &e.to_string(),
                    ));
                    early_nodes_by_component
                        .entry(comp_name)
                        .or_default()
                        .push(leaf_node(&node_id, "unclaimed"));
                    continue;
                }
            };

            let explicit = claim
                .parsed_checks()
                .with_context(|| format!("parsing checks for {node_id}"))?;
            // A `ply.yaml` entry that declares a contract and asks for no
            // checks is a **boundary contract declaration** (§5.5): it
            // exists so callers can assume something about this fn, not so
            // this fn gets verified. It contributes an assumption, not a
            // node -- reporting it as an `unclaimed` claim would say the
            // opposite of what the user wrote.
            let declares_contract = !claim.requires.is_empty() || !claim.ensures.is_empty();
            if declares_contract && explicit.is_empty() && !cf.has_contract() {
                continue;
            }
            if declares_contract {
                diagnostics.push(declared_contract_not_anded_diag(&node_id, fn_name));
            }

            let checks = if !explicit.is_empty() {
                explicit
            } else {
                default_checks_for(&cf)
            };

            if let Err(e) = config::validate_mutate_has_kill_signal(&checks) {
                let msg = e.to_string();
                let code = msg.split(':').next().unwrap_or("E0504").trim().to_string();
                diagnostics.push(Diagnostic {
                    code,
                    severity: "error".into(),
                    phase: "verify".into(),
                    engine: "ply".into(),
                    check: "mutate".into(),
                    node_id: node_id.clone(),
                    title: msg,
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![
                        Fix {
                            title: format!("add `test` to `{fn_name}`'s checks list"),
                            edits: vec![],
                        },
                        Fix {
                            title: format!("add `fuzz(256)` to `{fn_name}`'s checks list"),
                            edits: vec![],
                        },
                    ],
                    assumptions: vec![],
                    open_item: Some("mutate_without_kill_signal".into()),
                });
                early_nodes_by_component
                    .entry(comp_name)
                    .or_default()
                    .push(leaf_node(&node_id, "unclaimed"));
                continue;
            }

            if checks.is_empty() {
                // "none otherwise" (§5.4c): either no contract at all, or a
                // contract whose shape neither gate can build inputs for.
                if cf.has_contract() {
                    diagnostics.push(unsupported_shape_diag(&node_id, fn_name, &cf));
                    early_nodes_by_component
                        .entry(comp_name)
                        .or_default()
                        .push(leaf_node(&node_id, "unsupported"));
                } else {
                    early_nodes_by_component
                        .entry(comp_name)
                        .or_default()
                        .push(leaf_node(&node_id, "unclaimed"));
                }
                continue;
            }

            // §5.5's three-way split, decided from the call graph before
            // any engine starts. Only `bounded` needs it: Kani descends into
            // callee bodies, proptest simply runs them.
            let boundary = if checks.iter().any(|c| matches!(c, Check::Bounded(_))) {
                boundary_plan(&mut resolver, &cf)
            } else {
                BoundaryPlan::default()
            };

            // §1: "every verdict, passing or failing, must name the
            // evidence that produced it concretely enough to reproduce it".
            // For a fuzz verdict that is the seed, and it is derived from
            // the fn's own contract so identical source always replays
            // identically (`--seed` overrides).
            let contract_text = format!(
                "{}|{}",
                cf.requires.as_ref().map(|(_, s)| s.as_str()).unwrap_or(""),
                cf.ensures.as_ref().map(|(_, s)| s.as_str()).unwrap_or(""),
            );
            let seed = opts
                .seed
                .unwrap_or_else(|| ply_core::fuzz_gen::derive_seed(fn_name, &contract_text));

            plans.push(Plan {
                node_id,
                fn_name,
                claim,
                cf,
                checks,
                boundary,
                seed,
            });
        }
    }

    // Pass 2: any fn needing fuzz/test/mutate shares one generated harness
    // crate per target crate (§5.4c) -- write it once, fully, before
    // running anything, so mutate's baseline sees every fn's tests.
    let needs_harness = plans.iter().any(|p| {
        p.checks
            .iter()
            .any(|c| matches!(c, Check::Fuzz(_) | Check::Test | Check::Mutate))
    });
    let mut harness_info: Option<(String, String)> = None; // (harness_package, target_lib_ident)
    if needs_harness {
        let cargo_toml_path = crate_dir.join("Cargo.toml");
        let cargo_toml_text = std::fs::read_to_string(&cargo_toml_path)
            .with_context(|| format!("reading {}", cargo_toml_path.display()))?;
        let target_names = harness_crate::read_crate_names(&cargo_toml_text)?;
        let harness_pkg = harness_crate::harness_package_name(&target_names.package_name);
        let harness_rel = harness_crate::harness_rel_path(&target_names.package_name);
        harness_crate::ensure_workspace_member(&cargo_toml_path, &harness_rel)?;
        let harness_dir = crate_dir.join(&harness_rel);
        harness_crate::write_harness_cargo_toml(&harness_dir, &harness_pkg, &target_names)?;

        let mut fn_modules = Vec::new();
        for plan in &plans {
            let has_fuzz = plan.checks.iter().find_map(|c| {
                if let Check::Fuzz(n) = c {
                    Some(*n)
                } else {
                    None
                }
            });
            let has_test = plan.checks.iter().any(|c| matches!(c, Check::Test));
            if has_fuzz.is_none() && !has_test {
                continue;
            }
            let mut bodies = Vec::new();
            if let Some(n) = has_fuzz
                && let Ok(body) = ply_core::fuzz_gen::generate_fuzz_test(&plan.cf, n, &plan.seed)
            {
                bodies.push(body);
            }
            // A build failure here (missing #[ply::ensures], etc.) is
            // reported as a diagnostic in pass 3; no body means nothing
            // to run for the fuzz half.
            if has_test {
                for (i, example) in plan.claim.examples.iter().enumerate() {
                    if let Ok(body) = ply_core::fuzz_gen::generate_example_test(
                        plan.fn_name,
                        (i + 1) as u32,
                        example,
                    ) {
                        bodies.push(body);
                    }
                }
                let direct = ply_core::fuzz_gen::generate_direct_contract_cases(&plan.cf);
                if !direct.is_empty() {
                    bodies.push(direct);
                }
            }
            if !bodies.is_empty() {
                fn_modules.push(ply_core::fuzz_gen::wrap_fn_harness_module(
                    plan.fn_name,
                    &target_names.lib_ident,
                    &bodies,
                ));
            }
        }
        harness_crate::write_harness_lib_rs(&harness_dir, &fn_modules)?;
        harness_info = Some((harness_pkg, target_names.lib_ident));
    }

    // Pass 3: run each fn's checks and assemble its verdict + diagnostics.
    let mut component_nodes: BTreeMap<&str, Vec<Node>> = early_nodes_by_component;
    for plan in &plans {
        let (node, mut fn_diags) = run_fn_checks(
            &plan.node_id,
            &src_dir,
            &lib_path,
            crate_dir,
            plan.fn_name,
            &plan.cf,
            &plan.checks,
            &plan.boundary,
            &plan.seed,
            harness_info.as_ref(),
            opts,
        )?;
        diagnostics.append(&mut fn_diags);
        let comp_name: &str = plan.node_id.split("::").next().unwrap_or("");
        component_nodes.entry(comp_name).or_default().push(node);
    }

    let mut components: Vec<Node> = Vec::new();
    for (comp_name, fn_nodes) in component_nodes {
        components.push(Node {
            id: comp_name.to_string(),
            kind: "component".into(),
            verdict: worst_of(&fn_nodes),
            // D6: statuses are not in the evidence order -- they propagate
            // upward as flags beside the verdict. A `conditional` leaf must
            // still be visible from the root, or the trust story stops at
            // the fn nobody expanded.
            statuses: union_statuses(&fn_nodes),
            evidence: None,
            children: fn_nodes,
        });
    }

    let root = Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: worst_of(&components),
        statuses: union_statuses(&components),
        evidence: None,
        children: components,
    };

    Ok(Envelope {
        command: "verify".into(),
        ply_version: PLY_VERSION.into(),
        root,
        diagnostics,
    })
}

/// The shape-aware default routing (§5.4c, the M4 non-negotiable MUST):
/// `[bounded(2)]` when the fn has a contract and its signature passes the
/// Kani gate; `[fuzz(256)]` when the shape is excluded from `bounded` but
/// the fuzz gate still passes; empty (checked elsewhere against
/// `has_contract`) otherwise. A flat `[bounded(2)]` default would route
/// most contracted functions in ordinary Rust into `unsupported` or a
/// multi-minute timeout (§5.4c).
fn default_checks_for(cf: &ContractFn) -> Vec<Check> {
    if !cf.has_contract() {
        return vec![];
    }
    if cf.is_bounded_supported() {
        vec![Check::Bounded(2)]
    } else if cf.is_fuzz_supported() {
        vec![Check::Fuzz(256)]
    } else {
        vec![]
    }
}

/// The names this crate answers to in an `anchor:` (§5.1): its `[lib] name`
/// and its package name, both normalised to Rust identifier spelling. Empty
/// when there is no readable `Cargo.toml`, in which case every component is
/// treated as local -- the pre-2026-08-25 behaviour, kept as the fallback so
/// a missing manifest degrades rather than mis-reports.
fn local_anchor_names(crate_dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(crate_dir.join("Cargo.toml")) else {
        return vec![];
    };
    match harness_crate::read_crate_names(&text) {
        Ok(names) => vec![
            names.lib_ident.replace('-', "_"),
            names.package_name.replace('-', "_"),
        ],
        Err(_) => vec![],
    }
}

/// What §5.5's split found in one function's body: callees nothing
/// describes (the third branch -- refuse to descend) and callees whose
/// declared contract will be assumed and stubbed (the second branch).
#[derive(Default)]
pub struct BoundaryPlan {
    /// `(callee path, where it is called)` for every callee no contract
    /// describes.
    unclaimed: Vec<(String, String)>,
    stubs: Vec<StubSpec>,
    /// Callees whose contract is declared but whose return type Ply's
    /// codegen cannot build a `kani::any()` for -- reported, never silently
    /// treated as either stubbed or unclaimed.
    unstubbable: Vec<(String, String)>,
    /// `(callee path, where it is called, why Ply could not read it)` for
    /// every callee inside this workspace whose source Ply was pointed at
    /// and could not open. Not the same fact as "no contract describes it",
    /// so not the same diagnostic.
    opaque: Vec<(String, String, String)>,
}

fn boundary_plan(resolver: &mut Resolver, cf: &ContractFn) -> BoundaryPlan {
    let mut plan = BoundaryPlan::default();
    for site in &cf.calls {
        match resolver.classify(site).status {
            // `Unresolved` means the call leads out of the workspace --
            // `std`, `core`, a registry crate -- which §5.5 states as this
            // rule's gap rather than pretending to have closed. It is the
            // one status that still licenses a descent, and it is now the
            // only one that can: before 2026-08-25 it also covered every
            // first-party callee Ply merely failed to look up (the review's
            // D1), so a `use` import bought a clean proof over an unclaimed
            // body.
            CalleeStatus::Contracted | CalleeStatus::Unresolved => {}
            CalleeStatus::Opaque(reason) => {
                if !plan.opaque.iter().any(|(p, _, _)| p == &site.path) {
                    plan.opaque
                        .push((site.path.clone(), site.where_text(), reason));
                }
            }
            CalleeStatus::Unclaimed => {
                if !plan.unclaimed.iter().any(|(p, _)| p == &site.path) {
                    plan.unclaimed.push((site.path.clone(), site.where_text()));
                }
            }
            CalleeStatus::Assumed {
                contract,
                signature,
            } => {
                if plan.stubs.iter().any(|s| s.callee_path == site.path) {
                    continue;
                }
                match signature.return_type {
                    Some(ret) => plan.stubs.push(StubSpec {
                        callee_path: site.path.clone(),
                        params: signature.params,
                        return_type: ret,
                        requires: contract.requires,
                        ensures: contract.ensures,
                    }),
                    None => plan
                        .unstubbable
                        .push((site.path.clone(), site.where_text())),
                }
            }
        }
    }
    plan
}

/// D5's third branch (§5.5): the caller's `bounded` check earns nothing,
/// and the diagnostic names the callee -- which is the whole point. Before
/// this rule, vetting 004's boundary function reported `timeout` after
/// 11m23s with a title that mentioned only the caller, so nothing told the
/// reader the cost came from across a boundary, let alone which call it was.
fn unclaimed_callee_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    unclaimed: &[(String, String)],
) -> Diagnostic {
    let named: Vec<String> = unclaimed
        .iter()
        .map(|(p, w)| format!("`{p}` (called at {w})"))
        .collect();
    let list = named.join(", ");
    let first = &unclaimed[0].0;
    Diagnostic {
        code: "W0512".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "Ply did not check `{fn_name}`: proving it would mean descending into {list}, and no \
             contract anywhere describes what that code promises -- not on the function itself, and \
             not in ply.yaml. A `bounded` proof reasons about a function's callees through their \
             contracts, so a callee with no contract leaves nothing to reason with. Ply refuses to \
             descend instead: pulling the real body into the proof either exhausts the time budget \
             and reports nothing, or produces a `bounded` verdict whose meaning quietly includes \
             code nobody vouched for. So this check earned no evidence at all -- the verdict is \
             `unclaimed`, never `{check_label}`, and never a violation. (W0512, §5.5)"
        ),
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "declare a contract for `{first}` in ply.yaml -- a `requires:`/`ensures:` entry \
                     under its component's `fns:`. Ply then assumes that contract, replaces the \
                     callee with it inside the proof, and marks `{fn_name}`'s verdict `conditional`, \
                     listing what was assumed"
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "swap `{check_label}` for `fuzz(256)` on `{fn_name}` -- the fuzz tier runs the \
                     real callee instead of reasoning about it, so it crosses this boundary without \
                     needing any contract (weaker evidence, but evidence)"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("unclaimed_callee".into()),
    }
}

/// The other way §5.5's third branch is reached: not "nothing describes this
/// callee" but "Ply could not read this callee at all", for code that is
/// inside the workspace and so ought to be readable. Kept a separate code
/// from `W0512` because the two say different things and offer different
/// repairs -- a `W0512` whose words claimed no contract existed, for a
/// callee Ply never even opened, would be false in exactly the way the M4
/// review's D7 was.
fn unreadable_callee_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    opaque: &[(String, String, String)],
) -> Diagnostic {
    let named: Vec<String> = opaque
        .iter()
        .map(|(p, w, why)| format!("`{p}` (called at {w}) -- {why}"))
        .collect();
    Diagnostic {
        code: "W0513".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "Ply did not check `{fn_name}`: proving it would mean descending into {list}. A \
             `bounded` proof takes on the meaning of every body it descends into, so descending \
             into a body Ply never read would produce a verdict that quietly covers code Ply \
             cannot show you. Not being able to look is not the same as there being nothing there, \
             so Ply refuses instead of assuming: this check earned no evidence at all -- the \
             verdict is `unclaimed`, never `{check_label}`. (W0513, §5.5)",
            list = named.join(", ")
        ),
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: "check that the module file is where Rust expects it -- Ply follows `mod \
                        foo;` to `foo.rs` or `foo/mod.rs` beside the file that declares it, and \
                        follows no `#[path = \"...\"]` attribute"
                    .to_string(),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "swap `{check_label}` for `fuzz(256)` on `{fn_name}` -- the fuzz tier runs the \
                     real callee instead of reasoning about it, so it needs no source Ply can read \
                     (weaker evidence, but evidence)"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("unreadable_callee".into()),
    }
}

/// D5's second branch (§5.5) reached through a `ply.yaml`-declared contract:
/// the verdict is real evidence *about the contract*, and the assumption is
/// owed evidence until something exercises it against the real body.
fn conditional_verdict_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    stubs: &[StubSpec],
) -> Diagnostic {
    let assumed: Vec<String> = stubs.iter().map(|s| s.assumption_text()).collect();
    let list = assumed.join("; ");
    let first = &stubs[0].callee_path;
    Diagnostic {
        code: "W0511".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` earned {check_label}, but conditionally: the proof used the contract \
             declared in ply.yaml for each callee it crosses into, instead of that callee's real \
             body. Assumed: {list}. That is what `conditional` means here -- the result holds if \
             those promises do. Nothing has checked them against the real code yet, so each one is \
             owed evidence rather than settled: an assumed contract nobody exercises is green paint. \
             (W0511, §5.5)"
        ),
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "claim `{first}` with `fuzz(256)` against the same contract -- the fuzz tier runs \
                     the real body, so it turns this assumption into a measured fact without asking \
                     Kani to descend into code it cannot finish"
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "or pass what `{first}` returns into `{fn_name}` as a parameter, so the value is \
                     the caller's own data and no assumption is needed at all"
                ),
                edits: vec![],
            },
        ],
        assumptions: stubs
            .iter()
            .map(|s| Assumption {
                kind: "assumed_contract".into(),
                fn_path: s.callee_path.clone(),
                verdict: "unclaimed".into(),
                contract: s.assumption_text(),
            })
            .collect(),
        open_item: Some("owed_evidence".into()),
    }
}

/// `verify` resolves every claim against one crate's `src/lib.rs`. A claim
/// whose component anchors elsewhere is not checked here, and that is said
/// rather than reported as a missing function (which is what happened before
/// `anchor:` was consumed -- vetting 004 s5's misleading `E0301`).
fn cross_crate_claim_diag(node_id: &str, fn_name: &str, anchor: &str) -> Diagnostic {
    Diagnostic {
        code: "W0303".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` is claimed under a component anchored at `{anchor}`, which is not the crate \
             this run is verifying, and `cargo ply verify` checks one crate at a time. Its `checks:` \
             were not run and no verdict is reported for it. Any `requires:`/`ensures:` this entry \
             declares is still read: that is how a callee outside this crate gets a contract Ply can \
             assume at the boundary (§5.5). (W0303)"
        ),
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "run `cargo ply verify` against the crate `{anchor}` itself to check its own \
                     claims there"
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "or drop `checks:` from this entry and keep only `requires:`/`ensures:`, if its \
                     purpose is to give `{fn_name}` a contract for callers in this crate to assume"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("not_verified_here".into()),
    }
}

/// A callee whose declared contract cannot be turned into a stub, because
/// Ply's codegen has no way to build an arbitrary value of what it returns.
fn unstubbable_callee_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    unstubbable: &[(String, String)],
) -> Diagnostic {
    let named: Vec<String> = unstubbable
        .iter()
        .map(|(p, w)| format!("`{p}` (called at {w})"))
        .collect();
    Diagnostic {
        code: "W0512".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` calls {list}, which ply.yaml declares a contract for -- but that callee \
             returns nothing (`-> ()`), so there is no returned value for the declared `ensures` to \
             constrain and nothing for Ply to stand in for it with. The assumption cannot be \
             encoded, so this check earned no evidence: the verdict is `unclaimed`, never \
             `{check_label}`. (W0512, §5.5)",
            list = named.join(", ")
        ),
        primary_span: None,
        counterexample: None,
        fixes: vec![Fix {
            title: format!(
                "swap `{check_label}` for `fuzz(256)` on `{fn_name}` -- the fuzz tier runs the real \
                 callee rather than standing in for it"
            ),
            edits: vec![],
        }],
        assumptions: vec![],
        open_item: Some("unclaimed_callee".into()),
    }
}

/// Union of every status on a set of child nodes, sorted and deduplicated
/// (D6: statuses propagate upward as flags, not as evidence).
fn union_statuses(children: &[Node]) -> Vec<String> {
    let mut out: Vec<String> = children
        .iter()
        .flat_map(|c| c.statuses.iter().cloned())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// §5.4 says `ply.yaml` `requires`/`ensures` "are ANDed in" to a fn's own
/// contract. That merge is not implemented (it needs the contract to reach
/// harness codegen as an expression, not a string). What *is* implemented is
/// the boundary use (§5.5): callers assume it. Saying which of the two a
/// user is getting beats letting them assume the other -- the silent drop
/// this replaces is vetting 004 finding 7.
fn declared_contract_not_anded_diag(node_id: &str, fn_name: &str) -> Diagnostic {
    Diagnostic {
        code: "W0510".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title: format!(
            "the `requires:`/`ensures:` declared for `{fn_name}` in ply.yaml is used where §5.5 \
             needs it -- callers of `{fn_name}` may assume it at a boundary -- but it is **not** \
             yet ANDed into `{fn_name}`'s own checks, which §5.4 says it should be. So this run \
             checked `{fn_name}` against its inline `#[ply::requires]`/`#[ply::ensures]` only. \
             (W0510)"
        ),
        primary_span: None,
        counterexample: None,
        fixes: vec![Fix {
            title: format!(
                "move the clause onto `{fn_name}` itself as `#[ply::requires(..)]`/\
                 `#[ply::ensures(..)]` if you want this run to check it -- inline attributes are \
                 the canonical contract source (D2)"
            ),
            edits: vec![],
        }],
        assumptions: vec![],
        open_item: Some("declared_contract_not_merged".into()),
    }
}

fn rank(v: &str) -> i32 {
    if v == "violation" {
        0
    } else if v.starts_with("tool_error") {
        1
    } else if v == "timeout" {
        2
    } else if v.starts_with("unsupported") {
        3
    } else if v == "unclaimed" {
        4
    } else if v == "tested" {
        5
    } else if v.starts_with("fuzzed") {
        6
    } else if v.starts_with("bounded") {
        7
    } else if v == "proved" {
        8
    } else {
        4
    }
}

/// Worst-of over the evidence order (D6) for aggregating *across* fns and
/// components -- a weak leaf must drag its parent down, so this always
/// takes the minimum rank among children.
fn worst_of(children: &[Node]) -> String {
    children
        .iter()
        .min_by_key(|n| rank(&n.verdict))
        .map(|n| n.verdict.clone())
        .unwrap_or_else(|| "unclaimed".into())
}

/// Combines the results of *one fn's own* checks list (§5.4c: "a function's
/// verdict is the strongest evidence its passing checks earned; a failing
/// check is a violation regardless of what else passed") -- the opposite
/// direction from `worst_of`: when nothing failed, this takes the
/// *strongest* passing verdict, not the weakest.
fn combine_fn_check_verdicts(labels: &[String]) -> String {
    let worst = labels
        .iter()
        .filter(|l| rank(l) <= 4)
        .min_by_key(|l| rank(l));
    if let Some(w) = worst {
        return w.clone();
    }
    labels
        .iter()
        .max_by_key(|l| rank(l.as_str()))
        .cloned()
        .unwrap_or_else(|| "unclaimed".into())
}

#[allow(clippy::too_many_arguments)]
fn run_fn_checks(
    node_id: &str,
    src_dir: &Path,
    lib_path: &Path,
    crate_dir: &Path,
    fn_name: &str,
    cf: &ContractFn,
    checks: &[Check],
    boundary: &BoundaryPlan,
    seed: &[u8; 32],
    harness_info: Option<&(String, String)>,
    opts: &VerifyOptions,
) -> Result<(Node, Vec<Diagnostic>)> {
    let mut diagnostics = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut statuses: Vec<String> = Vec::new();

    for check in checks {
        match check {
            Check::Bounded(k) => {
                let (label, mut s, mut d) = run_bounded_check(
                    cf, src_dir, lib_path, crate_dir, node_id, fn_name, *k, boundary, opts,
                )?;
                labels.push(label);
                statuses.append(&mut s);
                diagnostics.append(&mut d);
            }
            Check::Prove => {
                // M7, not yet implemented -- D9: a missing engine downgrades
                // the check, never fails the run.
                diagnostics.push(Diagnostic {
                    code: "W0110".into(),
                    severity: "warning".into(),
                    phase: "verify".into(),
                    engine: "verus".into(),
                    check: "prove".into(),
                    node_id: node_id.into(),
                    title: format!(
                        "`{fn_name}` declares `prove`, but Ply's Verus adapter does not exist yet (M7) -- \
                         this is reported as a missing engine, never as a failure of the check itself."
                    ),
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![
                        Fix {
                            title: format!(
                                "drop `prove` from `{fn_name}`'s checks list until M7 lands the Verus \
                                 adapter, so the run reports only checks Ply can actually perform"
                            ),
                            edits: vec![],
                        },
                        Fix {
                            title: format!(
                                "use `bounded(k)` for `{fn_name}` meanwhile -- exhaustive up to k, which is \
                                 the strongest evidence Ply can earn today"
                            ),
                            edits: vec![],
                        },
                    ],
                    assumptions: vec![],
                    open_item: Some("engine_missing".into()),
                });
                labels.push("engine-missing".into());
            }
            Check::Mutate => {} // handled after the loop, once the base verdict is known
            _ => {}
        }
    }

    // Fuzz/test share one harness-crate test run per fn (both check kinds'
    // generated tests live in the same `{fn}_harness` module).
    let wants_fuzz = checks.iter().find_map(|c| {
        if let Check::Fuzz(n) = c {
            Some(*n)
        } else {
            None
        }
    });
    let wants_test = checks.iter().any(|c| matches!(c, Check::Test));
    if wants_fuzz.is_some() || wants_test {
        if !cf.is_fuzz_supported() {
            diagnostics.push(unsupported_shape_diag(node_id, fn_name, cf));
            labels.push("unsupported".into());
        } else if cf.ensures.is_none() && wants_fuzz.is_some() {
            diagnostics.push(Diagnostic {
                code: "V0505".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: format!("fuzz({})", wants_fuzz.unwrap()),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` declares `fuzz` but has no `#[ply::ensures]` -- there is no \
                     postcondition to check against, so nothing was run. Add an `#[ply::ensures]` \
                     clause naming what `{fn_name}` promises about its result."
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![
                    Fix {
                        title: format!(
                            "add `#[ply::ensures(|result| ...)]` to `{fn_name}` -- one line naming what \
                             its return value always satisfies"
                        ),
                        edits: vec![],
                    },
                    Fix {
                        title: format!(
                            "or drop `fuzz` from `{fn_name}`'s checks and declare `test` with `examples:` \
                             instead, which needs no postcondition"
                        ),
                        edits: vec![],
                    },
                ],
                assumptions: vec![],
                open_item: Some("no_contract_to_check".into()),
            });
            labels.push("unsupported".into());
        } else if let Some((harness_pkg, _)) = harness_info {
            let (fuzz_label, test_label, mut d) = run_fuzz_and_test_checks(
                cf,
                src_dir,
                lib_path,
                crate_dir,
                harness_pkg,
                node_id,
                fn_name,
                wants_fuzz,
                wants_test,
                seed,
                opts,
            )?;
            diagnostics.append(&mut d);
            if let Some(l) = fuzz_label {
                labels.push(l);
            }
            if let Some(l) = test_label {
                labels.push(l);
            }
        }
    }

    let mut verdict = combine_fn_check_verdicts(&labels);

    // `mutate` runs last, and only against a genuinely passing base verdict
    // (D12/§5.4c): mutation-testing an already-failing check has no
    // baseline to mutate from, and cargo-mutants itself refuses to proceed
    // past a failing baseline.
    if checks.iter().any(|c| matches!(c, Check::Mutate)) {
        if rank(&verdict) > 4 {
            if let Some((harness_pkg, _)) = harness_info {
                let (outcome, mut d) =
                    run_mutate_check(crate_dir, harness_pkg, node_id, fn_name, checks, opts)?;
                diagnostics.append(&mut d);
                apply_mutate_outcome(&mut verdict, &mut statuses, outcome);
            }
        } else {
            diagnostics.push(Diagnostic {
                code: "W0110".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "cargo-mutants".into(),
                check: "mutate".into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}`'s `mutate` check was skipped: mutation testing plants deliberate bugs \
                     and asks whether the tests catch them, which only means anything once the tests \
                     pass on the real code -- and `{fn_name}`'s own `test`/`fuzz` check did not. Fix that \
                     check first; this run says nothing either way about spec strength."
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![
                    Fix {
                        title: format!(
                            "fix whatever made `{fn_name}`'s `test`/`fuzz` check fail (its own diagnostic is \
                             in this same report), then re-run -- `mutate` will run on the next pass"
                        ),
                        edits: vec![],
                    },
                ],
                assumptions: vec![],
                open_item: Some("mutate_skipped_no_baseline".into()),
            });
        }
    }

    // The fuzz tier's verdict names the run that produced it (§1): seed
    // plus the case count actually asked for. Without it, `fuzzed(256)`
    // describes a run nobody can repeat, and the run that missed a bug is
    // indistinguishable from one that could not have found it.
    let evidence = wants_fuzz.map(|n| Evidence {
        engine: "proptest".into(),
        seed: Some(ply_core::fuzz_gen::seed_hex(seed)),
        cases: Some(n),
    });

    Ok((
        Node {
            id: fn_name.to_string(),
            kind: "fn".into(),
            verdict,
            statuses,
            evidence,
            children: vec![],
        },
        diagnostics,
    ))
}

fn unresolved_anchor_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    err: &str,
) -> Diagnostic {
    Diagnostic {
        code: "E0301".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!("Ply could not find the function `{fn_name}` this claim anchors to. {err}"),
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("unresolvable_anchor".into()),
    }
}

fn unsupported_shape_diag(node_id: &str, fn_name: &str, cf: &ContractFn) -> Diagnostic {
    let bad: Vec<String> = cf
        .params
        .iter()
        .filter(|p| !p.ty.is_fuzz_supported())
        .map(|p| format!("{}: {:?}", p.name, p.ty))
        .collect();
    let (title, fixes) = if bad.is_empty() {
        (
            format!(
                "Ply cannot check `{fn_name}`: none of its declared checks apply to this \
                 function's shape. This is reported as unsupported, not attempted."
            ),
            vec![],
        )
    } else {
        (
            format!(
                "Ply cannot check `{fn_name}`: parameter(s) {} use a type neither the bounded \
                 (Kani) nor the fuzz (proptest) codegen builds inputs for. This is reported as \
                 unsupported, not attempted -- it never silently hangs.",
                bad.join(", ")
            ),
            vec![Fix {
                title: format!(
                    "add a `pure`-marked generator hook for `{fn_name}`'s parameter type (§5.4b)"
                ),
                edits: vec![],
            }],
        )
    };
    Diagnostic {
        code: "V0505".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title,
        primary_span: None,
        counterexample: None,
        fixes,
        assumptions: vec![],
        open_item: Some("unsupported_signature".into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_bounded_check(
    cf: &ContractFn,
    src_dir: &Path,
    lib_path: &Path,
    crate_dir: &Path,
    node_id: &str,
    fn_name: &str,
    bound_k: u32,
    boundary: &BoundaryPlan,
    opts: &VerifyOptions,
) -> Result<(String, Vec<String>, Vec<Diagnostic>)> {
    let check_label = format!("bounded({bound_k})");

    if !cf.is_bounded_supported() {
        return Ok((
            "unsupported".into(),
            vec![],
            vec![unsupported_shape_diag(node_id, fn_name, cf)],
        ));
    }

    // §5.5's third branch, decided before any engine starts: a callee no
    // contract describes is not descended into, and the refusal costs
    // milliseconds rather than the whole budget.
    if !boundary.unclaimed.is_empty() {
        return Ok((
            "unclaimed".into(),
            vec![],
            vec![unclaimed_callee_diag(
                node_id,
                fn_name,
                &check_label,
                &boundary.unclaimed,
            )],
        ));
    }
    if !boundary.opaque.is_empty() {
        return Ok((
            "unclaimed".into(),
            vec![],
            vec![unreadable_callee_diag(
                node_id,
                fn_name,
                &check_label,
                &boundary.opaque,
            )],
        ));
    }
    if !boundary.unstubbable.is_empty() {
        return Ok((
            "unclaimed".into(),
            vec![],
            vec![unstubbable_callee_diag(
                node_id,
                fn_name,
                &check_label,
                &boundary.unstubbable,
            )],
        ));
    }

    let generated = harness::generate_proof_module(cf, bound_k, &boundary.stubs)?;
    harness::write_generated_module(src_dir, lib_path, &generated.module_source)?;

    let engine_timeout_secs = opts
        .engine_timeout_secs
        .unwrap_or_else(|| default_engine_timeout_secs(cf.has_vec_param(), bound_k));

    let run_cfg = KaniRunConfig {
        crate_dir: crate_dir.to_path_buf(),
        harness_path: generated.proof_fn_path.clone(),
        engine_timeout_secs,
        enable_stubbing: !generated.stubbed.is_empty(),
    };
    let outcome = kani::run(&run_cfg)?;

    // §9's cex validity oracle demands the SAME rendered test transitions
    // FAIL -> PASS once a fix lands (see docs/m3-slice-findings.md finding
    // 6): persist any witness found, and re-render its regression test
    // against the CURRENT contract text on every run.
    let witness_path = crate_dir
        .join("target/ply/witness")
        .join(format!("{fn_name}.json"));
    if let KaniOutcome::Violation { witness_bytes, .. } = &outcome {
        if let Some(parent) = witness_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&witness_path, serde_json::to_string(witness_bytes)?)?;
    }
    if witness_path.exists() {
        let stored: Vec<Vec<u8>> = serde_json::from_str(&std::fs::read_to_string(&witness_path)?)?;
        // A parameter shape with no witness decoder yet (`char`,
        // `Option`, `Result`, `[T; N]` -- all reachable by the engines
        // since 2026-08-25, none of them spellable as a `WitnessValue`)
        // leaves nothing to re-render. That is a missing renderer, never a
        // reason to fail the whole run.
        if let Ok(values) = kani::decode_witness(&stored, &cf.params, bound_k) {
            let rendered = contract_rt::render_cex_test(cf, &values, &check_label, "K0502", 1)?;
            let module_source = contract_rt::wrap_test_module(&[RenderedTest {
                test_name: rendered.test_name,
                source: rendered.source,
            }]);
            harness::write_generated_test(src_dir, lib_path, &module_source)?;
        }
    }

    match outcome {
        KaniOutcome::Verified => {
            if generated.stubbed.is_empty() {
                Ok((check_label, vec![], vec![]))
            } else {
                // §5.5's second branch: real evidence, resting on a
                // declared assumption. `conditional` is a status (D6), not
                // a weaker rung -- the verdict stays `bounded(k)` and the
                // assumption travels beside it.
                let d =
                    conditional_verdict_diag(node_id, fn_name, &check_label, &generated.stubbed);
                Ok((
                    check_label,
                    vec!["conditional".into(), "owed-evidence".into()],
                    vec![d],
                ))
            }
        }
        KaniOutcome::Timeout { raw_output } => {
            let _ = raw_output;
            let d = Diagnostic {
                code: "K0601".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "kani".into(),
                check: check_label,
                node_id: node_id.into(),
                title: format!(
                    "Kani could not finish checking `{fn_name}` within its {engine_timeout_secs}s time \
                     budget -- this is an exhausted search, not a broken promise: Kani never got far \
                     enough to say whether the contract holds or not, so this is reported as `timeout`, \
                     never as a violation. (K0601)"
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![
                    Fix {
                        title: format!(
                            "raise --engine-timeout past {engine_timeout_secs}s (Kani's CBMC solve time \
                             varies run to run; docs/m3-slice-findings.md measured ~1s-107s on an \
                             identical harness)"
                        ),
                        edits: vec![],
                    },
                    Fix {
                        title: format!("lower `bounded({bound_k})` to a smaller bound"),
                        edits: vec![],
                    },
                    Fix {
                        title: format!(
                            "switch `{fn_name}` to `fuzz(256)` -- proptest has no unwind-bound cost"
                        ),
                        edits: vec![],
                    },
                ],
                assumptions: vec![],
                open_item: Some("timeout".into()),
            };
            Ok(("timeout".into(), vec![], vec![d]))
        }
        KaniOutcome::ToolError { reason, raw_output } => {
            let _ = raw_output;
            let d = Diagnostic {
                code: "X0901".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "kani".into(),
                check: check_label,
                node_id: node_id.into(),
                title: format!("Ply's Kani adapter could not interpret Kani's output: {reason}"),
                primary_span: None,
                counterexample: None,
                fixes: vec![],
                assumptions: vec![],
                open_item: Some("tool_error".into()),
            };
            Ok(("tool_error".into(), vec![], vec![d]))
        }
        KaniOutcome::Violation {
            witness_bytes,
            raw_output,
        } => {
            let _ = raw_output;
            let values = match kani::decode_witness(&witness_bytes, &cf.params, bound_k) {
                Ok(values) => values,
                Err(e) => {
                    // §5.4c's MUST: never a `violation` Ply cannot show the
                    // input for. Kani really did falsify the claim, but the
                    // witness is in a shape this decoder cannot read yet, so
                    // the honest report is a tool error naming the shape.
                    let unreadable: Vec<String> = cf
                        .params
                        .iter()
                        .filter(|p| p.ty.scalar_byte_width().is_none())
                        .map(|p| format!("`{}: {}`", p.name, p.ty.rust_name().unwrap_or_default()))
                        .collect();
                    let d = Diagnostic {
                        code: "X0901".into(),
                        severity: "error".into(),
                        phase: "verify".into(),
                        engine: "kani".into(),
                        check: check_label,
                        node_id: node_id.into(),
                        title: format!(
                            "Kani found an input for which `{fn_name}` breaks its contract, but Ply \
                             cannot yet read that input back for parameter(s) {list}: it has no \
                             decoder for how Kani encodes those types. So there is a real failure \
                             here and no counterexample to show you, which is reported as a tool \
                             error rather than as a violation Ply cannot evidence ({e}). (X0901)",
                            list = unreadable.join(", ")
                        ),
                        primary_span: None,
                        counterexample: None,
                        fixes: vec![Fix {
                            title: format!(
                                "re-run `{fn_name}` under `fuzz(256)` -- the fuzz tier prints its own \
                                 failing input, so it can show you the case Kani found"
                            ),
                            edits: vec![],
                        }],
                        assumptions: vec![],
                        open_item: Some("tool_error".into()),
                    };
                    return Ok(("tool_error".into(), vec![], vec![d]));
                }
            };
            let rendered = contract_rt::render_cex_test(cf, &values, &check_label, "K0502", 1)?;
            let test_file = harness::write_generated_test(
                src_dir,
                lib_path,
                &contract_rt::wrap_test_module(&[RenderedTest {
                    test_name: rendered.test_name.clone(),
                    source: rendered.source.clone(),
                }]),
            )?;
            let mut inputs = BTreeMap::new();
            for (p, v) in cf.params.iter().zip(values.iter()) {
                inputs.insert(p.name.clone(), format_value(v));
            }
            let contract_text = cf
                .ensures
                .as_ref()
                .map(|(_, t)| t.clone())
                .unwrap_or_default();
            let d = Diagnostic {
                code: "K0502".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "kani".into(),
                check: check_label,
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` breaks its own postcondition `{contract_text}` for at least one input \
                     -- a postcondition is the guarantee a function makes about its return value, and \
                     Kani found a case where that guarantee does not hold. (K0502)"
                ),
                primary_span: None,
                counterexample: Some(Counterexample {
                    inputs,
                    kani_witness: Some(format!(
                        "captured from `cargo kani --concrete-playback print` on harness `{}`",
                        generated.proof_fn_path
                    )),
                    cargo_test: Some(
                        test_file
                            .strip_prefix(src_dir.parent().unwrap_or(src_dir))
                            .unwrap_or(&test_file)
                            .display()
                            .to_string(),
                    ),
                }),
                fixes: vec![],
                assumptions: vec![],
                open_item: None,
            };
            Ok(("violation".into(), vec![], vec![d]))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_fuzz_and_test_checks(
    cf: &ContractFn,
    src_dir: &Path,
    lib_path: &Path,
    crate_dir: &Path,
    harness_pkg: &str,
    node_id: &str,
    fn_name: &str,
    wants_fuzz: Option<u32>,
    wants_test: bool,
    seed: &[u8; 32],
    opts: &VerifyOptions,
) -> Result<(Option<String>, Option<String>, Vec<Diagnostic>)> {
    let timeout = opts
        .engine_timeout_secs
        .unwrap_or_else(default_secondary_engine_timeout_secs);
    let filter = format!("{fn_name}_harness::");
    let run = fuzz_engine::run_harness_tests(crate_dir, harness_pkg, &filter, timeout)?;

    let mut diagnostics = Vec::new();
    let fuzz_test_name = format!("{fn_name}_harness::ply_fuzz_{fn_name}");
    let mut fuzz_label = None;
    let mut test_label = None;

    // The harness never ran at all (2026-08-24 M4 review, D1 -- the review's
    // most serious finding, and this file's own fail-open bug one level up
    // from the parser one docs/m4-findings.md finding 3 records). A run that
    // did not succeed, did not time out, and named no failing test executed
    // zero cases: the commonest cause is a harness crate that failed to
    // *compile*, which produces no libtest `failures:` block at all. Both
    // the `fuzz` and the `test` check live in that one crate, so neither
    // ran. §8: adapters never pass engine output through raw -- they parse
    // it or fail with `X0901` attaching what the engine said. It is
    // emphatically not a pass (there is no evidence) and not a violation
    // (§5.4c MUST: no violation without a witness).
    if !run.success && !run.timed_out && run.failed_tests.is_empty() {
        let cause = fuzz_engine::first_build_error(&run.combined_output);
        if let Some(n) = wants_fuzz {
            diagnostics.push(harness_did_not_run_diag(
                node_id,
                fn_name,
                &format!("fuzz({n})"),
                harness_pkg,
                cause.as_deref(),
            ));
        }
        if wants_test {
            diagnostics.push(harness_did_not_run_diag(
                node_id,
                fn_name,
                "test",
                harness_pkg,
                cause.as_deref(),
            ));
        }
        return Ok((
            wants_fuzz.map(|_| "tool_error".to_string()),
            if wants_test {
                Some("tool_error".to_string())
            } else {
                None
            },
            diagnostics,
        ));
    }

    if let Some(n) = wants_fuzz {
        let check_label = format!("fuzz({n})");
        if run.timed_out {
            diagnostics.push(Diagnostic {
                code: "P0601".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: check_label,
                node_id: node_id.into(),
                title: format!(
                    "proptest could not finish {n} cases for `{fn_name}` within its {timeout}s time \
                     budget -- reported as `timeout`, never as a violation."
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![Fix { title: "lower fuzz(n)'s case count, or raise --engine-timeout".into(), edits: vec![] }],
                assumptions: vec![],
                open_item: Some("timeout".into()),
            });
            fuzz_label = Some("timeout".into());
        } else if run.failed_tests.iter().any(|t| t == &fuzz_test_name) {
            // The label comes from what the renderer could actually
            // establish (2026-08-24 M4 review, D6): when no witness could be
            // recovered, the honest label is `tool_error` -- §5.4c MUST NOT
            // emit a `violation` without a witness, and pushing it
            // unconditionally here did exactly that.
            let (label, d) = render_fuzz_violation(
                cf,
                &run.combined_output,
                node_id,
                fn_name,
                &check_label,
                harness_pkg,
                &ply_core::fuzz_gen::seed_hex(seed),
                src_dir,
                lib_path,
            )?;
            diagnostics.push(d);
            fuzz_label = Some(label);
        } else if let Some(abort) = fuzz_engine::parse_abort_marker(&run.combined_output) {
            // proptest abandoned the run (2026-08-24 M4 review, D4): its own
            // global-reject limit fired, so approximately no case was ever
            // checked. The verdict must not read `fuzzed(n)` -- a warning
            // beside a number that never happened still reports n cases of
            // evidence. There is nothing to claim here, so nothing is
            // claimed.
            let total = abort.accepted + abort.rejected;
            let reason = &abort.reason;
            let (accepted, rejected) = (abort.accepted, abort.rejected);
            diagnostics.push(Diagnostic {
                code: "W0503".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: check_label.clone(),
                node_id: node_id.into(),
                title: format!(
                    "proptest gave up on `{fn_name}` before it could run the {n} cases `fuzz({n})` \
                     asked for: {rejected} of the {total} inputs it generated were thrown away by the \
                     function's own `#[ply::requires]` precondition and only {accepted} were ever \
                     checked, which tripped proptest's own limit ({reason}). So this function has no \
                     fuzz evidence at all -- its verdict is `unclaimed`, not `fuzzed({n})`. (W0503)"
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![
                    Fix {
                        title: format!(
                            "widen `{fn_name}`'s `#[ply::requires]`, or give its parameters a type whose \
                             values satisfy it by construction -- proptest can only check inputs its \
                             generator actually produces"
                        ),
                        edits: vec![],
                    },
                    Fix {
                        title: format!(
                            "swap `fuzz({n})` for `test` plus `examples:` entries naming concrete inputs \
                             inside `{fn_name}`'s allowed domain"
                        ),
                        edits: vec![],
                    },
                ],
                assumptions: vec![],
                open_item: Some("no_cases_ran".into()),
            });
            fuzz_label = Some("unclaimed".into());
        } else {
            if let Some((_, detail)) = fuzz_engine::parse_high_reject_marker(&run.combined_output) {
                diagnostics.push(Diagnostic {
                    code: "W0503".into(),
                    severity: "warning".into(),
                    phase: "verify".into(),
                    engine: "proptest".into(),
                    check: check_label.clone(),
                    node_id: node_id.into(),
                    title: format!(
                        "most of the inputs generated for `{fn_name}` were thrown away by its own \
                         `#[ply::requires]` precondition ({detail} draws rejected). proptest kept \
                         drawing until it had {n} accepted cases, so the count is honest -- but those \
                         cases all come from the narrow corner of the input space the precondition \
                         allows, which is weaker evidence than {n} on its own suggests. (W0503)"
                    ),
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![
                        Fix {
                            title: format!(
                                "give `{fn_name}` a parameter type whose values are valid by \
                                 construction, so the generator hits the allowed domain directly \
                                 instead of being filtered into it"
                            ),
                            edits: vec![],
                        },
                        Fix {
                            title: format!("relax `{fn_name}`'s `#[ply::requires]` if it is stricter than the function really needs"),
                            edits: vec![],
                        },
                    ],
                    assumptions: vec![],
                    open_item: Some("high_rejection_rate".into()),
                });
            }
            // The earned verdict is `fuzzed(n)` (past tense, §5.4c's own
            // check->verdict table), never the declared check spelling
            // `fuzz(n)` -- those two strings look alike enough that this
            // was wrong here once already; the difference matters because
            // `rank()`/`combine_fn_check_verdicts` key off the `fuzzed`
            // prefix.
            fuzz_label = Some(format!("fuzzed({n})"));
        }
    }

    if wants_test {
        let failing_test_checks: Vec<&String> = run
            .failed_tests
            .iter()
            .filter(|t| t.contains("::ply_example_") || t.contains("::ply_direct_"))
            .collect();
        if run.timed_out {
            diagnostics.push(Diagnostic {
                code: "R0601".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "ply".into(),
                check: "test".into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}`'s `examples` and generated boundary-case tests did not finish within \
                     their {timeout}s budget, so the `test` check has no result -- reported as \
                     `timeout`, never as a violation."
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![
                    Fix { title: format!("raise --engine-timeout above {timeout}s"), edits: vec![] },
                    Fix {
                        title: format!(
                            "check whether `{fn_name}` can loop forever on one of its `examples` inputs -- \
                             these tests call the real function directly"
                        ),
                        edits: vec![],
                    },
                ],
                assumptions: vec![],
                open_item: Some("timeout".into()),
            });
            test_label = Some("timeout".into());
        } else if !failing_test_checks.is_empty() {
            diagnostics.push(Diagnostic {
                code: "R0502".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "ply".into(),
                check: "test".into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` failed {} of its own example/generated direct-contract test(s): {}. \
                     Each of these is a concrete input asserted directly against the contract, so this \
                     is a real, reproduced violation, not a probabilistic one.",
                    failing_test_checks.len(),
                    failing_test_checks.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![],
                assumptions: vec![],
                open_item: None,
            });
            test_label = Some("violation".into());
        } else {
            test_label = Some("tested".into());
        }
    }

    Ok((fuzz_label, test_label, diagnostics))
}

/// The `X0901` a check earns when its generated harness never ran a single
/// case (2026-08-24 M4 review, D1). Written to the newbie bar: what
/// happened, what it means for the verdict, what most likely caused it, and
/// the compiler's own words -- then concrete `fixes`, per §8's non-result
/// rule ("a non-result is still feedback").
fn harness_did_not_run_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    harness_pkg: &str,
    cause: Option<&str>,
) -> Diagnostic {
    let compiler_says = match cause {
        Some(c) => format!(" The compiler's own first error was: {c}."),
        None => String::new(),
    };
    Diagnostic {
        code: "X0901".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}`'s `{check_label}` check ran zero cases: the test harness Ply generates for it \
             failed to compile, so nothing was checked at all. This is reported as a tool error -- \
             never as a pass, because no evidence was gathered, and never as a violation, because \
             there is no failing input to show.{compiler_says} The usual cause is an `examples:` \
             entry in ply.yaml that does not type-check against `{fn_name}`'s real signature: Ply \
             compiles those entries exactly as written (they are ordinary Rust `==` expressions), so \
             a wrong type or a typo first shows up here. (X0901)"
        ),
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "check every `examples:` entry for `{fn_name}` in ply.yaml -- each one must compile \
                     as a Rust expression against `{fn_name}`'s real parameter and return types"
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "see the full compiler output by running `cargo test -p {harness_pkg} --lib \
                     {fn_name}_harness::` from the crate root (Ply regenerates that harness crate on \
                     every run, so editing it is never the fix)"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("tool_error".into()),
    }
}

/// Renders a fuzz-found failure into (verdict label, diagnostic). The label
/// is part of the return value on purpose: only the branch that recovered a
/// real failing input may say `violation` (§5.4c).
#[allow(clippy::too_many_arguments)]
fn render_fuzz_violation(
    cf: &ContractFn,
    combined_output: &str,
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    harness_pkg: &str,
    seed_hex: &str,
    src_dir: &Path,
    lib_path: &Path,
) -> Result<(String, Diagnostic)> {
    let contract_text = cf
        .ensures
        .as_ref()
        .map(|(_, t)| t.clone())
        .unwrap_or_default();
    // Two places a failing input can come from, and Ply reads both before
    // giving up (2026-08-25). Ply's own `PLY_FUZZED_CEX` marker prints only
    // from the postcondition arm, so a body that *panics* never reaches it
    // -- but proptest catches that panic, shrinks it, and prints the minimal
    // input in its own report, which this adapter used to discard. Reading
    // only the first meant a genuine crash bug could never be reported as a
    // `violation` at any seed: the two available answers were "all green"
    // and "Ply's harness had a problem".
    let from_panic = fuzz_engine::parse_fuzz_marker(combined_output).is_none();
    let recovered = match fuzz_engine::parse_fuzz_marker(combined_output) {
        Some((_, fields)) => Some(fields),
        None => fuzz_engine::parse_proptest_minimal_input(combined_output)
            .filter(|values| values.len() == cf.params.len())
            .map(|values| {
                cf.params
                    .iter()
                    .map(|p| p.name.clone())
                    .zip(values)
                    .collect::<BTreeMap<String, String>>()
            }),
    };
    let Some(fields) = recovered else {
        return Ok((
            "tool_error".to_string(),
            Diagnostic {
                code: "X0901".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: check_label.into(),
                node_id: node_id.into(),
                title: format!(
                    "proptest reported a failing case for `{fn_name}`, but Ply could not recover the \
                     failing input from the run -- neither from the line its own generated harness \
                     prints, nor from proptest's own `minimal failing input:` report -- so there is no \
                     counterexample to show you. This is reported as a tool error, not as a violation: \
                     Ply never reports a broken promise it cannot show you the input for. (X0901)"
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![
                    Fix {
                        title: format!(
                            "run the harness yourself -- `cargo test -p {harness_pkg} --lib \
                             {fn_name}_harness::` from the crate root -- and read proptest's own report \
                             of the failing input"
                        ),
                        edits: vec![],
                    },
                    Fix {
                        title: format!(
                            "if `{fn_name}` is meant to panic on some inputs, add a `#[ply::requires]` \
                             that rules them out, so those inputs are never generated"
                        ),
                        edits: vec![],
                    },
                ],
                assumptions: vec![],
                open_item: Some("tool_error".into()),
            },
        ));
    };

    match fuzz_engine::decode_marker_fields(&fields, &cf.params) {
        Some(values) => {
            let rendered = contract_rt::render_cex_test(cf, &values, check_label, "P0502", 1)?;
            let test_file = harness::write_generated_test(
                src_dir,
                lib_path,
                &contract_rt::wrap_test_module(&[RenderedTest {
                    test_name: rendered.test_name.clone(),
                    source: rendered.source.clone(),
                }]),
            )?;
            let mut inputs = BTreeMap::new();
            for p in &cf.params {
                // Look fields up by name: `fields` is a BTreeMap (sorted by
                // key), so zipping params against `fields.values()` mislabels
                // every fn whose parameter order is not alphabetical -- the
                // rendered cex test was right but the JSON `inputs` map
                // carried swapped values (found in the 2026-08-24 M4 review
                // with a (z, a)-ordered probe fn).
                if let Some(raw) = fields.get(&p.name) {
                    inputs.insert(p.name.clone(), raw.clone());
                }
            }
            Ok((
                "violation".to_string(),
                Diagnostic {
                    code: "P0502".into(),
                    severity: "error".into(),
                    phase: "verify".into(),
                    engine: "proptest".into(),
                    check: check_label.into(),
                    node_id: node_id.into(),
                    title: if from_panic {
                        format!(
                            "`{fn_name}` does not return at all for this input -- it panicked before its \
                             postcondition `{contract_text}` could even be evaluated. proptest shrank the \
                             failing case to the smallest input that still crashes, and it is below. A \
                             function that panics inside its own declared precondition has broken its \
                             promise as surely as one that returns a wrong answer, so this is a \
                             violation, with a witness. (P0502)"
                        )
                    } else {
                        format!(
                            "`{fn_name}` breaks its own postcondition `{contract_text}` for at least one input -- \
                     proptest shrank a failing case to this minimal example. (P0502)"
                        )
                    },
                    primary_span: None,
                    counterexample: Some(Counterexample {
                        inputs,
                        kani_witness: Some(format!(
                            "captured from proptest shrinking on harness `{fn_name}_harness::ply_fuzz_{fn_name}`, \
                         replayable with `--seed {seed_hex}` (field named `kani_witness` for §8 schema \
                         stability; this witness is proptest-, not Kani-, sourced -- see \
                         docs/m4-findings.md)"
                        )),
                        cargo_test: Some(
                            test_file
                                .strip_prefix(src_dir.parent().unwrap_or(src_dir))
                                .unwrap_or(&test_file)
                                .display()
                                .to_string(),
                        ),
                    }),
                    fixes: vec![],
                    assumptions: vec![],
                    open_item: None,
                },
            ))
        }
        None => {
            let mut inputs = BTreeMap::new();
            for p in &cf.params {
                // By-name lookup for the same reason as the P0502 branch
                // above: never zip params against a sorted map's values.
                if let Some(raw) = fields.get(&p.name) {
                    inputs.insert(p.name.clone(), raw.clone());
                }
            }
            Ok((
                "violation".to_string(),
                Diagnostic {
                    code: "W0541".into(),
                    severity: "error".into(),
                    phase: "verify".into(),
                    engine: "proptest".into(),
                    check: check_label.into(),
                    node_id: node_id.into(),
                    title: format!(
                        "`{fn_name}` fails its own contract `{contract_text}` for at least one input, \
                     and proptest shrank that input down to the smallest one that still fails. Ply \
                     cannot turn it into a runnable Rust test, though: it has no way yet to spell a \
                     `BTreeSet`, or a `Vec` of anything but `u8`, as a literal value. The failing input \
                     is recorded below exactly as the engine reported it -- Ply never invents one. \
                     (W0541, reason: inputs_unrenderable)"
                    ),
                    primary_span: None,
                    counterexample: Some(Counterexample {
                        inputs,
                        kani_witness: None,
                        cargo_test: None,
                    }),
                    fixes: vec![],
                    assumptions: vec![],
                    open_item: Some("inputs_unrenderable".into()),
                },
            ))
        }
    }
}

/// The whole-run wall-clock cap for one `mutate` invocation (2026-08-24 M4
/// review, D5). `--engine-timeout` caps each *mutant's* test phase
/// (cargo-mutants' own `-t`), but a run is a tree copy plus an unmutated
/// baseline build plus one test run per mutant, so the run as a whole needs
/// its own cap or a hang outside the test phase hangs `verify` silently --
/// which §5.4c forbids outright. Ten times the per-mutant budget, never less
/// than 120s: M4's measured runs took 24-26s for 2-4 mutants at the 60s
/// default (docs/m4-findings.md), i.e. ~4% of the resulting cap, so this
/// cannot turn a healthy run into a spurious `timeout`.
fn mutate_wall_clock_secs(per_mutant_secs: u32) -> u32 {
    per_mutant_secs.saturating_mul(10).max(120)
}

/// What one `mutate` run actually established. Three outcomes, not two: a
/// run that never produced a mutant count (engine missing, killed by the
/// wall-clock cap, or output Ply could not read) says *nothing* about spec
/// strength, and must not be reported as if it had found surviving mutants
/// (2026-08-24 M4 review, D5's neighbourhood).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutateOutcome {
    /// Every viable mutant was caught -- the verdict earns `·spec-strong`.
    SpecStrong,
    /// Mutants survived: the spec is weaker than the code it describes.
    WeakSpec,
    /// The run produced no verdict either way.
    Inconclusive,
}

fn apply_mutate_outcome(verdict: &mut String, statuses: &mut Vec<String>, outcome: MutateOutcome) {
    match outcome {
        MutateOutcome::SpecStrong => verdict.push_str("\u{00b7}spec-strong"),
        MutateOutcome::WeakSpec => statuses.push("weak-spec".into()),
        // Nothing was established either way: the engine never reported a
        // mutant count. `inconclusive` is D6's own status for that, and it
        // is emphatically not `weak-spec`, which asserts a real finding.
        MutateOutcome::Inconclusive => statuses.push("inconclusive".into()),
    }
}

fn run_mutate_check(
    crate_dir: &Path,
    harness_pkg: &str,
    node_id: &str,
    fn_name: &str,
    checks: &[Check],
    opts: &VerifyOptions,
) -> Result<(MutateOutcome, Vec<Diagnostic>)> {
    let _ = checks;
    if !mutants::is_available() {
        return Ok((
            MutateOutcome::Inconclusive,
            vec![Diagnostic {
                code: "W0110".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "cargo-mutants".into(),
                check: "mutate".into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` declares `mutate`, but `cargo-mutants` is not installed -- run \
                     `cargo install cargo-mutants --locked` (see `cargo ply doctor`). Reported as a \
                     missing engine, never as a failure of the check itself."
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![Fix {
                    title: "cargo install cargo-mutants --locked".into(),
                    edits: vec![],
                }],
                assumptions: vec![],
                open_item: Some("engine_missing".into()),
            }],
        ));
    }

    let cargo_toml_text = std::fs::read_to_string(crate_dir.join("Cargo.toml"))?;
    let target_names = harness_crate::read_crate_names(&cargo_toml_text)?;
    let timeout = opts
        .engine_timeout_secs
        .unwrap_or_else(default_secondary_engine_timeout_secs);
    let wall_clock = mutate_wall_clock_secs(timeout);
    let cfg = MutantsRunConfig {
        workspace_root: crate_dir.to_path_buf(),
        mutated_package: target_names.package_name,
        harness_package: harness_pkg.to_string(),
        // Unanchored: cargo-mutants' `--re` matches against the whole
        // descriptive mutant name (e.g. "src/lib.rs:8:5: replace vacuous ->
        // u32 with 0"), not the bare fn name, so `^{fn}$` matches nothing --
        // confirmed against a real run (docs/m4-findings.md) and matching
        // the spike's own usage (`--re strong_target`, no anchors).
        fn_regex: fn_name.to_string(),
        test_filter: format!("{fn_name}_harness::"),
        timeout_secs: timeout,
        wall_clock_secs: wall_clock,
    };
    let outcome = mutants::run(&cfg)?;

    let check_label = "mutate".to_string();
    match outcome {
        MutantsRunOutcome::Completed(o) => {
            if o.all_caught() {
                Ok((MutateOutcome::SpecStrong, vec![]))
            } else if o.missed.is_empty() {
                // Nothing to mutate (unviable-only, or zero mutants found)
                // is not evidence of strength either way.
                Ok((
                    MutateOutcome::Inconclusive,
                    vec![Diagnostic {
                        code: "W0502".into(),
                        severity: "warning".into(),
                        phase: "verify".into(),
                        engine: "cargo-mutants".into(),
                        check: check_label,
                        node_id: node_id.into(),
                        title: format!(
                            "`{fn_name}`'s `mutate` check produced no viable mutants to test against -- \
                             this is not evidence of a strong spec, just an absence of a signal either way."
                        ),
                        primary_span: None,
                        counterexample: None,
                        fixes: vec![
                            Fix {
                                title: format!(
                                    "check that `{fn_name}` has a body cargo-mutants can alter at all (a \
                                     one-line delegation or a constant often has no viable mutant)"
                                ),
                                edits: vec![],
                            },
                            Fix {
                                title: "run `cargo mutants --list` in the crate to see what it would try".into(),
                                edits: vec![],
                            },
                        ],
                        assumptions: vec![],
                        open_item: Some("no_mutants".into()),
                    }],
                ))
            } else {
                Ok((
                    MutateOutcome::WeakSpec,
                    vec![Diagnostic {
                        code: "W0502".into(),
                        severity: "warning".into(),
                        phase: "verify".into(),
                        engine: "cargo-mutants".into(),
                        check: check_label,
                        node_id: node_id.into(),
                        title: format!(
                            "weak spec ({} surviving mutants): `{fn_name}`'s `test`/`fuzz` checks did \
                             not catch every deliberately-broken version of its own body -- caught {}, \
                             missed {}. Note: this count is an upper bound on spec weakness, not an exact \
                             one -- a survivor whose change cannot alter the function's observable output \
                             (an equivalent mutant) survives any spec, however strong, so not every entry \
                             below is necessarily a gap to close. Surviving: {}",
                            o.missed.len(),
                            o.caught,
                            o.missed.len(),
                            o.missed.join("; ")
                        ),
                        primary_span: None,
                        counterexample: None,
                        fixes: vec![Fix {
                            title: format!(
                                "strengthen `{fn_name}`'s `#[ply::ensures]` or add `examples` that pin the surviving behavior"
                            ),
                            edits: vec![],
                        }],
                        assumptions: vec![],
                        open_item: Some("weak_spec".into()),
                    }],
                ))
            }
        }
        MutantsRunOutcome::Timeout { raw_output } => {
            let _ = raw_output;
            Ok((
                MutateOutcome::Inconclusive,
                vec![Diagnostic {
                    code: "M0601".into(),
                    severity: "warning".into(),
                    phase: "verify".into(),
                    engine: "cargo-mutants".into(),
                    check: check_label,
                    node_id: node_id.into(),
                    title: format!(
                        "`{fn_name}`'s `mutate` run was stopped after {wall_clock}s, the cap Ply puts \
                         on the whole cargo-mutants invocation (it plants one deliberate bug at a time \
                         and re-runs the tests for each, so a run is many test runs plus a copy of the \
                         crate). Nothing is known about how many of those bugs the spec would have \
                         caught -- this is reported as an exhausted run, never as a weak spec. (M0601)"
                    ),
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![
                        Fix {
                            title: format!(
                                "raise --engine-timeout above {timeout}s (Ply caps the whole mutate run \
                                 at ten times that, currently {wall_clock}s)"
                            ),
                            edits: vec![],
                        },
                        Fix {
                            title: format!(
                                "narrow what `{fn_name}` mutates, or drop `mutate` from its checks list \
                                 while keeping `test`/`fuzz` -- mutation testing is the slowest check \
                                 Ply runs"
                            ),
                            edits: vec![],
                        },
                    ],
                    assumptions: vec![],
                    open_item: Some("timeout".into()),
                }],
            ))
        }
        MutantsRunOutcome::ToolError { raw_output, reason } => {
            let _ = raw_output;
            Ok((
                MutateOutcome::Inconclusive,
                vec![Diagnostic {
                    code: "X0901".into(),
                    severity: "error".into(),
                    phase: "verify".into(),
                    engine: "cargo-mutants".into(),
                    check: check_label,
                    node_id: node_id.into(),
                    title: format!(
                        "Ply's cargo-mutants adapter could not interpret its output: {reason}"
                    ),
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![],
                    assumptions: vec![],
                    open_item: Some("tool_error".into()),
                }],
            ))
        }
    }
}

fn format_value(v: &kani::WitnessValue) -> String {
    match v {
        kani::WitnessValue::UInt(u) => u.to_string(),
        kani::WitnessValue::Int(i) => i.to_string(),
        kani::WitnessValue::Bool(b) => b.to_string(),
        kani::WitnessValue::VecU8(bytes) => format!("{bytes:?}"),
    }
}

fn leaf_node(node_id: &str, verdict: &str) -> Node {
    let fn_part = node_id.rsplit("::").next().unwrap_or(node_id);
    Node {
        id: fn_part.to_string(),
        kind: "fn".into(),
        verdict: verdict.to_string(),
        statuses: vec![],
        evidence: None,
        children: vec![],
    }
}

#[allow(dead_code)]
fn unused(_p: &PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_timeout_scales_only_for_vec_shaped_harnesses() {
        assert_eq!(
            default_engine_timeout_secs(false, 2),
            60,
            "scalar-only stays at the M3 default"
        );
        assert_eq!(
            default_engine_timeout_secs(true, 8),
            150,
            "the M3 review measured bounded(8) over Vec<u8> needing 150s -- the formula must reproduce that exactly"
        );
        assert_eq!(default_engine_timeout_secs(true, 2), 60);
    }

    #[test]
    fn fuzz_success_label_is_past_tense_not_the_declared_check_spelling() {
        // Regression test: the earned verdict must read `fuzzed(256)`, never
        // `fuzz(256)` (the check's own declared spelling) -- `rank()` keys
        // off the `fuzzed` prefix, and this was wrong once already (see
        // docs/m4-findings.md).
        assert_eq!(rank("fuzzed(256)"), 6);
        assert_eq!(
            rank("fuzz(256)"),
            4,
            "an unrecognized label falls back to the neutral rank, not a passing one"
        );
    }

    #[test]
    fn combine_picks_the_strongest_passing_check_when_nothing_failed() {
        let labels = vec!["tested".to_string(), "fuzzed(256)".to_string()];
        assert_eq!(combine_fn_check_verdicts(&labels), "fuzzed(256)");
    }

    #[test]
    fn combine_picks_the_failure_regardless_of_what_else_passed() {
        let labels = vec!["fuzzed(256)".to_string(), "violation".to_string()];
        assert_eq!(
            combine_fn_check_verdicts(&labels),
            "violation",
            "§5.4c: a failing check is a violation regardless of what else passed"
        );
    }

    /// D6: "a timeout is not a weaker proof, it is a different kind of
    /// fact." A `mutate` run that produced no mutant count at all -- killed
    /// by its wall-clock cap, engine not installed, output unreadable -- has
    /// established nothing about the spec, and reporting it as `weak-spec`
    /// puts a finding in the tree that no engine ever made (2026-08-24 M4
    /// review, D5's own neighbourhood: `M0601` was unreachable, so this
    /// mislabel was invisible until the timeout became constructible).
    #[test]
    fn a_mutate_run_that_produced_no_result_is_not_reported_as_a_weak_spec() {
        let mut verdict = "fuzzed(256)".to_string();
        let mut statuses: Vec<String> = vec![];
        apply_mutate_outcome(&mut verdict, &mut statuses, MutateOutcome::Inconclusive);
        assert_eq!(
            verdict, "fuzzed(256)",
            "an inconclusive mutate run neither strengthens nor weakens the verdict"
        );
        assert!(
            !statuses.contains(&"weak-spec".to_string()),
            "no mutant survived, because none ever ran -- `weak-spec` here is a finding no engine made: {statuses:?}"
        );
        assert_eq!(
            statuses,
            vec!["inconclusive".to_string()],
            "D6's own status for exactly this: {statuses:?}"
        );
    }

    #[test]
    fn a_completed_mutate_run_still_reports_both_of_its_real_outcomes() {
        let mut verdict = "fuzzed(256)".to_string();
        let mut statuses: Vec<String> = vec![];
        apply_mutate_outcome(&mut verdict, &mut statuses, MutateOutcome::SpecStrong);
        assert_eq!(verdict, "fuzzed(256)\u{00b7}spec-strong");
        assert!(statuses.is_empty());

        let mut verdict = "fuzzed(64)".to_string();
        let mut statuses: Vec<String> = vec![];
        apply_mutate_outcome(&mut verdict, &mut statuses, MutateOutcome::WeakSpec);
        assert_eq!(verdict, "fuzzed(64)");
        assert_eq!(statuses, vec!["weak-spec".to_string()]);
    }

    #[test]
    fn worst_of_picks_the_weakest_child_not_the_strongest() {
        let children = vec![
            Node {
                id: "a".into(),
                kind: "fn".into(),
                verdict: "bounded(2)".into(),
                statuses: vec![],
                evidence: None,
                children: vec![],
            },
            Node {
                id: "b".into(),
                kind: "fn".into(),
                verdict: "tested".into(),
                statuses: vec![],
                evidence: None,
                children: vec![],
            },
        ];
        assert_eq!(
            worst_of(&children),
            "tested",
            "D6: a weak leaf drags its parent down"
        );
    }
}
