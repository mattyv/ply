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
use ply_core::diag::{Assumption, Counterexample, Diagnostic, Envelope, Evidence, Fix, Node, Span};
use ply_core::engines::fuzz as fuzz_engine;
use ply_core::engines::kani::ProbeOutcome;
use ply_core::engines::kani::{self, KaniOutcome, KaniRunConfig};
use ply_core::engines::mutants::{self, MutantsRunConfig, MutantsRunOutcome};
use ply_core::harness::{self, ContractFn, Param, RustType, StubKind, StubSpec};
use ply_core::harness_crate;
use ply_core::model::{
    Check, Component, Document, FnClaim, InheritedChecks, component_default_checks,
    effective_checks,
};
use ply_core::promise::{ClauseKind, ClauseVerdict, HarnessAnswer, PromiseFinding, PromisePlan};
use ply_core::reach;
use ply_core::record::{self, AssumedPromise, EngineId, FingerprintInputs, Match, RecordEntry};

use crate::shared::{self, declared_contracts, local_anchor_names, sorted_by_key};

/// Ply's own identity, as §5.2a input 11 needs it: not the hand-edited
/// `version` field in `Cargo.toml` (fourteen false-clean fixes on this
/// branch never moved it, docs/review-silent-narrowing.md §6), but a hash
/// of the source that decides what a verdict means, computed once at
/// compile time by `build.rs` and baked in here. There is no fallback
/// string -- if `build.rs` could not compute one, the build itself failed,
/// so reaching this line at all means a real identity was computed.
pub const PLY_VERSION: &str = env!("PLY_BUILD_ID");

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
    /// Where every probe above must run. The rustup shims resolve a
    /// toolchain from the current directory, and the engines run against
    /// this crate -- so probing anywhere else records a compiler and engine
    /// that were never the ones used (external review, 2026-08-30).
    crate_dir: std::path::PathBuf,
}

