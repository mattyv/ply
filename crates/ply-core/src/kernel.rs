//! Ply's pure verdict kernel: evidence levels,
//! statuses, and the worst-of aggregation rule (The-Ply-Spec.md §7, D5, D6). No I/O,
//! no external crates, no code that anchors to real source -- that belongs
//! to `ply-model`/`ply-core` later. `ply-model`'s own doc comment already
//! calls this split out explicitly ("verdicts, statuses, and fingerprints
//! need anchored code and are out of scope for this crate"); this crate is
//! that carve-out, kept dependency-free so its four standing invariants
//! (below) can be checked exhaustively and proved.
//!
//! ## Scope
//!
//! The-Ply-Spec.md §7 describes a full tree node as
//! `{ id, kind, anchor, content_hash, verdict, statuses, worst_descendant,
//! open_items }`. This kernel models only the part that is pure computation
//! over already-known verdicts: a node's own claim ([`NodeKind`]), its own
//! [`StatusKind`] set and conditional assumptions, plus [`aggregate`], which
//! computes `worst_descendant`/`statuses`/`open_items` from a tree of such
//! nodes. `id`/`anchor`/`content_hash` are identity and provenance concerns
//! that need a real codebase to anchor to; they are out of scope here and
//! belong to `ply-model`/`ply-core`.
//!
//! The-Ply-Spec.md §7's paragraph "Aggregation rules the verdict kernel
//! (`tools/kernel`) checks exhaustively" is now the normative source for the
//! four rules below; earlier revisions of this file carried conservative
//! readings for two of them (own-evidence-for-containers, and
//! conditional/assumptions pairing) that the amendment has since settled --
//! see the doc comments on [`NodeKind`] and [`VerdictNode::conditional`].
//!
//! ## Where the four standing obligations are proved
//!
//! CLAUDE.md's four standing obligations -- aggregation never reports
//! evidence stronger than the weakest child; `conditional` never disappears
//! without its assumptions being discharged; a `violation` anywhere always
//! reaches the root; no rule sequence assigns one node two different
//! verdicts -- have two pieces of evidence behind them, and **neither is a
//! Kani proof** (see the historical note below for why there are no Kani
//! harnesses in this file):
//!
//! - **`tests/enumeration.rs`** checks all four against an independent
//!   oracle at *every node* of *every* tree in a bounded corpus -- 991,389
//!   trees, ~2.3s under `cargo test --release`. Exhaustive within its stated
//!   bound over tree shape and node kind; *representative*, not exhaustive,
//!   over status and assumption payloads. `docs/kernel-honesty-cleanups.md`
//!   is where that payload reduction is argued rather than asserted, and
//!   where the one combination it still does not reach is named.
//! - **`tests/spike/verus/`** proves all four **unbounded, by structural
//!   induction, in ~2 seconds** -- every tree of every size, not just the
//!   ones small enough to enumerate. Read that claim exactly as stated: Verus
//!   proved a faithful *shadow* of these rules (children as `Seq<Node>`,
//!   assumptions as an abstract `Set<int>`), never this file's literal
//!   source. What binds the shadow back to this crate is a differential test
//!   that runs the real `ply_kernel::aggregate` and a plain-Rust
//!   transcription of the shadow over thousands of generated trees and
//!   compares them at every node. So the chain is "proved for the shadow,
//!   and the shadow matches production across a large generated corpus" --
//!   strictly weaker than "this source is proved", and
//!   `tests/spike/verus/FINDINGS.md` says so at length. Do not shorten it to
//!   "the kernel is proved."
//!
//! ### Historical note: the three Kani harnesses this file used to carry
//!
//! Until 2026-08-25 this file ended in a `#[cfg(kani)] mod kani_proofs` with
//! three `#[kani::proof]` harnesses for the same four obligations, driven by
//! a hand-rolled depth-2, <=2-children symbolic tree shim. **None of them
//! ever returned a verdict.** They are deleted rather than left gated,
//! because keeping them would have this tool contradicting its own published
//! advice: The-Ply-Spec.md §5.4b now names **recursive or self-referential
//! types (`Vec<Self>`, `Box<Self>` -- any tree or linked structure)** an
//! explicit v1 exclusion, and Ply's rule for an excluded shape is to *refuse*
//! it with a named status, never to route it to an engine that will burn
//! minutes and time out. `VerdictNode` is exactly that shape. The
//! investigation is worth keeping even though the code is not:
//!
//! 1. **First stall: `BTreeSet<StatusKind>`.** The original three harnesses
//!    ran past an hour with *no* verdict at all. Bisecting to single
//!    operations found the cause: a single `Claimable` leaf with an
//!    always-empty `statuses` and no recursion verified in ~1.1s (2942
//!    checks, 0 failures), but the same leaf built through a shim that
//!    *might* insert one concrete status -- gated by one symbolic `bool` --
//!    did not complete in over 2 minutes, and a root `Container` with exactly
//!    one `Claimable` child did not complete in over 5. CBMC's own trace said
//!    why: it unwound `<BTreeMap<K,V,A> as Clone>::clone::clone_subtree` on
//!    every `aggregate_raw` call, because nothing tells it the set only ever
//!    holds 0 or 1 element -- it unwinds the *generic* B-tree algorithm
//!    regardless of actual content. `--unwind 2`/`--unwind 3`, `--object-bits
//!    8` vs. the default 16, and the `kissat` solver in place of `cadical`
//!    were all tried and made no material difference: the stall is in CBMC's
//!    symbolic execution and slicing, not in SAT solving.
//! 2. **The fix that worked, and still stands.** [`StatusSet`] -- a `u8`
//!    bitmask -- replaced `BTreeSet<StatusKind>` on both node types. `Copy`,
//!    no heap, no generic collection algorithm to unwind. That is a
//!    behaviour-preserving representation change, not a semantic one, and it
//!    remains the right representation for its own sake: the enumeration got
//!    *faster* (~8.8s -> ~5.2s at the time) purely from dropping the B-tree
//!    allocations. It also changed the Kani outcome materially -- all three
//!    harnesses began *terminating* with a definitive verdict instead of
//!    hanging.
//! 3. **Second stall: the bottleneck moved one field over.** The verdict
//!    they terminated with was `VERIFICATION:- FAILED (CBMC timed out)`, all
//!    three, at exactly the 300s `--harness-timeout` (Kani 0.67.0 / CBMC
//!    6.8.0, `--unwind 3`, ~5:03 wall each). The new trace stalled inside
//!    `core::slice::sort::shared::pivot::median3_rec::<String, ...>` and
//!    repeated `memcmp` unwinding on `String` -- i.e. `aggregate_raw`'s
//!    `c.sort(); c.dedup();` running the *generic* `Vec<String>` sort on
//!    every conditional node, regardless of how little the shim actually put
//!    there. Same shape of problem as (1), one field over, on
//!    [`VerdictNode::conditional`].
//! 4. **Why the (2) fix was not repeated for `conditional`.** Unlike
//!    `StatusKind`'s fixed variants, an assumption is free-form text a reader
//!    has to be able to read back (D5). Encoding it as, say, a count would
//!    change what the production type *means* -- and therefore what a passing
//!    proof would mean -- to make a verifier happy. That is the "design smell
//!    to raise, not route around" case, and reshaping the kernel to suit the
//!    verifier was refused on evidence as well as principle.
//! 5. **Why it was never going to work, established afterwards.**
//!    `tests/spike/scale/SCALE-FINDINGS.md` measured the general case rather
//!    than this instance: a **3-node** tree produced 64,147 verification
//!    conditions after ~104s of symbolic execution alone and did not finish
//!    in 180s, with the unwind bound demonstrably in effect. Recursion is
//!    outside Kani's measured reach in principle, not by tuning -- so the
//!    stall would simply have moved to the next unbounded field. That finding
//!    is what §5.4b's exclusion was written from.
//! 6. **Where the obligations went instead.** `tests/spike/verus/` -- see
//!    above. Deductive induction over an unbounded tree, ~2s, all four,
//!    against a shadow bound to this crate by a differential test.
//!
//! The deleted harness bodies are in this file's git history if the exact
//! shim is ever wanted again.

