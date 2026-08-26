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
use ply_core::callgraph::{CalleeStatus, Resolution, Resolver};
use ply_core::config;
use ply_core::contract_rt::{self, RenderedTest};
use ply_core::diag::{Assumption, Counterexample, Diagnostic, Envelope, Evidence, Fix, Node};
use ply_core::engines::fuzz as fuzz_engine;
use ply_core::engines::kani::ProbeOutcome;
use ply_core::engines::kani::{self, KaniOutcome, KaniRunConfig};
use ply_core::engines::mutants::{self, MutantsRunConfig, MutantsRunOutcome};
use ply_core::harness::{self, ContractFn, Param, StubKind, StubSpec};
use ply_core::harness_crate;
use ply_core::model::{
    Check, Component, FnClaim, InheritedChecks, component_default_checks, effective_checks,
};
use ply_core::promise::{ClauseKind, ClauseVerdict, HarnessAnswer, PromiseFinding, PromisePlan};
use ply_core::reach;
use ply_core::record::{self, AssumedPromise, EngineId, FingerprintInputs, Match, RecordEntry};

use crate::shared::{self, declared_contracts, local_anchor_names, sorted_by_key};

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
/// **The stub premium** (2026-08-25, adversarial review of the post-004
/// fixes, G1). §5.5's second branch replaces a callee with its declared
/// contract: where the real body returned one of four concrete values, the
/// stub returns a symbolic one constrained only by `ensures`. That is
/// strictly less information for CBMC, and it is knowable *before the run*,
/// which is what makes it a shape the default can key on at all -- the same
/// standard `Vec` meets. Without it the tranche's headline capability was
/// dead at the tool's own defaults: vetting 004's `tier_fee_cents` is
/// scalar-signature, so it got 60s, and its stubbed proof needs 201.77s
/// (measured). A user who declared a boundary contract and ran plain
/// `cargo ply verify` got `timeout`, and the diagnostic that should have
/// carried the assumption never appeared.
///
/// The split is derived; **the 300 is fitted to one data point**, and the
/// second data point says the cost is not the stub's alone: the
/// `boundarycontract` fixture's stubbed proof -- same rule, same stub
/// mechanism, smaller body -- verifies in **9.72s** (measured with
/// `cargo kani` on the generated harness). So a stub does not imply 200s;
/// what it implies is the expensive direction, and that 60s is not a budget
/// the feature can live at. 300 is 201.77s plus room for the run-to-run CBMC
/// variance docs/m3-slice-findings.md measured on an identical harness
/// (~1s-107s). Nothing here claims more than that, and §6 says so too.
pub fn default_engine_timeout_secs(has_vec_param: bool, bound_k: u32, has_stubs: bool) -> u32 {
    let base = if has_vec_param { 30 + 15 * bound_k } else { 60 };
    if has_stubs {
        base.max(STUBBED_HARNESS_SECS)
    } else {
        base
    }
}

/// The floor a *stubbed* `bounded` harness gets by default. See
/// `default_engine_timeout_secs` for where the number comes from and what it
/// does not claim.
const STUBBED_HARNESS_SECS: u32 = 300;

/// The `fuzz`/`test`/`mutate` engines never carry Kani's `Vec`-unwind cost
/// profile -- proptest's own strategies and plain `cargo test` do not blow
/// up the way CBMC's symbolic `Vec` construction does, so a single flat
/// default suffices (still explicitly overridable via `--engine-timeout`).
fn default_secondary_engine_timeout_secs() -> u32 {
    60
}

/// Everything outside the user's source that a result depended on, probed
/// once per run (§5.2a's inputs 5 and 6).
///
struct Toolchain {
    /// The target triple this run builds for.
    target: String,
    /// The compiler behind every engine here. A different rustc is a
    /// different build of the code that was checked, which is D9's "an old
    /// success must not bless ... a different toolchain".
    rustc: String,
    /// The crate's declared `[features]` table. Ply passes no `--features`,
    /// so the set that is active is the default set this text defines --
    /// and a change to the table is a change to what was built.
    features: String,
    /// Probed on first use, not at startup: a crate of `fuzz` claims must
    /// not pay a `cargo kani --version` subprocess, and a machine with no
    /// Kani installed must not be slower for having none.
    kani: std::cell::OnceCell<Option<String>>,
    mutants: std::cell::OnceCell<Option<String>>,
}

impl Toolchain {
    fn probe(crate_dir: &Path) -> Toolchain {
        let (rustc, target) = rustc_identity();
        Toolchain {
            target,
            rustc,
            features: declared_features(crate_dir),
            kani: std::cell::OnceCell::new(),
            mutants: std::cell::OnceCell::new(),
        }
    }

