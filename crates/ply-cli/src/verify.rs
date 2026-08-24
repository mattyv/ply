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
use ply_core::config::{self, Check, FnClaim};
use ply_core::contract_rt::{self, RenderedTest};
use ply_core::diag::{Counterexample, Diagnostic, Envelope, Fix, Node};
use ply_core::engines::fuzz as fuzz_engine;
use ply_core::engines::kani::{self, KaniOutcome, KaniRunConfig};
use ply_core::engines::mutants::{self, MutantsRunConfig, MutantsRunOutcome};
use ply_core::harness::{self, ContractFn};
use ply_core::harness_crate;

pub const PLY_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct VerifyOptions {
    /// `None` means "use the shape-aware default" (Task 0: a flat default
    /// cannot fit every §5.4b-supported shape -- see
    /// `default_engine_timeout_secs`). An explicit value is always honored
    /// as-is, for every check kind.
    pub engine_timeout_secs: Option<u32>,
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
/// The fix here is the shape-aware one the brief asks for, not a bigger
/// magic number: only `Vec`-typed harnesses get scaled, and the scaling is
/// derived from the measured cost, not guessed. The M3 e2e suite's own
/// vecbound fixture passes 150s explicitly for `bounded(8)`; solving
/// `150 = base + rate * 8` with the M3-observed floor (`bounded(2)` proofs
/// over scalars finished in ~1s in that same suite, so a small fixed base is
/// safe) gives `base = 30, rate = 15`. A scalar-only (no `Vec` parameter)
/// harness keeps the original 60s default unchanged: nothing in the M3
/// findings shows that budget insufficient for any scalar-only fixture, and
/// widening it without evidence would be exactly the "bigger magic number"
/// this task warns against.
pub fn default_engine_timeout_secs(has_vec_param: bool, bound_k: u32) -> u32 {
    if has_vec_param {
        30 + 15 * bound_k
    } else {
        60
    }
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
    }
    let mut plans: Vec<Plan> = Vec::new();
    let mut early_nodes_by_component: BTreeMap<&str, Vec<Node>> = BTreeMap::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    for (comp_name, comp) in &file.components {
        for (fn_name, claim) in &comp.fns {
            let node_id = format!("{comp_name}::{fn_name}");
            let cf = match harness::discover_fn(&lib_path, fn_name) {
                Ok(cf) => cf,
                Err(e) => {
                    diagnostics.push(unresolved_anchor_diag(&node_id, fn_name, "none", &e.to_string()));
                    early_nodes_by_component.entry(comp_name).or_default().push(leaf_node(&node_id, "unclaimed"));
                    continue;
                }
            };

            let explicit = claim.parsed_checks().with_context(|| format!("parsing checks for {node_id}"))?;
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
                        Fix { title: format!("add `test` to `{fn_name}`'s checks list"), edits: vec![] },
                        Fix { title: format!("add `fuzz(256)` to `{fn_name}`'s checks list"), edits: vec![] },
                    ],
                    open_item: Some("mutate_without_kill_signal".into()),
                });
                early_nodes_by_component.entry(comp_name).or_default().push(leaf_node(&node_id, "unclaimed"));
                continue;
            }

            if checks.is_empty() {
                // "none otherwise" (§5.4c): either no contract at all, or a
                // contract whose shape neither gate can build inputs for.
                if cf.has_contract() {
                    diagnostics.push(unsupported_shape_diag(&node_id, fn_name, &cf));
                    early_nodes_by_component.entry(comp_name).or_default().push(leaf_node(&node_id, "unsupported"));
                } else {
                    early_nodes_by_component.entry(comp_name).or_default().push(leaf_node(&node_id, "unclaimed"));
                }
                continue;
            }

            plans.push(Plan { node_id, fn_name, claim, cf, checks });
        }
    }

    // Pass 2: any fn needing fuzz/test/mutate shares one generated harness
    // crate per target crate (§5.4c) -- write it once, fully, before
    // running anything, so mutate's baseline sees every fn's tests.
    let needs_harness = plans.iter().any(|p| {
        p.checks.iter().any(|c| matches!(c, Check::Fuzz(_) | Check::Test | Check::Mutate))
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
            let has_fuzz = plan.checks.iter().find_map(|c| if let Check::Fuzz(n) = c { Some(*n) } else { None });
            let has_test = plan.checks.iter().any(|c| matches!(c, Check::Test));
            if has_fuzz.is_none() && !has_test {
                continue;
            }
            let mut bodies = Vec::new();
            if let Some(n) = has_fuzz {
                if let Ok(body) = ply_core::fuzz_gen::generate_fuzz_test(&plan.cf, n) {
                    bodies.push(body);
                }
                // A build failure here (missing #[ply::ensures], etc.) is
                // reported as a diagnostic in pass 3; no body means nothing
                // to run for the fuzz half.
            }
            if has_test {
                for (i, example) in plan.claim.examples.iter().enumerate() {
                    if let Ok(body) = ply_core::fuzz_gen::generate_example_test(plan.fn_name, (i + 1) as u32, example)
                    {
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
            statuses: vec![],
            children: fn_nodes,
        });
    }

    let root = Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: worst_of(&components),
        statuses: vec![],
        children: components,
    };

    Ok(Envelope { command: "verify".into(), ply_version: PLY_VERSION.into(), root, diagnostics })
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
    children.iter().min_by_key(|n| rank(&n.verdict)).map(|n| n.verdict.clone()).unwrap_or_else(|| "unclaimed".into())
}