/// The-Ply-Spec.md D6 / §7: "The evidence order compares the six kinds" --
/// `violation < unclaimed < tested < fuzzed < bounded < proved`. Declaration
/// order below *is* that order, so `#[derive(PartialOrd, Ord)]` gives the
/// comparison for free -- "worst" is simply the smaller value.
///
/// Conservative reading (spec silent): §7 also says the `n`/`k` parameters of
/// `fuzzed(n)`/`bounded(k)` are "reported in the verdict, never compared" --
/// two claims of the same kind are the same rung regardless of parameter.
/// The kernel therefore compares kinds only and carries no n/k payload; a
/// parameter-aware tie-break, if ever wanted, is a model-layer concern built
/// on top of this order, not a change to it.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Evidence {
    Violation,
    Unclaimed,
    Tested,
    Fuzzed,
    Bounded,
    Proved,
}

/// The-Ply-Spec.md §7: "Only claimable items contribute their own evidence. fns fold
/// their own verdict into `worst_descendant`; containers (workspace,
/// components) fold over children only."
///
/// This is a node-kind distinction rather than a sentinel `Evidence` value
/// so a container is unable to carry its own evidence *by construction*: a
/// `Container` value simply has no `Evidence` payload to set, so there is no
/// representable state for "a component claims its own verdict" to be
/// mistakenly read as one -- the compiler rules it out, not a convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A claimable item (a fn claim, in the real model) with its own
    /// evidence.
    Claimable(Evidence),
    /// workspace or component: no claim of its own, ever.
    Container,
}

