//! `ply-schedule`: the `stub_verified` gate for the future `cargo ply verify`
//! (The-Ply-Spec.md D5, §5.5). No I/O, no external crates beyond `ply-kernel` (for
//! [`Evidence`], reused rather than duplicated).
//!
//! ## Why this crate exists and is soundness-critical
//!
//! The M0 spike (docs/adr/0003-m0-feasibility.md, tests/spike/FINDINGS.md finding 1)
//! proved Kani gives **no backstop** for D5's core rule. Kani's `stub_verified(g)`
//! checks only that a `#[proof_for_contract]` harness *exists* for `g` -- never that
//! it ran, never that it passed. A caller was observed reporting `VERIFICATION
//! SUCCESSFUL` while assuming a deliberately falsified callee contract, and
//! whole-crate `cargo kani` does not help either: harnesses run in arbitrary order,
//! and nothing retracts a caller's already-reported verdict when its callee's
//! harness later fails in the same invocation.
//!
//! D5's own text states the consequence plainly: "Ply's scheduler -- callees proved
//! first, the caller credited only if those proofs passed -- is therefore the entire
//! soundness guarantee; an implementation that relaxes it is unsound and nothing
//! downstream will notice." That makes this crate as soundness-critical as the verdict
//! kernel (`tools/kernel`), so it gets the same treatment: a pure module, invariants
//! checked exhaustively (`tests/enumeration.rs`) rather than by spot-check alone.
//!
//! ## Scope
//!
//! This crate answers one question, a pure function of already-known data:
//!
//! - [`may_stub`]: given proof results that have actually arrived this run, may
//!   `caller` assume `callee`'s contract via `stub_verified`? [`StubDecision::Allowed`]
//!   is returned only when `callee` itself passed a Kani contract proof this run, in a
//!   way `caller` can actually rely on (same crate, or a caller-local cross-crate
//!   re-proof per D5's workaround) -- never merely because `callee` was scheduled,
//!   attempted, or exists. Every weaker case names one of D5's own refusal reasons
//!   (§5.5's "anything else" list) via [`StubRefusalReason`], which is what a
//!   `conditional` verdict's assumption-listing (D5, `ply-kernel`'s
//!   `VerdictNode::conditional`) is built from.
//!
//! The other half of D5's scheduler -- *what order* functions must actually be
//! verified in so that every callee is attempted before its caller, and which
//! claims a cycle (or a transitive dependency on one) denies assumed-contract
//! credit to -- is the part that actually ships, and it lives in
//! `ply_core::schedule::order` (`crates/ply-core/src/schedule.rs`), consumed
//! directly by `ply-cli`. It used to be duplicated here as `plan`/`Batch`, which
//! implemented a *different*, more permissive contract (a cycle's dependents were
//! layered into batches after the cycle, as though cleanly orderable) that nothing
//! in the product ever called; it was deleted in favour of the one real
//! implementation rather than kept as a second, untested opinion.
//!
//! Anchoring (`id`/source spans), engine invocation, and Diagnostic assembly are a
//! model-layer/CLI concern (M3, ADR-0003) and out of scope here, exactly as
//! `ply-kernel`'s own doc comment draws that line for the verdict tree.

use std::collections::{BTreeMap, BTreeSet};

pub use ply_kernel::Evidence;

/// A function's identity in the call graph. `ply-model`/the CLI owns mapping this to
/// a real anchored fn; this crate treats it as an opaque, totally-ordered key so
/// scheduling is deterministic (`BTreeMap`/`BTreeSet`, never a hash-based collection
/// whose iteration order could vary between two equal `CallGraph`s -- the same
/// footgun `ply-kernel`'s `StatusSet` doc comment calls out for `HashSet`/`HashMap`).
pub type FnId = u32;

/// The crate a function lives in, for D5's same-crate-vs-cross-crate distinction.
/// Opaque and totally ordered for the same determinism reason as [`FnId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CrateId(pub u32);

/// Whether a Kani contract proof for a function has run this verification session,
/// and if so, its outcome. This is *results data* ("as results arrive"), deliberately
/// kept separate from [`FnInfo`] (which is static per-fn claim data unrelated to any
/// particular run) -- a fn's `declares_contract`/`is_pub`/`crate_id` do not change
/// between verification runs, but its proof outcome does, and [`may_stub`] takes a
/// fresh [`ProofResults`] each call precisely so the same [`CallGraph`] can be
/// re-queried as results arrive over the course of one `cargo ply verify` run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProofStatus {
    /// No Kani contract proof for this fn has completed in this run yet -- it may not
    /// have been attempted, or may still be in flight. The default: a [`ProofResults`]
    /// says `NotRun` for any fn it has no explicit entry for.
    #[default]
    NotRun,
    /// This fn's own `#[kani::proof_for_contract]` harness ran and verified.
    Passed,
    /// This fn's own `#[kani::proof_for_contract]` harness ran and failed.
    Failed,
}