    /// The engines one claim's checks stand on, in check order. An engine
    /// that could not be probed is recorded as `not installed`: a check with
    /// no engine behind it earns no evidence and is never recorded, so this
    /// value can never end up guarding a stored result.
    fn engines_for(&self, checks: &[Check], has_stubs: bool) -> Vec<EngineId> {
        let missing = || "not installed".to_string();
        let mut out: Vec<EngineId> = Vec::new();
        for check in checks {
            let id = match check {
                Check::Bounded(_) => EngineId {
                    name: "kani".into(),
                    version: self
                        .kani
                        .get_or_init(kani::version)
                        .clone()
                        .unwrap_or_else(missing),
                    // The flags that shape the obligation, exactly as
                    // `engines::kani::invoke` passes them. The wall-clock
                    // budget is deliberately absent (§5.2a).
                    flags: kani_flags(has_stubs),
                },
                Check::Fuzz(_) | Check::Test => EngineId {
                    name: "proptest".into(),
                    // The requirement Ply writes into the harness crate it
                    // generates, which is the version identity Ply itself
                    // controls. KNOWN GAP: a 1.x release of proptest that
                    // changes how a strategy draws would keep this string,
                    // so a record written before it can be reused after it.
                    version: harness_crate::PROPTEST_REQUIREMENT.to_string(),
                    flags: String::new(),
                },
                Check::Mutate => EngineId {
                    name: "cargo-mutants".into(),
                    version: self
                        .mutants
                        .get_or_init(mutants::version)
                        .clone()
                        .unwrap_or_else(missing),
                    flags: String::new(),
                },
                Check::Prove => EngineId {
                    name: "verus".into(),
                    version: missing(),
                    flags: String::new(),
                },
            };
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

/// The flags that shape what a `cargo kani` run checks, as one recorded
/// string. The `-Z` set comes from the adapter itself
/// (`engines::kani::unstable_flags`) rather than being copied here: a flag
/// list that could change without this string changing would let a proof
/// earned under the old flags be reused under the new ones. The constant
/// tail is the rest of what every invocation passes; the per-run harness
/// name and the wall-clock budget are deliberately not here (§5.2a).
fn kani_flags(has_stubs: bool) -> String {
    format!(
        "{} --exact --concrete-playback print",
        kani::unstable_flags(has_stubs).join(" ")
    )
}

/// `rustc -vV`, split into the version line and the host triple. Both
/// `unknown` when rustc will not answer -- which cannot happen in a run
/// that gets far enough to compile anything.
///
/// The safe direction holds where it matters: a record written by a healthy
/// probe can never be matched by a broken one. It is not unconditional, and
/// the comment here used to say it was: two runs whose probes both fail
/// hash the same `unknown`/`unknown` whatever compilers are really behind
/// them. That needs a broken `rustc -vV` beside a working cargo on both
/// machines, which is exotic -- but "would only ever make a fingerprint
/// match less often" was a claim with an exception in it.
fn rustc_identity() -> (String, String) {
    let out = std::process::Command::new("rustc").arg("-vV").output();
    let text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return ("unknown".into(), "unknown".into()),
    };
    let mut version = "unknown".to_string();
    let mut host = "unknown".to_string();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("host: ") {
            host = rest.trim().to_string();
        } else if line.starts_with("rustc ") {
            version = line.trim().to_string();
        }
    }
    (version, host)
}

/// The crate's `[features]` table, verbatim. A hand-rolled section scan
/// rather than a TOML parse: this text is hashed, never interpreted, so the
/// only property it needs is that it changes when the table changes.
fn declared_features(crate_dir: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(crate_dir.join("Cargo.toml")) else {
        return "(no manifest)".into();
    };
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[features]";
            continue;
        }
        if inside && !trimmed.is_empty() && !trimmed.starts_with('#') {
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    if out.is_empty() {
        "(no features declared)".into()
    } else {
        out
    }
}

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
        /// The qualified name of the component that declares this claim —
        /// `billing`, or `ingest.book`. Carried rather than recovered from
        /// `node_id`, which cannot be split back apart: a fn key may itself
        /// contain `::` (`rates::legacy_rate`).
        component_path: String,
        fn_name: &'a str,
        claim: &'a FnClaim,
        cf: ContractFn,
        checks: Vec<Check>,
        boundary: BoundaryPlan,
        seed: [u8; 32],
        /// Everything this claim's result depends on (§5.2a), hashed
        /// before anything runs -- one hash answers both questions: may a
        /// recorded result be reused, and what is a newly earned one
        /// stored under.
        inputs: FingerprintInputs,
        /// Why the call walk was abandoned for this claim, when it was.
        /// Deliberately *not* a fingerprint input: the scope itself already
        /// is one, and the reason is derived from the same source, so
        /// hashing it would only make a reworded sentence invalidate every
        /// stored result.
        widened_because: Option<String>,
        /// The checks as `ply.yaml` spells them, which is what a recorded
        /// verdict is checked against for possibility before it is trusted.
        check_spellings: Vec<String>,
    }
    let mut plans: Vec<Plan> = Vec::new();
    let mut early_nodes_by_component: BTreeMap<String, Vec<Node>> = BTreeMap::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // `anchor:` is finally consumed (vetting 004 finding 7: it was parsed
    // and ignored, so *every* component's fns were looked for in this
    // crate's own `src/lib.rs` -- which is why a claim written against a
    // dependency died with a misleading `E0301`). A component whose anchor
    // names another crate is a **boundary component**: Ply does not verify
    // its fns from here, it reads the contracts they declare (§5.5).
    let local_anchors = local_anchor_names(crate_dir);
    let is_local = |anchor: &str| -> bool { shared::is_local(&local_anchors, anchor) };

    // §5.4's external-spec route, read for the first time on the verify
    // path: a `requires:`/`ensures:` entry declares a contract for a fn,
    // keyed by the path a caller writes. `audit` reads the same map to list
    // what this crate's proofs rest on, so the two commands cannot disagree
    // about which callee has a promise behind it.
    let declared = declared_contracts(&file, &local_anchors);
    let lib_src = std::fs::read_to_string(&lib_path).unwrap_or_default();
    let mut resolver = Resolver::new(&lib_src, crate_dir, declared)?;

    // §5.2a: the committed record of what earlier runs earned, and the
    // toolchain facts every fingerprint in this run is taken over. Loading
    // the record can fail (a merge conflict in a committed file is the
    // likely cause) and that is reported rather than swallowed -- silently
    // continuing would re-pay every proof and tell nobody why.
    let record_path = crate_dir.join("ply.lock");
    let mut record = record::load(&record_path, PLY_VERSION)?;
    let toolchain = Toolchain::probe(crate_dir);
    // §5.2a's largest input, read once for the whole run: every first-party
    // source file this crate can reach, and the resolved versions of
    // everything outside it. A check does not run the claimed function
    // alone -- it runs whatever that function calls, and a proof descends
    // into it -- so the bodies reachable from a claim are part of what its
    // result stood on. Leaving them out is what made a broken helper reuse
    // a green verdict (adversarial review of result reuse, D1).
    let first_party = reach::scan_first_party(crate_dir);
    let deps_at_plan_time = reach::dependency_identity(crate_dir);
    // Every claim this run either reused or earned. Everything else is
    // dropped from the record at the end: a claim somebody deleted, one
    // whose function no longer resolves, one this run checked and got no
    // evidence for. What survives is exactly what this run stands behind,
    // so a reviewer reading the committed file is never looking at a verdict
    // the last run did not produce.
    let mut kept_claims: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Name order, not declaration order. The promoted model preserves the
    // order the author wrote (the renderer lays boxes out that way); `verify`
    // read a `BTreeMap` before Phase 1a, so its node and diagnostic order was
    // sorted, and the goldens in tests/e2e pin that. Sorting here keeps the
    // envelope byte-identical across the promotion instead of quietly
    // reordering every multi-fn run.
    for (comp_name, comp, inherited) in flatten_components(&file) {
        for (fn_name, claim) in sorted_by_key(&comp.fns) {
            let node_id = format!("{comp_name}::{fn_name}");
            // §5.1: the list that actually governs this fn — its own if it
            // wrote one (an empty one included, §5.4c), else the nearest
            // ancestor component's default, else nothing written anywhere.
            // `verify` used to read the fn's own list and nothing else, so a
            // component default was resolved by `check` and silently ignored
            // here: one document, two answers about which check runs.
            let governing = effective_checks(claim, inherited);
            if !is_local(&comp.anchor) {
                // A boundary component. Its contracts are already in
                // `declared`; its `checks` cannot run from here, and saying
                // so is the honest report (`verify` is single-crate).
                if governing.is_some_and(|c| !c.is_empty()) {
                    diagnostics.push(cross_crate_claim_diag(
                        &node_id,
                        fn_name,
                        &comp.anchor,
                        &local_anchors,
                    ));
                }
                continue;
            }
            let cf = match harness::discover_fn_with(&mut resolver, fn_name, &lib_path) {
                Ok(cf) => cf,
                Err(e) => {
                    diagnostics.push(unresolved_anchor_diag(
                        &node_id,
                        fn_name,
                        "none",
                        &e.to_string(),
                    ));
                    early_nodes_by_component
                        .entry(comp_name.clone())
                        .or_default()
                        .push(leaf_node(&node_id, "unclaimed"));
                    continue;
                }
            };

            let explicit = governing
                .unwrap_or(&[])
                .iter()
                .map(|c| config::parse_check_string(c))
                .collect::<Result<Vec<Check>>>()
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

            // §5.4c: **an empty list is a list.** `checks: []` reads to a
            // person as "do not check this", and it is now what it reads
            // as: nothing runs, and the claim earns no evidence. Reading it
            // as *no* list -- which is what "is the list empty?" does -- put
            // the shape-aware default back and proved the function anyway,
            // silently doing the opposite of what the document said.
            let declared_empty = governing.is_some_and(|c| c.is_empty());
            let checks = if !explicit.is_empty() {
                explicit
            } else if declared_empty {
                vec![]
            } else {
                default_checks_for(&cf)
            };

            // D12's E0504, from the one rule both commands share
            // (`ply_core::check`). Before Phase 1a `verify` carried its own
            // copy and its own sentence, and recovered the code by splitting
            // the message on its first colon -- so the wording and the code
            // were coupled by accident.
            if ply_core::check::mutate_lacks_kill_signal(&checks) {
                let msg = ply_core::check::mutate_kill_signal_message(&format!("fn {fn_name}"));
                diagnostics.push(Diagnostic {
                    code: "E0504".into(),
                    severity: "error".into(),
                    phase: "verify".into(),
                    engine: "ply".into(),
                    check: "mutate".into(),
                    node_id: node_id.clone(),
                    title: msg,
                    pointer: None,
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
                    .entry(comp_name.clone())
                    .or_default()
                    .push(leaf_node(&node_id, "unclaimed"));
                continue;
            }

            if checks.is_empty() {
                if declared_empty {
                    // The author asked for nothing, so nothing ran -- and
                    // that is said out loud rather than left as a node
                    // nobody expands.
                    // Whose empty list it is: the fn's own, or a component
                    // default it inherited. Saying "`f` has an empty
                    // `checks:` list" about a fn whose entry has no such
                    // line would send a reader looking at the wrong line.
                    let from = match claim.checks {
                        Some(_) => None,
                        None => inherited.map(|d| d.from_component),
                    };
                    diagnostics.push(empty_checks_diag(&node_id, fn_name, &cf, from));
                    early_nodes_by_component
                        .entry(comp_name.clone())
                        .or_default()
                        .push(leaf_node(&node_id, "unclaimed"));
                    continue;
                }
                // "none otherwise" (§5.4c): either no contract at all, or a
                // contract whose shape neither gate can build inputs for.
                if cf.has_contract() {
                    diagnostics.push(unsupported_shape_diag(&node_id, fn_name, &cf));
                    early_nodes_by_component
                        .entry(comp_name.clone())
                        .or_default()
                        .push(leaf_node(&node_id, "unsupported"));
                } else {
                    early_nodes_by_component
                        .entry(comp_name.clone())
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

            // The callees this claim's proof replaces with a promise, and
            // therefore never looks inside. Only a claim whose every check
            // is `bounded` gets that: `fuzz`, `test` and `mutate` run the
            // real body however many promises are declared for it, so for
            // them the body is part of what the result stood on.
            let all_bounded = checks.iter().all(|c| matches!(c, Check::Bounded(_)));
            let stubbed: std::collections::BTreeSet<String> = if all_bounded {
                boundary
                    .stubs
                    .iter()
                    .map(|s| s.callee_path.clone())
                    .collect()
            } else {
                std::collections::BTreeSet::new()
            };
            let code = reach::code_scope(&mut resolver, &first_party, &cf.path, &stubbed);
            // Taken before `code.units` is moved into the fingerprint below.
            let widened_because = code.widened_because.clone();
            let check_spellings: Vec<String> = checks.iter().map(check_spelling).collect();

            let inputs = FingerprintInputs {
                node_id: node_id.clone(),
                fn_path: cf.path.clone(),
                fn_source: cf.source.clone(),
                inline_requires: cf
                    .requires
                    .as_ref()
                    .map(|(_, t)| t.clone())
                    .unwrap_or_default(),
                inline_ensures: cf
                    .ensures
                    .as_ref()
                    .map(|(_, t)| t.clone())
                    .unwrap_or_default(),
                declared_requires: claim.requires.clone(),
                declared_ensures: claim.ensures.clone(),
                assumed: boundary
                    .stubs
                    .iter()
                    .map(|s| AssumedPromise {
                        callee: s.callee_path.clone(),
                        requires: s.requires.clone(),
                        ensures: s.ensures.clone(),
                        signature: format!(
                            "({}) -> {}",
                            s.params
                                .iter()
                                .map(|(n, t)| format!("{n}: {t}"))
                                .collect::<Vec<_>>()
                                .join(", "),
                            s.return_type
                        ),
                    })
                    .collect(),
                // Filled in only for a bounded-eligible claim, only once
                // `resolve_contracted_calls` has run for it (the ordered
                // pass below) -- empty here since D5's branch decision, and
                // therefore the bound this claim composes against, is not
                // yet known at plan time.
                verified_bounds: Vec::new(),
                examples: claim.examples.clone(),
                code_scope: code.scope.to_string(),
                code: code.units,
                deps: deps_at_plan_time.clone(),
                checks: check_spellings.clone(),
                seed: ply_core::fuzz_gen::seed_hex(&seed),
                engines: toolchain.engines_for(&checks, !boundary.stubs.is_empty()),
                target: toolchain.target.clone(),
                rustc: toolchain.rustc.clone(),
                features: toolchain.features.clone(),
                ply_version: PLY_VERSION.to_string(),
            };

            plans.push(Plan {
                node_id,
                component_path: comp_name.clone(),
                fn_name,
                claim,
                cf,
                checks,
                boundary,
                seed,
                inputs,
                widened_because,
                check_spellings,
            });
        }
    }

    // §5.2a's honesty rule, in the one place it can be enforced: a recorded
    // result is looked up *by today's hash*, so no path exists that reaches
    // a stored verdict without re-deriving what it depended on first.
    // Why a stored result could not be used, per claim -- the names of the
    // inputs that moved. A full-price re-run that says nothing about what
    // changed is the very experience this feature exists to end.
    //
    // A bounded-eligible claim's lookup is **deferred**, not done here: D5's
    // first branch (§5.5) composes its fingerprint against a callee's own
    // *earned* bound, which is not known until that callee has itself been
    // resolved in dependency order (below). Deciding reuse from the
    // Pass-1 fingerprint here, before that composition, is exactly the gap
    // an adversarial review found 2026-08-26: editing only a callee's
    // declared `checks:` (its bound going from 5 down to 2, no source
    // touched anywhere) correctly re-earned the callee's own record but
    // left the caller's fingerprint -- and therefore its stored, now-stale
    // `bounded(5)` -- untouched. So a bounded-eligible claim's fingerprint
    // is finalised, and only then looked up, in the ordered pass below;
    // every other claim's has nothing left to wait for and is decided here
    // exactly as it always was.
    let bounded_eligible: std::collections::BTreeSet<usize> = plans
        .iter()
        .enumerate()
        .filter(|(_, p)| p.checks.iter().any(|c| matches!(c, Check::Bounded(_))))
        .map(|(i, _)| i)
        .collect();
    let mut not_carried_forward: Vec<ply_core::diag::NotCarriedForward> = Vec::new();
    let mut reused: Vec<Option<RecordEntry>> = vec![None; plans.len()];
    for (i, p) in plans.iter().enumerate() {
        if bounded_eligible.contains(&i) {
            continue;
        }
        reused[i] = lookup_record(
            &record,
            &p.node_id,
            &p.inputs,
            &p.check_spellings,
            &mut diagnostics,
            &mut not_carried_forward,
            &p.widened_because,
        );
    }

    // Pass 2: any fn needing fuzz/test/mutate shares one generated harness
    // crate per target crate (§5.4c) -- write it once, fully, before
    // running anything, so mutate's baseline sees every fn's tests.
    // A reused claim runs no engine, so it needs no harness. A crate whose
    // every fuzz claim is reused therefore writes no harness crate and
    // compiles nothing at all.
    let needs_harness = plans.iter().zip(&reused).any(|(p, r)| {
        r.is_none()
            && p.checks
                .iter()
                .any(|c| matches!(c, Check::Fuzz(_) | Check::Test | Check::Mutate))
    });
    let mut harness_info: Option<HarnessInfo> = None;
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

        let mut modules: Vec<harness_crate::HarnessModule> = Vec::new();
        for (plan, reused) in plans.iter().zip(&reused) {
            if reused.is_some() {
                continue;
            }
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
            // A build failure here (missing #[ply::ensures], a postcondition
            // that reads a moved parameter, etc.) is reported as a
            // diagnostic in pass 3; no body means nothing to run for the
            // fuzz half.
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
                modules.push(harness_crate::HarnessModule {
                    fn_ident: plan.cf.ident(),
                    source: ply_core::fuzz_gen::wrap_fn_harness_module(
                        &plan.cf,
                        &target_names.lib_ident,
                        &bodies,
                    ),
                });
            }
        }

        // The misattribution fix. Before this, one broken function's
        // generated module took the *entire* harness crate's compile down
        // with it, and every other claim sharing the crate -- however
        // correct -- reported the same tool error, quoting the same
        // compiler message about a variable it does not have (§9: "a
        // defect found by review enters the suite as a fixture of its own
        // shape").
        //
        // The fix compiles the shared crate once (`--no-run`: never
        // executing a single case, only checking it builds). A failure's
        // compiler errors are mapped back to the one generated module each
        // came from by the line its own `--> ` span names (each module's
        // line range is known exactly, since Ply itself just wrote the
        // file) -- so a build with two independently broken functions is
        // resolved in one extra compile, not one per claim, and not a
        // bisection search. Broken module(s) are then dropped and the
        // remainder rebuilt, so every innocent claim still gets to run for
        // real and earn its own verdict. Bounded, not looped forever: a
        // build that keeps finding new attributable breakage is vanishingly
        // unlikely (rustc reports independent errors together), but nothing
        // here should spin.
        const MAX_BUILD_ATTEMPTS: u32 = 4;
        let mut broken: BTreeMap<String, String> = BTreeMap::new();
        let mut unattributed_cause: Option<String> = None;
        let timeout = opts
            .engine_timeout_secs
            .unwrap_or_else(default_secondary_engine_timeout_secs);
        let mut attempt: u32 = 0;
        loop {
            let (_, spans) = harness_crate::write_harness_lib_rs(&harness_dir, &modules)?;
            if modules.is_empty() {
                break;
            }
            let check = fuzz_engine::check_harness_builds(crate_dir, &harness_pkg, timeout)?;
            if check.build_ok || check.timed_out {
                break;
            }
            let lib_suffix = format!("{harness_rel}/src/lib.rs");
            let errors = fuzz_engine::build_errors_with_lines(&check.combined_output, &lib_suffix);
            let attributed = fuzz_engine::attribute_build_errors(&errors, &spans);
            if attributed.is_empty() {
                // No error the compiler reported carries a span Ply can
                // place inside any known module -- §1: an honest "Ply
                // could not tell which" beats guessing and blaming a
                // function that might be entirely innocent.
                unattributed_cause = Some(
                    fuzz_engine::first_build_error(&check.combined_output)
                        .unwrap_or_else(|| "the compiler gave no specific error line".to_string()),
                );
                break;
            }
            for (ident, cause) in attributed {
                broken.entry(ident).or_insert(cause);
            }
            modules.retain(|m| !broken.contains_key(&m.fn_ident));
            attempt += 1;
            if attempt >= MAX_BUILD_ATTEMPTS && !modules.is_empty() {
                unattributed_cause = Some(
                    "Ply kept finding new compile failures in this crate's generated harness \
                     even after removing every function it could pin one to, and gave up \
                     rather than loop forever"
                        .to_string(),
                );
                break;
            }
        }

        harness_info = Some(HarnessInfo {
            package: harness_pkg,
            broken,
            unattributed_cause,
        });
    }

    // D5's ordering (§5.5): "within a crate, verify claimed functions
    // callees-before-callers". Only claims with a `bounded` check need it --
    // `boundary.contracted` is empty for every other kind, since only
    // `bounded` ever descends into a callee's body at all.
    //
    // `path_to_idx` is every bounded-eligible claim, *not* filtered by
    // reuse: whether one turns out to be reused is exactly the question
    // this ordered pass answers for it (above), so it cannot be known yet.
    // Excluding an about-to-be-decided claim from the graph would silently
    // exempt it from ordering instead of correctly placing it.
    let path_to_idx: BTreeMap<String, usize> = bounded_eligible
        .iter()
        .map(|&i| (plans[i].cf.path.clone(), i))
        .collect();
    let mut edges: BTreeMap<usize, std::collections::BTreeSet<usize>> = BTreeMap::new();
    for &f_idx in path_to_idx.values() {
        for cc in &plans[f_idx].boundary.contracted {
            if let Some(&g_idx) = path_to_idx.get(&cc.canonical_path)
                && g_idx != f_idx
            {
                edges.entry(g_idx).or_default().insert(f_idx);
            }
        }
    }
    let node_ids: Vec<String> = plans.iter().map(|p| p.node_id.clone()).collect();
    let (topo_order, cyclic) = topological_order(&bounded_eligible, &node_ids, &edges);
    // Processing order: callees before callers among the orderable
    // bounded-eligible claims, then the ones a cycle left unorderable (D5's
    // second branch covers every one of their contracted-callee edges, so
    // their own place relative to each other cannot matter -- and no cycle
    // is introduced by this graph itself: `g` never depends on `f` under
    // callees-first construction, edges only ever point callee-to-caller,
    // so the only way an index lands in `cyclic` is a genuine call cycle in
    // the source, exactly D5's own "`f` and `g` in a cycle" case), then
    // every other fresh claim in the order Pass 1 already produced --
    // fuzz/test/mutate claims and unsupported/unclaimed ones never consult
    // `known_bounded` at all, so nothing about their order is load-bearing.
    let mut processing_order: Vec<usize> = topo_order;
    processing_order.extend(cyclic.iter().copied());
    let ordered: std::collections::BTreeSet<usize> = processing_order.iter().copied().collect();
    processing_order.extend(
        (0..plans.len()).filter(|i| {
            !bounded_eligible.contains(i) && reused[*i].is_none() && !ordered.contains(i)
        }),
    );

    // Every same-crate claim this same run stood on cleanly (never
    // `conditional`) with a `bounded(k)` verdict, populated as this loop
    // resolves each bounded-eligible claim in dependency order -- from a
    // fresh run (below) or, just as validly (§5.5's honesty condition 3,
    // sound since commit c650e55), from a reuse hit whose *finalised*
    // fingerprint matched. Since a cyclic claim always composes with
    // `bound: None` (never eligible for branch one), nothing is ever
    // inserted for one here, confirming the graph really does carry no
    // cycle through this map.
    let mut known_bounded: BTreeMap<String, u32> = BTreeMap::new();

    // Run each fn's checks and assemble its verdict + diagnostics, in
    // `processing_order` so a caller's D5 decision can see its own
    // callees' fresh verdicts -- then present in Pass 1's original
    // (name-sorted) order below, so execution order never leaks into the
    // tree's own layout. For a bounded-eligible claim this is also where
    // its fingerprint is *finalised* (once `resolve_contracted_calls` has
    // decided `boundary.verified`) and only then looked up: a reuse hit
    // here must run no engine and write nothing, exactly like any other
    // reused claim -- an earlier version of this pass decided reuse before
    // ordering and then unconditionally re-ran every bounded-eligible claim
    // regardless, which both wasted the exact engine cost reuse exists to
    // avoid and could write a proof module for a claim the envelope then
    // reported as `reused: true` (caught by `resultreuse_fixture`, 2026-08-26).
    let mut results: Vec<Option<(Node, Vec<Diagnostic>)>> =
        (0..plans.len()).map(|_| None).collect();
    for idx in processing_order {
        if bounded_eligible.contains(&idx) {
            resolve_contracted_calls(
                &mut plans[idx].boundary,
                cyclic.contains(&idx),
                &known_bounded,
            );
            plans[idx].inputs.verified_bounds = plans[idx].boundary.verified.clone();
            let hit = lookup_record(
                &record,
                &plans[idx].node_id,
                &plans[idx].inputs,
                &plans[idx].check_spellings,
                &mut diagnostics,
                &mut not_carried_forward,
                &plans[idx].widened_because,
            );
            if let Some(entry) = hit {
                if !entry.statuses.iter().any(|s| s == "conditional")
                    && let Some(k) = parse_bound(&entry.verdict)
                {
                    known_bounded.insert(plans[idx].cf.path.clone(), k);
                }
                reused[idx] = Some(entry);
                continue;
            }
        }
        let (node, fn_diags) = run_fn_checks(
            &plans[idx].node_id,
            &src_dir,
            &lib_path,
            crate_dir,
            plans[idx].fn_name,
            &plans[idx].cf,
            &plans[idx].checks,
            &plans[idx].boundary,
            &plans[idx].seed,
            harness_info.as_ref(),
            !plans[idx].claim.examples.is_empty(),
            opts,
        )?;
        if node.verdict.starts_with("bounded(")
            && !node.statuses.iter().any(|s| s == "conditional")
            && let Some(k) = parse_bound(&node.verdict)
        {
            known_bounded.insert(plans[idx].cf.path.clone(), k);
        }
        results[idx] = Some((node, fn_diags));
    }

    let mut component_nodes: BTreeMap<String, Vec<Node>> = early_nodes_by_component;
    for (idx, (plan, reused)) in plans.iter().zip(&reused).enumerate() {
        // Carried forward, and said so on the node: everything the recorded
        // run reported about this claim, re-emitted as it was, because a
        // reused `conditional` verdict whose assumption paragraph went
        // missing would be a worse report than no reuse at all.
        if let Some(entry) = reused {
            kept_claims.insert(plan.node_id.clone());
            diagnostics.extend(entry.diagnostics.iter().cloned());
            component_nodes
                .entry(plan.component_path.clone())
                .or_default()
                .push(Node {
                    id: plan.fn_name.to_string(),
                    kind: "fn".into(),
                    verdict: entry.verdict.clone(),
                    statuses: entry.statuses.clone(),
                    reused: true,
                    evidence: entry.evidence.clone(),
                    children: vec![],
                });
            continue;
        }
        let (node, mut fn_diags) = results[idx]
            .take()
            .expect("every fresh plan was processed above");
        // Recorded only when this run earned evidence: a violation, a
        // timeout or any other absence is never stored, so nothing that
        // failed can ever be carried forward (§5.2a).
        if earned_evidence(&node, &fn_diags) {
            kept_claims.insert(plan.node_id.clone());
            // The dependency versions are read again here, not reused from
            // plan time: a crate that had never been built has no lockfile
            // until this run compiled it, and the versions that governed
            // the run that just happened are the ones the result stood on.
            // Without this a first run would record a fingerprint no second
            // run could ever match, and every crate would pay twice.
            let mut inputs = plan.inputs.clone();
            inputs.deps = reach::dependency_identity(crate_dir);
            record.record(
                &plan.node_id,
                RecordEntry {
                    fingerprint: record::fingerprint(&inputs),
                    verdict: node.verdict.clone(),
                    statuses: node.statuses.clone(),
                    evidence: node.evidence.clone(),
                    diagnostics: fn_diags.clone(),
                    inputs: inputs.per_group_digests(),
                },
            );
        }
        diagnostics.append(&mut fn_diags);
        component_nodes
            .entry(plan.component_path.clone())
            .or_default()
            .push(node);
    }

    record.retain_claims(&kept_claims);
    record::save(&record_path, &record)?;

    // The tree the document declares, with each claim's node under the
    // component that declares it however deep that is (§5.1's nested
    // `components:`, §7's containment tree). A component that produced no
    // node at all -- no claim of its own, none in its subtree -- is left
    // out rather than drawn empty, which is the shape `verify` has always
    // reported for a component whose claims are checked elsewhere.
    let mut components: Vec<Node> = Vec::new();
    for (name, comp) in sorted_by_key(&file.components) {
        if let Some(node) = component_node(name, comp, &mut component_nodes) {
            components.push(node);
        }
    }

    let root = Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: worst_of(&components),
        statuses: union_statuses(&components),
        reused: false,
        evidence: None,
        children: components,
    };

    Ok(Envelope {
        command: "verify".into(),
        ply_version: PLY_VERSION.into(),
        root,
        diagnostics,
        coverage: None,
        trust_surface: None,
        open_items: None,
        not_carried_forward,
    })
}