/// The-Ply-Spec.md §0 / §7: "Statuses ... do not sit in that [evidence] order; they
/// propagate upward as flags and open-item counts alongside it." This is the
/// status vocabulary named in §0's glossary row for `status`, minus
/// `conditional` -- see [`VerdictNode::conditional`] for why that one moved
/// out of this set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusKind {
    Stale,
    WeakSpec,
    Unsupported,
    EngineMissing,
    Timeout,
    Inconclusive,
    /// The debt half of `conditional` (§5.5, D6): the verdict rests on an
    /// assumed contract and nothing has yet checked that contract against
    /// the real body. Added 2026-08-25, when the adversarial review of the
    /// post-004 fixes found `verify` emitting this status while D6 and §0
    /// still defined neither it nor a home for it -- the vocabulary here
    /// mirrors §0's glossary row, so it moves when that row moves.
    OwedEvidence,
}

/// Declaration-order table of every [`StatusKind`] variant, used by
/// [`StatusSet::iter`] to walk set bits back into values and by tests that
/// want them all without hand-maintaining a second list.
const ALL_STATUS_KINDS: [StatusKind; 7] = [
    StatusKind::Stale,
    StatusKind::WeakSpec,
    StatusKind::Unsupported,
    StatusKind::EngineMissing,
    StatusKind::Timeout,
    StatusKind::Inconclusive,
    StatusKind::OwedEvidence,
];

/// A set of [`StatusKind`] flags, stored as a `u8` bitmask (one bit per
/// variant; 7 variants fit in 7 of the 8 bits) instead of
/// `std::collections::BTreeSet<StatusKind>`.
///
/// This started as the fix for a Kani/CBMC symbolic-execution stall (see the
/// crate doc comment's historical note, item 1): `aggregate_raw` clones and
/// extends a node's `statuses` on every recursive call, and CBMC does not
/// know a `BTreeSet` only ever holds 0 or 1 element here -- it symbolically
/// unwinds the *generic* B-tree `clone_subtree`/insert algorithm regardless,
/// which is what stalled every harness. The harnesses are gone; the bitmask
/// stays on its own merits. It has no such algorithm to unwind:
/// `insert`/`contains`/`union` are single machine-word bitwise ops, `Clone`
/// is `Copy` (no allocation, no loop), and nothing generic for anyone to walk
/// -- which is also why it dropped the enumeration's runtime by a third when
/// it landed.
///
/// Those same bitwise ops are what make the enumeration's status reduction
/// defensible: the fold over a subtree is a per-bit `OR`, so no bit's fate
/// depends on any other bit's. `docs/kernel-honesty-cleanups.md` makes that
/// argument properly.
///
/// Semantically this is still exactly a set of `StatusKind`: no duplicates
/// (a bit is either set or not), unioned via bitwise OR (order-independent,
/// same as `BTreeSet::extend`), with a canonical (declaration-order)
/// iteration order that -- unlike a hash-based set -- can never vary between
/// two equal instances.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusSet(u8);