/// Static per-fn claim data: what a fn declares about itself, independent of any
/// particular verification run's results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FnInfo {
    pub crate_id: CrateId,
    /// Whether this fn has a Kani contract (`requires`/`ensures`) declared at all.
    /// D5: only a fn with a contract can ever be a `stub_verified` target -- a fn
    /// with no contract is refused as [`StubRefusalReason::CalleeWeakerEvidence`]
    /// regardless of what its `own_evidence` says, because there is no contract for
    /// a caller to assume.
    pub declares_contract: bool,
    /// D5/§5.5's cross-crate workaround requires the remote item be `pub` for a
    /// caller-local re-proof harness to name it at all (ADR-0003 item 5: "the target
    /// must be `pub`"). Irrelevant when `crate_id` matches the caller's.
    pub is_pub: bool,
    /// The strongest evidence this fn's non-Kani-contract check pipeline (test/fuzz)
    /// has recorded, independent of `declares_contract` and of Kani proof status.
    /// Purely informational here: [`may_stub`]'s gate reads only `declares_contract`
    /// and the run's [`ProofResults`], never this field -- D5 says a caller may
    /// assume a callee's contract only when the callee "passed its own Kani contract
    /// proof this run"; a callee merely `fuzzed`/`tested`, however strong, never
    /// licenses `stub_verified` (that is exactly D5's "callee merely fuzzed or
    /// tested" refusal case, [`StubRefusalReason::CalleeWeakerEvidence`]). Carried on
    /// `FnInfo` so a real scheduler has one place to keep it, not a second table.
    pub own_evidence: Evidence,
}

/// A pure, callee-before-caller call graph: nodes are [`FnId`]s carrying [`FnInfo`],
/// edges are directed `caller -> callee` calls. May contain cycles (D5 names this
/// case explicitly; it must degrade to `conditional`, never deadlock the planner).
///
/// Both endpoints of an edge must already be registered via [`CallGraph::add_fn`] --
/// enforced by panicking, so a caller of this crate cannot silently query
/// [`may_stub`]/[`plan`] about a fn this graph never learned anything about.
#[derive(Debug, Clone, Default)]
pub struct CallGraph {
    fns: BTreeMap<FnId, FnInfo>,
    edges: BTreeSet<(FnId, FnId)>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a fn's static claim data. Calling this again for an already-known
    /// `id` replaces its `FnInfo`.
    pub fn add_fn(&mut self, id: FnId, info: FnInfo) {
        self.fns.insert(id, info);
    }

    /// Records a `caller -> callee` call edge. Panics if either endpoint was never
    /// registered via [`CallGraph::add_fn`] -- see the struct doc comment.
    pub fn add_edge(&mut self, caller: FnId, callee: FnId) {
        assert!(
            self.fns.contains_key(&caller),
            "add_edge: caller {caller} was never registered via add_fn"
        );
        assert!(
            self.fns.contains_key(&callee),
            "add_edge: callee {callee} was never registered via add_fn"
        );
        self.edges.insert((caller, callee));
    }

    /// Every registered fn id, in ascending order (a `BTreeMap`'s natural iteration
    /// order -- deterministic by construction, not by convention).
    pub fn fn_ids(&self) -> impl Iterator<Item = FnId> + '_ {
        self.fns.keys().copied()
    }

    pub fn info(&self, id: FnId) -> Option<&FnInfo> {
        self.fns.get(&id)
    }

    /// Every fn `caller` directly calls, in ascending order.
    pub fn callees(&self, caller: FnId) -> impl Iterator<Item = FnId> + '_ {
        self.edges
            .range((caller, FnId::MIN)..=(caller, FnId::MAX))
            .map(|&(_, callee)| callee)
    }
}

/// D5's refusal taxonomy: every reason [`may_stub`] can refuse a `stub_verified`,
/// each mapping directly to one of §5.5's "anything else" cases -- this *is* the
/// assumptions-listing a `conditional` verdict (`ply-kernel`'s
/// `VerdictNode::conditional`) is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubRefusalReason {
    /// `caller` and `callee` are the same fn, or mutually reachable through the call
    /// graph (D5: "f and g in a cycle"). Stubbing across a cycle is unsound by
    /// construction: it would let each member's proof rest on an assumption the
    /// other member's own (not-yet-trustworthy) proof was meant to discharge.
    CalleeInCycle,
    /// `callee` declares a contract, but no Kani contract proof for it has completed
    /// in this run yet.
    CalleeUnproved,
    /// `callee` declares a contract, and its Kani contract proof ran this run and
    /// failed.
    CalleeFailed,
    /// `callee` declares no Kani contract at all -- D5's "callee merely fuzzed or
    /// tested" case. However strong its test/fuzz evidence, there is no contract for
    /// `caller` to assume.
    CalleeWeakerEvidence,
    /// `callee` is in a different crate than `caller` and is not `pub`, so D5/§5.5's
    /// caller-local cross-crate re-proof workaround (ADR-0003 item 5) has no `pub`
    /// item to name -- there is no re-proof `caller` could even attempt.
    CalleeCrossCrateNotReprovable,
}