/// A stored result whose verdict none of its own checks could have earned:
/// the file was edited by something that is not Ply. Reported, and the
/// claim checked again -- never used.
/// Looks a claim's fingerprint up against the committed record, reporting
/// why it could not be used when it could not. Shared by the immediate
/// (non-bounded-eligible) reuse pass and the ordered pass below it, which
/// calls this only once a bounded-eligible claim's fingerprint is
/// *finalised* -- `inputs` must already carry `verified_bounds` by the time
/// this is called for one, or the lookup would be exactly the stale-bound
/// gap this function exists to close.
fn lookup_record(
    record: &record::Record,
    node_id: &str,
    inputs: &FingerprintInputs,
    check_spellings: &[String],
    diagnostics: &mut Vec<Diagnostic>,
    not_carried_forward: &mut Vec<ply_core::diag::NotCarriedForward>,
    widened_because: &Option<String>,
) -> Option<RecordEntry> {
    let fingerprint = record::fingerprint(inputs);
    match record.matching(
        node_id,
        &fingerprint,
        check_spellings,
        &inputs.verified_bounds,
    ) {
        Match::Hit(entry) => Some(entry.clone()),
        Match::Impossible(sentence) => {
            diagnostics.push(impossible_record_diag(node_id, sentence));
            None
        }
        Match::Miss => {
            if let Some(because) = record.displaced_by(node_id, inputs) {
                not_carried_forward.push(ply_core::diag::NotCarriedForward {
                    node_id: node_id.to_string(),
                    widened_because: because
                        .iter()
                        .any(|b| b == "the code it runs")
                        .then(|| widened_because.clone())
                        .flatten(),
                    because,
                });
            }
            None
        }
    }
}

fn impossible_record_diag(node_id: &str, sentence: String) -> Diagnostic {
    Diagnostic {
        code: "W0516".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "record".into(),
        node_id: node_id.into(),
        title: sentence,
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![Fix {
            title: "delete `ply.lock` and run `cargo ply verify` again -- the file is rebuilt \
                    from what this run earns, and nothing is lost but the engine time"
                .into(),
            edits: vec![],
        }],
        assumptions: vec![],
        open_item: None,
    }
}