impl StatusSet {
    /// The empty set.
    pub const fn new() -> Self {
        StatusSet(0)
    }

    /// Adds `status` to the set. A no-op if it is already present.
    pub fn insert(&mut self, status: StatusKind) {
        self.0 |= 1 << (status as u8);
    }

    /// Whether `status` is a member of this set.
    pub fn contains(&self, status: StatusKind) -> bool {
        self.0 & (1 << (status as u8)) != 0
    }

    /// Whether this set has no members.
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// The number of distinct `StatusKind`s in this set -- The-Ply-Spec.md §7's
    /// `open_items` counts "flag instances", i.e. exactly this.
    pub fn len(&self) -> usize {
        self.0.count_ones() as usize
    }

    /// The union of two sets (every status in either).
    pub fn union(&self, other: &StatusSet) -> StatusSet {
        StatusSet(self.0 | other.0)
    }

    /// Iterates the set's members in declaration order (see
    /// `ALL_STATUS_KINDS`) -- the display order tooltips/diagnostics need.
    pub fn iter(&self) -> impl Iterator<Item = StatusKind> + '_ {
        ALL_STATUS_KINDS
            .iter()
            .copied()
            .filter(move |&k| self.contains(k))
    }
}

impl Extend<StatusKind> for StatusSet {
    fn extend<I: IntoIterator<Item = StatusKind>>(&mut self, iter: I) {
        for status in iter {
            self.insert(status);
        }
    }
}

impl FromIterator<StatusKind> for StatusSet {
    fn from_iter<I: IntoIterator<Item = StatusKind>>(iter: I) -> Self {
        let mut set = StatusSet::new();
        set.extend(iter);
        set
    }
}

/// Prints like a set literal, e.g. `{Stale, Timeout}`, matching how
/// `BTreeSet<StatusKind>`'s derived `Debug` used to render this field --
/// this crate's tests/doc comments format status sets this way.
impl std::fmt::Debug for StatusSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// One node of a verdict tree (The-Ply-Spec.md §7), reduced to exactly what
/// [`aggregate`] needs: this node's own claim ([`NodeKind`]), its own status
/// flags, its own conditional assumptions (if any), and its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictNode {
    pub kind: NodeKind,
    pub statuses: StatusSet,
    /// The-Ply-Spec.md §7 / D5: "`conditional` structurally carries its assumptions:
    /// a conditional status without an assumptions list is unrepresentable
    /// in the kernel, not validated against." `None` = not conditional;
    /// `Some(assumptions)` = conditional, with exactly the assumptions it
    /// rests on riding along in the same value -- there is no way to
    /// construct "conditional" without also supplying the list, because
    /// they are the same field rather than a flag plus an independent one
    /// that could disagree with it.
    pub conditional: Option<Vec<String>>,
    pub children: Vec<VerdictNode>,
}

/// The result of [`aggregate`] at one tree position: this node's subtree
/// (itself plus every descendant) folded into `worst_descendant`, unioned
/// `statuses`, unioned conditional assumptions, and a total `open_items`
/// count. Mirrors the input tree's shape via `children`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedNode {
    /// The-Ply-Spec.md §7's `worst_descendant`: the worst-of (D6) over every
    /// *claimable* node's own evidence in the subtree. `Evidence::Unclaimed`
    /// exactly when the subtree contains no claimable node at all (§7: "A
    /// container with no claimable descendants reads `unclaimed`").
    pub evidence: Evidence,
    /// Union of this node's own `statuses` and every descendant's own
    /// `statuses` -- D6/§7: "statuses ... propagate upward as flags."
    pub statuses: StatusSet,
    /// Sorted, deduplicated union of the assumptions carried by every node
    /// in the subtree (self included) whose own [`VerdictNode::conditional`]
    /// is `Some(_)`. `None` exactly when no node in the subtree is
    /// conditional -- this is standing obligation 2: a conditional status
    /// never reaches an ancestor without the assumptions that justify it.
    pub conditional: Option<Vec<String>>,
    /// The-Ply-Spec.md §7's `open_items`, folded as a count: "`open_items` counts
    /// flag instances, not flagged nodes: a node carrying two statuses
    /// contributes 2." This kernel counts every `StatusKind` flag plus a
    /// `conditional` (when present) as one flag instance each, summed over
    /// every node in the subtree. Unresolved markers (`ply.yaml` registry
    /// entries, §5.6) have no representation in this kernel's node type --
    /// they anchor to code the kernel never sees -- so they are not part of
    /// this count; a model layer that also tracks them would add its own
    /// count to this one.
    pub open_items: usize,
    pub children: Vec<AggregatedNode>,
}