/// The outcome of asking whether `caller` may use `#[kani::stub_verified(callee)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubDecision {
    Allowed,
    Refused(StubRefusalReason),
}

/// Computes, for every registered fn, the set of fns reachable from it by following
/// `caller -> callee` edges (reflexive: a fn always reaches itself). One BFS per fn;
/// fine for the graph sizes this crate's callers verify per run.
fn reachable_from(graph: &CallGraph, start: FnId) -> BTreeSet<FnId> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![start];
    while let Some(u) = stack.pop() {
        if seen.insert(u) {
            for v in graph.callees(u) {
                if !seen.contains(&v) {
                    stack.push(v);
                }
            }
        }
    }
    seen
}

fn all_reachability(graph: &CallGraph) -> BTreeMap<FnId, BTreeSet<FnId>> {
    graph
        .fn_ids()
        .map(|id| (id, reachable_from(graph, id)))
        .collect()
}

/// Whether `a` and `b` are mutually reachable (the same strongly-connected
/// component), `a == b` included. This is D5's "in a cycle" test.
fn same_scc(reach: &BTreeMap<FnId, BTreeSet<FnId>>, a: FnId, b: FnId) -> bool {
    a == b || (reach[&a].contains(&b) && reach[&b].contains(&a))
}

/// Decides whether `caller` may verify itself with `#[kani::stub_verified(callee)]`
/// (The-Ply-Spec.md D5, §5.5). [`StubDecision::Allowed`] only when `callee` itself
/// passed a Kani contract proof *this run* (per `results`) in a way `caller` can
/// actually lean on -- never merely because `callee` was scheduled, attempted, or
/// exists (standing obligation 2). Every weaker case names one of D5's own refusal
/// reasons via [`StubRefusalReason`].
///
/// Checked in the order D5's own text lists them: a cycle (including `caller ==
/// callee`, a trivial self-cycle) always refuses first, regardless of `callee`'s
/// proof state -- assuming a cycle member's contract is unsound no matter how
/// "proved" it looks, because that very proof may itself be resting on the
/// assumption this call would grant. Then crate/`pub` reprovability, then the
/// contract/proof-status ladder itself.
///
/// # Panics
///
/// If `caller` or `callee` was never registered in `graph` via
/// [`CallGraph::add_fn`] -- see [`CallGraph`]'s doc comment.
pub fn may_stub(
    graph: &CallGraph,
    caller: FnId,
    callee: FnId,
    results: &ProofResults,
) -> StubDecision {
    let caller_info = graph
        .info(caller)
        .expect("may_stub: caller was never registered via add_fn");
    let callee_info = graph
        .info(callee)
        .expect("may_stub: callee was never registered via add_fn");

    if caller == callee {
        return StubDecision::Refused(StubRefusalReason::CalleeInCycle);
    }
    let reach = all_reachability(graph);
    if same_scc(&reach, caller, callee) {
        return StubDecision::Refused(StubRefusalReason::CalleeInCycle);
    }

    if caller_info.crate_id != callee_info.crate_id && !callee_info.is_pub {
        return StubDecision::Refused(StubRefusalReason::CalleeCrossCrateNotReprovable);
    }

    if !callee_info.declares_contract {
        return StubDecision::Refused(StubRefusalReason::CalleeWeakerEvidence);
    }

    match results.status_of(callee) {
        ProofStatus::Passed => StubDecision::Allowed,
        ProofStatus::NotRun => StubDecision::Refused(StubRefusalReason::CalleeUnproved),
        ProofStatus::Failed => StubDecision::Refused(StubRefusalReason::CalleeFailed),
    }
}

/// Per-fn Kani contract proof results for one verification run. Absent entries read
/// as [`ProofStatus::NotRun`] (see [`ProofStatus`]'s doc comment).
#[derive(Debug, Clone, Default)]
pub struct ProofResults(BTreeMap<FnId, ProofStatus>);

impl ProofResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, fn_id: FnId, status: ProofStatus) {
        self.0.insert(fn_id, status);
    }

    pub fn status_of(&self, fn_id: FnId) -> ProofStatus {
        self.0.get(&fn_id).copied().unwrap_or_default()
    }
}