/// Every component in the document, depth first, each paired with its
/// qualified name (`billing`, `ingest.book`) and the §5.1 checks default in
/// force for the fns inside it — parents before their children, siblings in
/// name order at every level.
///
/// `verify` used to iterate the top level of this tree only, so a claim
/// written inside a nested component produced no node, no diagnostic and no
/// mention at all, while `cargo ply check` walked the whole tree and
/// reported the same claim as pointing at real code. The two commands
/// disagreed about which claims exist, and the disagreement was silent —
/// the worst shape a gap can take (§1).
///
/// The inherited default is carried down the same walk, from the same
/// shared resolution `check` and the renderer use
/// (`ply_core::model::component_default_checks`), so the three cannot
/// disagree about which list governs a fn.
fn flatten_components(
    doc: &ply_core::model::Document,
) -> Vec<(
    String,
    &ply_core::model::Component,
    Option<InheritedChecks<'_>>,
)> {
    type Row<'a> = (String, &'a Component, Option<InheritedChecks<'a>>);
    fn walk<'a>(
        path: String,
        leaf: &'a str,
        comp: &'a Component,
        inherited: Option<InheritedChecks<'a>>,
        out: &mut Vec<Row<'a>>,
    ) {
        let below = component_default_checks(leaf, comp, inherited);
        for (child, nested) in sorted_by_key(&comp.components) {
            walk(format!("{path}.{child}"), child, nested, below, out);
        }
        out.push((path, comp, below));
    }
    let mut out = Vec::new();
    for (name, comp) in sorted_by_key(&doc.components) {
        walk(name.clone(), name, comp, None, &mut out);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// One component's §7 node: its own claims' nodes, then a node per nested
/// component that has anything to report. `None` when nothing in the whole
/// subtree produced a node.
fn component_node(
    path: &str,
    comp: &ply_core::model::Component,
    fn_nodes: &mut BTreeMap<String, Vec<Node>>,
) -> Option<Node> {
    let mut children: Vec<Node> = fn_nodes.remove(path).unwrap_or_default();
    for (child, nested) in sorted_by_key(&comp.components) {
        if let Some(node) = component_node(&format!("{path}.{child}"), nested, fn_nodes) {
            children.push(node);
        }
    }
    if children.is_empty() {
        return None;
    }
    Some(Node {
        id: path.to_string(),
        kind: "component".into(),
        verdict: worst_of(&children),
        // D6: statuses are not in the evidence order -- they propagate
        reused: false,
        // upward as flags beside the verdict. A `conditional` leaf must
        // still be visible from the root, or the trust story stops at
        // the fn nobody expanded.
        statuses: union_statuses(&children),
        evidence: None,
        children,
    })
}

/// The shape-aware default routing (§5.4c, the M4 non-negotiable MUST):
/// `[bounded(2)]` when the fn has a contract and its signature passes the
/// Kani gate; `[fuzz(256)]` when the shape is excluded from `bounded` but
/// the fuzz gate still passes; empty (checked elsewhere against
/// `has_contract`) otherwise. A flat `[bounded(2)]` default would route
/// most contracted functions in ordinary Rust into `unsupported` or a
/// multi-minute timeout (§5.4c).
pub(crate) fn default_checks_for(cf: &ContractFn) -> Vec<Check> {
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

/// A same-crate callee whose own inline `#[ply::requires]`/`#[ply::ensures]`
/// makes it D5's territory (§5.5) -- resolved once (`boundary_plan`, before
/// anything runs), decided later once every claim in this run has an
/// ordering (`resolve_contracted_calls`): a `StubKind::Contracted` either
/// way, `bound: Some(k)` when the callee earned a clean `bounded(k)` this
/// run and is not part of an unorderable cycle with this caller (branch
/// one), `bound: None` otherwise (branch two).
struct ContractedCall {
    /// The callee's own crate-root canonical path -- what a claimed fn's
    /// own `cf.path` is compared against to ask "did THIS run prove it".
    canonical_path: String,
    /// The callee's own normalised parameters -- what the never-run
    /// existence harness `StubKind::Contracted` renders needs to call it
    /// with symbolic arguments (Kani's plain `#[kani::stub]` cannot target
    /// a contracted function at all, so this is the only mechanism either
    /// of D5's branches can use for one -- see `StubKind`'s own doc).
    params: Vec<Param>,
    /// Best-effort raw return-type text, for `crate::promise`'s `ensures`
    /// probe (which needs a type to bind `__ply_result` at) -- `None`
    /// (rendered as `"()"`) when the callee returns nothing.
    raw_return: Option<String>,
    requires: Vec<String>,
    ensures: Vec<String>,
}

/// What §5.5's split found in one function's body: callees nothing
/// describes (the third branch -- refuse to descend), callees whose
/// declared contract will be assumed and stubbed (the second branch), and
/// same-crate contracted callees whose branch is not yet decided
/// (`contracted`, D5's first two branches).
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
    /// Same-crate callees carrying their own inline contract, one entry per
    /// distinct callee, deduplicated by canonical path. Decided into
    /// `stubs`/`unstubbable` once ordering is known (see
    /// `contracted_calls_for_claim`); empty for a call whose only
    /// same-crate contracted callees are none (nothing to decide) and for
    /// any callee reached through a path dependency (cross-crate
    /// `stub_verified` is out of scope for v1, §5.5).
    contracted: Vec<ContractedCall>,
    /// `(canonical path, where it is called, why Ply could not build a
    /// stub for it)` for a same-crate contracted callee the stub builder
    /// cannot handle at all -- a `self` parameter, a non-identifier
    /// parameter pattern, a private module, or a contract attribute Ply
    /// cannot parse. Found by adversarial review, 2026-08-26: every one of
    /// these used to fall through silently (no stub, no refusal, no
    /// diagnostic), so Kani inlined the callee's real body and anything
    /// unclaimed beneath it travelled into the caller's proof unnamed --
    /// the exact outcome §5.5's second honesty condition says can no
    /// longer happen. Refused here the same way `unstubbable` already
    /// refuses a `()`-returning boundary-contract callee.
    unstubbable_contracted: Vec<(String, String, String)>,
    /// D5's first branch, once decided: the same-crate callees this claim
    /// stands on rather than owes evidence for, each with the `bounded(k)`
    /// it earned this run -- carried so the caller's own bound composes as
    /// `min(k_caller, k)` (§5.5) and so the tree can still show the
    /// dependency even though the caller is not `conditional` for it.
    verified: Vec<(String, u32)>,
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
            CalleeStatus::Unresolved => {}
            // D5's first two branches (§5.5): resolved again here (`classify`
            // only says "this site's callee carries its own contract", not
            // which callee or what it says) to get everything a later
            // decision needs. Cross-crate is out of scope for v1: a callee
            // reached through a path dependency is left exactly as before
            // (full descent), never added here.
            CalleeStatus::Contracted => {
                let already_known = |plan: &BoundaryPlan, path: &str| {
                    plan.contracted.iter().any(|c| c.canonical_path == path)
                        || plan
                            .unstubbable_contracted
                            .iter()
                            .any(|(p, _, _)| p == path)
                };
                match resolver.lookup_fn(&site.path) {
                    Resolution::Found(found) if !found.local => {
                        // Cross-crate `stub_verified` is out of scope for
                        // v1 (§5.5): left exactly as before this feature,
                        // full descent -- a stated exception, not a silent
                        // fallthrough.
                    }
                    Resolution::Found(found) => {
                        let canonical = found.canonical.clone();
                        if already_known(&plan, &canonical) {
                            // already decided (stubbed or refused) by an
                            // earlier call site to the same callee
                        } else if let Some(reason) = &found.unnameable {
                            plan.unstubbable_contracted.push((
                                canonical,
                                site.where_text(),
                                reason.clone(),
                            ));
                        } else {
                            match harness::build_contract_fn(
                                &found.item,
                                &harness::alias_map(&found.file),
                                &canonical,
                            ) {
                                Ok(callee_cf) => {
                                    let raw_return =
                                        ply_core::callgraph::signature_of(&found.item).return_type;
                                    plan.contracted.push(ContractedCall {
                                        canonical_path: canonical,
                                        params: callee_cf.params,
                                        raw_return,
                                        requires: callee_cf
                                            .requires
                                            .map(|(_, t)| vec![t])
                                            .unwrap_or_default(),
                                        ensures: callee_cf
                                            .ensures
                                            .map(|(_, t)| vec![t])
                                            .unwrap_or_default(),
                                    });
                                }
                                Err(e) => {
                                    plan.unstubbable_contracted.push((
                                        canonical,
                                        site.where_text(),
                                        e.to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    // `classify` already resolved this site as `Contracted`,
                    // so neither of these should be reachable in practice --
                    // but "cannot happen" is exactly the reasoning that let
                    // the fallthrough above go silent for two review
                    // cycles, so both refuse rather than assume.
                    Resolution::Opaque(reason) => {
                        if !plan.opaque.iter().any(|(p, _, _)| p == &site.path) {
                            plan.opaque
                                .push((site.path.clone(), site.where_text(), reason));
                        }
                    }
                    Resolution::NotFound => {
                        if !plan.unclaimed.iter().any(|(p, _)| p == &site.path) {
                            plan.unclaimed.push((site.path.clone(), site.where_text()));
                        }
                    }
                }
            }
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
                canonical_path,
                signature,
            } => {
                if plan.stubs.iter().any(|s| s.callee_path == canonical_path) {
                    continue;
                }
                match signature.return_type {
                    Some(ret) => plan.stubs.push(StubSpec {
                        callee_path: canonical_path,
                        params: signature.params,
                        return_type: ret,
                        requires: contract.requires,
                        ensures: contract.ensures,
                        kind: StubKind::Assumed,
                    }),
                    None => plan.unstubbable.push((canonical_path, site.where_text())),
                }
            }
        }
    }
    plan
}

/// Decides D5's first-vs-second branch (§5.5) for every same-crate
/// contracted callee this claim's body reaches, once ordering says whether
/// the answer is known: `is_cyclic` is true when this claim could not be
/// placed before its own bounded-eligible callees (an unorderable cycle,
/// §5.5's "`f` and `g` in a cycle" case -- every claim in one falls back to
/// branch two, for every edge, not only the ones inside the cycle, since
/// this claim's own place in the run's ordering is exactly what branch one
/// needs and a cyclic claim has none). `known_bounded` maps a callee's own
/// canonical path to the `bounded(k)` it earned *this run* (freshly, or
/// carried forward from a still-valid record, commit c650e55) -- present
/// only for a callee whose own verdict was clean (never `conditional`,
/// since standing on an already-assumed proof would launder that debt out
/// of view, §5.5).
///
/// Drains `boundary.contracted` into `boundary.stubs` (either `StubKind`)
/// (every entry becomes a `StubKind::Contracted` -- Kani's plain
/// `#[kani::stub]` cannot target a contracted function at all, so there is
/// no `unstubbable` outcome here the way D5's second branch through a
/// `ply.yaml`-declared contract has), and fills `boundary.verified` with
/// what branch one decided, for the caller to report and compose its own
/// bound against.
fn resolve_contracted_calls(
    boundary: &mut BoundaryPlan,
    is_cyclic: bool,
    known_bounded: &BTreeMap<String, u32>,
) {
    for cc in std::mem::take(&mut boundary.contracted) {
        // D1 (adversarial review, 2026-08-26): branch one is sound only
        // when the callee's own proof already covers its *entire* argument
        // space -- otherwise a caller can pass a value outside the domain
        // that proof established (a longer `Vec` than the callee's own
        // bound ever built), getting the contract assumed on an input it
        // was never checked against. A callee with any non-full-domain
        // parameter is therefore never eligible for branch one, however
        // clean its own verdict is this run -- it falls back to branch two
        // exactly like a cycle or an unclean callee does.
        let domain_covered = cc.params.iter().all(|p| p.ty.is_full_domain());
        let bound = if is_cyclic || !domain_covered {
            None
        } else {
            known_bounded.get(&cc.canonical_path).copied()
        };
        if let Some(k) = bound {
            boundary.verified.push((cc.canonical_path.clone(), k));
        }
        // Best-effort raw text for `crate::promise`'s own `requires` probe,
        // which re-parses this back into a type (`Ok` exactly when the
        // shape is one Ply's codegen can build a `kani::any()` for --
        // already guaranteed for a branch-one callee, since that is what
        // earned it its own `bounded(k)`; not guaranteed for branch two,
        // whose promise probe is then reported `W0514` "not checked" the
        // same as any other unsupported shape, never guessed at).
        let params: Vec<(String, String)> = cc
            .params
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.ty.rust_name().unwrap_or_else(|| format!("{:?}", p.ty)),
                )
            })
            .collect();
        boundary.stubs.push(StubSpec {
            callee_path: cc.canonical_path,
            params,
            return_type: cc.raw_return.unwrap_or_else(|| "()".into()),
            requires: cc.requires,
            ensures: cc.ensures,
            kind: StubKind::Contracted {
                bound,
                params: cc.params,
            },
        });
    }
}

/// Topological order (Kahn's algorithm, deterministic: ties break on node
/// id, never on `Vec` insertion order) over the call graph restricted to
/// this run's bounded-eligible, not-yet-known claims (§5.5's "within a
/// crate, verify claimed functions callees-before-callers"). Returns the
/// orderable claims callees-first, and separately the ones a cycle left
/// unorderable -- "a cycle cannot be ordered" is not a failure of this
/// function, it is the fact D5's second branch exists to catch.
fn topological_order(
    domain: &std::collections::BTreeSet<usize>,
    node_ids: &[String],
    edges: &BTreeMap<usize, std::collections::BTreeSet<usize>>,
) -> (Vec<usize>, std::collections::BTreeSet<usize>) {
    use std::collections::BTreeSet;
    // Restricted to `domain` throughout -- an earlier version of this
    // function sized everything off `node_ids.len()` (every plan, reused
    // and non-bounded-eligible ones included), so a reused or fuzz-only
    // claim with in-degree 0 by default silently entered the topological
    // order and was then run through the ordered pass unconditionally
    // (adversarial review, 2026-08-26). `domain` is the only set this
    // function may ever place a node from or return in `cyclic`.
    let mut indegree: BTreeMap<usize, usize> = domain.iter().map(|&i| (i, 0)).collect();
    for succs in edges.values() {
        for &j in succs {
            if let Some(d) = indegree.get_mut(&j) {
                *d += 1;
            }
        }
    }
    let mut ready: BTreeSet<(String, usize)> = domain
        .iter()
        .filter(|&&i| indegree[&i] == 0)
        .map(|&i| (node_ids[i].clone(), i))
        .collect();
    let mut order = Vec::new();
    let mut placed: BTreeSet<usize> = BTreeSet::new();
    while let Some(&(ref id, i)) = ready.iter().next() {
        let id = id.clone();
        ready.remove(&(id, i));
        order.push(i);
        placed.insert(i);
        if let Some(succs) = edges.get(&i) {
            for &j in succs {
                if let Some(d) = indegree.get_mut(&j) {
                    *d -= 1;
                    if *d == 0 {
                        ready.insert((node_ids[j].clone(), j));
                    }
                }
            }
        }
    }
    let cyclic: BTreeSet<usize> = domain
        .iter()
        .copied()
        .filter(|i| !placed.contains(i))
        .collect();
    (order, cyclic)
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
        pointer: None,
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
        pointer: None,
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

/// D5's first branch (§5.5): the claim is not `conditional` and owes no
/// evidence, but a clean verdict is not a standalone one -- it still rests
/// on another function's own proof, and that dependency has to be visible
/// somewhere a reader can see it rather than silently dropped. `W0517` is
/// `info`, not a warning: nothing here is owed, wrong, or worth a second
/// look, only worth naming.
fn verified_dependency_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    verified: &[(String, u32)],
) -> Diagnostic {
    let named: Vec<String> = verified
        .iter()
        .map(|(path, k)| format!("`{path}` (bounded({k}) this run)"))
        .collect();
    let list = named.join(", ");
    Diagnostic {
        code: "W0517".into(),
        severity: "info".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` earned {check_label} by standing on another function's own proof \
             instead of re-checking its body: {list}. That is real evidence, not a promise -- \
             Ply proved the callee itself this run (or found a still-valid earlier proof of it), \
             so `{fn_name}` is not marked `conditional` and owes nothing for it. But the callee's \
             own proof only went as deep as its own bound, so `{fn_name}`'s reported bound is \
             capped at the weakest of the two rather than its own declared one -- reporting more \
             would be claiming a depth nothing actually checked. (W0517, §5.5)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: verified
            .iter()
            .map(|(path, k)| Assumption {
                kind: "verified_dependency".into(),
                fn_path: path.clone(),
                verdict: format!("bounded({k})"),
                contract: String::new(),
            })
            .collect(),
        open_item: None,
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
    promise: &[PromiseFinding],
) -> Diagnostic {
    let assumed: Vec<String> = stubs.iter().map(|s| s.assumption_text()).collect();
    let list = assumed.join("; ");
    let first = &stubs[0].callee_path;
    // A clause that turned out to be true of every value is not an
    // assumption and is owed nothing. Saying otherwise here would send a
    // reader off to discharge a debt that does not exist -- and the sentence
    // this paragraph exists to write ("each one is owed evidence") would be
    // false about it.
    let empty: Vec<String> = promise
        .iter()
        .filter(|f| f.verdict == ClauseVerdict::TriviallyTrue)
        .map(|f| format!("`{}`'s `{}: {}`", f.callee, f.kind.key(), f.clause))
        .collect();
    let empty_note = if empty.is_empty() {
        String::new()
    } else {
        format!(
            " Not all of them, though, and that is why this run does not pass: {} constrained \
             nothing -- it is true of every value, so the proof assumed nothing there and there \
             is nothing to owe. E0503 below says what to do about it.",
            empty.join(" and ")
        )
    };
    Diagnostic {
        code: "W0511".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` earned {check_label}, but conditionally: the proof used the contract \
             declared for each callee it crosses into (in ply.yaml for a legacy callee, inline on \
             the callee itself for a same-crate one this run could not stand fully on), instead of \
             that callee's real \
             body. Assumed: {list}. That is what `conditional` means here -- the result holds if \
             those promises do. Nothing has checked them against the real code yet, so each one is \
             owed evidence rather than settled: an assumed contract nobody exercises is green paint.\
             {empty_note} (W0511, §5.5)"
        ),
        pointer: None,
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

/// One check, spelled the way it is written in `ply.yaml`.
fn check_spelling(c: &Check) -> String {
    match c {
        Check::Test => "test".into(),
        Check::Fuzz(n) => format!("fuzz({n})"),
        Check::Bounded(k) => format!("bounded({k})"),
        Check::Prove => "prove".into(),
        Check::Mutate => "mutate".into(),
    }
}

/// The `k` out of a `bounded(k)` verdict string, or `None` for any other
/// verdict (`fuzzed(256)`, `timeout`, a `conditional bounded(k)`'s own
/// string is still `"bounded(k)"` -- callers filter on the `conditional`
/// status separately, this only ever reads the number). Strips a trailing
/// `\u{00b7}spec-strong` decoration first: `apply_mutate_outcome` appends it
/// in place to a fully-passing claim's verdict string, and before this a
/// callee that *strengthened* its own evidence with `mutate` silently
/// vanished from every caller's `known_bounded` -- the one place D5's first
/// branch parses this string, both at reuse time and fresh (adversarial
/// review, 2026-08-26), and neither is the check that owns the decoration
/// (`record::verdict_is_earnable` already strips it for the same reason).
fn parse_bound(verdict: &str) -> Option<u32> {
    let verdict = verdict
        .strip_suffix("\u{00b7}spec-strong")
        .unwrap_or(verdict);
    verdict
        .strip_prefix("bounded(")
        .and_then(|r| r.strip_suffix(')'))
        .and_then(|n| n.parse().ok())
}

/// §5.4c: a claim whose `checks:` list is written and empty asked for
/// nothing, and nothing ran. The node reads `unclaimed`; this is the
/// sentence beside it, because a node nobody expands is not a report.
///
/// It names the default the author gave up, when there is one: the whole
/// trap was that an empty list used to *be* that default, so a reader who
/// wanted it needs to know how to ask for it back.
fn empty_checks_diag(
    node_id: &str,
    fn_name: &str,
    cf: &ContractFn,
    from_component: Option<&str>,
) -> Diagnostic {
    let default: Vec<String> = default_checks_for(cf).iter().map(check_spelling).collect();
    let whose = match from_component {
        Some(c) => format!(
            "`{fn_name}` writes no `checks:` of its own and the component `{c}` declares an empty \
             list as the default for everything inside it, so"
        ),
        None => format!("`{fn_name}` has an empty `checks:` list, so"),
    };
    let default_note = if default.is_empty() {
        String::new()
    } else {
        format!(
            "Deleting the `checks:` line entirely would run `{}`, the check Ply picks from this \
             function's shape. ",
            default.join(", ")
        )
    };
    Diagnostic {
        code: "W0515".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title: format!(
            "{whose} nothing was run against it and it earned no evidence: an empty list means \
             \"check nothing\", not \"use the default\". {default_note}Write the checks you \
             want to run it; leave the list empty to record a function you have deliberately not \
             checked, and its verdict stays `unclaimed` — Ply's word for \"nothing was checked \
             here\". (W0515, §5.4c)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![Fix {
            title: match from_component {
                Some(c) => format!(
                    "give `{fn_name}` a `checks:` list of its own, or delete the empty one on \
                     component `{c}`"
                ),
                None => format!(
                    "delete the `checks: []` line from `{fn_name}` to take the default Ply picks \
                     from its shape"
                ),
            },
            edits: vec![],
        }],
        assumptions: vec![],
        open_item: Some("declared_unchecked".into()),
    }
}

/// `verify` resolves every claim against one crate's `src/lib.rs`. A claim
/// whose component anchors elsewhere is not checked here, and that is said
/// rather than reported as a missing function (which is what happened before
/// `anchor:` was consumed -- vetting 004 s5's misleading `E0301`).
///
/// Two different things land here and they need two different sentences.
/// An anchor naming another crate is the case this diagnostic was written
/// for. An anchor naming a *module inside this crate* -- `ingest::book`
/// while verifying `ingest`, the ordinary way a nested component is
/// written -- is not another crate at all, and saying so would send a
/// reader looking for a crate that does not exist. What is true of it is
/// narrower and fixable: `verify` reads a fn key as a path from the crate
/// root, so it cannot resolve a key written relative to a module.
fn cross_crate_claim_diag(
    node_id: &str,
    fn_name: &str,
    anchor: &str,
    local_anchors: &[String],
) -> Diagnostic {
    let (crate_name, module_path) = match anchor.split_once("::") {
        Some((root, rest)) => (root.replace('-', "_"), Some(rest)),
        None => (anchor.replace('-', "_"), None),
    };
    let inside_this_crate =
        module_path.is_some() && !local_anchors.is_empty() && local_anchors.contains(&crate_name);
    let title = match module_path {
        Some(module) if inside_this_crate => format!(
            "`{fn_name}` is claimed under a component anchored at `{anchor}`, which is a module \
             inside this crate rather than the crate itself. `cargo ply verify` reads a function \
             key as a path from the crate root, so it has no way to resolve a key written relative \
             to a module: this entry's `checks:` were not run and no verdict is reported for it. \
             Move the claim to a component anchored at `{crate_name}` and spell the key from the \
             crate root -- `{module}::{fn_name}` -- and it will run. (W0303, §5.2)"
        ),
        _ => format!(
            "`{fn_name}` is claimed under a component anchored at `{anchor}`, which is not the crate \
             this run is verifying, and `cargo ply verify` checks one crate at a time. Its `checks:` \
             were not run and no verdict is reported for it. Any `requires:`/`ensures:` this entry \
             declares is still read: that is how a callee outside this crate gets a contract Ply can \
             assume at the boundary (§5.5). (W0303)"
        ),
    };
    Diagnostic {
        code: "W0303".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title,
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: if inside_this_crate {
            let module = module_path.unwrap_or_default();
            vec![
                Fix {
                    title: format!(
                        "move `{fn_name}` to a component anchored at `{crate_name}`, keyed \
                         `{module}::{fn_name}`"
                    ),
                    edits: vec![],
                },
                Fix {
                    title: format!(
                        "or drop `checks:` from this entry and keep only `requires:`/`ensures:`, if \
                         its purpose is to give `{fn_name}` a contract for its callers to assume"
                    ),
                    edits: vec![],
                },
            ]
        } else {
            vec![
                Fix {
                    title: format!(
                        "run `cargo ply verify` against the crate `{anchor}` itself to check its own \
                         claims there"
                    ),
                    edits: vec![],
                },
                Fix {
                    title: format!(
                        "or drop `checks:` from this entry and keep only `requires:`/`ensures:`, if \
                         its purpose is to give `{fn_name}` a contract for callers in this crate to \
                         assume"
                    ),
                    edits: vec![],
                },
            ]
        },
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
        pointer: None,
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

/// A same-crate contracted callee (§5.5's first two branches) whose stub
/// Ply's codegen cannot build at all -- a `self` parameter, a
/// non-identifier parameter pattern, a private module, or a contract
/// attribute Ply cannot parse. Found by adversarial review, 2026-08-26: this
/// used to fall through silently and let Kani inline the callee's real body
/// -- and everything unclaimed beneath it -- which is exactly what §5.5's
/// second honesty condition says a same-crate contracted callee can no
/// longer do, whichever branch reaches it. Refusing here, by name, is what
/// keeps that sentence true.
fn unbuildable_contracted_stub_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    unstubbable: &[(String, String, String)],
) -> Diagnostic {
    let named: Vec<String> = unstubbable
        .iter()
        .map(|(p, w, why)| format!("`{p}` (called at {w}): {why}"))
        .collect();
    let first = &unstubbable[0].0;
    Diagnostic {
        code: "W0512".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` calls {list} -- that callee carries its own contract, so §5.5's first              two branches would normally stand on it or assume it, but Ply cannot build a              stand-in for its exact shape. Descending into its real body instead would silently              give this proof the meaning of code nobody vouched for, which §5.5 refuses -- so              this check earned no evidence: the verdict is `unclaimed`, never `{check_label}`.              (W0512, §5.5)",
            list = named.join(", ")
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![Fix {
            title: format!(
                "swap `{check_label}` for `fuzz(256)` on `{fn_name}` -- the fuzz tier runs the real                  callee rather than standing in for it, so it never needs a stub of `{first}` at all"
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
        pointer: None,
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

/// What Pass 2's shared-harness build established for the crate, once
/// (misattribution fix): whether it needed rebuilding to isolate a broken
/// function's own compile failure from its crate-mates, and what it could
/// and could not pin down.
struct HarnessInfo {
    package: String,
    /// `ContractFn::ident()` -> the specific compiler error attributed to
    /// exactly that function's own generated module. A fn in this map never
    /// runs its harness test at all (its module was dropped from the crate
    /// before the build that finally succeeded) -- it is reported as a
    /// tool error against itself, never against the fns it shared a crate
    /// with.
    broken: BTreeMap<String, String>,
    /// Set only when the crate's harness still would not build and Ply
    /// could not place the failure inside any specific function's module --
    /// every claim that still needed this harness must say so honestly
    /// (§1) rather than either reporting a clean pass on an unbuilt harness
    /// or guessing which one function is at fault.
    unattributed_cause: Option<String>,
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
    harness_info: Option<&HarnessInfo>,
    has_examples: bool,
    opts: &VerifyOptions,
) -> Result<(Node, Vec<Diagnostic>)> {
    let mut diagnostics = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut statuses: Vec<String> = Vec::new();
    let mut fuzz_evidence: Option<Evidence> = None;

    for check in checks {
        match check {
            Check::Bounded(k) => {
                let mut promise_diags = Vec::new();
                let (label, mut s, mut d) = run_bounded_check(
                    cf,
                    src_dir,
                    lib_path,
                    crate_dir,
                    node_id,
                    fn_name,
                    *k,
                    boundary,
                    opts,
                    &mut promise_diags,
                )?;
                labels.push(label);
                statuses.append(&mut s);
                diagnostics.append(&mut d);
                diagnostics.append(&mut promise_diags);
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
                    pointer: None,
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
                pointer: None,
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
        } else if let Some(p) = ply_core::fuzz_gen::moved_param_read_in_ensures(cf) {
            // §5.4a: `old(param)` reads a by-value parameter's *entry*
            // value; nothing outside `old()` can read it after the call,
            // because it has been moved into it. Refused by name rather
            // than handed to codegen, which would emit a harness that
            // cannot compile (`error[E0382]: borrow of moved value`).
            diagnostics.push(moved_param_diag(node_id, fn_name, p));
            labels.push("unsupported".into());
        } else if let Some(info) = harness_info {
            let ident = cf.ident();
            if let Some(cause) = info.broken.get(&ident) {
                // Misattribution fix: this exact function's own generated
                // code is what the compiler pointed at, so it alone is
                // reported broken -- its harness test never even runs
                // (the module was dropped before the crate's remaining
                // fns were built), and no crate-mate's verdict is touched.
                if let Some(n) = wants_fuzz {
                    diagnostics.push(harness_did_not_run_diag(
                        node_id,
                        fn_name,
                        &format!("fuzz({n})"),
                        &info.package,
                        Some(cause.as_str()),
                        has_examples,
                    ));
                    labels.push("tool_error".into());
                }
                if wants_test {
                    diagnostics.push(harness_did_not_run_diag(
                        node_id,
                        fn_name,
                        "test",
                        &info.package,
                        Some(cause.as_str()),
                        has_examples,
                    ));
                    labels.push("tool_error".into());
                }
            } else if let Some(cause) = &info.unattributed_cause {
                // Ply could not isolate the failure to a specific function
                // -- honestly reported against everyone still waiting on
                // this harness, never pinned to one that might be
                // innocent (§1).
                if let Some(n) = wants_fuzz {
                    diagnostics.push(harness_unattributed_diag(
                        node_id,
                        fn_name,
                        &format!("fuzz({n})"),
                        cause,
                    ));
                    labels.push("tool_error".into());
                }
                if wants_test {
                    diagnostics.push(harness_unattributed_diag(node_id, fn_name, "test", cause));
                    labels.push("tool_error".into());
                }
            } else {
                let mut run = run_fuzz_and_test_checks(
                    cf,
                    src_dir,
                    lib_path,
                    crate_dir,
                    &info.package,
                    node_id,
                    fn_name,
                    wants_fuzz,
                    wants_test,
                    seed,
                    has_examples,
                    opts,
                )?;
                diagnostics.append(&mut run.diagnostics);
                if let Some(l) = run.fuzz_label {
                    labels.push(l);
                }
                if let Some(l) = run.test_label {
                    labels.push(l);
                }
                // §1: a verdict names the evidence that produced it. Only a
                // run that happened has any to name.
                if run.fuzz_ran && wants_fuzz.is_some() {
                    fuzz_evidence = Some(Evidence {
                        engine: "proptest".into(),
                        seed: Some(ply_core::fuzz_gen::seed_hex(seed)),
                        cases: run.fuzz_cases_reached,
                    });
                }
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
            if let Some(info) = harness_info {
                let (outcome, mut d) =
                    run_mutate_check(crate_dir, &info.package, node_id, fn_name, checks, opts)?;
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
                pointer: None,
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

    // The fuzz tier's verdict names the run that produced it (§1): the seed
    // it used, and the number of cases it actually reached. Without it,
    // `fuzzed(256)` describes a run nobody can repeat, and the run that
    // missed a bug is indistinguishable from one that could not have found
    // it. It was first written as `wants_fuzz.map(..)` -- attached whenever
    // `fuzz(n)` was *declared* -- so a check that was refused as
    // `unsupported`, abandoned by proptest, timed out, or died in a harness
    // that never compiled still reported `cases: n` for a run of zero
    // (adversarial review of the post-004 fixes, D5). `evidence` is built
    // where the run happens, or not at all.
    Ok((
        Node {
            id: fn_name.to_string(),
            kind: "fn".into(),
            verdict,
            statuses,
            reused: false,
            evidence: fuzz_evidence,
            children: vec![],
        },
        diagnostics,
    ))
}

/// Runs every promise-content probe for this proof and reads the answers
/// (§5.5). One `cargo kani` invocation per probe: the crate is already
/// compiled by then, and each probe carries no function body, so they cost
/// well under a second apiece.
fn promise_findings(plan: &PromisePlan, run_cfg: &KaniRunConfig) -> Vec<PromiseFinding> {
    if plan.is_empty() {
        return vec![];
    }
    ply_core::promise::findings(plan, |h| {
        let cfg = KaniRunConfig {
            crate_dir: run_cfg.crate_dir.clone(),
            harness_path: format!("ply_generated::{}", h.fn_name),
            // A probe over one scalar type is solved in hundredths of a
            // second (measured 2026-08-25). A minute is generous; the point
            // of capping it is that a pathological clause must not eat the
            // proof's own budget before the proof has started.
            engine_timeout_secs: run_cfg.engine_timeout_secs.min(60),
            enable_stubbing: run_cfg.enable_stubbing,
        };
        match kani::run_probe(&cfg) {
            Ok(ProbeOutcome::Holds) => HarnessAnswer::Holds,
            Ok(ProbeOutcome::Refuted) => HarnessAnswer::Refuted,
            Ok(ProbeOutcome::Undecided(why)) => HarnessAnswer::Undecided(why),
            Err(e) => HarnessAnswer::Undecided(e.to_string()),
        }
    })
}

/// §5.5's promise-content findings, in the words a user needs. Three
/// sentences per finding, in the order the newbie bar asks for: what Ply
/// looked for, what it found, and why that matters for the verdict.
fn promise_diagnostics(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    findings: &[PromiseFinding],
) -> Vec<Diagnostic> {
    findings
        .iter()
        .map(|f| match &f.verdict {
            ClauseVerdict::Unsatisfiable => {
                unsatisfiable_promise_diag(node_id, fn_name, check_label, f)
            }
            ClauseVerdict::TriviallyTrue => trivial_promise_diag(node_id, fn_name, check_label, f),
            ClauseVerdict::Undecided(why) => undecided_promise_diag(node_id, check_label, f, why),
            // `Meaningful` never reaches here -- a promise that says
            // something is the ordinary case and earns no diagnostic.
            ClauseVerdict::Meaningful => unreachable!(),
        })
        .collect()
}

fn unsatisfiable_promise_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    f: &PromiseFinding,
) -> Diagnostic {
    let callee = &f.callee;
    let clause = &f.clause;
    let what = match f.kind {
        ClauseKind::Ensures => format!(
            "Ply searched every value a `{domain}` can hold -- that is what `{callee}` returns -- \
             and found none that satisfies `{clause}`",
            domain = f.domain
        ),
        ClauseKind::Requires => format!(
            "Ply searched every combination of `{callee}`'s arguments ({domain}) and found none \
             that satisfies `{clause}`",
            domain = f.domain
        ),
    };
    Diagnostic {
        code: "E0502".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "Ply did not check `{fn_name}`: the promise declared for `{callee}` \
             cannot be true of anything. {what}. A proof that assumes something impossible proves \
             everything -- it would have come back green for `{fn_name}` whatever `{fn_name}` \
             actually does, and that green would have meant nothing. So Ply did not run it: this \
             check earned no evidence and the verdict is `unclaimed`, never `{check_label}`. Fix \
             the promise and re-run. (E0502, §5.5)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "rewrite `{callee}`'s `{key}:` entry so that at least one value satisfies it \
                     -- `{clause}` currently rules out everything",
                    key = f.kind.key()
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "or delete the entry: `{callee}` is then reported as a callee nobody has \
                     vouched for (W0512), which is a smaller claim than a false one"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("unsatisfiable_promise".into()),
    }
}

fn trivial_promise_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    f: &PromiseFinding,
) -> Diagnostic {
    let callee = &f.callee;
    let clause = &f.clause;
    let domain = &f.domain;
    let title = match f.kind {
        ClauseKind::Ensures => format!(
            "The promise declared for `{callee}` says nothing: `{clause}` is true of \
             every `{domain}`, and `{domain}` is what `{callee}` returns. Ply searched for a \
             value that would break it and there is none. Inside the proof of `{fn_name}` that \
             clause constrained nothing: `{callee}` was replaced by an arbitrary `{domain}`, so \
             `{fn_name}`'s {check_label} result is real and holds whatever `{callee}` returns -- \
             but nothing about `{callee}` was assumed, and nothing is owed on this clause. If you \
             meant to state a real property of `{callee}`, this one does not. (E0503, §5.5)"
        ),
        ClauseKind::Requires => format!(
            "The promise declared for `{callee}` says nothing: `{clause}` is true for \
             every value of its arguments ({domain}). Ply searched for a combination that would \
             break it and there is none. A `requires:` entry is what a caller must establish \
             before calling, so this one asks `{fn_name}` for nothing at all while still being \
             listed as a condition the result rests on. If you meant to state a real precondition \
             for `{callee}`, this one does not. (E0503, §5.5)"
        ),
    };
    Diagnostic {
        code: "E0503".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title,
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "replace `{clause}` with what `{callee}` actually guarantees -- a bound, a \
                     range, a relationship to its arguments"
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "or delete it: `{callee}` is then reported as a callee nobody has vouched for \
                     (W0512), which is the truth this clause was hiding"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("empty_promise".into()),
    }
}

fn undecided_promise_diag(
    node_id: &str,
    check_label: &str,
    f: &PromiseFinding,
    why: &str,
) -> Diagnostic {
    let callee = &f.callee;
    let key = f.kind.key();
    Diagnostic {
        code: "W0514".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "Ply could not tell whether the `{key}:` promise declared for `{callee}` says \
             anything at all. Ply normally asks two questions about a declared promise -- can any \
             value satisfy it, and can any value break it -- so that a promise which is \
             impossible, or trivially true, is caught before a proof rests on it. Here it could \
             not ask: {why}. So that promise is reported as unchecked, not as sound: the verdict \
             beside it still assumes it. (W0514, §5.5)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("promise_not_checked".into()),
    }
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
        pointer: None,
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
        pointer: None,
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
    // §5.5's promise-content findings go straight into the caller's list
    // rather than travelling back through the return value. Whatever the
    // proof then said -- verified, violated, timed out -- a promise that
    // says nothing is a defect in the document, and dropping it on the
    // unhappy path would be exactly the quiet failure this gate exists to
    // stop.
    promise_out: &mut Vec<Diagnostic>,
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
    if !boundary.unstubbable_contracted.is_empty() {
        return Ok((
            "unclaimed".into(),
            vec![],
            vec![unbuildable_contracted_stub_diag(
                node_id,
                fn_name,
                &check_label,
                &boundary.unstubbable_contracted,
            )],
        ));
    }

    // D5's first branch (§5.5): "never report evidence stronger than the
    // weakest thing it rests on". `f`'s own proof only holds *given* each
    // stub-verified callee's contract, and that was only established to the
    // depth its own proof earned -- so the honest bound this claim can
    // report is capped there, never left at its own declared `bound_k`.
    let composed_k = boundary
        .verified
        .iter()
        .map(|(_, k)| *k)
        .fold(bound_k, u32::min);

    let generated = harness::generate_proof_module(cf, bound_k, &boundary.stubs)?;
    harness::write_generated_module(src_dir, lib_path, &generated.module_source)?;

    let engine_timeout_secs = opts.engine_timeout_secs.unwrap_or_else(|| {
        default_engine_timeout_secs(cf.has_vec_param(), bound_k, !generated.stubbed.is_empty())
    });

    let run_cfg = KaniRunConfig {
        crate_dir: crate_dir.to_path_buf(),
        harness_path: generated.proof_fn_path.clone(),
        engine_timeout_secs,
        enable_stubbing: !generated.stubbed.is_empty(),
    };

    // §5.5's promise-content gate, before the proof rather than after it: a
    // proof that rests on a promise nothing can satisfy holds vacuously, so
    // running it would produce a green verdict that means nothing. The
    // probes carry no function body and solve in well under a second each
    // (measured 2026-08-25), and they ride in the same generated module, so
    // the crate is compiled once for the whole set.
    let promise_findings = promise_findings(&generated.promise, &run_cfg);
    promise_out.append(&mut promise_diagnostics(
        node_id,
        fn_name,
        &check_label,
        &promise_findings,
    ));
    if promise_findings
        .iter()
        .any(|f| f.verdict == ClauseVerdict::Unsatisfiable)
    {
        return Ok(("unclaimed".into(), vec![], vec![]));
    }

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

    // Only D5's second branch (`Assumed`) costs a symbolic value in place of
    // the real callee and owes evidence -- `Verified` (branch one) is real
    // evidence the caller does not owe anything for, so it must not be
    // named as the cost of a timeout, or counted toward `conditional`.
    let assumed: Vec<StubSpec> = generated
        .stubbed
        .iter()
        .filter(|s| s.is_assumed())
        .cloned()
        .collect();

    match outcome {
        KaniOutcome::Verified => {
            let composed_label = format!("bounded({composed_k})");
            let mut ds = Vec::new();
            // D5's first branch: real evidence, and the caller is not
            // conditional for it -- but a clean verdict is not a standalone
            // one, so the dependency still has to appear somewhere a reader
            // can see it (§5.5).
            if !boundary.verified.is_empty() {
                ds.push(verified_dependency_diag(
                    node_id,
                    fn_name,
                    &composed_label,
                    &boundary.verified,
                ));
            }
            if assumed.is_empty() {
                Ok((composed_label, vec![], ds))
            } else {
                // §5.5's second branch: real evidence, resting on a
                // declared assumption. `conditional` is a status (D6), not
                // a weaker rung -- the verdict stays `bounded(k)` and the
                // assumption travels beside it.
                ds.push(conditional_verdict_diag(
                    node_id,
                    fn_name,
                    &composed_label,
                    &assumed,
                    &promise_findings,
                ));
                Ok((
                    composed_label,
                    vec!["conditional".into(), "owed-evidence".into()],
                    ds,
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
                title: kani_timeout_title(fn_name, engine_timeout_secs, &assumed),
                pointer: None,
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
                pointer: None,
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
                        // `display_name`, never `rust_name`: the latter is
                        // `None` for exactly the shapes that reach this
                        // branch, and `unwrap_or_default` printed "`xs: `" --
                        // a message that names a parameter and then omits the
                        // type the reader needs (same defect class as D4 of
                        // the 2026-08-25 review, one diagnostic over).
                        .map(|p| format!("`{}: {}`", p.name, p.ty.display_name()))
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
                        pointer: None,
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
                pointer: None,
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

/// What one invocation of the generated harness crate established. The two
/// fields beyond the labels exist because §8's `evidence` block must
/// describe a run that happened: `fuzz_ran` says whether the harness
/// executed at all, and `fuzz_cases_reached` is the number of cases proptest
/// actually accepted and checked -- never the number the checks list asked
/// for (adversarial review of the post-004 fixes, D5).
struct HarnessRun {
    fuzz_label: Option<String>,
    test_label: Option<String>,
    fuzz_ran: bool,
    fuzz_cases_reached: Option<u32>,
    diagnostics: Vec<Diagnostic>,
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
    has_examples: bool,
    opts: &VerifyOptions,
) -> Result<HarnessRun> {
    let timeout = opts
        .engine_timeout_secs
        .unwrap_or_else(default_secondary_engine_timeout_secs);
    let filter = format!("{fn_name}_harness::");
    let run = fuzz_engine::run_harness_tests(crate_dir, harness_pkg, &filter, timeout)?;

    let mut diagnostics = Vec::new();
    let fuzz_test_name = format!("{fn_name}_harness::ply_fuzz_{fn_name}");
    let mut fuzz_label = None;
    let mut test_label = None;
    let mut fuzz_cases_reached: Option<u32> = None;

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
                has_examples,
            ));
        }
        if wants_test {
            diagnostics.push(harness_did_not_run_diag(
                node_id,
                fn_name,
                "test",
                harness_pkg,
                cause.as_deref(),
                has_examples,
            ));
        }
        return Ok(HarnessRun {
            fuzz_label: wants_fuzz.map(|_| "tool_error".to_string()),
            test_label: if wants_test {
                Some("tool_error".to_string())
            } else {
                None
            },
            // Zero cases ran, so there is no run to name in §8's `evidence`.
            fuzz_ran: false,
            fuzz_cases_reached: None,
            diagnostics,
        });
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
                pointer: None,
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
                pointer: None,
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
            // The cases proptest really did check before giving up. It is a
            // small number beside a `fuzz(256)` declaration, and that is the
            // point: it is the one that happened.
            fuzz_cases_reached = Some(accepted);
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
                    pointer: None,
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
            fuzz_cases_reached = Some(n);
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
                pointer: None,
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
                pointer: None,
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

    Ok(HarnessRun {
        fuzz_label,
        test_label,
        fuzz_ran: true,
        fuzz_cases_reached,
        diagnostics,
    })
}

/// The `X0901` a check earns when its generated harness never ran a single
/// case (2026-08-24 M4 review, D1). Written to the newbie bar: what
/// happened, what it means for the verdict, and the compiler's own words --
/// then concrete `fixes`, per §8's non-result rule ("a non-result is still
/// feedback").
///
/// `has_examples` gates the one sentence that names a *specific* likely
/// cause: an `examples:` entry that does not type-check is a real, common
/// way to break this build, but only when `{fn_name}` actually declares any
/// -- stating it as "the usual cause" regardless used to run even on a
/// crate with no `examples:` entries at all, which is a cause Ply never
/// established (misattribution fix, 2026-08-26).
fn harness_did_not_run_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    harness_pkg: &str,
    cause: Option<&str>,
    has_examples: bool,
) -> Diagnostic {
    let compiler_says = match cause {
        Some(c) => format!(" The compiler's own first error was: {c}."),
        None => String::new(),
    };
    let examples_hint = if has_examples {
        format!(
            " `{fn_name}` declares `examples:` entries in ply.yaml, which compile exactly as \
             written -- they are ordinary Rust `==` expressions, never type-checked before \
             codegen -- so a wrong type or a typo there is one thing worth checking first."
        )
    } else {
        String::new()
    };
    let mut fixes = vec![Fix {
        title: format!(
            "see the full compiler output by running `cargo test -p {harness_pkg} --lib \
             {fn_name}_harness::` from the crate root (Ply regenerates that harness crate on \
             every run, so editing it is never the fix)"
        ),
        edits: vec![],
    }];
    if has_examples {
        fixes.insert(
            0,
            Fix {
                title: format!(
                    "check every `examples:` entry for `{fn_name}` in ply.yaml -- each one must \
                     compile as a Rust expression against `{fn_name}`'s real parameter and return \
                     types"
                ),
                edits: vec![],
            },
        );
    }
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
             there is no failing input to show.{compiler_says}{examples_hint} (X0901)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes,
        assumptions: vec![],
        open_item: Some("tool_error".into()),
    }
}

/// The honest fallback (misattribution fix): the shared harness crate would
/// not build, and Ply could not place the failure inside any one function's
/// own generated module -- so it says that plainly, against every claim
/// still waiting on the harness, rather than pinning the blame on one
/// function that might be entirely innocent.
fn harness_unattributed_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    cause: &str,
) -> Diagnostic {
    Diagnostic {
        code: "X0901".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}`'s `{check_label}` check ran zero cases: the generated test harness this \
             crate's checks share failed to compile, so nothing in it ran -- including \
             `{fn_name}`'s own tests, even though Ply could not tell whether `{fn_name}`'s own \
             generated code is what broke it. Rather than guess and blame a function that might \
             be completely fine, Ply reports every function still waiting on this harness as a \
             tool error. The compiler's own first error was: {cause}. (X0901)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![Fix {
            title: "run `cargo build --tests` in the crate root to see the full compiler output, \
                    then fix whichever function it names -- every other claim in this crate will \
                    be checked again once the harness builds"
                .to_string(),
            edits: vec![],
        }],
        assumptions: vec![],
        open_item: Some("tool_error".into()),
    }
}