/// Combines the results of *one fn's own* checks list (§5.4c: "a function's
/// verdict is the strongest evidence its passing checks earned; a failing
/// check is a violation regardless of what else passed") -- the opposite
/// direction from `worst_of`: when nothing failed, this takes the
/// *strongest* passing verdict, not the weakest.
fn combine_fn_check_verdicts(labels: &[String]) -> String {
    let worst = labels.iter().filter(|l| rank(l) <= 4).min_by_key(|l| rank(l));
    if let Some(w) = worst {
        return w.clone();
    }
    labels.iter().max_by_key(|l| rank(l.as_str())).cloned().unwrap_or_else(|| "unclaimed".into())
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
    harness_info: Option<&(String, String)>,
    opts: &VerifyOptions,
) -> Result<(Node, Vec<Diagnostic>)> {
    let mut diagnostics = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut statuses: Vec<String> = Vec::new();

    for check in checks {
        match check {
            Check::Bounded(k) => {
                let (label, mut d) = run_bounded_check(cf, src_dir, lib_path, crate_dir, node_id, fn_name, *k, opts)?;
                labels.push(label);
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
                    fixes: vec![],
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
    let wants_fuzz = checks.iter().find_map(|c| if let Check::Fuzz(n) = c { Some(*n) } else { None });
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
                fixes: vec![],
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
                let (suffix_ok, mut d) =
                    run_mutate_check(crate_dir, harness_pkg, node_id, fn_name, checks, opts)?;
                diagnostics.append(&mut d);
                if suffix_ok {
                    verdict.push_str("\u{00b7}spec-strong");
                } else {
                    statuses.push("weak-spec".into());
                }
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
                    "`{fn_name}`'s `mutate` check was skipped: its own `test`/`fuzz` check did not \
                     pass, so there is no working baseline to mutate against."
                ),
                primary_span: None,
                counterexample: None,
                fixes: vec![],
                open_item: Some("mutate_skipped_no_baseline".into()),
            });
        }
    }

    Ok((
        Node { id: fn_name.to_string(), kind: "fn".into(), verdict, statuses, children: vec![] },
        diagnostics,
    ))
}