fn merge_conditional(a: Option<Vec<String>>, b: Option<Vec<String>>) -> Option<Vec<String>> {
    match (a, b) {
        (None, None) => None,
        (Some(v), None) | (None, Some(v)) => Some(v),
        (Some(mut v), Some(w)) => {
            v.extend(w);
            Some(v)
        }
    }
}

/// Combines two "min over claimable-only evidence so far" accumulators,
/// where `None` means "no claimable node found yet" rather than any real
/// evidence value. Not-yet-found must be a true no-op here, never folded in
/// through `Evidence::min` as if `Unclaimed` were a placeholder identity:
/// `Unclaimed` is a real, comparable rung (weaker than `Tested` but
/// stronger than `Violation`), so treating "nothing here" as literal
/// `Unclaimed` and folding it via `.min()` would let an empty container
/// wrongly drag down a genuinely stronger claimable sibling or child. (An
/// earlier version of `aggregate` did exactly this -- see the note on
/// `aggregate_raw` below.)
fn combine_claimable(a: Option<Evidence>, b: Option<Evidence>) -> Option<Evidence> {
    match (a, b) {
        (None, x) => x,
        (x, None) => x,
        (Some(a), Some(b)) => Some(a.min(b)),
    }
}

/// The recursive core of [`aggregate`]. Returns the subtree's "min over
/// claimable-only evidence" as an `Option<Evidence>` (`None` = no claimable
/// node anywhere in the subtree) *alongside* the public [`AggregatedNode`]
/// for this position, whose own `evidence` field is that option already
/// defaulted to `Evidence::Unclaimed` for display (The-Ply-Spec.md §7: "A container
/// with no claimable descendants reads `unclaimed`").
///
/// The two-value return exists because those are genuinely different
/// things: a parent folding its children must be able to tell "this child
/// subtree has no claimable node at all" (skip it -- it contributes
/// nothing) apart from "this child subtree's real answer happens to be the
/// value `Unclaimed`" (a genuine claimable leaf that earned no checks --
/// fold it in like any other evidence value). Both display as `Unclaimed`
/// on that child's own `AggregatedNode`, but only the raw `Option` lets an
/// ancestor two levels up combine them correctly. Collapsing to the
/// defaulted `Evidence` one level too early -- i.e. having `aggregate`
/// call itself recursively and fold children's already-defaulted
/// `agg.evidence` via a plain `.min()` -- is precisely the bug
/// `tests/enumeration.rs` caught: a bare container with one
/// `Claimable(Tested)` child aggregated to `Unclaimed` instead of `Tested`,
/// because the container's own placeholder `Unclaimed` won the `.min()`
/// against the real, stronger child.
fn aggregate_raw(node: &VerdictNode) -> (Option<Evidence>, AggregatedNode) {
    let mut raw_evidence = match node.kind {
        NodeKind::Claimable(e) => Some(e),
        NodeKind::Container => None,
    };
    let mut statuses: StatusSet = node.statuses;
    let mut conditional = node.conditional.clone();
    let mut open_items = node.statuses.len() + if node.conditional.is_some() { 1 } else { 0 };
    let mut children = Vec::with_capacity(node.children.len());

    for child in &node.children {
        let (child_raw, child_agg) = aggregate_raw(child);
        raw_evidence = combine_claimable(raw_evidence, child_raw);
        statuses = statuses.union(&child_agg.statuses);
        conditional = merge_conditional(conditional, child_agg.conditional.clone());
        open_items += child_agg.open_items;
        children.push(child_agg);
    }

    if let Some(c) = &mut conditional {
        c.sort();
        c.dedup();
    }

    let evidence = raw_evidence.unwrap_or(Evidence::Unclaimed);
    (
        raw_evidence,
        AggregatedNode {
            evidence,
            statuses,
            conditional,
            open_items,
            children,
        },
    )
}