/// §5.4a: `old(param)` reads a by-value parameter's *entry* value; nothing
/// outside `old()` can read it after the call, because a non-`Copy`
/// parameter taken by value has been moved into it. Refused by name
/// (`V0506`) rather than hand it to codegen, which would emit a harness
/// that cannot compile (`error[E0382]: borrow of moved value`).
fn moved_param_diag(node_id: &str, fn_name: &str, p: &Param) -> Diagnostic {
    let ty = p.ty.display_name();
    let pname = &p.name;
    Diagnostic {
        code: "V0506".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}`'s postcondition reads `{pname}` after `{pname}` has already been moved \
             into the call: `{pname}: {ty}` is passed by value, so once `{fn_name}({pname})` \
             returns, the original `{pname}` no longer exists for the postcondition to read. Ply \
             refuses to generate a test for this rather than write code that cannot compile. \
             (V0506)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "wrap the read in `old({pname})` if the postcondition means `{pname}`'s value \
                     before the call -- `old(expr)` is captured before `{fn_name}` runs, while \
                     `{pname}` still exists"
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "or change `{pname}`'s type to a reference (`&{ty}`) if `{fn_name}` only needs \
                     to read it, not own it -- a borrowed parameter is still there after the call"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("unsupported".into()),
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
                pointer: None,
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
                    pointer: None,
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
                    title: unrenderable_inputs_title(fn_name, &contract_text, &cf.params),
                    pointer: None,
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

/// `K0601`'s words. A timeout on a *stubbed* proof has a cause the reader
/// cannot see from the body in front of them -- the assumption they declared
/// is what turned a concrete callee into a symbolic value -- so the message
/// says it, with the numbers (2026-08-25, adversarial review G1).
fn kani_timeout_title(fn_name: &str, secs: u32, stubbed: &[StubSpec]) -> String {
    let mut title = format!(
        "Kani could not finish checking `{fn_name}` within its {secs}s time budget -- this is an \
         exhausted search, not a broken promise: Kani never got far enough to say whether the \
         contract holds or not, so this is reported as `timeout`, never as a violation. (K0601)"
    );
    if let Some(first) = stubbed.first() {
        let names: Vec<String> = stubbed
            .iter()
            .map(|s| format!("`{}`", s.callee_path))
            .collect();
        title.push_str(&format!(
            " This proof stood in for {list} with the contract declared for it in ply.yaml, which \
             is what makes the verdict `conditional` -- and it is also why it costs more than the \
             same function with the call removed: the stub hands Kani a symbolic value constrained \
             only by that contract, where the real body returns a handful of concrete ones, and \
             less information is more work. Measured on a stubbed proof of this shape: 201.77s \
             (vetting 004's `tier_fee_cents`) against 9.72s for a smaller body, so the cost is the \
             body's as much as the stub's. If `{first_callee}`'s contract is wider than the real \
             code needs, narrowing it is the cheapest thing that helps.",
            list = names.join(", "),
            first_callee = first.callee_path
        ));
    }
    title
}

/// `W0541`'s words. Extracted and made shape-aware 2026-08-25 (adversarial
/// review of the post-004 fixes, D4): the message used to say Ply "has no way
/// yet to spell a `BTreeSet`, or a `Vec` of anything but `u8`, as a literal
/// value", which was true for the only shape that could reach it when it was
/// written and false the moment `char`, `Option`, `Result` and `[T; N]`
/// entered the fragment without witness decoders. That is exactly the defect
/// class the M4 review's D7 closed once already -- a diagnostic whose words
/// are false for the case in front of the reader -- so this names the
/// parameters and types that actually blocked the rendering, the way the Kani
/// side's `X0901` already does.
fn unrenderable_inputs_title(
    fn_name: &str,
    contract_text: &str,
    params: &[harness::Param],
) -> String {
    let blocked: Vec<String> = params
        .iter()
        .filter(|p| !p.ty.is_witness_renderable())
        .map(|p| format!("`{}: {}`", p.name, p.ty.display_name()))
        .collect();
    // The fallback covers the one case the filter cannot explain: every
    // parameter is renderable in principle and the engine's own text still
    // would not parse back. Saying "that input" is vague, but a vague true
    // sentence beats a precise false one.
    let what = if blocked.is_empty() {
        "that input".to_string()
    } else {
        format!("parameter(s) {}", blocked.join(", "))
    };
    format!(
        "`{fn_name}` fails its own contract `{contract_text}` for at least one input, and proptest \
         shrank that input down to the smallest one that still fails. Ply cannot turn it into a \
         runnable Rust test, though: it has no way yet to write {what} back out as a literal value \
         in Rust source. The failing input is recorded below exactly as the engine reported it -- \
         Ply never invents one. (W0541, reason: inputs_unrenderable)"
    )
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
    /// The run produced no verdict either way, and the payload says which
    /// absence it was: `engine-missing`, `timeout`, `tool_error`, or plain
    /// `inconclusive` (the run completed and found nothing to mutate). All
    /// four are absences of evidence (§1) and fail the run; naming them
    /// apart is what lets §6's exit table tell "no engine" (3) from "the
    /// tool broke" (2) from "it ran and settled nothing" (1), which a single
    /// `inconclusive` could not (adversarial review of the post-004 fixes,
    /// D2).
    Inconclusive(&'static str),
}

fn apply_mutate_outcome(verdict: &mut String, statuses: &mut Vec<String>, outcome: MutateOutcome) {
    match outcome {
        MutateOutcome::SpecStrong => verdict.push_str("\u{00b7}spec-strong"),
        MutateOutcome::WeakSpec => statuses.push("weak-spec".into()),
        // Nothing was established either way: the engine never reported a
        // mutant count. These are D6's own statuses for that, and none of
        // them is `weak-spec`, which asserts a real finding. Each is an
        // absence of evidence, so the run fails (§1) -- the verdict itself
        // is untouched, because the `test`/`fuzz` check that produced it
        // really did run.
        MutateOutcome::Inconclusive(status) => statuses.push(status.to_string()),
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
            MutateOutcome::Inconclusive("engine-missing"),
            vec![Diagnostic {
                code: "W0110".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "cargo-mutants".into(),
                check: "mutate".into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` declares `mutate`, but `cargo-mutants` is not installed -- run \
                     `cargo install cargo-mutants --locked` (see `cargo ply doctor`). This is \
                     reported as a missing engine and never as a failure of the check itself: \
                     nothing here says the spec is weak. It does mean the `mutate` check `{fn_name}` \
                     declares produced no evidence, so the run does not pass -- it exits 3, the \
                     code §6 reserves for an explicitly requested check with no engine behind it."
                ),
                pointer: None,
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
                    MutateOutcome::Inconclusive("inconclusive"),
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
                        pointer: None,
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
                        pointer: None,
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
                MutateOutcome::Inconclusive("timeout"),
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
                    pointer: None,
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
                MutateOutcome::Inconclusive("tool_error"),
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
                    pointer: None,
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