fn unresolved_anchor_diag(node_id: &str, fn_name: &str, check_label: &str, err: &str) -> Diagnostic {
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
                title: format!("add a `pure`-marked generator hook for `{fn_name}`'s parameter type (§5.4b)"),
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
    opts: &VerifyOptions,
) -> Result<(String, Vec<Diagnostic>)> {
    let check_label = format!("bounded({bound_k})");

    if !cf.is_bounded_supported() {
        return Ok(("unsupported".into(), vec![unsupported_shape_diag(node_id, fn_name, cf)]));
    }

    let generated = harness::generate_proof_module(cf, bound_k)?;
    harness::write_generated_module(src_dir, lib_path, &generated.module_source)?;

    let engine_timeout_secs = opts
        .engine_timeout_secs
        .unwrap_or_else(|| default_engine_timeout_secs(cf.has_vec_param(), bound_k));

    let run_cfg = KaniRunConfig {
        crate_dir: crate_dir.to_path_buf(),
        harness_path: generated.proof_fn_path.clone(),
        engine_timeout_secs,
    };
    let outcome = kani::run(&run_cfg)?;

    // §9's cex validity oracle demands the SAME rendered test transitions
    // FAIL -> PASS once a fix lands (see docs/m3-slice-findings.md finding
    // 6): persist any witness found, and re-render its regression test
    // against the CURRENT contract text on every run.
    let witness_path = crate_dir.join("target/ply/witness").join(format!("{fn_name}.json"));
    if let KaniOutcome::Violation { witness_bytes, .. } = &outcome {
        if let Some(parent) = witness_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&witness_path, serde_json::to_string(witness_bytes)?)?;
    }
    if witness_path.exists() {
        let stored: Vec<Vec<u8>> = serde_json::from_str(&std::fs::read_to_string(&witness_path)?)?;
        let values = kani::decode_witness(&stored, &cf.params, bound_k)?;
        let rendered = contract_rt::render_cex_test(cf, &values, &check_label, "K0502", 1)?;
        let module_source =
            contract_rt::wrap_test_module(&[RenderedTest { test_name: rendered.test_name, source: rendered.source }]);
        harness::write_generated_test(src_dir, lib_path, &module_source)?;
    }

    match outcome {
        KaniOutcome::Verified => Ok((check_label, vec![])),
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
                        title: format!("switch `{fn_name}` to `fuzz(256)` -- proptest has no unwind-bound cost"),
                        edits: vec![],
                    },
                ],
                open_item: Some("timeout".into()),
            };
            Ok(("timeout".into(), vec![d]))
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
                open_item: Some("tool_error".into()),
            };
            Ok(("tool_error".into(), vec![d]))
        }
        KaniOutcome::Violation { witness_bytes, raw_output } => {
            let _ = raw_output;
            let values = kani::decode_witness(&witness_bytes, &cf.params, bound_k)?;
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
            let contract_text = cf.ensures.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
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
                        test_file.strip_prefix(src_dir.parent().unwrap_or(src_dir)).unwrap_or(&test_file).display().to_string(),
                    ),
                }),
                fixes: vec![],
                open_item: None,
            };
            Ok(("violation".into(), vec![d]))
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
    opts: &VerifyOptions,
) -> Result<(Option<String>, Option<String>, Vec<Diagnostic>)> {
    let timeout = opts.engine_timeout_secs.unwrap_or_else(default_secondary_engine_timeout_secs);
    let filter = format!("{fn_name}_harness::");
    let run = fuzz_engine::run_harness_tests(crate_dir, harness_pkg, &filter, timeout)?;

    let mut diagnostics = Vec::new();
    let fuzz_test_name = format!("{fn_name}_harness::ply_fuzz_{fn_name}");
    let mut fuzz_label = None;
    let mut test_label = None;

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
                open_item: Some("timeout".into()),
            });
            fuzz_label = Some("timeout".into());
        } else if run.failed_tests.iter().any(|t| t == &fuzz_test_name) {
            let d = render_fuzz_violation(cf, &run.combined_output, node_id, fn_name, &check_label, src_dir, lib_path)?;
            diagnostics.push(d);
            fuzz_label = Some("violation".into());
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
                        "most of the generated inputs for `{fn_name}` were rejected by its own \
                         `#[ply::requires]` ({detail}) -- the fuzz check still ran, but on far fewer \
                         real cases than {n} suggests. A precondition this strict makes fuzzing weak \
                         evidence; consider narrowing the parameter type or the requires clause."
                    ),
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![],
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
                title: format!("`{fn_name}`'s example/direct-case tests did not finish within {timeout}s."),
                primary_span: None,
                counterexample: None,
                fixes: vec![],
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
                open_item: None,
            });
            test_label = Some("violation".into());
        } else {
            test_label = Some("tested".into());
        }
    }

    Ok((fuzz_label, test_label, diagnostics))
}

fn render_fuzz_violation(
    cf: &ContractFn,
    combined_output: &str,
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    src_dir: &Path,
    lib_path: &Path,
) -> Result<Diagnostic> {
    let contract_text = cf.ensures.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
    let Some((_, fields)) = fuzz_engine::parse_fuzz_marker(combined_output) else {
        return Ok(Diagnostic {
            code: "X0901".into(),
            severity: "error".into(),
            phase: "verify".into(),
            engine: "proptest".into(),
            check: check_label.into(),
            node_id: node_id.into(),
            title: format!(
                "`{fn_name}`'s fuzz check found a failing case, but Ply's adapter could not find the \
                 shrunk input marker in proptest's output."
            ),
            primary_span: None,
            counterexample: None,
            fixes: vec![],
            open_item: Some("tool_error".into()),
        });
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
            Ok(Diagnostic {
                code: "P0502".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: check_label.into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` breaks its own postcondition `{contract_text}` for at least one input -- \
                     proptest shrank a failing case to this minimal example. (P0502)"
                ),
                primary_span: None,
                counterexample: Some(Counterexample {
                    inputs,
                    kani_witness: Some(format!(
                        "captured from proptest shrinking on harness `{fn_name}_harness::ply_fuzz_{fn_name}` \
                         (field named `kani_witness` for §8 schema stability; this witness is proptest-, \
                         not Kani-, sourced -- see docs/m4-findings.md)"
                    )),
                    cargo_test: Some(
                        test_file.strip_prefix(src_dir.parent().unwrap_or(src_dir)).unwrap_or(&test_file).display().to_string(),
                    ),
                }),
                fixes: vec![],
                open_item: None,
            })
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
            Ok(Diagnostic {
                code: "W0541".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: check_label.into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` breaks its own postcondition `{contract_text}` for at least one input, \
                     but Ply cannot render it as a Rust literal (a `Vec`/`BTreeSet` of anything but `u8` \
                     has no renderer yet) -- the raw values are recorded below; inputs are never \
                     fabricated. (W0541, reason: inputs_unrenderable)"
                ),
                primary_span: None,
                counterexample: Some(Counterexample { inputs, kani_witness: None, cargo_test: None }),
                fixes: vec![],
                open_item: Some("inputs_unrenderable".into()),
            })
        }
    }
}