/// Fold a verdict tree into its per-node aggregated results (The-Ply-Spec.md §7,
/// D6). Pure: no I/O, no randomness, no shared mutable state -- calling it
/// twice on equal inputs always yields equal outputs (standing obligation
/// 4). Every collection used for aggregated state (`StatusSet`'s bitmask,
/// plus a sorted+deduplicated `Vec` for conditional assumptions) is
/// order-independent by construction, specifically to avoid the classic
/// footgun where a hash-based set's iteration order can differ between two
/// otherwise-equal instances (Rust's `HashSet`/`HashMap` seed their hasher
/// per instance) -- that would make two `AggregatedNode` values compare
/// unequal, or print differently, for no reason tied to the data itself.
pub fn aggregate(node: &VerdictNode) -> AggregatedNode {
    aggregate_raw(node).1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `is_empty` has exactly one consumer that changes behaviour on the
    /// answer -- the renderer, which picks what to draw from it -- and that
    /// lives in another crate, so nothing here used to notice if the answer
    /// were wrong. A mutation run found that: "always report empty" survived
    /// the whole suite. Both directions are pinned so it cannot come back.
    #[test]
    fn an_empty_status_set_reports_empty_and_a_populated_one_does_not() {
        let empty = StatusSet::new();
        assert!(
            empty.is_empty(),
            "a set with nothing in it must report empty"
        );

        let mut populated = StatusSet::new();
        populated.insert(ALL_STATUS_KINDS[0]);
        assert!(
            !populated.is_empty(),
            "a set holding {:?} must not report empty -- the renderer decides \
             what to draw from this answer",
            ALL_STATUS_KINDS[0]
        );
    }

    /// Collecting statuses into a set is public API that nothing in the
    /// product calls, so a mutation replacing it with "always produce the
    /// empty set" survived. Pinned rather than deleted: it is the idiomatic
    /// partner to `Extend`, and an untested constructor is how a later
    /// caller inherits a silent bug.
    #[test]
    fn collecting_statuses_into_a_set_keeps_every_distinct_one() {
        let collected: StatusSet = ALL_STATUS_KINDS.iter().copied().collect();
        assert_eq!(
            collected.len(),
            ALL_STATUS_KINDS.len(),
            "collecting every status kind must yield a set holding every one of them"
        );
        for kind in ALL_STATUS_KINDS {
            assert!(
                collected.contains(kind),
                "{kind:?} was lost while collecting"
            );
        }
    }

    /// This set's printed form is what the exhaustive check prints when it
    /// finds a counterexample. CLAUDE.md's rule is that a failure must name
    /// the actual defect -- so a mutation blanking this output degrades every
    /// future failure message into something unreadable, and survived because
    /// no test read it. Exact-string, because the words are the point.
    #[test]
    fn a_status_set_prints_its_members_so_a_failure_message_names_them() {
        assert_eq!(format!("{:?}", StatusSet::new()), "{}");

        let mut one = StatusSet::new();
        one.insert(ALL_STATUS_KINDS[0]);
        assert_eq!(format!("{one:?}"), format!("{{{:?}}}", ALL_STATUS_KINDS[0]));

        let all: StatusSet = ALL_STATUS_KINDS.iter().copied().collect();
        let printed = format!("{all:?}");
        for kind in ALL_STATUS_KINDS {
            assert!(
                printed.contains(&format!("{kind:?}")),
                "printed form {printed} omits {kind:?}, so a counterexample \
                 mentioning it would be unreadable"
            );
        }
    }

    fn leaf(kind: NodeKind) -> VerdictNode {
        VerdictNode {
            kind,
            statuses: StatusSet::new(),
            conditional: None,
            children: Vec::new(),
        }
    }

    /// The status vocabulary here mirrors §0's glossary row, and a name the
    /// spec defines must have a representation. `owed-evidence` did not: the
    /// tool emitted it from 2026-08-25, §5.5 called it an open item, and
    /// neither D6's list nor §0's glossary nor this set had it at all
    /// (adversarial review of the post-004 fixes, D6). This walks every
    /// variant through the bitset, so a variant added to the vocabulary
    /// without a bit -- or a bit that collides with another -- fails here
    /// rather than silently dropping a flag out of an aggregation.
    #[test]
    fn every_status_in_the_glossary_round_trips_through_the_bitset() {
        for kind in ALL_STATUS_KINDS {
            let mut set = StatusSet::new();
            set.insert(kind);
            assert!(
                set.contains(kind),
                "{kind:?} did not survive its own insert"
            );
            assert_eq!(set.len(), 1, "{kind:?} set more than one bit");
            assert_eq!(
                set.iter().collect::<Vec<_>>(),
                vec![kind],
                "{kind:?} came back as something else"
            );
        }
        let mut all = StatusSet::new();
        all.extend(ALL_STATUS_KINDS);
        assert_eq!(
            all.len(),
            ALL_STATUS_KINDS.len(),
            "two variants share a bit, so one of them is invisible in every aggregation"
        );
        assert!(
            all.contains(StatusKind::OwedEvidence),
            "the debt half of `conditional` (§5.5, D6) is part of the vocabulary"
        );
    }

    #[test]
    fn evidence_order_matches_d6() {
        assert!(Evidence::Violation < Evidence::Unclaimed);
        assert!(Evidence::Unclaimed < Evidence::Tested);
        assert!(Evidence::Tested < Evidence::Fuzzed);
        assert!(Evidence::Fuzzed < Evidence::Bounded);
        assert!(Evidence::Bounded < Evidence::Proved);
    }

    #[test]
    fn a_single_claimable_leaf_aggregates_to_its_own_evidence() {
        let agg = aggregate(&leaf(NodeKind::Claimable(Evidence::Proved)));
        assert_eq!(agg.evidence, Evidence::Proved);
        assert!(agg.statuses.is_empty());
        assert!(agg.conditional.is_none());
        assert_eq!(agg.open_items, 0);
    }

    #[test]
    fn an_empty_container_aggregates_to_unclaimed() {
        let agg = aggregate(&leaf(NodeKind::Container));
        assert_eq!(agg.evidence, Evidence::Unclaimed);
    }

    /// Regression test for the exact bug `tests/enumeration.rs` caught
    /// during the The-Ply-Spec.md §7 rework: a container must not seed its own fold
    /// with a placeholder `Unclaimed` and then `.min()` it against a real,
    /// stronger claimable child -- that wrongly produces `Unclaimed` here
    /// instead of `Tested`.
    #[test]
    fn a_container_with_one_claimable_child_reports_the_childs_evidence() {
        let tree = VerdictNode {
            kind: NodeKind::Container,
            statuses: StatusSet::new(),
            conditional: None,
            children: vec![leaf(NodeKind::Claimable(Evidence::Tested))],
        };
        let agg = aggregate(&tree);
        assert_eq!(agg.evidence, Evidence::Tested);
    }

    #[test]
    fn a_violated_child_drags_a_proved_root_down_to_violation() {
        let tree = VerdictNode {
            kind: NodeKind::Container,
            statuses: StatusSet::new(),
            conditional: None,
            children: vec![
                leaf(NodeKind::Claimable(Evidence::Violation)),
                leaf(NodeKind::Claimable(Evidence::Tested)),
            ],
        };
        let agg = aggregate(&tree);
        assert_eq!(
            agg.evidence,
            Evidence::Violation,
            "a violation anywhere must reach the root"
        );
    }

    #[test]
    fn conditional_on_a_child_propagates_with_its_assumptions() {
        let mut conditional_child = leaf(NodeKind::Claimable(Evidence::Bounded));
        conditional_child.conditional = Some(vec!["parser::parse fuzzed(256)".to_string()]);

        let tree = VerdictNode {
            kind: NodeKind::Container,
            statuses: StatusSet::new(),
            conditional: None,
            children: vec![conditional_child],
        };
        let agg = aggregate(&tree);
        assert_eq!(
            agg.conditional,
            Some(vec!["parser::parse fuzzed(256)".to_string()])
        );
    }
}