/// Whether this run earned something worth recording (§5.2a).
///
/// Only a result that **earned evidence** is stored. A violation is a real
/// result and still not stored: its whole value is the witness and the red
/// test beside it, and those are artifacts on disk that a later run would
/// have to re-render anyway. Everything else that could be stored here is an
/// absence — a timeout, a missing engine, a shape Ply cannot build, a
/// harness that would not compile, a claim that asked for nothing — and
/// carrying an absence forward would mean reporting "nothing was checked"
/// without having looked.
///
/// The absence vocabulary is `ply_core::diag::is_absence`, the same one the
/// exit code reads. Two copies of it is how the next absence gets missed by
/// one of them (§1: an absence is a name, not a slot).
fn earned_evidence(node: &Node, diagnostics: &[Diagnostic]) -> bool {
    !ply_core::diag::is_absence(&node.verdict)
        && node.verdict != "violation"
        && !node.statuses.iter().any(|s| ply_core::diag::is_absence(s))
        && !diagnostics.iter().any(|d| d.severity == "error")
}

fn leaf_node(node_id: &str, verdict: &str) -> Node {
    let fn_part = node_id.rsplit("::").next().unwrap_or(node_id);
    Node {
        id: fn_part.to_string(),
        kind: "fn".into(),
        verdict: verdict.to_string(),
        statuses: vec![],
        reused: false,
        evidence: None,
        children: vec![],
    }
}