fn run_mutate_check(
    crate_dir: &Path,
    harness_pkg: &str,
    node_id: &str,
    fn_name: &str,
    checks: &[Check],
    opts: &VerifyOptions,
) -> Result<(bool, Vec<Diagnostic>)> {
    let _ = checks;
    if !mutants::is_available() {
        return Ok((
            false,
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
                fixes: vec![Fix { title: "cargo install cargo-mutants --locked".into(), edits: vec![] }],
                open_item: Some("engine_missing".into()),
            }],
        ));
    }

    let cargo_toml_text = std::fs::read_to_string(crate_dir.join("Cargo.toml"))?;
    let target_names = harness_crate::read_crate_names(&cargo_toml_text)?;
    let timeout = opts.engine_timeout_secs.unwrap_or_else(default_secondary_engine_timeout_secs);
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
    };
    let outcome = mutants::run(&cfg)?;

    let check_label = "mutate".to_string();
    match outcome {
        MutantsRunOutcome::Completed(o) => {
            if o.all_caught() {
                Ok((true, vec![]))
            } else if o.missed.is_empty() {
                // Nothing to mutate (unviable-only, or zero mutants found)
                // is not evidence of strength either way.
                Ok((
                    false,
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
                        fixes: vec![],
                        open_item: Some("no_mutants".into()),
                    }],
                ))
            } else {
                Ok((
                    false,
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
                            title: format!("strengthen `{fn_name}`'s `#[ply::ensures]` or add `examples` that pin the surviving behavior"),
                            edits: vec![],
                        }],
                        open_item: Some("weak_spec".into()),
                    }],
                ))
            }
        }
        MutantsRunOutcome::Timeout { raw_output } => {
            let _ = raw_output;
            Ok((
                false,
                vec![Diagnostic {
                    code: "M0601".into(),
                    severity: "warning".into(),
                    phase: "verify".into(),
                    engine: "cargo-mutants".into(),
                    check: check_label,
                    node_id: node_id.into(),
                    title: format!("`{fn_name}`'s `mutate` run did not finish within {timeout}s per mutant."),
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![Fix { title: "raise --engine-timeout".into(), edits: vec![] }],
                    open_item: Some("timeout".into()),
                }],
            ))
        }
        MutantsRunOutcome::ToolError { raw_output, reason } => {
            let _ = raw_output;
            Ok((
                false,
                vec![Diagnostic {
                    code: "X0901".into(),
                    severity: "error".into(),
                    phase: "verify".into(),
                    engine: "cargo-mutants".into(),
                    check: check_label,
                    node_id: node_id.into(),
                    title: format!("Ply's cargo-mutants adapter could not interpret its output: {reason}"),
                    primary_span: None,
                    counterexample: None,
                    fixes: vec![],
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
    Node { id: fn_part.to_string(), kind: "fn".into(), verdict: verdict.to_string(), statuses: vec![], children: vec![] }
}

#[allow(dead_code)]
fn unused(_p: &PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_timeout_scales_only_for_vec_shaped_harnesses() {
        assert_eq!(default_engine_timeout_secs(false, 2), 60, "scalar-only stays at the M3 default");
        assert_eq!(
            default_engine_timeout_secs(true, 8), 150,
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
        assert_eq!(rank("fuzz(256)"), 4, "an unrecognized label falls back to the neutral rank, not a passing one");
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
            combine_fn_check_verdicts(&labels), "violation",
            "§5.4c: a failing check is a violation regardless of what else passed"
        );
    }

    #[test]
    fn worst_of_picks_the_weakest_child_not_the_strongest() {
        let children = vec![
            Node { id: "a".into(), kind: "fn".into(), verdict: "bounded(2)".into(), statuses: vec![], children: vec![] },
            Node { id: "b".into(), kind: "fn".into(), verdict: "tested".into(), statuses: vec![], children: vec![] },
        ];
        assert_eq!(worst_of(&children), "tested", "D6: a weak leaf drags its parent down");
    }
}