impl Toolchain {
    fn probe(crate_dir: &Path) -> Toolchain {
        let (rustc, target) = rustc_identity(crate_dir);
        Toolchain {
            target,
            rustc,
            features: declared_features(crate_dir),
            kani: std::cell::OnceCell::new(),
            mutants: std::cell::OnceCell::new(),
            crate_dir: crate_dir.to_path_buf(),
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
                        .get_or_init(|| kani::version(&self.crate_dir))
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
                        .get_or_init(|| mutants::version(&self.crate_dir))
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
/// Probed **in the crate being verified**, not in whatever directory the
/// user happened to run from.
///
/// The rustup shim picks a toolchain from the current directory, and the
/// engines run `cargo test` with `current_dir` set to the target crate. So
/// probing from the caller's cwd recorded a different compiler than the one
/// Cargo actually used, whenever a `rust-toolchain.toml` sat in the project
/// and the caller was somewhere else. Both directions were demonstrated:
/// stored evidence survived a real change to the project's toolchain, and a
/// re-run from inside the project reported the compiler as changed when only
/// the shell's directory had (external review, 2026-08-30).
///
/// The fingerprint decides whether a recorded verdict may be carried
/// forward. Recording a compiler that never compiled anything here is the
/// kind of quiet wrongness that lets stale evidence look current.
fn rustc_identity(crate_dir: &Path) -> (String, String) {
    let out = std::process::Command::new("rustc")
        .arg("-vV")
        .current_dir(crate_dir)
        .output();
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

/// The completed verification together with the immutable inputs publication
/// needs but the editor-neutral §8 envelope deliberately does not contain.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// The exact parsed document this verification used. Callers must render
    /// this value, never reload `ply.yaml` after verification completes.
    pub document: Document,
    pub envelope: Envelope,
    /// Qualified claim id (`component.path::function-key`) to exact source.
    pub source_map: BTreeMap<String, Span>,
}

/// Verifies one config snapshot and returns that same parsed document for
/// publication. This is the integration API for visual publication.
pub fn verify_crate_result(crate_dir: &Path, opts: &VerifyOptions) -> Result<VerificationResult> {
    let yaml_path = crate_dir.join("ply.yaml");
    let file = config::load(&yaml_path)?;
    verify_loaded_crate(crate_dir, opts, file)
}

fn verify_loaded_crate(
    crate_dir: &Path,
    opts: &VerifyOptions,
    file: Document,
) -> Result<VerificationResult> {
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
    let mut source_map: BTreeMap<String, Span> = BTreeMap::new();

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
            // A `Type::method` claim that names something real but out of
            // this task's scope (a receiver, a generic `impl` block, a
            // trait method) or that Ply's syntactic reader cannot pick
            // between (two `impl` blocks defining the same name): checked
            // *before* `discover_fn_with`, and given its own diagnostic,
            // because "could not find the function" (E0301, below) would be
            // false about every one of these -- Ply found it and is
            // refusing it for a stated reason, which is a different fact a
            // reader needs a different sentence for. `discover_fn_with`
            // still sees exactly the same three outcomes it always has
            // (found, opaque, not-found) for everything that reaches it.
            let mut cf = match resolver.lookup_fn(fn_name) {
                Resolution::Refused(reason) => {
                    // A `&self` method is exactly this refusal's own shape
                    // (`callgraph::receiver_refusal_reason`) -- before
                    // reporting it, try the second, narrower path
                    // (docs/review-self-construction.md's "fourth option"):
                    // a constructor plus a bounded sequence of the type's
                    // own operations, read straight from the one module file
                    // the claim names. It fails closed for every other
                    // refusal reason (a trait-impl method, a generic `impl`
                    // block, a `&mut self` target) by simply not finding
                    // what it is looking for, so falling back to `reason`
                    // below is always the right thing on its own `Err`.
                    match harness::discover_method_with_receiver(crate_dir, fn_name) {
                        Ok(cf) => cf,
                        // Two kinds of `Err` here, and they need two
                        // different sentences (2026-08-27). A `NoConstructor`/
                        // `UnsupportedConstructorParam`/`UnsupportedParamPattern`
                        // is this scan's *own* finding -- it read the type,
                        // found real associated functions, and named exactly
                        // which one blocked it, which is a sharper, truer
                        // sentence than the resolver's generic "constructing
                        // a receiver is not supported yet" and must be shown
                        // instead of it. Every other variant
                        // (`MethodNotFound`, `MutableOrOwnedReceiver`,
                        // `UnsupportedModulePath`, `Unreadable`) means this
                        // scan simply did not find what it was looking for --
                        // the resolver's own `reason` (already correct for a
                        // trait-impl method, a generic `impl` block, a
                        // `&mut self` target) is the truer sentence there.
                        Err(
                            err @ (harness::ReceiverError::NoConstructor { .. }
                            | harness::ReceiverError::UnsupportedConstructorParam { .. }
                            | harness::ReceiverError::PrivateConstructor { .. }
                            | harness::ReceiverError::UnsupportedParamPattern),
                        ) => {
                            diagnostics.push(refused_anchor_diag(&node_id, &err.to_string()));
                            early_nodes_by_component
                                .entry(comp_name.clone())
                                .or_default()
                                .push(leaf_node(fn_name, "unsupported"));
                            continue;
                        }
                        Err(_receiver_err) => {
                            diagnostics.push(refused_anchor_diag(&node_id, &reason));
                            early_nodes_by_component
                                .entry(comp_name.clone())
                                .or_default()
                                .push(leaf_node(fn_name, "unsupported"));
                            continue;
                        }
                    }
                }
                Resolution::Ambiguous(reason) => {
                    diagnostics.push(ambiguous_anchor_diag(&node_id, fn_name, &reason));
                    early_nodes_by_component
                        .entry(comp_name.clone())
                        .or_default()
                        .push(leaf_node(fn_name, "unsupported"));
                    continue;
                }
                Resolution::Found(_) | Resolution::Opaque(_) | Resolution::NotFound => {
                    match harness::discover_fn_with(&mut resolver, fn_name, &lib_path) {
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
                                .push(leaf_node(fn_name, "unclaimed"));
                            continue;
                        }
                    }
                }
            };

            // Struct/enum parameters (this task, 2026-08-27): a parameter
            // whose type parsed to `Unsupported` may still be a struct/enum
            // Ply itself knows how to build -- via the type's own
            // constructor (rule 1), or by direct field/variant construction
            // when nothing is private (rule 2), per
            // `docs/review-self-construction.md`. Applied here, after `cf`
            // is resolved and before any check decision reads its params,
            // so `default_checks_for`/`is_fuzz_supported` see the upgraded
            // type from this point on. A parameter this scan recognised as
            // a real struct/enum declaration but still could not build
            // (rule 3) earns its own named diagnostic rather than the
            // generic "type neither engine builds inputs for" one.
            for (param_name, type_name, reason) in
                harness::enrich_contract_fn_user_types(&mut cf, crate_dir)
            {
                diagnostics.push(user_type_param_refused_diag(
                    &node_id,
                    fn_name,
                    &param_name,
                    &type_name,
                    &reason,
                ));
            }

            if let Some(span) = cf.source_span.clone() {
                source_map.insert(node_id.clone(), span);
            }

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
            // This must fire whenever ply.yaml declares a contract here,
            // full stop -- not only when there also happens to be an
            // inline `#[ply::requires]`/`#[ply::ensures]` attribute.
            // `declared_contract_not_anded_diag` used to be gated on
            // `cf.has_contract()` too (2026-08-30), on the theory that
            // without an inline attribute, `V0505`'s "there is nothing to
            // check its result against" already said enough -- but that
            // reasoning only holds when `V0505` actually fires, and it does
            // not when the fn also has `examples:` entries (or any other
            // check that finds something to run): a `checks: [test]` fn
            // with a passing example and a *wrong* ply.yaml `ensures`
            // reported a clean `tested` with zero diagnostics, in total
            // silence (regression, found 2026-08-31). The old wording's own
            // flaw -- claiming unconditionally that "this run checked
            // `{fn_name}` against its inline attributes only", false when
            // there is no inline attribute -- is fixed in the diagnostic's
            // own text below instead, so the fix is restoring when this
            // fires, not narrowing it further.
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
                    .push(leaf_node(fn_name, "unclaimed"));
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
                        .push(leaf_node(fn_name, "unclaimed"));
                    continue;
                }
                // "none otherwise" (§5.4c): either no contract at all, or a
                // contract whose shape neither gate can build inputs for.
                if cf.has_contract() {
                    diagnostics.push(unsupported_shape_diag(&node_id, fn_name, &cf));
                    early_nodes_by_component
                        .entry(comp_name.clone())
                        .or_default()
                        .push(leaf_node(fn_name, "unsupported"));
                } else {
                    early_nodes_by_component
                        .entry(comp_name.clone())
                        .or_default()
                        .push(leaf_node(fn_name, "unclaimed"));
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

            // Every declared example is parsed here, before this claim can
            // earn anything, and a parse failure stops it dead.
            //
            // It used to be parsed only while building the harness, where
            // the `Err` was dropped by an `if let Ok(...)`: the malformed
            // example was silently skipped, the remaining ones ran, and the
            // claim earned `tested` with exit 0 and no diagnostic -- then
            // the result was recorded and reused. `generate_example_test`'s
            // own doc comment promised the opposite ("never a silently
            // skipped example") and had promised it for as long as the call
            // site had broken it (external review, 2026-08-30).
            //
            // A typo'd example is the worst possible thing to drop quietly:
            // it is the one assertion the author wrote out by hand, and the
            // verdict claimed it had been checked.
            if let Some(bad) = claim.examples.iter().enumerate().find_map(|(i, example)| {
                ply_core::fuzz_gen::generate_example_test(fn_name, (i + 1) as u32, example)
                    .err()
                    .map(|e| e.to_string())
            }) {
                diagnostics.push(malformed_example_diag(&node_id, &bad));
                early_nodes_by_component
                    .entry(comp_name.clone())
                    .or_default()
                    .push(leaf_node(fn_name, "unsupported"));
                continue;
            }

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

    // Every `examples:` entry declared anywhere in this crate (docs/reach-
    // measurement-2.md's seeded-generation source 1) -- not just the fn
    // being generated for, so a seed written against a constructor from a
    // sibling claim still counts. Gathered once, up front, since building
    // one fn's harness must never depend on iteration order over the rest.
    let examples_pool: Vec<String> = plans
        .iter()
        .flat_map(|p| p.claim.examples.iter().cloned())
        .collect();

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
    // Lives until `verify_crate` returns, so the harness stays a member for
    // every engine invocation below and the user's `Cargo.toml` goes back to
    // what they wrote the moment the run ends -- on the error paths too.
    let _manifest_registration: Option<harness_crate::ManifestRegistration>;
    if needs_harness {
        let cargo_toml_path = crate_dir.join("Cargo.toml");
        let cargo_toml_text = std::fs::read_to_string(&cargo_toml_path)
            .with_context(|| format!("reading {}", cargo_toml_path.display()))?;
        let target_names = harness_crate::read_crate_names(&cargo_toml_text)?;
        let harness_pkg = harness_crate::harness_package_name(&target_names.package_name);
        let harness_rel = harness_crate::harness_rel_path(&target_names.package_name);
        let harness_dir = crate_dir.join(&harness_rel);
        // docs/review-caveats.md N1: registering the harness as a member of
        // the target crate's own workspace only happens when that crate
        // already opted into having one. Otherwise (an ordinary crate, or a
        // member of someone else's workspace with no `[workspace]` table of
        // its own) Ply never edits the target's `Cargo.toml` at all -- the
        // harness gets its own isolated `[workspace]` table instead
        // (`harness_crate` module doc), and every `cargo test`/`cargo
        // mutants` invocation against it below runs from *its own*
        // directory, never the target crate's.
        let standalone = !harness_crate::crate_has_workspace_table(&cargo_toml_text);
        _manifest_registration = if standalone {
            None
        } else {
            Some(harness_crate::ManifestRegistration::register(
                &cargo_toml_path,
                &harness_rel,
                &harness_dir,
                &harness_pkg,
                &target_names,
            )?)
        };
        let harness_workspace_root: PathBuf = if standalone {
            harness_dir.clone()
        } else {
            crate_dir.to_path_buf()
        };
        harness_crate::write_harness_cargo_toml(
            &harness_dir,
            &harness_pkg,
            &target_names,
            standalone,
        )?;

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
                && let Ok(body) = ply_core::fuzz_gen::generate_fuzz_test_with_examples(
                    &plan.cf,
                    n,
                    &plan.seed,
                    &examples_pool,
                )
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
            let check =
                fuzz_engine::check_harness_builds(&harness_workspace_root, &harness_pkg, timeout)?;
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
            workspace_root: harness_workspace_root,
            standalone,
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
    let (topo_order, tainted) = ply_core::schedule::order(&bounded_eligible, &node_ids, &edges);
    // Processing order: callees before callers among the orderable
    // bounded-eligible claims, then the ones a cycle (or a transitive
    // dependency on one -- see `ply_core::schedule`'s module doc comment for
    // why `tainted` holds more than just the cycle's own members) left
    // unorderable (D5's second branch covers every one of their
    // contracted-callee edges, so their own place relative to each other
    // cannot matter -- and no cycle is introduced by this graph itself: `g`
    // never depends on `f` under callees-first construction, edges only
    // ever point callee-to-caller, so the only way an index lands in
    // `tainted` is a genuine call cycle in the source, or a dependency on
    // one, exactly D5's own "`f` and `g` in a cycle" case), then every other
    // fresh claim in the order Pass 1 already produced -- fuzz/test/mutate
    // claims and unsupported/unclaimed ones never consult `known_bounded` at
    // all, so nothing about their order is load-bearing.
    let mut processing_order: Vec<usize> = topo_order;
    processing_order.extend(tainted.iter().copied());
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
    // Every rendered cex test any fn in this run earns, across the whole
    // loop below -- written to `ply_generated_cex.rs` exactly once, after
    // the loop, so a second fn's counterexample never overwrites a first
    // fn's (`push_cex_test`'s own doc comment has the full story).
    let mut all_cex_tests: Vec<RenderedTest> = Vec::new();
    for idx in processing_order {
        if bounded_eligible.contains(&idx) {
            resolve_contracted_calls(
                &mut plans[idx].boundary,
                tainted.contains(&idx),
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
            &mut all_cex_tests,
        )?;
        if node.verdict.starts_with("bounded(")
            && !node.statuses.iter().any(|s| s == "conditional")
            && let Some(k) = parse_bound(&node.verdict)
        {
            known_bounded.insert(plans[idx].cf.path.clone(), k);
        }
        results[idx] = Some((node, fn_diags));
    }

    // One combined write for every cex test this whole run earned (§9,
    // `push_cex_test`'s own doc comment): each diagnostic above already
    // promised its own test lives at this path, so this write must include
    // every one of them, not just the last fn's.
    if !all_cex_tests.is_empty() {
        let module_source = contract_rt::wrap_test_module(&all_cex_tests);
        harness::write_generated_test(&src_dir, &lib_path, &module_source)?;
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
                .push({
                    let mut n = Node {
                        id: plan.fn_name.to_string(),
                        kind: "fn".into(),
                        verdict: entry.verdict.clone(),
                        statuses: entry.statuses.clone(),
                        reused: true,
                        evidence: entry.evidence.clone(),
                        children: vec![],
                        ..Default::default()
                    };
                    attach_claim_text(&mut n, &plan.cf, plan.claim);
                    n
                });
            continue;
        }
        let (mut node, mut fn_diags) = results[idx]
            .take()
            .expect("every fresh plan was processed above");
        attach_claim_text(&mut node, &plan.cf, plan.claim);
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
        ..Default::default()
    };

    let envelope = Envelope {
        command: "verify".into(),
        ply_version: PLY_VERSION.into(),
        root,
        diagnostics,
        coverage: None,
        trust_surface: None,
        open_items: None,
        not_carried_forward,
    };
    Ok(VerificationResult {
        document: file,
        envelope,
        source_map,
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
        ..Default::default()
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
                                found.is_method,
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
                    // `classify` already resolved this exact site as
                    // `Contracted`, which only ever follows from `Found` --
                    // so, like the two arms above, this should not be
                    // reachable. Refused rather than assumed, same reason.
                    Resolution::Refused(_) | Resolution::Ambiguous(_) => {
                        if !plan.opaque.iter().any(|(p, _, _)| p == &site.path) {
                            plan.opaque.push((
                                site.path.clone(),
                                site.where_text(),
                                "resolution disagreed with itself between classification and \
                                 lookup (should not happen)"
                                    .to_string(),
                            ));
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
/// Pushes a rendered cex test into the run-wide accumulator that gets
/// written to `ply_generated_cex.rs` exactly once, after every fn has been
/// checked (see the single `write_generated_test` call at the end of
/// `verify_loaded_crate`). Two fns that both break their promise must both
/// still have a test in that file -- `write_generated_test` itself only
/// ever overwrites the whole file, so calling it once per fn silently
/// dropped every counterexample but the last (found 2026-08-30 pointing
/// Ply at a fixture with two broken fns; only the second's test survived on
/// disk while the terminal reported both as generated). A fn's own witness
/// can legitimately be rendered twice in one run (a fresh violation is
/// stored, then immediately re-read back for §9's oracle check) with an
/// identical `test_name`, which `retain` here drops rather than duplicates
/// -- two `fn` items with the same name in one generated file would not
/// compile.
fn push_cex_test(tests_out: &mut Vec<RenderedTest>, rendered: RenderedTest) {
    tests_out.retain(|t| t.test_name != rendered.test_name);
    tests_out.push(rendered);
}

/// Where the combined cex-test file lives, computed the same way
/// `write_generated_test` computes it -- needed before that single
/// end-of-run write happens, so a diagnostic built mid-run can still name
/// the path its counterexample will land in.
fn cex_test_display_path(src_dir: &Path) -> String {
    let test_file = src_dir.join("ply_generated_cex.rs");
    test_file
        .strip_prefix(src_dir.parent().unwrap_or(src_dir))
        .unwrap_or(&test_file)
        .display()
        .to_string()
}

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
            "the `requires:`/`ensures:` declared for `{fn_name}` in ply.yaml is used only where \
             §5.5 needs it -- callers of `{fn_name}` may assume it at a boundary -- it is \
             **not** yet ANDed into `{fn_name}`'s own checks, which §5.4 says it should be. So \
             this run does not check `{fn_name}` against it; only an inline \
             `#[ply::requires]`/`#[ply::ensures]` attribute written on `{fn_name}` itself counts \
             toward `{fn_name}`'s own checks. (W0510)"
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
    /// Where `fuzz`/`test` cargo invocations against this harness must run
    /// from (docs/review-caveats.md N1): the target crate's own root when
    /// it was already registered into that crate's existing workspace, or
    /// the harness crate's own directory when it was instead given an
    /// isolated `[workspace]` table of its own (`standalone` below).
    workspace_root: PathBuf,
    /// Whether the harness was placed in its own isolated `[workspace]`
    /// (true) rather than registered as a member of the target crate's own
    /// (false). `mutate` needs the registered-member shape specifically
    /// (`engines::mutants`' own module doc) and cannot be attempted at all
    /// when this is true -- see its call site in `run_fn_checks`.
    standalone: bool,
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
    // Every rendered cex test earned by any fn in this whole run, so far --
    // accumulated rather than written per-fn (`push_cex_test`'s own doc).
    cex_tests_out: &mut Vec<RenderedTest>,
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
                    cex_tests_out,
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
        } else if cf.ensures.is_none() && (wants_fuzz.is_some() || (wants_test && !has_examples)) {
            // Widened 2026-08-27 (docs/review-strings-receivers.md finding 1,
            // "one step milder"): a `test`-only claim with no `#[ply::ensures]`
            // and no `examples:` entries has nothing at all for `test` to
            // assert -- `generate_direct_contract_cases` silently produces no
            // body for exactly this shape (no closure to check against). This
            // used to fall through, ungated, into harness generation, which
            // wrote nothing, matched no test, and reported `tested`/held with
            // zero cases run. Naming it here, before codegen, gives a
            // specific "no contract to check" diagnostic instead of a bare
            // "ran zero cases" tool error further downstream.
            let check_label = if let Some(n) = wants_fuzz {
                format!("fuzz({n})")
            } else {
                "test".into()
            };
            // This diagnostic used to also name, inline, whether ply.yaml
            // *also* declares a `requires:`/`ensures:` contract for this
            // same fn (2026-08-30, "a documented way of writing contracts
            // is accepted, then silently ignored"). That note is gone: the
            // fix for a later regression (2026-08-31, "a promise nobody
            // checks is now reported green in total silence") made
            // `declared_contract_not_anded_diag` (`W0510`) fire unconditionally
            // whenever ply.yaml declares a contract here, which is the same
            // condition this note used to check -- so the two diagnostics
            // said the same thing about the ply.yaml contract whenever both
            // fired, and repeating it here would just be noise now that
            // `W0510` always carries it.
            diagnostics.push(Diagnostic {
                code: "V0505".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: check_label,
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` declares `{}` but has no `#[ply::ensures]` and no `examples:` entries -- \
                     there is nothing to check its result against, so nothing was run. Add an \
                     `#[ply::ensures]` clause naming what `{fn_name}` promises about its result, or add \
                     `examples:` entries naming concrete calls to assert.",
                    if wants_fuzz.is_some() { "fuzz" } else { "test" }
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
                        title: if wants_fuzz.is_some() {
                            format!(
                                "or drop `fuzz` from `{fn_name}`'s checks and declare `test` with \
                                 `examples:` instead, which needs no postcondition"
                            )
                        } else {
                            format!(
                                "or add `examples:` entries for `{fn_name}` in ply.yaml -- concrete \
                                 calls asserted directly, which need no `#[ply::ensures]`"
                            )
                        },
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
        } else if let Some(field) = self_return_reads_private_field_on_sampling_tier(cf, lib_path) {
            // The "a `Self` answer is always fine" rule's own blind spot
            // on the sampling tier (adversarial review, 2026-08-27):
            // refused by name, before codegen runs, rather than left to
            // fail as a tool error quoting a private-field compiler
            // message the harness crate could never avoid.
            diagnostics.push(self_return_private_field_diag(node_id, fn_name, &field));
            labels.push("unsupported".into());
        } else if let Some(info) = harness_info {
            let ident = cf.ident();
            if let Some(cause) = info.broken.get(&ident) {
                // Misattribution fix: this exact function's own generated
                // code is what the compiler pointed at, so it alone is
                // reported broken -- its harness test never even runs
                // (the module was dropped before the crate's remaining
                // fns were built), and no crate-mate's verdict is touched.
                let module = harness_module_name(cf);
                if let Some(n) = wants_fuzz {
                    diagnostics.push(harness_did_not_run_diag(
                        node_id,
                        fn_name,
                        &module,
                        &format!("fuzz({n})"),
                        &info.package,
                        Some(cause.as_str()),
                        has_examples,
                        cf.receiver.is_some(),
                    ));
                    labels.push("tool_error".into());
                }
                if wants_test {
                    diagnostics.push(harness_did_not_run_diag(
                        node_id,
                        fn_name,
                        &module,
                        "test",
                        &info.package,
                        Some(cause.as_str()),
                        has_examples,
                        cf.receiver.is_some(),
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
                    &info.workspace_root,
                    &info.package,
                    node_id,
                    fn_name,
                    wants_fuzz,
                    wants_test,
                    seed,
                    has_examples,
                    opts,
                    cex_tests_out,
                )?;
                diagnostics.append(&mut run.diagnostics);
                if let Some(l) = run.fuzz_label {
                    labels.push(l);
                }
                if let Some(l) = run.test_label {
                    labels.push(l);
                }
                // The same structural pattern `conditional`/`partial-history`
                // already follow (CLAUDE.md: "read how `conditional` is
                // carried ... and follow that pattern"): a status travels
                // beside the verdict as a plain flag, propagates upward
                // through this same `statuses` vec, and (via `RecordEntry`)
                // survives a reused verdict unchanged -- never a warning,
                // since this describes what the evidence *is*, not
                // something incidental about the run.
                if run.seeded && !statuses.iter().any(|s| s == "seeded") {
                    statuses.push("seeded".into());
                }
                // §1: a verdict names the evidence that produced it. Only a
                // run that happened has any to name.
                if run.fuzz_ran
                    && let Some(n) = wants_fuzz
                {
                    fuzz_evidence = Some(Evidence {
                        engine: "proptest".into(),
                        seed: Some(ply_core::fuzz_gen::seed_hex(seed)),
                        cases: run.fuzz_cases_reached,
                    });
                    // The NaN/infinity decision's own visibility
                    // requirement (task, 2026-08-27): only a run that
                    // actually sampled a float owes the reader this
                    // disclosure, so this is gated exactly like the
                    // evidence block above it, never on `fuzz` merely being
                    // declared.
                    if cf.has_float_shape() {
                        diagnostics.push(float_sampling_diag(
                            node_id,
                            fn_name,
                            &format!("fuzz({n})"),
                        ));
                    }
                    // The string exclusion's own disclosure, wired in now
                    // (also-fix, task 2026-08-27): gated identically to the
                    // float one just above -- a run that actually sampled a
                    // string, never merely the check being declared.
                    if cf.has_string_shape() {
                        diagnostics.push(string_sampling_diag(
                            node_id,
                            fn_name,
                            &format!("fuzz({n})"),
                        ));
                    }
                    // The public-fields assumption's own disclosure
                    // (docs/review-self-construction.md's rule 2, this
                    // task): gated identically to the float/string ones
                    // just above -- a run that actually built at least one
                    // parameter by direct field/variant construction, never
                    // merely the check being declared.
                    let public_field_types = cf.public_fields_param_type_names();
                    if !public_field_types.is_empty() {
                        diagnostics.push(public_fields_assumed_diag(
                            node_id,
                            fn_name,
                            &format!("fuzz({n})"),
                            &public_field_types,
                            &cf.skipped_constructor_notes(),
                        ));
                    }
                    // The sequence-length honesty requirement
                    // (docs/review-self-construction.md's "fourth option",
                    // 2026-08-27): a receiver method's verdict rests on a
                    // value Ply built itself, over a *bounded* number of
                    // prior operations, and that bound must be as visible
                    // as a loop bound already is -- gated the same way the
                    // float disclosure just above is, on a run that
                    // actually happened, never on the check merely being
                    // declared.
                    if let Some(plan) = &cf.receiver {
                        diagnostics.push(receiver_sequence_diag(node_id, fn_name, plan));
                        // "the fourteenth false clean" (docs/review-structs-
                        // enums.md finding 1, 2026-08-28): a verdict resting
                        // on a receiver history that could not include one
                        // of the type's own operations is narrower than a
                        // `fuzzed(n)` verdict alone reads. `partial-history`
                        // travels beside the verdict rather than replacing
                        // it -- real cases really ran, against the
                        // operations that could be called, so this is not
                        // an absence of evidence (D6's closed vocabulary,
                        // `is_absence`, is deliberately not extended here);
                        // it is a fact about what that evidence does and
                        // does not cover, the same role `weak-spec` already
                        // plays for a passing check with a weak spec behind
                        // it. Distinct status from `weak-spec` because it is
                        // a distinct fact (D6: "a proof in one corner must
                        // not hide a merely-tested boundary in another") --
                        // one is about the spec's strength, this is about
                        // how much of the type's own behaviour the run
                        // could even attempt.
                        // Finding 3 (docs/review-silent-narrowing.md, "the
                        // type has a second constructor Ply never calls",
                        // 2026-08-28): a receiver history that only ever
                        // starts from one of a type's several usable
                        // constructors is exactly as narrow as one that
                        // could not call one of the type's operations --
                        // both are states this run never explored, however
                        // many cases ran -- so both trip the same status.
                        if (!plan.excluded_operations.is_empty()
                            || !plan.other_constructors.is_empty())
                            && !statuses.iter().any(|s| s == "partial-history")
                        {
                            statuses.push("partial-history".into());
                        }
                    }
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
                let (outcome, mut d) = run_mutate_check(
                    crate_dir,
                    &info.package,
                    info.standalone,
                    node_id,
                    fn_name,
                    &harness_test_filter(cf),
                    checks,
                    opts,
                )?;
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
            ..Default::default()
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

/// A `Type::method` claim that named something real -- a method with a
/// receiver, an item in a generic `impl` block, or a trait method -- which
/// this task's scope refuses to check. Distinct from `unresolved_anchor_diag`
/// (E0301) on purpose: "could not find the function" is false here, and a
/// false "not found" is exactly the defect this feature exists to close (see
/// the module doc at the top of this file's `verify_crate`). `reason` is
/// already a complete, plain-language sentence (`callgraph::Resolution`
/// composes it), so this wraps it rather than re-deriving it.
pub(crate) fn refused_anchor_diag(node_id: &str, reason: &str) -> Diagnostic {
    Diagnostic {
        code: "V0507".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        // `reason` is already a complete, self-contained sentence naming the
        // function and why Ply refuses it (`callgraph::Resolution` composes
        // it) -- wrapping it in another "Ply found `{fn_name}`" would either
        // repeat that fact verbatim (the receiver case already opens with
        // exactly that clause) or say it twice in different words (the
        // trait/generic cases already open by naming `{fn_name}`).
        title: format!("{reason}."),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("unsupported_signature".into()),
    }
}

/// An `examples:` entry that is not a Rust expression at all -- `syn` never
/// even parses it, so it dies before the compiler or an engine ever sees it
/// (§14: "the contract expression subset is documented but not validated;
/// an expression outside it fails later" -- this is that failure, for the
/// one clause exempt from the subset, `examples`, per §5.4a). Deliberately a
/// separate constructor from [`refused_anchor_diag`] rather than a second
/// call site for it: that one's `V0507`/`unsupported_signature` describes a
/// real function this task's *scope* declines to check (a receiver method,
/// a generic `impl`) -- the signature is fine. Here the signature is not
/// the problem at all; the document is malformed, the author wrote a typo,
/// and `generate_example_test`'s own error message already names the
/// offending entry (`bad`, formatted as `E0501: could not parse ...`), so
/// reusing `V0507`/`warning`/`unsupported_signature` for it told the wrong
/// story on all three axes: the wrong code (one no documentation names),
/// the wrong severity (a warning, for something that refuses the claim and
/// exits non-zero), and a false reason (`unsupported_signature`, when
/// nothing about the signature is unsupported).
fn malformed_example_diag(node_id: &str, bad: &str) -> Diagnostic {
    Diagnostic {
        code: "E0501".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        // `bad` already is a complete sentence naming the offending entry
        // (`generate_example_test`'s own `E0501: could not parse
        // `examples` entry `...` as a Rust expression: ...`), so this wraps
        // it exactly the way `refused_anchor_diag` wraps `reason` rather
        // than re-deriving the same fact in different words.
        // `bad` opens with its own `E0501: ` prefix (the generator words the
        // whole sentence), and the renderer already prints `[E0501]` in
        // front of every diagnostic -- so wrapping it verbatim printed the
        // code twice on one line. The prefix is stripped here rather than
        // dropped at the source, because `generate_example_test`'s message
        // is also surfaced where nothing prepends a code.
        title: format!("{}.", bad.strip_prefix("E0501: ").unwrap_or(bad)),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("malformed_example".into()),
    }
}

/// `Type::method` matched more than one real candidate and Ply refuses to
/// guess which one a claim means -- picking wrong would attach a verdict to
/// the wrong function, which is worse than reporting nothing (see this
/// task's own "get the ambiguity right" brief).
pub(crate) fn ambiguous_anchor_diag(node_id: &str, fn_name: &str, reason: &str) -> Diagnostic {
    Diagnostic {
        code: "E0306".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title: format!("`{fn_name}` does not name one function: {reason}."),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("ambiguous_anchor".into()),
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

/// The sampling/proving split's own honesty requirement (task, 2026-08-27):
/// a `bounded`/`proved` check on a type the *fuzz* engine could build inputs
/// for, but the *proving* engine cannot, must be refused **by name** --
/// naming what blocked it and what would work instead -- never folded into
/// `unsupported_shape_diag`'s "none of its declared checks apply" wording,
/// which is simply false when a check the user did not ask for would have
/// worked (a real defect this fixed: before this function existed,
/// `run_bounded_check` reported that exact false sentence for any
/// sample-only-typed fn, e.g. a plain `f64` parameter, because
/// `unsupported_shape_diag`'s own "bad" list is filtered by
/// `is_fuzz_supported`, which is true for every sample-only type by
/// definition).
fn bounded_refused_sample_only_diag(
    node_id: &str,
    fn_name: &str,
    cf: &ContractFn,
    check_label: &str,
) -> Diagnostic {
    // The receiver case is a *different reason* to refuse, and must say so
    // (adversarial review, 2026-08-27, "a proof refused on a method blames
    // a type that is not the problem"): `ContractFn::is_bounded_supported`
    // refuses every receiver method outright, before it even looks at
    // params or the return type (see that method's own doc: the
    // sequence-of-operations approach was only ever scoped to the sampling
    // tier, and `bounded` on a receiver is an unmeasured Kani harness, not
    // a type Kani cannot reason about). The code below this branch assumes
    // the *opposite* -- that anything reaching here was refused for a type
    // reason -- and when every param and the return type both check out
    // fine, it fell back to blaming the return type by name, even though
    // that type is often the cheapest one Kani handles (`u32`, here). This
    // must never report a real blocker under a false name.
    if cf.receiver.is_some() {
        return Diagnostic {
            code: "V0508".into(),
            severity: "warning".into(),
            phase: "verify".into(),
            engine: "kani".into(),
            check: check_label.into(),
            node_id: node_id.into(),
            title: format!(
                "Ply did not run `{check_label}` on `{fn_name}`: it needs a value to call it on \
                 (it takes `&self`/`&mut self`), and `bounded`'s exhaustive search has not been \
                 extended to receiver methods -- only the sampling tier (`fuzz`/`test`) builds a \
                 receiver value today. This is not about any parameter or the return type: \
                 `{fn_name}`'s own types are all fine for `bounded`. Rather than attempt an \
                 unmeasured Kani harness, Ply reports this honestly as unsupported for \
                 `{check_label}` specifically -- `{fn_name}` is not unchecked, `fuzz(n)` does \
                 check it. (V0508)"
            ),
            pointer: None,
            primary_span: None,
            counterexample: None,
            fixes: vec![Fix {
                title: format!(
                    "replace `{check_label}` with `fuzz(n)` on `{fn_name}` -- it builds its own \
                     receiver value and will earn a real `fuzzed(n)` verdict"
                ),
                edits: vec![],
            }],
            assumptions: vec![],
            open_item: Some("unsupported_signature".into()),
        };
    }
    let bad_params: Vec<String> = cf
        .params
        .iter()
        .filter(|p| !p.ty.is_bounded_supported())
        .map(|p| format!("`{}: {}`", p.name, p.ty.display_name()))
        .collect();
    let what = if !bad_params.is_empty() {
        format!("parameter(s) {}", bad_params.join(", "))
    } else {
        // Retraction (measured 2026-09-01, The-Ply-Spec.md §5.4b): this
        // branch used to name the *return* type, back when
        // `is_bounded_return_supported` could say `false`. It cannot
        // anymore -- the return type never gates `bounded` on either
        // engine now -- so with the receiver case already handled above
        // and every parameter fine, `is_bounded_supported()` being false at
        // all has no reason left to point at. Kept as an honest,
        // never-blame-the-wrong-thing fallback rather than a `panic!` on a
        // diagnostic path, in case a future change reopens a case this
        // reasoning does not foresee.
        "its signature -- Ply could not determine which part".to_string()
    };
    Diagnostic {
        code: "V0508".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "Ply did not run `{check_label}` on `{fn_name}`: {what} can be checked with random \
             sample values just fine -- `fuzz`/`test` both work here -- but `{check_label}` needs \
             to reason about *every* possible value at once, and this type is real, substantial \
             work for that (or, for a floating-point type, a deliberate choice not to attempt it \
             at all -- see §5.4b). Rather than let the attempt hang or silently fall back to a \
             weaker check, Ply reports this honestly as unsupported for `{check_label}` specifically \
             -- `{fn_name}` is not unchecked, it just needs a different check. (V0508)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!(
                    "replace `{check_label}` with `fuzz(256)` on `{fn_name}` -- this shape \
                     supports it, and it will earn a real `fuzzed(256)` verdict"
                ),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "or use `test` with `examples:` entries for `{fn_name}`, which needs no \
                     random sampling at all"
                ),
                edits: vec![],
            },
        ],
        assumptions: vec![],
        open_item: Some("unsupported_signature".into()),
    }
}

/// The NaN/infinity decision's own visibility requirement (task,
/// 2026-08-27, "make the choice visible to the user rather than silent"):
/// `info`, not a warning -- nothing here is wrong or owed, it just needs
/// naming, the same reasoning `verified_dependency_diag`'s own `W0517` uses.
/// Fires once per fuzz/test run that actually sampled a float-shaped fn
/// (`ContractFn::has_float_shape`), never merely because a float check was
/// *declared* -- only a run that happened owes the reader this disclosure.
fn float_sampling_diag(node_id: &str, fn_name: &str, check_label: &str) -> Diagnostic {
    Diagnostic {
        code: "W0518".into(),
        severity: "info".into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` was checked using randomly generated floating-point values. By \
             default, Ply never generates two special values every floating-point type has: NaN \
             (\"not a number\", what you get from things like 0.0/0.0) and infinity. Comparisons \
             and equality checks involving NaN are always false -- even `NaN == NaN` -- so a \
             generated NaN would make almost any promise about `{fn_name}`'s result look broken, \
             even on a value `{fn_name}` might never actually receive. That would be a false \
             alarm, not a real bug, so Ply leaves NaN and infinity out of this run. If `{fn_name}` \
             needs to handle NaN or infinity correctly, this run says nothing about that -- it \
             was never asked to. (W0518, §5.4c)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// The string exclusion's own visibility requirement (also-fix, task
/// 2026-08-27, docs/review-strings-receivers.md: "the string control-
/// character exclusion is real but never disclosed to the user, while the
/// float NaN exclusion is"). `ContractFn::has_string_shape` and the
/// exclusion itself (`harness.rs`'s `RustType::String` doc,
/// `fuzz_gen::strategy_expr`'s own arm) were already built and tested this
/// same session; only this CLI-level disclosure was left unwired, recorded
/// honestly rather than silently shipped as done. `info`, not a warning --
/// nothing here is wrong or owed, the same reasoning `float_sampling_diag`
/// uses -- and gated the same way: only a run that actually sampled a
/// string owes the reader this sentence, never merely declaring the check.
fn string_sampling_diag(node_id: &str, fn_name: &str, check_label: &str) -> Diagnostic {
    Diagnostic {
        code: "W0521".into(),
        severity: "info".into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` was checked using randomly generated text (up to 32 characters). By \
             default, Ply never generates ASCII or Latin-1 control characters (raw bytes like a \
             null byte or an escape code, `0x00`-`0x1F` and `0x7F`-`0x9F`) -- the kind of byte \
             real user-facing text almost never contains, and the kind most likely to trip an \
             unrelated assumption (a log line, a terminal, a CSV cell) rather than `{fn_name}`'s \
             own logic, so including them would risk a false alarm rather than a real bug. \
             Multi-byte Unicode text (accented letters, CJK characters, symbols) is NOT excluded \
             -- Ply generates it deliberately, since a `String` a real caller holds can already \
             contain it. If `{fn_name}` needs to handle control characters correctly, this run \
             says nothing about that -- it was never asked to. (W0521, §5.4c)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// The honesty requirement docs/review-self-construction.md's "fourth
/// option" is built on (task, 2026-08-27): a receiver method's verdict rests
/// on a value Ply built itself, over a *bounded* number of the type's own
/// prior operations, never on one a user declared or one filled in field by
/// field. That bound must be visible on the same verdict the way a `bounded`
/// check's own loop bound already is (§5.4c) -- "checked on receivers
/// reachable in at most N operations from a fresh one" is the honest reading
/// of what this run does and does not cover, and a reader must be able to
/// see it without reading source.
///
/// **Extended 2026-08-28** (docs/review-structs-enums.md finding 1, "the
/// fourteenth false clean") to name the operations `plan.excluded_operations`
/// records: an operation Ply could not build an argument for is not merely
/// missing from the pool, it is missing from *history* -- no case this run
/// generated ever called it, so no case ever explored what it does to the
/// receiver's state. The old wording ("every value this run saw was
/// reachable by calling the type's own code, nothing else, so nothing here
/// was assumed") is true only when nothing was excluded; said over an
/// excluded mutator it asserts the opposite of what happened, which is
/// exactly the shape of the false clean this fixes. So the sentence now
/// splits: when nothing was excluded, it keeps the old, true claim; when
/// something was, it names every excluded operation and its reason instead,
/// and drops the completeness claim entirely rather than leave a milder
/// version of it standing.
///
/// **Severity escalates to `warning` when an operation was excluded, or the
/// receiver was only ever built from one of several usable constructors.**
/// Neither is a deliberate, documented design choice the way the
/// float/string sampling exclusions are (there `info` is right: nothing is
/// wrong, a choice was made on purpose and is being disclosed). Here a
/// mutator or a constructor that exists in the user's own code was left out
/// of the run for a reason that has nothing to do with the promise being
/// checked, and that is a real gap in what the verdict covers -- serious
/// enough that `--fail-on warn` should be able to catch it, the same lever
/// `weak-spec` already gives a stricter caller (§5.4c's own "a finding
/// beside real evidence, not an absence of it" precedent). It does not join
/// the D6 absence-of-evidence vocabulary and does not change the verdict
/// string: real cases really did run, against the operations and the
/// constructor that could be called, so calling the *fuzzed(n)* verdict
/// itself an absence would overclaim in the other direction. See this fn's
/// caller for the `partial-history` status this pairs with on the node.
///
/// **Extended again 2026-08-28** (docs/review-silent-narrowing.md, the
/// three false cleans found beside the fourteenth's own fix) in two ways:
///
/// - `plan.excluded_operations` now also carries a mutating method Ply
///   found but could not call because it lives behind a `trait`
///   implementation (finding 2) -- the old wording after the list assumed
///   every exclusion meant "an unbuildable argument" and said so in a
///   sentence of its own; that is no longer true of every entry, so the
///   sentence now says only what is true of *every* reason (no case
///   generated ever called it), and leaves *why* entirely to each
///   operation's own `reason` text, which is already specific.
/// - `plan.other_constructors`, when non-empty, earns its own sentence and
///   the same severity escalation (finding 3): a receiver history that only
///   ever starts from one of a type's several usable constructors never
///   explores whatever is reachable only through the others, exactly the
///   same shape of gap as an uncalled operation.
///
/// **Tightened 2026-08-28, same-day review (docs/review-silent-narrowing.md
/// §6): this disclosure was measured at 193 words per method per run and
/// changed neither the verdict nor the exit code -- all cost, no benefit
/// collected.** Every fact above still has a sentence: the receiver was
/// built by Ply, which constructor, the pool and the bound it drew from,
/// which operation or constructor this run could never reach and why, and
/// the caveat that a promise depending on that is unchecked. What is cut is
/// pure restatement: the old closing sentence re-said the bound the opening
/// clause had already given in different words ("outside what this run
/// checked... rather than leaving a reader to assume the check covers every
/// possible history"), and the "nothing here was assumed" / "no case this
/// run generated changed its state that way" pair each said their branch's
/// point twice. Cutting those took this to 49 words with nothing narrowed
/// and under 85 with every kind of gap present at once -- still a complete
/// sentence, no code or § reference doing the work prose should.
fn receiver_sequence_diag(
    node_id: &str,
    fn_name: &str,
    plan: &harness::ReceiverPlan,
) -> Diagnostic {
    let others: Vec<&str> = plan
        .operations
        .iter()
        .skip(1)
        .map(|op| op.call_path.as_str())
        .collect();
    let pool_sentence = if others.is_empty() {
        format!("`{fn_name}`")
    } else {
        format!(
            "`{fn_name}` and {}",
            others
                .iter()
                .map(|o| format!("`{o}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let narrowed = !plan.excluded_operations.is_empty() || !plan.other_constructors.is_empty();
    let severity = if narrowed { "warning" } else { "info" };

    let base = format!(
        "`{fn_name}` needs a `{type_name}`, so Ply built one itself: `{constructor}`, then up \
         to {max} calls to {pool_sentence}, in random order, before the checked call.",
        fn_name = fn_name,
        type_name = plan.type_name,
        constructor = plan.constructor,
        max = plan.max_sequence_len,
        pool_sentence = pool_sentence,
    );

    let tail = if !narrowed {
        format!(
            "That covers every value `{type_name}`'s own code can reach within {max} steps of a \
             fresh one -- nothing else was assumed.",
            type_name = plan.type_name,
            max = plan.max_sequence_len,
        )
    } else {
        let mut gaps = Vec::new();
        if !plan.excluded_operations.is_empty() {
            let excluded_list = plan
                .excluded_operations
                .iter()
                .map(|op| format!("`{}` ({})", op.call_path, op.reason))
                .collect::<Vec<_>>()
                .join("; ");
            gaps.push(format!(
                "can also be changed by {excluded_list}, which this run never called"
            ));
        }
        if !plan.other_constructors.is_empty() {
            let other_list = plan
                .other_constructors
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ");
            gaps.push(format!(
                "was only ever built by calling `{}`, never by calling {other_list}",
                plan.constructor
            ));
        }
        format!(
            "`{type_name}` {gaps}. If `{fn_name}`'s promise depends on what this run never \
             reached, this run says nothing about it.",
            type_name = plan.type_name,
            gaps = gaps.join(", and "),
            fn_name = fn_name,
        )
    };

    Diagnostic {
        code: "W0520".into(),
        severity: severity.into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: "fuzz".into(),
        node_id: node_id.into(),
        title: format!("{base} {tail} (W0520, §5.4c)"),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// Struct/enum parameters (this task, 2026-08-27): `param_name`'s type is a
/// real struct/enum `type_name` this scan found declared in the crate, but
/// Ply could not build a value of it either way
/// (`docs/review-self-construction.md`'s rule 3, "otherwise refuse by
/// name") -- `reason` is already the complete, type-naming sentence
/// `resolve_user_type` built at the point of refusal (no usable
/// constructor, a private field, a nested type Ply cannot build). Distinct
/// from the generic `V0505`/`V0508` "type neither engine builds inputs
/// for": that message is true but generic; this one says *why*, which is
/// what a reader needs to fix it (declare a constructor, make the fields
/// public, or add a generator hook).
fn user_type_param_refused_diag(
    node_id: &str,
    fn_name: &str,
    param_name: &str,
    type_name: &str,
    reason: &str,
) -> Diagnostic {
    Diagnostic {
        code: "V0509".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}`'s parameter `{param_name}: {type_name}` cannot be built. {reason} \
             (V0509)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("unsupported_signature".into()),
    }
}

/// The named assumption `docs/review-self-construction.md` requires for
/// rule 2 (direct field/variant construction): "Ply assumes a public-field
/// type has no invariant" is false in general (the review's own
/// `SweepReport`/`Decision` counterexamples), so every verdict resting on
/// it says so, the same way the float/string sampling choices disclose
/// themselves rather than staying implicit. `info`, not a warning -- this is
/// not wrong or owed, it is a fact about what the evidence assumed. Fires
/// once per fuzz run that actually built at least one parameter this way
/// (gated on a run that happened, matching `float_sampling_diag`'s own
/// discipline), naming every such parameter together.
///
/// **Extended 2026-08-28** (docs/review-structs-enums.md finding 2, "is the
/// disclosure enough? -- No") to carry `skipped_constructor_notes`: this
/// route is only ever taken when rule 1 (the type's own constructor) could
/// not build a value, and the old wording never said whether that was
/// because no constructor exists at all or because one exists and was
/// found but could not be used -- the second case is a materially different
/// fact (a value the constructor exists to forbid may now be built), and a
/// reader deciding whether to trust this run needs to know which one
/// happened.
fn public_fields_assumed_diag(
    node_id: &str,
    fn_name: &str,
    check_label: &str,
    type_names: &[String],
    skipped_constructor_notes: &[String],
) -> Diagnostic {
    let names = type_names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let skipped_sentence = if skipped_constructor_notes.is_empty() {
        String::new()
    } else {
        format!(" {}", skipped_constructor_notes.join(" "))
    };
    Diagnostic {
        code: "W0522".into(),
        severity: "info".into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` takes {names} by value, built by filling in its fields/variant data \
             directly (every one of them is already public, so nothing here restricts what a \
             caller could build). Ply assumes that means there is no hidden invariant among \
             those fields -- but a type's own methods can maintain a relationship between public \
             fields that nothing in the type itself enforces, so a value this run built could, in \
             principle, be one the real program never produces. This run's evidence rests on that \
             assumption; it is not proved.{skipped_sentence} (W0522, §5.4b)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// A sampled check on a zero-input fn's own honesty requirement
/// (adversarial review, 2026-08-27, "the count the user chose appearing as
/// if it measured coverage"): `info`, not a warning -- nothing here is
/// wrong or owed, only worth naming, the same reasoning `float_sampling_diag`
/// already uses. `check_label` is the check the user actually wrote
/// (`fuzz(64)`), reported in the diagnostic's own `check` field for
/// traceability even though the *verdict* this run earns is `tested`.
fn zero_input_sampled_diag(node_id: &str, fn_name: &str, check_label: &str) -> Diagnostic {
    Diagnostic {
        code: "W0519".into(),
        severity: "info".into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "`{fn_name}` takes no input, so there was only one possible call to make -- not \
             several different ones. Ply made that one call and it held. This is reported as \
             `tested`, not as a fuzzed case count: `{fn_name}` has no input space for a bigger \
             number to sample more of, so a bigger number here would not have looked at anything \
             new. (W0519, §5.4c)"
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// The "a `Self` answer is always fine" rule (§5.4b) is correct that Ply
/// never has to *build* a `Self` return value on any tier -- the real call
/// produces it. It is not the only question, though: Ply still has to
/// *read* the value, because that is what the promise says, and where the
/// generated harness lives differs by tier. The exhaustive/bounded tier's
/// harness sits inside this crate and sees a private field fine; the
/// fuzz/test tier's harness is a *separate* crate, so the exact same
/// promise cannot compile there (adversarial review, 2026-08-27:
/// demonstrated with the shipped fixture's own `Bucket::new`, checked
/// exhaustively -- a real verdict -- versus the identical shape checked by
/// sampling, which came back a bare `field ... is private` tool error).
/// Returns the private field name the promise reads, so the caller can
/// refuse this by name before codegen ever runs, rather than leaving it to
/// fail as an unexplained compiler error.
fn self_return_reads_private_field_on_sampling_tier(
    cf: &ContractFn,
    lib_path: &Path,
) -> Option<String> {
    if cf.return_type != RustType::SelfType {
        return None;
    }
    let (_, ensures_src) = cf.ensures.as_ref()?;
    let mut resolver = harness::resolver_for(lib_path).ok()?;
    let private_fields = resolver.private_field_names(&cf.import_path())?;
    private_fields
        .into_iter()
        .find(|field| ensures_mentions_field(ensures_src, field))
}

/// Whether `text` reads a `.field` access on `field`, checked on a word
/// boundary so `.n` does not also match a field named `note`.
fn ensures_mentions_field(text: &str, field: &str) -> bool {
    let needle = format!(".{field}");
    let mut start = 0;
    while let Some(pos) = text[start..].find(&needle) {
        let abs = start + pos;
        let after = abs + needle.len();
        let boundary_ok = match text[after..].chars().next() {
            Some(c) => !c.is_alphanumeric() && c != '_',
            None => true,
        };
        if boundary_ok {
            return true;
        }
        start = abs + 1;
    }
    false
}

fn self_return_private_field_diag(node_id: &str, fn_name: &str, field: &str) -> Diagnostic {
    Diagnostic {
        code: "V0510".into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "ply".into(),
        check: "".into(),
        node_id: node_id.into(),
        title: format!(
            "Ply cannot check `{fn_name}` this way: its promise reads `.{field}`, a private field \
             of the value it returns, but the code Ply generates for this kind of check lives in \
             a separate crate that cannot see a private field of your type -- so this is refused \
             before it ever fails to compile, rather than after. `bounded`/`proved` do not have \
             this problem, because their generated code lives inside your own crate."
        ),
        pointer: None,
        primary_span: None,
        counterexample: None,
        fixes: vec![
            Fix {
                title: format!("check `{fn_name}` with `bounded(k)` instead of `fuzz`/`test`"),
                edits: vec![],
            },
            Fix {
                title: format!(
                    "or add a public accessor for `{field}` and write `{fn_name}`'s promise \
                     against that instead of the field directly"
                ),
                edits: vec![],
            },
        ],
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
    // Every rendered cex test this run earns, from every fn, accumulated
    // here rather than written immediately -- see `push_cex_test`.
    cex_tests_out: &mut Vec<RenderedTest>,
) -> Result<(String, Vec<String>, Vec<Diagnostic>)> {
    let check_label = format!("bounded({bound_k})");

    if !cf.is_bounded_supported() {
        // The split (task, 2026-08-27): a type the fuzz engine can build
        // inputs for is refused *by name*, naming what would work instead
        // (`V0508`) -- never `unsupported_shape_diag`'s "none of its
        // declared checks apply", which is false exactly here.
        let diag = if cf.is_fuzz_supported() {
            bounded_refused_sample_only_diag(node_id, fn_name, cf, &check_label)
        } else {
            unsupported_shape_diag(node_id, fn_name, cf)
        };
        return Ok(("unsupported".into(), vec![], vec![diag]));
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
            push_cex_test(
                cex_tests_out,
                RenderedTest {
                    test_name: rendered.test_name,
                    source: rendered.source,
                },
            );
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
            let test_file_display = cex_test_display_path(src_dir);
            push_cex_test(
                cex_tests_out,
                RenderedTest {
                    test_name: rendered.test_name.clone(),
                    source: rendered.source.clone(),
                },
            );
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
                    cargo_test: Some(test_file_display),
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
    /// Whether `fuzz` earned a `fuzzed(n)` verdict whose cases were grown
    /// from a corpus of known-valid values rather than sampled uniformly
    /// (docs/reach-measurement-2.md) -- `false` for every fn whose
    /// constructor was not seeded at all, which is the vast majority.
    /// `run_fn_checks` turns this into the `seeded` status (the same
    /// structural pattern `conditional` already follows: a status that
    /// travels with the verdict, propagates into the recorded result, and
    /// survives a reused verdict, never a warning about an incidental fact).
    seeded: bool,
}

#[allow(clippy::too_many_arguments)]
fn run_fuzz_and_test_checks(
    cf: &ContractFn,
    src_dir: &Path,
    harness_workspace_root: &Path,
    harness_pkg: &str,
    node_id: &str,
    fn_name: &str,
    wants_fuzz: Option<u32>,
    wants_test: bool,
    seed: &[u8; 32],
    has_examples: bool,
    opts: &VerifyOptions,
    cex_tests_out: &mut Vec<RenderedTest>,
) -> Result<HarnessRun> {
    let timeout = opts
        .engine_timeout_secs
        .unwrap_or_else(default_secondary_engine_timeout_secs);
    // The generated harness names its module and test fn from `cf.ident()`
    // (`path.replace("::", "_")`), never from the claim's own spelling --
    // a `Type::method` claim (or a free fn nested in a module) still
    // carries `::` in `fn_name`, and Rust identifiers cannot. Building this
    // filter from `fn_name` instead matched nothing at all (`cargo test`'s
    // filter is a plain substring on the real, underscored test path), so
    // every method/nested-fn fuzz or test check silently ran zero cases
    // and fell through to the same "no failure seen" success branch a real
    // pass takes -- a run that checked nothing, reading as a clean verdict
    // (adversarial review, 2026-08-27, found while building this task's own
    // multi-module fixture: `Root::five`'s own violated promise reported
    // `tested`/held instead of a violation, because the filter below
    // matched zero of the harness crate's tests). See `harness_module_name`.
    let filter = harness_test_filter(cf);
    let run =
        fuzz_engine::run_harness_tests(harness_workspace_root, harness_pkg, &filter, timeout)?;
    let module_prefix = format!("{}::", harness_module_name(cf));
    // The harness ever having *run* at all is judged from the actual libtest
    // per-test lines Ply's own module contributed, never from the process
    // exit code: `cargo test`'s exit code is 0 whenever nothing it was asked
    // to run failed, and that is also exactly what happens when the filter
    // matched no test at all -- a receiver method with no worked examples
    // and no direct-contract cases generates no test module, the filter then
    // selects nothing, and "0 passed; 0 failed" used to be read as a clean
    // pass with zero evidence behind it (the eleventh false pass,
    // docs/review-strings-receivers.md finding 1; §1's absence-of-evidence
    // rule applies to this check the same as any other). Scoped to this
    // function's own module prefix, not a raw count of everything cargo
    // happened to execute, because that count can include another
    // function's tests too (finding 2's `parse`/`util::parse` collision) --
    // see `count_tests_executed`'s own doc.
    let tests_executed = fuzz_engine::count_tests_executed(&run.combined_output, &module_prefix);
    // Per-*check* counts, not just the per-function one above (2026-08-27,
    // "also fix": declaring `[test, fuzz(n)]` together on one fn silently
    // dropped whichever check ran nothing, undoing this morning's own "a
    // pass must prove a case ran" fix the moment two checks shared a
    // harness module. `test` and `fuzz` generate their tests into that one
    // module under their own name prefixes (`fuzz_gen`'s `ply_fuzz_`,
    // `ply_example_`, `ply_direct_`), so each check's own count is that
    // prefix's own line count under this fn's module, never the module-wide
    // total: a `fuzz` test executing must not paper over a `test` check
    // that compiled to nothing beside it, and the reverse.
    let fuzz_tests_executed = fuzz_engine::count_tests_executed(
        &run.combined_output,
        &format!("{module_prefix}ply_fuzz_"),
    );
    let test_tests_executed = fuzz_engine::count_tests_executed(
        &run.combined_output,
        &format!("{module_prefix}ply_example_"),
    ) + fuzz_engine::count_tests_executed(
        &run.combined_output,
        &format!("{module_prefix}ply_direct_"),
    );

    let mut diagnostics = Vec::new();
    let fuzz_test_name = harness_fuzz_test_name(cf);
    let mut fuzz_label = None;
    let mut test_label = None;
    let mut fuzz_cases_reached: Option<u32> = None;
    // Whether `fuzz` specifically produced evidence worth naming in §8's
    // `evidence` block -- distinct from "the harness ran at all", since a
    // sibling `test` check running is not evidence for `fuzz` (see the
    // `fuzz_tests_executed == 0` branch below, "also fix" task 2026-08-27).
    let mut fuzz_ran = true;
    // Set once, in the one branch that earns a real `fuzzed(n)` verdict
    // whose cases were grown from a corpus of known-valid values (docs/
    // reach-measurement-2.md) -- `false` otherwise, including for every fn
    // whose constructor was never seeded at all.
    let mut seeded = false;

    // The harness never ran at all for this fn (2026-08-24 M4 review, D1 --
    // the review's most serious finding, widened 2026-08-27 to catch the
    // shape that slipped past it: a run that exits 0 with nothing to show
    // for it, not just a run that visibly fails to build). The commonest
    // cause is still a harness crate that failed to *compile*, which
    // produces no libtest per-test lines at all; the newer cause is a
    // function whose harness module compiled to nothing (no fuzz/test
    // bodies were ever generated for it) so the filter matched zero tests
    // and cargo exited success having done nothing. Both the `fuzz` and the
    // `test` check live in that one crate, so neither ran. §8: adapters
    // never pass engine output through raw -- they parse it or fail with
    // `X0901` attaching what the engine said. It is emphatically not a pass
    // (there is no evidence) and not a violation (§5.4c MUST: no violation
    // without a witness).
    if !run.timed_out && tests_executed == 0 {
        let cause = fuzz_engine::first_build_error(&run.combined_output);
        let module = harness_module_name(cf);
        if let Some(n) = wants_fuzz {
            diagnostics.push(harness_did_not_run_diag(
                node_id,
                fn_name,
                &module,
                &format!("fuzz({n})"),
                harness_pkg,
                cause.as_deref(),
                has_examples,
                cf.receiver.is_some(),
            ));
        }
        if wants_test {
            diagnostics.push(harness_did_not_run_diag(
                node_id,
                fn_name,
                &module,
                "test",
                harness_pkg,
                cause.as_deref(),
                has_examples,
                cf.receiver.is_some(),
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
            seeded: false,
        });
    }

    // Present exactly when this fn's own constructor was seeded at all
    // (`fuzz_gen::plan_receiver_seeding` decided to, and its generated
    // harness printed the marker unconditionally either way) -- `None`
    // means "not a seeded run", not "a seeded run this parser missed".
    let seed_stats = fuzz_engine::parse_seed_stats_marker(&run.combined_output);

    if let Some(n) = wants_fuzz {
        let check_label = format!("fuzz({n})");
        if !run.timed_out && fuzz_tests_executed == 0 {
            // The harness crate built, and *something* under this fn's
            // module ran (the module-wide gate above already returned
            // otherwise) -- but nothing `fuzz` itself generated executed.
            // The commonest way to reach this is `[test, fuzz(n)]`
            // declared together on a receiver method: `test` alone
            // generates nothing for it (see `harness_did_not_run_diag`'s
            // own receiver case below), which used to make the module-wide
            // count zero and correctly report `test` as a tool error --
            // but the moment `fuzz` also ran, the module-wide count turned
            // nonzero and this branch never even ran, so `fuzz` looked
            // like a pass. Never here: an X0901 tool error, exactly the
            // one `harness_did_not_run_diag` already writes for a check
            // that ran nothing, just keyed to `fuzz` specifically rather
            // than the whole fn.
            diagnostics.push(harness_did_not_run_diag(
                node_id,
                fn_name,
                &harness_module_name(cf),
                &check_label,
                harness_pkg,
                None,
                has_examples,
                cf.receiver.is_some(),
            ));
            fuzz_label = Some("tool_error".into());
            fuzz_ran = false;
        } else if run.timed_out && !run.failed_tests.iter().any(|t| t == &fuzz_test_name) {
            // The `failed_tests` guard is the whole point of this arm's
            // shape. `test` and `fuzz` share one cargo subprocess and one
            // deadline, so a slow fuzz run can outlive a test that already
            // failed -- and this arm used to fire first, relabelling an
            // observed, reported failure as a timeout and then saying, in as
            // many words, "never as a violation". The failure was sitting in
            // `run.failed_tests`, captured before the kill; the classifier
            // threw it away (external review, 2026-08-30).
            //
            // A timeout label now means what it says: this check produced no
            // failure of its own before the clock ran out. If it did, the
            // violation arm below owns it, whatever else was still running.
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
                cex_tests_out,
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
            // The seeded case's own dead end, repaired (docs/reach-
            // measurement-2.md): a gated text constructor that never
            // accepted even one draw -- no `examples:` seed and nothing the
            // constructor itself ever accepted -- has no case base to grow
            // from at all, and the refusal must name the exact action that
            // would fix it, not just restate the same generic advice every
            // other high-rejection abort already gets.
            let ctor_name = seed_stats
                .as_ref()
                .filter(|s| s.examples == 0 && s.accepted == 0)
                .and(cf.receiver.as_ref())
                .map(|plan| harness::last_two_segments(&plan.constructor));
            let title = match &ctor_name {
                Some(ctor_name) => format!(
                    "this type is built by parsing, and random text almost never parses ({rejected} \
                     of {total} draws rejected -- no case base to grow from). So this function has \
                     no fuzz evidence at all -- its verdict is `unclaimed`, not `fuzzed({n})`. Add an \
                     `examples:` entry showing one valid call to `{ctor_name}`, and Ply will grow \
                     inputs from it. (W0503)"
                ),
                None => format!(
                    "proptest gave up on `{fn_name}` before it could run the {n} cases `fuzz({n})` \
                     asked for: {rejected} of the {total} inputs it generated were thrown away by the \
                     function's own `#[ply::requires]` precondition and only {accepted} were ever \
                     checked, which tripped proptest's own limit ({reason}). So this function has no \
                     fuzz evidence at all -- its verdict is `unclaimed`, not `fuzzed({n})`. (W0503)"
                ),
            };
            let mut fixes = vec![
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
            ];
            if let Some(ctor_name) = &ctor_name {
                fixes.push(Fix {
                    title: format!(
                        "add an `examples:` entry in ply.yaml with one valid call to `{ctor_name}` -- \
                         Ply extracts its literal arguments and grows future draws from them"
                    ),
                    edits: vec![],
                });
            }
            diagnostics.push(Diagnostic {
                code: "W0503".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "proptest".into(),
                check: check_label.clone(),
                node_id: node_id.into(),
                title,
                pointer: None,
                primary_span: None,
                counterexample: None,
                fixes,
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
            // A receiver method is never the zero-input shape below, even
            // when its own checked call takes no parameters
            // (`Bucket::capacity`, this fixture's own case, found by the
            // e2e run itself, 2026-08-27): each of the `n` cases builds a
            // receiver over a *randomly chosen* sequence length (0 up to
            // `MAX_RECEIVER_SEQUENCE_LEN`), so `n` calls really do explore
            // `n` different receiver states, not one repeated case -- the
            // opposite mistake from the one this whole check exists to
            // prevent would be reporting real variation as if there were
            // none.
            if cf.params.is_empty() && cf.receiver.is_none() {
                // A zero-input fn's own honesty requirement (adversarial
                // review, 2026-08-27): `n` calls to a function with no
                // parameters are `n` repetitions of the *one* possible
                // call, not `n` different samples of an input space that
                // does not exist here. `fuzzed(n)` reads as "n samples
                // were checked", which is false, and reads a bigger `n` as
                // stronger evidence, which would be false too -- raising
                // it would look at nothing new. `tested` (§5.4c's own word
                // for "a concrete case was run and held") is the honest
                // reading, and the diagnostic says why so the
                // smaller-looking verdict does not read as an unexplained
                // demotion.
                diagnostics.push(zero_input_sampled_diag(node_id, fn_name, &check_label));
                fuzz_label = Some("tested".into());
                fuzz_cases_reached = Some(1);
            } else {
                // The earned verdict is `fuzzed(n)` (past tense, §5.4c's
                // own check->verdict table), never the declared check
                // spelling `fuzz(n)` -- those two strings look alike
                // enough that this was wrong here once already; the
                // difference matters because `rank()`/
                // `combine_fn_check_verdicts` key off the `fuzzed` prefix.
                fuzz_label = Some(format!("fuzzed({n})"));
                fuzz_cases_reached = Some(n);
                // The honest disclosure the design brief calls the "part
                // that must not be cut": a run whose cases were grown from
                // a corpus of known-valid values, not sampled uniformly,
                // says so -- named with the real counts this run actually
                // produced, never the illustrative numbers from any write-
                // up. Gated on there being at least one seed (an example or
                // a runtime accept): a seeded constructor whose corpus
                // stayed empty the whole run drew uniformly throughout,
                // indistinguishable from an unseeded one, and must read
                // that way.
                if let Some(stats) = &seed_stats
                    && (stats.examples > 0 || stats.accepted > 0)
                {
                    let ctor_name = cf
                        .receiver
                        .as_ref()
                        .map(|plan| harness::last_two_segments(&plan.constructor))
                        .unwrap_or_else(|| fn_name.to_string());
                    let corpus_total = stats.examples + stats.accepted;
                    diagnostics.push(Diagnostic {
                        code: "W0523".into(),
                        severity: "info".into(),
                        phase: "verify".into(),
                        engine: "proptest".into(),
                        check: check_label.clone(),
                        node_id: node_id.into(),
                        title: format!(
                            "random text almost never satisfies `{ctor_name}` ({rejected} of \
                             {total} draws thrown away by its own precondition), so the {n} \
                             cases were grown from {corpus_total} known-valid values: \
                             {examples} from the `examples:` you wrote, {accepted} that \
                             `{ctor_name}` accepted from random draws during this run. This is \
                             evidence about inputs *near* ones already known to be valid, not \
                             about the whole space of text. The {n} cases are real and each one \
                             ran. (W0523)",
                            rejected = stats.rejected,
                            total = stats.total,
                            examples = stats.examples,
                            accepted = stats.accepted,
                        ),
                        pointer: None,
                        primary_span: None,
                        counterexample: None,
                        fixes: vec![Fix {
                            title: "add an `examples:` entry for an extreme case you care about \
                                 (a very long value, a boundary number) -- mutating short, \
                                 ordinary seeds rarely reaches one on its own"
                                .to_string(),
                            edits: vec![],
                        }],
                        assumptions: vec![],
                        open_item: Some("seeded_generation".into()),
                    });
                    seeded = true;
                }
            }
        }
    }

    if wants_test {
        // Anchored to this fn's own module (2026-08-27, misattribution fix,
        // docs/review-strings-receivers.md finding 2): `cargo test`'s own
        // filter argument is a plain substring match, so the invocation
        // above can have executed another function's tests too whenever one
        // harness module's identifier is a suffix of another's (a top-level
        // `parse` and a `util::parse` collide this way: `parse_harness::` is
        // a substring of `util_parse_harness::`). Filtering
        // `run.failed_tests` by `.contains("::ply_direct_")` alone -- with
        // no check that the failing test belongs to *this* fn's module --
        // let `parse` be blamed for `util::parse`'s own broken promise, in a
        // sentence that called those tests "its own". Requiring the name to
        // start with `module_prefix` is what keeps this fn's verdict keyed
        // to its own tests only.
        let failing_test_checks: Vec<&String> = run
            .failed_tests
            .iter()
            .filter(|t| {
                t.starts_with(&module_prefix)
                    && (t.contains("::ply_example_") || t.contains("::ply_direct_"))
            })
            .collect();
        if !run.timed_out && test_tests_executed == 0 {
            // Same guard as `fuzz`'s own above, for the same reason: the
            // module-wide count above only proves *something* under this
            // fn ran, which a sibling `fuzz` check declared on the same fn
            // is enough to satisfy even when `test` itself generated
            // nothing (no `examples:`, no direct-contract case -- the
            // common receiver-method shape). Without this, `[test,
            // fuzz(n)]` on such a fn reported `test` as `tested` on zero
            // cases the moment `fuzz` ran at all: `failing_test_checks` is
            // trivially empty when nothing of `test`'s own ran either, and
            // the `else` arm below used to read that silence as a pass.
            diagnostics.push(harness_did_not_run_diag(
                node_id,
                fn_name,
                &harness_module_name(cf),
                "test",
                harness_pkg,
                None,
                has_examples,
                cf.receiver.is_some(),
            ));
            test_label = Some("tool_error".into());
        } else if run.timed_out && failing_test_checks.is_empty() {
            // Same guard, same reason as `fuzz`'s arm above: one subprocess
            // and one deadline serve both checks, so a slow sibling can
            // outlive a test of this check's own that already failed and was
            // reported. This arm firing first turned an observed violation
            // into a timeout, and told the reader there was no violation
            // (external review, 2026-08-30). A concrete failure dominates.
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
        fuzz_ran,
        fuzz_cases_reached,
        diagnostics,
        seeded,
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
/// The generated harness's own module name for `cf`: `cf.ident()`
/// (`path.replace("::", "_")`) with `_harness` appended, matching
/// `fuzz_gen`'s `mod {module_ident}_harness { ... }` exactly. Every place
/// that names or filters this module *from outside* the generated code --
/// a `cargo test`/cargo-mutants filter, a check against `run.failed_tests`,
/// a diagnostic telling a user what to run themselves -- must go through
/// this (or [`harness_test_filter`]/[`harness_fuzz_test_name`] below),
/// never through the claim's own spelling (`fn_name`): a `Type::method`
/// claim (or a free fn nested in a module) still carries `::`, which
/// cannot appear in a Rust identifier, so a filter or name built from
/// `fn_name` instead matched *none* of the harness crate's real tests.
/// `cargo test`'s own filter is a plain substring match, so that read as
/// "0 passed; 0 failed" -- success, with nothing checked -- and fell
/// through to the very same branch a genuine clean pass takes (adversarial
/// review, 2026-08-27, found building this task's own multi-module
/// fixture: a method's own violated promise reported as holding).
fn harness_module_name(cf: &ContractFn) -> String {
    format!("{}_harness", cf.ident())
}

/// The `cargo test`/cargo-mutants filter that selects exactly `cf`'s own
/// generated tests and nothing else's -- see [`harness_module_name`].
fn harness_test_filter(cf: &ContractFn) -> String {
    format!("{}::", harness_module_name(cf))
}

/// The fully qualified name of `cf`'s own generated fuzz test, matching an
/// entry in `run.failed_tests` exactly -- see [`harness_module_name`].
fn harness_fuzz_test_name(cf: &ContractFn) -> String {
    format!("{}::ply_fuzz_{}", harness_module_name(cf), cf.ident())
}

#[allow(clippy::too_many_arguments)]
fn harness_did_not_run_diag(
    node_id: &str,
    fn_name: &str,
    module: &str,
    check_label: &str,
    harness_pkg: &str,
    cause: Option<&str>,
    has_examples: bool,
    is_receiver_method: bool,
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
    // The lede names *what actually happened*, and it must not claim a
    // compile failure that never occurred (adversarial review, 2026-08-27):
    // before this, every "ran zero cases" report said "failed to compile"
    // unconditionally, even for a harness crate that built cleanly but had
    // nothing generated for this check to run at all -- the shape behind
    // the eleventh false pass (docs/review-strings-receivers.md finding 1).
    // A receiver method (`&self`/`&mut self`) declared with only `test` and
    // no `examples:` is the concrete, nameable case: `test` only ever runs
    // a fixed example or a concrete-input case built from the function's
    // own parameters, and a receiver method has no such value for this tier
    // to call it on -- that is the sampling tier's own job (`fuzz(n)`),
    // never this one's.
    let (lede, receiver_fix) = if cause.is_some() {
        (
            format!(
                "`{fn_name}`'s `{check_label}` check ran zero cases: the test harness Ply generates \
                 for it failed to compile, so nothing was checked at all."
            ),
            None,
        )
    } else if check_label == "test" && is_receiver_method && !has_examples {
        (
            format!(
                "`{fn_name}`'s `test` check ran zero cases: `{fn_name}` needs a value to call it on \
                 (it takes `&self`/`&mut self`), and `test` only ever runs a fixed `examples:` entry \
                 or a concrete-input case built directly from the function's own parameters -- \
                 `{fn_name}` declares no `examples:` and has no such receiver value for this check to \
                 build one from, so Ply's generated harness has nothing of that kind to run for it. \
                 The `fuzz(n)` check builds its own receiver value and does check this function; \
                 `test` alone cannot."
            ),
            Some(Fix {
                title: format!(
                    "add `fuzz(n)` to `{fn_name}`'s checks -- it builds its own receiver value to \
                     call `{fn_name}` on -- or add `examples:` entries in ply.yaml naming concrete \
                     calls for `test` to assert"
                ),
                edits: vec![],
            }),
        )
    } else {
        (
            format!(
                "`{fn_name}`'s `{check_label}` check ran zero cases: Ply's generated test harness \
                 compiled, but nothing was ever generated for it to run under this check."
            ),
            None,
        )
    };
    let mut fixes = vec![Fix {
        title: format!(
            "see the full compiler output by running `cargo test -p {harness_pkg} --lib \
             {module}::` from the crate root (Ply regenerates that harness crate on \
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
    if let Some(f) = receiver_fix {
        fixes.insert(0, f);
    }
    Diagnostic {
        code: "X0901".into(),
        severity: "error".into(),
        phase: "verify".into(),
        engine: "proptest".into(),
        check: check_label.into(),
        node_id: node_id.into(),
        title: format!(
            "{lede} This is reported as a tool error -- never as a pass, because no evidence was \
             gathered, and never as a violation, because there is no failing input to show.\
             {compiler_says}{examples_hint} (X0901)"
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
    cex_tests_out: &mut Vec<RenderedTest>,
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
        // A receiver method's shrunk minimal input describes the *whole*
        // generated value -- the constructor's own arguments, the bounded
        // operation sequence, and only then the checked call's own
        // arguments -- never a 1:1 positional match to `cf.params` alone
        // (2026-08-27, receiver construction). Zipping it against
        // `cf.params` by position the way a plain fn's panic-recovered
        // input already is would silently mislabel it. Carried as one
        // opaque field instead: `decode_marker_fields` below will not find
        // any of `cf.params`' own names in it, so this naturally falls
        // through to the witness-only (W0541) branch -- honest evidence,
        // never a fabricated per-parameter reading.
        None if cf.receiver.is_some() => fuzz_engine::parse_proptest_minimal_input(combined_output)
            .map(|values| {
                let mut m = BTreeMap::new();
                m.insert("__ply_receiver_and_sequence".to_string(), values.join(", "));
                m
            }),
        // A struct/enum parameter (2026-08-28, docs/review-structs-enums.md's
        // "Also fix" list, "a crash in a function whose only parameter is a
        // struct loses its witness"): proptest's shrunk minimal input
        // describes the struct's own *leaf fields* -- `(lo, hi)` for a
        // two-field `Window`, say -- never a single value per entry in
        // `cf.params`, so a one-struct-parameter fn has one declared
        // parameter and two (or more, or zero) recovered values. The plain
        // zip below requires the counts to match and silently discards the
        // witness the moment they do not, which is every time a struct
        // parameter's own field count is not exactly one -- the same
        // mismatch the receiver arm just above was already taught to carry
        // through as one opaque field instead of losing, in this same
        // window (2026-08-27), and this arm was not. Any struct/enum
        // parameter routes here, matching that same discipline: carried as
        // one opaque field, which naturally fails `decode_marker_fields`'s
        // per-parameter decode below and falls through to the honest
        // witness-only (`W0541`) branch, never a renderer that would guess
        // wrong about which leaf belongs to which field.
        None if cf.params.iter().any(|p| {
            matches!(
                p.ty,
                RustType::UserTypeCtor(_) | RustType::UserTypeFields(_)
            )
        }) =>
        {
            fuzz_engine::parse_proptest_minimal_input(combined_output).map(|values| {
                let mut m = BTreeMap::new();
                m.insert("__ply_struct_params_raw".to_string(), values.join(", "));
                m
            })
        }
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
                             {}_harness::` from the crate root -- and read proptest's own report \
                             of the failing input",
                            cf.ident()
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

    // A receiver method is never rendered as a replay test, however this
    // failure was found (2026-08-27, Blocker 3: "Ply breaks the user's own
    // build"). `decode_marker_fields` only ever looks at `cf.params` -- the
    // *checked call's own* arguments -- and for a receiver method with no
    // parameters of its own (`Calc::value(&self) -> u32`, this task's own
    // repro) that is an empty list, which trivially decodes as `Some(vec![])`
    // even though the real counterexample also depends on a receiver Ply
    // built (the constructor call plus a sequence of operations) that is not
    // in `cf.params` at all. Taking the `Some` branch there rendered
    // `Calc::value()` with no receiver argument at all -- a replay test that
    // does not compile, breaking the user's own `cargo test` the moment a
    // broken promise was found on any receiver method with an empty or fully
    // scalar parameter list. There is no renderer for "the receiver Ply
    // built plus a bounded operation sequence" (only `cf.params` ever gets
    // written back out as Rust source), so a receiver method always takes
    // the witness-only (`W0541`) path below -- honest evidence, never a
    // test that cannot build.
    let decoded = if cf.receiver.is_some() {
        None
    } else {
        fuzz_engine::decode_marker_fields(&fields, &cf.params)
    };
    match decoded {
        Some(values) => {
            let rendered = contract_rt::render_cex_test(cf, &values, check_label, "P0502", 1)?;
            let test_file_display = cex_test_display_path(src_dir);
            push_cex_test(
                cex_tests_out,
                RenderedTest {
                    test_name: rendered.test_name.clone(),
                    source: rendered.source.clone(),
                },
            );
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
                            "captured from proptest shrinking on harness `{}`, replayable with \
                             `--seed {seed_hex}` (field named `kani_witness` for §8 schema \
                             stability; this witness is proptest-, not Kani-, sourced -- see \
                             docs/m4-findings.md)",
                            harness_fuzz_test_name(cf)
                        )),
                        cargo_test: Some(test_file_display),
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
            // A receiver method's raw shrunk witness lives under its own
            // synthetic key (`__ply_receiver_and_sequence`, set above),
            // never under one of `cf.params`' own names -- so the loop just
            // above never copies it, and the reader would see an empty
            // `inputs` map despite real evidence existing. Carried through
            // here under a name the title above already explains, rather
            // than silently dropped (2026-08-27, found running this
            // against `tests/fixtures/receiverseq`).
            if let Some(raw) = fields.get("__ply_receiver_and_sequence") {
                inputs.insert("receiver_and_sequence".to_string(), raw.clone());
            }
            // A struct/enum parameter's raw shrunk witness, carried under
            // its own synthetic key exactly like the receiver case just
            // above -- same reason, same fix (2026-08-28).
            if let Some(raw) = fields.get("__ply_struct_params_raw") {
                inputs.insert("params_raw".to_string(), raw.clone());
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
                    title: if cf.receiver.is_some() {
                        unrenderable_receiver_inputs_title(fn_name, &contract_text, from_panic)
                    } else {
                        unrenderable_inputs_title(fn_name, &contract_text, &cf.params, from_panic)
                    },
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
    from_panic: bool,
) -> String {
    unrenderable_inputs_title_impl(fn_name, contract_text, params, false, from_panic)
}

/// Same message, with one extra sentence when the failing case came from a
/// receiver Ply built itself (2026-08-27, receiver construction): the
/// shrunk minimal input the engine printed describes the constructor call
/// and the bounded operation sequence too, not only the checked method's own
/// arguments, and Ply cannot yet decompose that back into named steps --
/// naming that plainly beats letting a reader assume the raw text below is
/// just the checked call's own parameters.
fn unrenderable_receiver_inputs_title(
    fn_name: &str,
    contract_text: &str,
    from_panic: bool,
) -> String {
    unrenderable_inputs_title_impl(fn_name, contract_text, &[], true, from_panic)
}

/// `from_panic` (2026-08-27, docs/review-caveats.md N2: "a crash is
/// described as fails its own contract"): a body that panics before its
/// postcondition is ever evaluated has broken its promise by crashing, not
/// by returning a value the promise rejects, and the plain-function path
/// (`render_fuzz_violation`'s `Some(values)` branch, a few lines above this
/// one) already says so in those words. This witness-only branch used to
/// say "fails its own contract" unconditionally, on a receiver method
/// always and on a plain function whenever its input could not be
/// rendered -- collapsing a crash and a broken postcondition into one
/// sentence, which reads a stack-unwinding bug as though the function ran
/// to completion and simply lied.
fn unrenderable_inputs_title_impl(
    fn_name: &str,
    contract_text: &str,
    params: &[harness::Param],
    from_receiver: bool,
    from_panic: bool,
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
    let what = if from_receiver {
        "the receiver it built (the constructor call and the sequence of operations run before \
         the checked call) together with that call's own arguments"
            .to_string()
    } else if blocked.is_empty() {
        "that input".to_string()
    } else {
        format!("parameter(s) {}", blocked.join(", "))
    };
    if from_panic {
        format!(
            "`{fn_name}` does not return at all for at least one input Ply generated -- it \
             panicked before its postcondition `{contract_text}` could even be evaluated, and \
             proptest shrank that input down to the smallest one that still crashes. Ply cannot \
             turn it into a runnable Rust test, though: it has no way yet to write {what} back \
             out as a literal value in Rust source. A function that panics has broken its promise \
             as surely as one that returns a wrong answer, so this is a violation, with a witness \
             -- just not a replayable one. The failing input is recorded below exactly as the \
             engine reported it -- Ply never invents one. (W0541, reason: inputs_unrenderable)"
        )
    } else {
        format!(
            "`{fn_name}` fails its own contract `{contract_text}` for at least one input, and \
             proptest shrank that input down to the smallest one that still fails. Ply cannot \
             turn it into a runnable Rust test, though: it has no way yet to write {what} back \
             out as a literal value in Rust source. The failing input is recorded below exactly \
             as the engine reported it -- Ply never invents one. (W0541, reason: \
             inputs_unrenderable)"
        )
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

#[allow(clippy::too_many_arguments)]
fn run_mutate_check(
    crate_dir: &Path,
    harness_pkg: &str,
    standalone: bool,
    node_id: &str,
    fn_name: &str,
    test_filter: &str,
    checks: &[Check],
    opts: &VerifyOptions,
) -> Result<(MutateOutcome, Vec<Diagnostic>)> {
    let _ = checks;
    if standalone {
        // docs/review-caveats.md N1: `cargo mutants -p <target> --test-package
        // <harness>` (`engines::mutants`) resolves both names against one
        // `cargo metadata` call rooted at the target crate, so it only works
        // when the harness is a member of *that* workspace. Ply no longer
        // makes it one uninvited (that used to mean either editing a crate
        // that had no workspace at all, or breaking a crate that already
        // belonged to someone else's -- both reproduced in the review this
        // fixes). So on this crate's layout, `mutate` is refused by name,
        // honestly, rather than handed to cargo-mutants to fail on a package
        // spec it cannot resolve.
        return Ok((
            MutateOutcome::Inconclusive("unsupported"),
            vec![Diagnostic {
                code: "V0505".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "cargo-mutants".into(),
                check: "mutate".into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` declares `mutate`, but this crate's layout doesn't support it yet: \
                     mutation testing needs the crate under test and Ply's generated test harness \
                     to sit in one shared Cargo workspace, and this crate has no `[workspace]` table \
                     of its own for Ply to register the harness into. Ply does not add one \
                     automatically -- doing that used to either do nothing or break a project that \
                     already belonged to a different workspace. This is reported as unsupported, not \
                     attempted, so nothing here says the spec is weak; it does mean `mutate` produced \
                     no evidence, so the run does not pass."
                ),
                pointer: None,
                primary_span: None,
                counterexample: None,
                fixes: vec![
                    Fix {
                        title:
                            "add an empty `[workspace]` table to this crate's own Cargo.toml to \
                                enable `mutate` (only safe when this crate is not already a member \
                                of a different workspace)"
                                .into(),
                        edits: vec![],
                    },
                    Fix {
                        title: format!(
                            "or drop `mutate` from `{fn_name}`'s checks for now -- `fuzz`/`test` run \
                             on this crate's layout already"
                        ),
                        edits: vec![],
                    },
                ],
                assumptions: vec![],
                open_item: Some("unsupported".into()),
            }],
        ));
    }
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
        // `test_filter` is the caller's `harness_test_filter(cf)`, never
        // built from `fn_name` here: the generated harness names its
        // module from `cf.ident()` (`path.replace("::", "_")`), and a
        // `Type::method` claim's own spelling still carries `::`, which
        // cannot appear in a Rust identifier. Building this filter from
        // `fn_name` matched none of the harness crate's real tests --
        // `cargo test`'s filter is a plain substring on the actual,
        // underscored path -- so `mutate` on any method silently ran its
        // kill-signal tests against nothing and could never have caught a
        // surviving mutant (adversarial review, 2026-08-27, the same root
        // cause as the fuzz/test tier's own version of this bug above).
        test_filter: test_filter.to_string(),
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
        kani::WitnessValue::Duration(secs, nanos) => format!("{secs}.{nanos:09}s"),
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

/// A fn node for one of `verify`'s early-exit paths (an anchor that did not
/// resolve, refused, or ambiguous; an empty checks list; an unsupported
/// signature). `fn_name` is the claim's own key, used verbatim as the node
/// `id` -- **not** derived by splitting the component-qualified `node_id`
/// diagnostics use, which a method's own key (`Bucket::capacity`) would
/// corrupt: rsplitting *that* on `::` throws away `Bucket` and leaves only
/// `capacity`, indistinguishable from any other type's same-named method
/// and inconsistent with the id a *successful* claim gets elsewhere in this
/// file (always the untouched `fn_name`). Harmless before method support,
/// since no free-function key ever contained `::` where this fired; live
/// the moment one does.
/// What a claim promised, and what rests on a human's word -- attached to
/// the node that carries its verdict.
///
/// Set from the plan rather than from the run, and therefore identical
/// whether the verdict was earned this time or carried forward. A promise
/// is a property of the claim; it does not stop existing because the result
/// was reused. Wired to the run first, the second run came back bare --
/// which is how that was found.
fn attach_claim_text(node: &mut Node, cf: &ContractFn, claim: &FnClaim) {
    let mut requires: Vec<String> = claim.requires.clone();
    if let Some((_, src)) = &cf.requires {
        requires.push(src.clone());
    }
    let mut ensures: Vec<String> = claim.ensures.clone();
    if let Some((_, src)) = &cf.ensures {
        ensures.push(src.clone());
    }
    node.contract = ply_core::diag::Contract { requires, ensures };
    node.trusted = claim
        .trusted
        .iter()
        .map(|t| ply_core::diag::TrustedClaim {
            claim: t.claim.clone(),
            evidence: Some(t.evidence.clone()),
        })
        .collect();
}

fn leaf_node(fn_name: &str, verdict: &str) -> Node {
    Node {
        id: fn_name.to_string(),
        kind: "fn".into(),
        verdict: verdict.to_string(),
        statuses: vec![],
        reused: false,
        evidence: None,
        children: vec![],
        ..Default::default()
    }
}

#[allow(dead_code)]
fn unused(_p: &PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Defect 2 (2026-08-30, "a documented way of writing contracts is
    /// accepted, then silently ignored"): a fn claimed with `checks:
    /// [fuzz(64)]` plus a `requires:`/`ensures:` contract written directly
    /// in ply.yaml, on a function with no inline `#[ply::requires]`/
    /// `#[ply::ensures]` attribute at all, used to earn *two* warnings that
    /// flatly contradicted each other: one (`W0510`) said the ply.yaml
    /// contract "is used ... so this run checked `seven` against its inline
    /// attributes only" -- which is false, there are no inline attributes,
    /// nothing was checked against them -- and the other (`V0505`) said
    /// "there is nothing to check its result against, so nothing was run".
    /// A reader had to reconcile those alone.
    ///
    /// The actual fix is not fewer diagnostics: it is diagnostics that never
    /// claim more than they know. `W0510` still fires every time ply.yaml
    /// declares a contract (a later regression, 2026-08-31, narrowed that to
    /// only fire alongside an inline attribute, which silently dropped it
    /// for the case that matters most -- a `ply.yaml` contract with no
    /// inline attribute at all). It just no longer says what specifically
    /// was or was not checked, so having both `V0505` and `W0510` fire
    /// together is no longer a contradiction, only two honest facts.
    #[test]
    fn a_yaml_only_contract_on_an_unattributed_fn_earns_two_diagnostics_that_agree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"yamlonly-demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn seven() -> u32 {\n    7\n}\n",
        )
        .unwrap();
        let yaml_path = dir.path().join("ply.yaml");
        std::fs::write(
            &yaml_path,
            "ply: 1\ncomponents:\n  demo:\n    anchor: yamlonly_demo\n    fns:\n      seven:\n        \
             checks: [fuzz(64)]\n        requires: [\"true\"]\n        ensures: [\"|result| \
             *result == 7\"]\n",
        )
        .unwrap();
        let loaded = config::load(&yaml_path).unwrap();

        let result = verify_loaded_crate(
            dir.path(),
            &VerifyOptions {
                engine_timeout_secs: Some(5),
                seed: None,
            },
            loaded,
        )
        .unwrap();

        let on_seven: Vec<&Diagnostic> = result
            .envelope
            .diagnostics
            .iter()
            .filter(|d| d.node_id == "demo::seven")
            .collect();
        assert_eq!(
            on_seven.len(),
            2,
            "two diagnostics that agree, not one that omits the ply.yaml fact: {on_seven:#?}"
        );
        for d in &on_seven {
            assert!(
                !d.title.contains("checked `seven` against its inline"),
                "must never claim inline attributes were checked when there are none: {}",
                d.title
            );
        }
        let v0505 = on_seven
            .iter()
            .find(|d| d.code == "V0505")
            .unwrap_or_else(|| panic!("expected a V0505 diagnostic: {on_seven:#?}"));
        let w0510 = on_seven
            .iter()
            .find(|d| d.code == "W0510")
            .unwrap_or_else(|| panic!("expected a W0510 diagnostic: {on_seven:#?}"));
        assert!(
            w0510.title.contains("ply.yaml"),
            "must name that ply.yaml declares a contract here, and that it is not checked: {}",
            w0510.title
        );
        assert!(
            w0510.title.contains("#[ply::requires]") || w0510.title.contains("#[ply::ensures]"),
            "must say what a reader should write instead: {}",
            w0510.title
        );
        assert!(
            v0505.title.contains("no `#[ply::ensures]`"),
            "must still say why nothing ran: {}",
            v0505.title
        );
    }

    #[test]
    fn verification_returns_the_loaded_snapshot_and_qualified_source_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"snapshot-demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn quote() -> u32 {\n    7\n}\n",
        )
        .unwrap();
        let yaml_path = dir.path().join("ply.yaml");
        std::fs::write(
            &yaml_path,
            "ply: 1\ncomponents:\n  alpha:\n    anchor: snapshot_demo\n    fns:\n      quote:\n        checks: []\n  beta:\n    anchor: snapshot_demo\n    fns:\n      quote:\n        checks: []\n",
        )
        .unwrap();
        let loaded = config::load(&yaml_path).unwrap();
        std::fs::write(&yaml_path, "this is no longer valid ply yaml: [").unwrap();

        let result = verify_loaded_crate(
            dir.path(),
            &VerifyOptions {
                engine_timeout_secs: None,
                seed: None,
            },
            loaded.clone(),
        )
        .unwrap();

        assert_eq!(
            result.document, loaded,
            "the API must return its input snapshot"
        );
        assert_eq!(
            result.source_map.keys().cloned().collect::<Vec<_>>(),
            vec!["alpha::quote", "beta::quote"],
            "same-named functions in separate components need distinct qualified keys"
        );
        for span in result.source_map.values() {
            assert_eq!(span.file, "src/lib.rs");
            assert_eq!(span.start, [0, 0]);
            assert_eq!(span.end, [2, 1]);
        }
    }

    /// §5.2a input 11: `PLY_VERSION` must be a real digest `build.rs`
    /// computed, never a placeholder or a hand-edited constant. This cannot
    /// prove the digest is *correct* (that needs an actual second build with
    /// different source, done at the e2e level -- `buildidentity_fixture.rs`),
    /// but it does pin the one property a unit test can: the value compiled
    /// into this binary looks like a blake3 hash, not the old `"0.1.0"`
    /// `CARGO_PKG_VERSION` string that never moved across fourteen fixes.
    #[test]
    fn ply_version_is_a_real_build_digest_not_the_old_hand_edited_constant() {
        assert_ne!(
            PLY_VERSION, "0.1.0",
            "PLY_VERSION must not be the hand-edited Cargo.toml version -- that constant is what \
             let fourteen fixes go unnoticed by every stored result (docs/review-silent-\
             narrowing.md §6)"
        );
        assert_eq!(
            PLY_VERSION.len(),
            64,
            "a blake3 hex digest is 64 characters; PLY_VERSION was {PLY_VERSION:?}"
        );
        assert!(
            PLY_VERSION.chars().all(|c| c.is_ascii_hexdigit()),
            "a blake3 hex digest is all hex characters; PLY_VERSION was {PLY_VERSION:?}"
        );
    }

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
            ..Default::default()
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
            false,
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
            false,
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
                ..Default::default()
            },
            Node {
                id: "b".into(),
                kind: "fn".into(),
                verdict: "tested".into(),
                statuses: vec![],
                reused: false,
                evidence: None,
                children: vec![],
                ..Default::default()
            },
        ];
        assert_eq!(
            worst_of(&children),
            "tested",
            "D6: a weak leaf drags its parent down"
        );
    }

    // -- the sampling/proving split (task, 2026-08-27): `bounded` on a
    // sample-only type is refused by name, distinctly from a type neither
    // engine can build inputs for, and the fuzz/test tier still runs.

    /// The defect this function replaces the old call site for: a plain
    /// `f64` parameter is fuzz-supported, so `unsupported_shape_diag`'s own
    /// "bad" list (filtered by `is_fuzz_supported`) comes back empty and it
    /// falls into the "none of its declared checks apply to this
    /// function's shape" branch -- false, since `fuzz`/`test` apply fine.
    /// `bounded_refused_sample_only_diag` must say the true thing instead:
    /// name the type, say it is sampleable but not provable, and say what
    /// to use instead.
    #[test]
    fn bounded_on_a_float_is_refused_by_name_and_says_what_would_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(
            &path,
            "#[ply::ensures(|result| *result >= x)]\npub fn increment(x: f64) -> f64 { x + 1.0 }\n",
        )
        .unwrap();
        let cf = ply_core::harness::discover_fn(&path, "increment").unwrap();
        let diag =
            bounded_refused_sample_only_diag("floats::increment", "increment", &cf, "bounded(2)");
        assert_eq!(diag.code, "V0508");
        assert_eq!(diag.severity, "warning");
        assert!(
            diag.title.contains("x: f64"),
            "must name the actual blocking parameter, not a vague sentence: {}",
            diag.title
        );
        assert!(
            diag.title.contains("fuzz") || diag.fixes.iter().any(|f| f.title.contains("fuzz(256)")),
            "must say what would work instead of `bounded`: {diag:?}"
        );
        assert!(
            !diag.title.contains("none of its declared checks apply"),
            "this is the exact false sentence a sample-only type must never get -- fuzz/test DO \
             apply here: {}",
            diag.title
        );
    }

    /// The other half of the split: a type neither engine can build inputs
    /// for (a plain struct, here) keeps its old `V0505` wording exactly --
    /// this fix narrows one call site, it does not touch the shared-by-both
    /// case.
    #[test]
    fn bounded_on_a_genuinely_unsupported_type_keeps_the_old_v0505_wording() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(
            &path,
            "pub struct Foo { pub x: u32 }\n#[ply::ensures(|result| *result >= 0)]\npub fn f(foo: Foo) -> i64 { 0 }\n",
        )
        .unwrap();
        let cf = ply_core::harness::discover_fn(&path, "f").unwrap();
        assert!(!cf.is_bounded_supported());
        assert!(
            !cf.is_fuzz_supported(),
            "a plain struct must stay refused on both engines, unchanged"
        );
        let diag = unsupported_shape_diag("structs::f", "f", &cf);
        assert_eq!(diag.code, "V0505");
        assert!(diag.title.contains("neither the bounded"), "{}", diag.title);
    }

    /// The NaN/infinity decision's own visibility requirement: the
    /// disclosure names the reason (false alarms on values the program may
    /// never see), not just the bare fact that floats were sampled.
    #[test]
    fn float_sampling_diag_names_nan_and_infinity_and_why_they_are_excluded() {
        let diag = float_sampling_diag("floats::increment", "increment", "fuzz(64)");
        assert_eq!(diag.code, "W0518");
        assert_eq!(
            diag.severity, "info",
            "nothing here is wrong or owed -- only worth naming"
        );
        assert!(diag.title.contains("NaN"), "{}", diag.title);
        assert!(diag.title.contains("infinity"), "{}", diag.title);
        assert!(
            diag.title.contains("false alarm") || diag.title.contains("false"),
            "must say *why* this matters, not merely that it happens: {}",
            diag.title
        );
    }

    /// The string exclusion's own visibility requirement, mirroring
    /// `float_sampling_diag_names_nan_and_infinity_and_why_they_are_excluded`
    /// exactly (also-fix, task 2026-08-27): the disclosure must name what
    /// is excluded (control characters), why, and what is deliberately NOT
    /// excluded (multi-byte Unicode) -- the same three-part shape the float
    /// precedent already earns.
    #[test]
    fn string_sampling_diag_names_control_characters_and_that_unicode_is_not_excluded() {
        let diag = string_sampling_diag("strings::preview", "preview", "fuzz(64)");
        assert_eq!(diag.code, "W0521");
        assert_eq!(
            diag.severity, "info",
            "nothing here is wrong or owed -- only worth naming"
        );
        assert!(diag.title.contains("control character"), "{}", diag.title);
        assert!(
            diag.title.contains("Unicode"),
            "must say multi-byte Unicode is NOT excluded, the way the float disclosure names \
             both sides of its own choice: {}",
            diag.title
        );
        assert!(
            diag.title.contains("false alarm"),
            "must say *why* this matters, not merely that it happens: {}",
            diag.title
        );
    }

    /// A minimal, hand-built `ReceiverPlan` for the disclosure-wording
    /// tests below -- every field `receiver_sequence_diag` does not read is
    /// filled with the cheapest value that type-checks, since the point of
    /// these tests is the sentence, not the plan.
    fn bare_receiver_plan(type_name: &str, constructor: &str) -> harness::ReceiverPlan {
        harness::ReceiverPlan {
            type_name: type_name.into(),
            import_path: type_name.into(),
            constructor: constructor.into(),
            ctor_params: vec![],
            ctor_requires: None,
            ctor_return: harness::CtorReturn::Bare,
            operations: vec![harness::Operation {
                call_path: "checked_method".into(),
                params: vec![],
                takes_mut_self: false,
            }],
            excluded_operations: vec![],
            other_constructors: vec![],
            max_sequence_len: 3,
        }
    }

    /// CLAUDE.md's rule for user-facing wording: pinned exact-string, so a
    /// later edit is reviewed like a diff to code, not waved through by a
    /// `.contains()` check that a much longer sentence would also satisfy.
    /// This is the tightened wording itself (docs/review-silent-narrowing.md
    /// §6: the old sentence measured 193 words per method per run and moved
    /// neither the verdict nor the exit code) -- every fact the old
    /// sentence carried is still here: the receiver was built by Ply, which
    /// constructor, the pool it could draw from, the bound, and the
    /// completeness claim that bound earns when nothing was excluded.
    #[test]
    fn receiver_sequence_diag_wording_is_pinned_when_nothing_was_narrowed() {
        let plan = bare_receiver_plan("Till", "Till::new");
        let diag = receiver_sequence_diag("till::Till::total", "total", &plan);
        assert_eq!(diag.code, "W0520");
        assert_eq!(diag.severity, "info");
        assert_eq!(
            diag.title,
            "`total` needs a `Till`, so Ply built one itself: `Till::new`, then up to 3 calls to \
             `total`, in random order, before the checked call. That covers every value `Till`'s \
             own code can reach within 3 steps of a fresh one -- nothing else was assumed. \
             (W0520, §5.4c)"
        );
    }

    /// Same pin for the narrowed case -- both an excluded operation and an
    /// unused second constructor at once, since that is the longest the
    /// sentence gets and is exactly what `docs/review-silent-narrowing.md`'s
    /// three reproductions need said together. The old wording could not
    /// say this at all (its "nothing here was assumed" claim is simply
    /// false the moment anything was excluded); the tightened wording says
    /// it in one sentence rather than two paragraphs.
    #[test]
    fn receiver_sequence_diag_wording_is_pinned_when_narrowed() {
        let mut plan = bare_receiver_plan("Acc", "Acc::new");
        plan.excluded_operations = vec![harness::ExcludedOperation {
            call_path: "Acc::note".into(),
            reason: "its `s: str` argument uses a type Ply cannot build a value for".into(),
        }];
        plan.other_constructors = vec!["Acc::preloaded".into()];
        let diag = receiver_sequence_diag("acc::Acc::get", "get", &plan);
        assert_eq!(diag.code, "W0520");
        assert_eq!(
            diag.severity, "warning",
            "an excluded operation or an unused constructor is a real coverage gap, not a \
             routine disclosure"
        );
        assert_eq!(
            diag.title,
            "`get` needs a `Acc`, so Ply built one itself: `Acc::new`, then up to 3 calls to \
             `get`, in random order, before the checked call. `Acc` can also be changed by \
             `Acc::note` (its `s: str` argument uses a type Ply cannot build a value for), which \
             this run never called, and was only ever built by calling `Acc::new`, never by \
             calling `Acc::preloaded`. If `get`'s promise depends on what this run never reached, \
             this run says nothing about it. (W0520, §5.4c)"
        );
        assert!(
            !diag.title.contains("nothing here was assumed"),
            "the superseded completeness claim must never survive alongside a real exclusion: {}",
            diag.title
        );
    }

    /// A `bounded` refusal on a receiver method must blame the receiver,
    /// never a param or return type that is perfectly fine (adversarial
    /// review, 2026-08-27, "a proof refused on a method blames the u32
    /// return type instead of the receiver").
    #[test]
    fn bounded_refused_on_a_receiver_method_names_the_receiver_not_the_return_type() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/lib.rs");
        std::fs::write(
            &path,
            "pub struct Gauge { n: u32 }\nimpl Gauge {\npub fn new(n: u32) -> Self { Gauge { n } }\n\
             #[ply::ensures(|result| *result == *result)]\npub fn level(&self) -> u32 { self.n }\n}\n",
        )
        .unwrap();
        let cf =
            ply_core::harness::discover_method_with_receiver(dir.path(), "Gauge::level").unwrap();
        assert!(cf.receiver.is_some());
        let diag = bounded_refused_sample_only_diag(
            "receiverboundedrefuse::Gauge::level",
            "Gauge::level",
            &cf,
            "bounded(2)",
        );
        assert_eq!(diag.code, "V0508");
        assert!(
            !diag.title.contains("its return type `u32`"),
            "must never blame a return type that is fine: {}",
            diag.title
        );
        assert!(
            diag.title.contains("needs a value to call it on"),
            "must name the real reason: {}",
            diag.title
        );
        assert!(
            diag.title.contains("fuzz"),
            "must point at the check that actually works here: {}",
            diag.title
        );
    }

    // -- a sampled check on a zero-input fn overstates its evidence
    // (adversarial review, 2026-08-27): one case run `n` times is not `n`
    // cases.

    #[test]
    fn zero_input_sampled_diag_says_one_call_not_many() {
        let diag = zero_input_sampled_diag("floats::zero", "zero", "fuzz(64)");
        assert_eq!(diag.code, "W0519");
        assert_eq!(
            diag.severity, "info",
            "nothing here is wrong or owed -- only worth naming"
        );
        assert!(
            diag.title.contains("one possible call"),
            "must say plainly that there was exactly one call to make: {}",
            diag.title
        );
        assert!(
            !diag.title.contains("fuzzed(64)"),
            "must not repeat the overstated verdict as if it were still true: {}",
            diag.title
        );
    }

    // -- the "a `Self` answer is always fine" rule's own blind spot on the
    // sampling tier (adversarial review, 2026-08-27): a promise reading a
    // private field of a returned `Self` cannot compile in the fuzz/test
    // harness crate, and must be refused by name rather than left to fail.

    #[test]
    fn a_self_returning_ctor_reading_a_private_field_is_refused_on_the_sampling_tier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(
            &path,
            "pub struct Bucket { capacity: u32 }\nimpl Bucket {\n#[ply::ensures(|result| \
             result.capacity == cap)]\npub fn new(cap: u32) -> Self { Bucket { capacity: cap } \
             }\n}\n",
        )
        .unwrap();
        let cf = ply_core::harness::discover_fn(&path, "Bucket::new").unwrap();
        let field = self_return_reads_private_field_on_sampling_tier(&cf, &path);
        assert_eq!(
            field.as_deref(),
            Some("capacity"),
            "must name the exact private field the promise reads"
        );
        let diag =
            self_return_private_field_diag("bucket::Bucket::new", "Bucket::new", &field.unwrap());
        assert_eq!(diag.code, "V0510");
        assert_eq!(diag.severity, "warning");
        assert!(diag.title.contains("capacity"), "{}", diag.title);
        assert!(
            diag.title.contains("bounded"),
            "must say bounded/proved do not have this problem: {}",
            diag.title
        );
    }

    #[test]
    fn a_self_returning_ctor_reading_only_public_fields_is_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(
            &path,
            "pub struct Bucket { pub capacity: u32 }\nimpl Bucket {\n#[ply::ensures(|result| \
             result.capacity == cap)]\npub fn new(cap: u32) -> Self { Bucket { capacity: cap } \
             }\n}\n",
        )
        .unwrap();
        let cf = ply_core::harness::discover_fn(&path, "Bucket::new").unwrap();
        assert_eq!(
            self_return_reads_private_field_on_sampling_tier(&cf, &path),
            None,
            "a promise reading only public fields compiles fine in a separate harness crate"
        );
    }

    #[test]
    fn ensures_mentions_field_does_not_false_positive_on_a_longer_field_name() {
        assert!(ensures_mentions_field("|result| result.n == 0", "n"));
        assert!(!ensures_mentions_field("|result| result.note == 0", "n"));
    }

    // -- a run that checked nothing reads as success, ninth-instance
    // variant found while building this task's own fixture (adversarial
    // review, 2026-08-27): the harness's own module/test names are always
    // built from `cf.ident()`, never from a claim's own spelling, which
    // still carries `::` for any method or module-nested free fn.

    #[test]
    fn harness_names_use_the_sanitized_identifier_never_the_claims_double_colons() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(
            &path,
            "pub struct Root;\nimpl Root {\n#[ply::ensures(|result| *result == 999)]\npub fn \
             five() -> u32 { 5 }\n}\n",
        )
        .unwrap();
        let cf = ply_core::harness::discover_fn(&path, "Root::five").unwrap();
        assert_eq!(
            harness_module_name(&cf),
            "Root_five_harness",
            "must be built from the sanitized identifier, matching the module `fuzz_gen` \
             actually generates"
        );
        assert_eq!(harness_test_filter(&cf), "Root_five_harness::");
        assert_eq!(
            harness_fuzz_test_name(&cf),
            "Root_five_harness::ply_fuzz_Root_five"
        );
        // The defect this pins directly: a filter or test name built from
        // the claim's own spelling (`Root::five`) contains `::`, which
        // cannot appear in a real Rust path segment the way the generated
        // module's does -- `cargo test`'s filter is a plain substring
        // match, so that variant matched none of the harness crate's real
        // tests at all.
        assert!(!harness_test_filter(&cf).contains("Root::five"));
    }
}