#[allow(dead_code)]
fn unused(_p: &PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// D6 (adversarial review, 2026-08-26): a `·spec-strong`-decorated verdict
    /// must still parse to its bound, or a mutation-tested callee silently
    /// drops out of `known_bounded` and every caller standing on it falls
    /// back to branch two with no explanation.
    #[test]
    fn a_spec_strong_decorated_bound_still_parses_to_its_number() {
        assert_eq!(parse_bound("bounded(2)\u{00b7}spec-strong"), Some(2));
        assert_eq!(parse_bound("bounded(2)"), Some(2));
        assert_eq!(parse_bound("fuzzed(64)\u{00b7}spec-strong"), None);
        assert_eq!(parse_bound("violation"), None);
    }

    /// An empty list inherited from a component default is not the fn's
    /// own line, and the sentence must not send a reader to a line that is
    /// not there.
    #[test]
    fn an_inherited_empty_list_names_the_component_that_declared_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(
            &path,
            "#[ply::ensures(|result| *result == x)]\npub fn quote(x: u32) -> u32 { x }\n",
        )
        .unwrap();
        let cf = ply_core::harness::discover_fn(&path, "quote").unwrap();
        let d = empty_checks_diag("pricing::quote", "quote", &cf, Some("pricing"));
        assert!(
            d.title.starts_with(
                "`quote` writes no `checks:` of its own and the component `pricing` declares an \
                 empty list as the default for everything inside it, so nothing was run against \
                 it"
            ),
            "{}",
            d.title
        );
    }

    fn node_with(verdict: &str, statuses: &[&str]) -> Node {
        Node {
            id: "f".into(),
            kind: "fn".into(),
            verdict: verdict.into(),
            statuses: statuses.iter().map(|s| (*s).to_string()).collect(),
            reused: false,
            evidence: None,
            children: vec![],
        }
    }

    fn diag_with_severity(severity: &str) -> Diagnostic {
        Diagnostic {
            code: "E0000".into(),
            severity: severity.into(),
            phase: "verify".into(),
            engine: "ply".into(),
            check: "bounded(2)".into(),
            node_id: "c::f".into(),
            title: "t".into(),
            pointer: None,
            primary_span: None,
            counterexample: None,
            fixes: vec![],
            assumptions: vec![],
            open_item: None,
        }
    }

    /// §5.2a: only a result that earned evidence is recorded, so nothing
    /// that failed can ever be carried forward into a later run. Written as
    /// one table because the interesting cases are the ones nobody thinks
    /// to list -- an absence recorded as a *status* beside a real verdict,
    /// and a violation, which is a real result and still must not be stored.
    #[test]
    fn only_a_result_that_earned_evidence_is_recorded() {
        let cases: [(Node, Vec<Diagnostic>, bool, &str); 9] = [
            (
                node_with("bounded(2)", &[]),
                vec![],
                true,
                "a clean proof is the case reuse exists for",
            ),
            (
                node_with("bounded(2)", &["conditional", "owed-evidence"]),
                vec![diag_with_severity("warning")],
                true,
                "a result resting on a declared promise is real evidence about that promise, \
                 and the most expensive kind to re-earn",
            ),
            (
                node_with("fuzzed(256)", &["weak-spec"]),
                vec![diag_with_severity("warning")],
                true,
                "a weak spec is a finding beside real evidence, not an absence of it",
            ),
            (
                node_with("violation", &[]),
                vec![diag_with_severity("error")],
                false,
                "a violation is a real result whose whole value is the witness beside it -- \
                 never carried forward as a bare verdict",
            ),
            (
                node_with("violation", &[]),
                vec![],
                false,
                "and never stored even if nothing else flagged it -- the rule is the verdict, \
                 not the diagnostic that usually accompanies it",
            ),
            (
                node_with("timeout", &[]),
                vec![],
                false,
                "an exhausted engine learned nothing; storing that would report `nothing was \
                 checked` without having looked",
            ),
            (
                node_with("fuzzed(64)", &["engine-missing"]),
                vec![],
                false,
                "an absence recorded as a status is an absence (§1: a name, not a slot)",
            ),
            (node_with("unclaimed", &[]), vec![], false, "nothing ran"),
            (
                node_with("bounded(2)", &[]),
                vec![diag_with_severity("error")],
                false,
                "an error-severity finding beside a verdict means the run did not stand behind \
                 it either",
            ),
        ];
        for (node, diags, expected, why) in cases {
            assert_eq!(
                earned_evidence(&node, &diags),
                expected,
                "{why} (verdict `{}`, statuses {:?})",
                node.verdict,
                node.statuses
            );
        }
    }

    /// The ordinary way a nested component is written is an anchor naming a
    /// module of the crate being verified. Calling that "not the crate this
    /// run is verifying" is false, and sends a reader hunting for a crate
    /// that does not exist -- so it gets its own sentence, and the sentence
    /// says what to write instead.
    #[test]
    fn a_claim_under_a_module_anchor_is_told_it_is_a_module_not_another_crate() {
        let d = cross_crate_claim_diag(
            "ingest.book::OrderBook::apply",
            "OrderBook::apply",
            "ingest::book",
            &["ingest".to_string()],
        );
        assert_eq!(
            d.title,
            "`OrderBook::apply` is claimed under a component anchored at `ingest::book`, which is \
             a module inside this crate rather than the crate itself. `cargo ply verify` reads a \
             function key as a path from the crate root, so it has no way to resolve a key written \
             relative to a module: this entry's `checks:` were not run and no verdict is reported \
             for it. Move the claim to a component anchored at `ingest` and spell the key from the \
             crate root -- `book::OrderBook::apply` -- and it will run. (W0303, §5.2)"
        );
        assert_eq!(
            d.fixes[0].title,
            "move `OrderBook::apply` to a component anchored at `ingest`, keyed \
             `book::OrderBook::apply`"
        );
    }

    /// An anchor that really does name another crate keeps the sentence
    /// written for it.
    #[test]
    fn a_claim_under_another_crates_anchor_still_says_another_crate() {
        let d = cross_crate_claim_diag(
            "ledger::post",
            "post",
            "ledger",
            &["ingest".to_string(), "ingest".to_string()],
        );
        assert_eq!(
            d.title,
            "`post` is claimed under a component anchored at `ledger`, which is not the crate this \
             run is verifying, and `cargo ply verify` checks one crate at a time. Its `checks:` \
             were not run and no verdict is reported for it. Any `requires:`/`ensures:` this entry \
             declares is still read: that is how a callee outside this crate gets a contract Ply \
             can assume at the boundary (§5.5). (W0303)"
        );
    }

    #[test]
    fn engine_timeout_scales_only_for_vec_shaped_harnesses() {
        assert_eq!(
            default_engine_timeout_secs(false, 2, false),
            60,
            "scalar-only stays at the M3 default"
        );
        assert_eq!(
            default_engine_timeout_secs(true, 8, false),
            150,
            "the M3 review measured bounded(8) over Vec<u8> needing 150s -- the formula must reproduce that exactly"
        );
        assert_eq!(default_engine_timeout_secs(true, 2, false), 60);
    }

    /// §5.5's second branch was dead at the tool's own defaults: vetting
    /// 004's `tier_fee_cents` is scalar-signature, so it got 60s, and its
    /// stubbed proof needs 201.77s. The measured cost is what the number has
    /// to clear -- so the test says the measurement, not the constant.
    #[test]
    fn a_stubbed_harness_gets_a_budget_that_clears_the_measured_conditional_proof() {
        const MEASURED_CONDITIONAL_PROOF_SECS: u32 = 202;
        assert!(
            default_engine_timeout_secs(false, 2, true) > MEASURED_CONDITIONAL_PROOF_SECS,
            "a `conditional` proof that cannot finish at the default budget is a feature nobody \
             can run without reading the source for a flag: got {}s for a proof measured at {}s",
            default_engine_timeout_secs(false, 2, true),
            MEASURED_CONDITIONAL_PROOF_SECS
        );
        assert_eq!(
            default_engine_timeout_secs(false, 2, false),
            60,
            "and an unstubbed scalar harness is untouched -- the premium is for the shape that \
             earns it, not a flat raise for everything"
        );
        assert!(
            default_engine_timeout_secs(true, 8, true)
                >= default_engine_timeout_secs(true, 8, false),
            "a stubbed Vec harness never gets *less* than the same harness unstubbed"
        );
    }

    /// The reader of a timed-out `conditional` proof cannot see the cause in
    /// the body in front of them: the assumption they declared is what turned
    /// a concrete callee into a symbolic value.
    #[test]
    fn a_timeout_on_a_stubbed_proof_names_the_assumption_as_the_cost() {
        let stub = StubSpec {
            callee_path: "ledger::fees::bps_for_tier".into(),
            params: vec![("tier".into(), "u8".into())],
            return_type: "u32".into(),
            requires: vec![],
            ensures: vec!["|result| *result <= 10_000".into()],
            kind: StubKind::Assumed,
        };
        let title = kani_timeout_title("tier_fee_cents", 300, std::slice::from_ref(&stub));
        assert!(
            title.contains("`ledger::fees::bps_for_tier`"),
            "name the callee that was stood in for: {title}"
        );
        assert!(
            title.contains("symbolic value constrained only by that contract"),
            "and say plainly why standing in for it costs more: {title}"
        );
        assert!(
            title.contains("201.77s") && title.contains("9.72s"),
            "with both measurements, so the reader can tell a stub premium from a heavy body: \
             {title}"
        );
        let plain = kani_timeout_title("clamp", 60, &[]);
        assert!(
            !plain.contains("stood in for"),
            "an unstubbed timeout must not carry an explanation that does not apply to it: {plain}"
        );
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
        apply_mutate_outcome(
            &mut verdict,
            &mut statuses,
            MutateOutcome::Inconclusive("inconclusive"),
        );
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

    fn param(name: &str, ty: ply_core::harness::RustType) -> harness::Param {
        harness::Param {
            name: name.into(),
            ty,
            by_ref: false,
        }
    }

    /// The wording must be true for the shape in front of the reader. This
    /// is the M4 review's D7 defect class, and `W0541` reintroduced it for
    /// every shape the 2026-08-25 fragment widening admitted.
    #[test]
    fn w0541_names_the_parameter_and_type_that_blocked_the_rendering() {
        use ply_core::harness::RustType;
        let title = unrenderable_inputs_title(
            "carded_fee",
            "|result| *result <= 10_000",
            &[
                param("amount_cents", RustType::U32),
                param("card_bps", RustType::Array(Box::new(RustType::U32), 4)),
            ],
        );
        assert!(
            title.contains("parameter(s) `card_bps: [u32; 4]`"),
            "the array is what Ply cannot spell, and the message must say so: {title}"
        );
        assert!(
            !title.contains("amount_cents"),
            "a `u32` renders fine -- naming it would send the reader after the wrong parameter: \
             {title}"
        );
        assert!(
            !title.contains("BTreeSet"),
            "and it must not describe a type this function does not have: {title}"
        );
    }

    #[test]
    fn w0541_still_names_the_btreeset_case_it_was_written_for() {
        use ply_core::harness::RustType;
        let title = unrenderable_inputs_title(
            "count",
            "|result| *result == xs.len() as u32",
            &[param("xs", RustType::BTreeSet(Box::new(RustType::U8)))],
        );
        assert!(
            title.contains("parameter(s) `xs: BTreeSet<u8>`"),
            "including the `BTreeSet<u8>` spelling the M4 review's D7 fixed once already: {title}"
        );
    }

    #[test]
    fn worst_of_picks_the_weakest_child_not_the_strongest() {
        let children = vec![
            Node {
                id: "a".into(),
                kind: "fn".into(),
                verdict: "bounded(2)".into(),
                statuses: vec![],
                reused: false,
                evidence: None,
                children: vec![],
            },
            Node {
                id: "b".into(),
                kind: "fn".into(),
                verdict: "tested".into(),
                statuses: vec![],
                reused: false,
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
