//! The pure verdict kernel of the future `cargo ply`: evidence levels,
//! statuses, and the worst-of aggregation rule (The-Ply-Spec.md §7, D5, D6). No I/O,
//! no external crates, no code that anchors to real source -- that belongs
//! to `ply-model`/`ply-core` later. `ply-model`'s own doc comment already
//! calls this split out explicitly ("verdicts, statuses, and fingerprints
//! need anchored code and are out of scope for this crate"); this crate is
//! that carve-out, kept dependency-free so its four standing invariants
//! (below) can be checked exhaustively and, once Kani is installed, proved.
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
/// `#[cfg_attr(kani, derive(kani::Arbitrary))]` mirrors The-Ply-Spec.md D2's own
/// idiom (attributes that vanish under plain cargo and activate only under
/// `cargo kani`) so the Kani harnesses below can draw real `Evidence` values
/// via `kani::any()` instead of a hand-rolled index-to-variant mapping.
#[cfg_attr(kani, derive(kani::Arbitrary))]
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
#[cfg_attr(kani, derive(kani::Arbitrary))]
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
}

/// Declaration-order table of every [`StatusKind`] variant, used by
/// [`StatusSet::iter`] to walk set bits back into values and by tests that
/// want "all six" without hand-maintaining a second list.
const ALL_STATUS_KINDS: [StatusKind; 6] = [
    StatusKind::Stale,
    StatusKind::WeakSpec,
    StatusKind::Unsupported,
    StatusKind::EngineMissing,
    StatusKind::Timeout,
    StatusKind::Inconclusive,
];

/// A set of [`StatusKind`] flags, stored as a `u8` bitmask (one bit per
/// variant; 6 variants fit in 6 of the 8 bits) instead of
/// `std::collections::BTreeSet<StatusKind>`.
///
/// This exists to remove a Kani/CBMC symbolic-execution stall documented in
/// the `kani_proofs` module doc comment below: `aggregate_raw` clones and
/// extends a node's `statuses` on every recursive call, and CBMC does not
/// know a `BTreeSet` only ever holds 0 or 1 element here -- it symbolically
/// unwinds the *generic* B-tree `clone_subtree`/insert algorithm regardless,
/// which is what stalled every harness. A bitmask has no such algorithm to
/// unwind: `insert`/`contains`/`union` are single machine-word bitwise ops,
/// `Clone` is `Copy` (no allocation, no loop), and there is nothing generic
/// for CBMC to walk.
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
        statuses.extend(child_agg.statuses.iter());
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

    fn leaf(kind: NodeKind) -> VerdictNode {
        VerdictNode {
            kind,
            statuses: StatusSet::new(),
            conditional: None,
            children: Vec::new(),
        }
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

/// Kani proof harnesses for the same four standing obligations, using
/// `kani::any()`-generated symbolic trees. Entirely `#[cfg(kani)]`-gated:
/// under plain `cargo build`/`cargo test` this module does not exist at all
/// (the `kani` crate is not a Cargo dependency -- `cargo kani` supplies it as
/// a compiler-provided pseudo-crate, per The-Ply-Spec.md D9's "engines run as
/// subprocesses ... never linked as libraries"), so there is nothing here
/// for plain cargo to fail to compile.
///
/// ## Status: `statuses` fixed the original stall; `conditional` is now the
/// ## same problem one field over, and none of these three still verifies
///
/// **This is the second investigation of this module**, after the first
/// (below, condensed) found that `VerdictNode::statuses:
/// BTreeSet<StatusKind>` was the cause: CBMC does not know a shim only ever
/// puts 0 or 1 element into a `BTreeSet`, so `node.statuses.clone()` /
/// `.extend()` in `aggregate_raw` made it symbolically unwind the *generic*
/// B-tree `clone_subtree` algorithm on every recursive call, regardless of
/// actual content. Fix: `statuses` (on both `VerdictNode` and
/// `AggregatedNode`) is now [`StatusSet`], a `u8` bitmask -- `Copy`, no
/// heap, no generic collection algorithm to unwind; `insert`/`contains`/
/// `union` are single bitwise ops. This is a behavior-preserving
/// representation change, not a semantic one: `tests/enumeration.rs`'s
/// 991,389-tree oracle comparison (unmodified in what it asserts) stays
/// green, all six unit tests stay green, and the enumeration run got
/// *faster* (~8.8s -> ~5.2s on this machine) purely from dropping the
/// B-tree allocations.
///
/// That fix changed the outcome materially: all three harnesses below now
/// **terminate** with a definitive CBMC verdict instead of hanging with no
/// result at all. Measured just now, Kani 0.67.0 / CBMC 6.8.0, `--unwind 3
/// --harness-timeout 300s` (`-Z unstable-options`), one run per harness:
///
/// | harness | verdict | wall time |
/// |---|---|---|
/// | `proof_worst_of_evidence` | `VERIFICATION:- FAILED` (CBMC timed out) | 5:03 |
/// | `proof_conditional_carries_its_assumptions` | `VERIFICATION:- FAILED` (CBMC timed out) | 5:03 |
/// | `proof_aggregate_is_deterministic` | `VERIFICATION:- FAILED` (CBMC timed out) | 5:03 |
///
/// None of the three *verifies* -- each one now runs to exactly the
/// `--harness-timeout` bound and reports "CBMC timed out", which is a real,
/// reproducible verdict rather than an indefinite hang, but it is still not
/// a proof. **The trace shows the bottleneck moved, not disappeared**: with
/// `statuses` no longer a `BTreeSet`, the CBMC trace for all three now
/// stalls inside `core::slice::sort::shared::pivot::median3_rec::<String,
/// ...>` and repeated `memcmp` unwinding on `std::string::String` -- i.e.
/// `aggregate_raw`'s `if let Some(c) = &mut conditional { c.sort();
/// c.dedup(); }`, which runs the *generic* `Vec<String>`/`String` sort and
/// dedup algorithm on every node with a conditional, unconditional of how
/// much content the shim actually put there. This is the exact same shape
/// of problem `statuses` had -- a generic-collection algorithm CBMC must
/// symbolically unwind regardless of the shim's real content -- just on
/// `VerdictNode::conditional: Option<Vec<String>>` instead of `statuses`.
///
/// This time the fix is not repeated: unlike `StatusKind` (6 fixed
/// variants, naturally a bitmask), `conditional`'s payload is free-form
/// assumption text (`"parser::parse fuzzed(256)"`-shaped strings The-Ply-Spec.md D5
/// requires callers to be able to read back) -- there is no fixed-width
/// encoding that preserves that without becoming exactly the kind of
/// content-lossy placeholder CLAUDE.md's "never weaken a harness to make it
/// pass" (and, more fundamentally, the newbie-bar rule that a status's
/// explanation must be real text, not a count) rules out. Swapping it for
/// e.g. an assumption *count* was the "next-cheapest representation"
/// suggested going in, but was not applied here: it would change what the
/// production type -- and therefore what a passing proof -- actually means,
/// not just what the shim exercises, since `aggregate_raw`'s own
/// `merge_conditional`/`sort`/`dedup` runs against the real field either
/// way. That is exactly the "design smell to raise, not route around" the
/// project's own working notes call for.
///
/// What *is* proved today, exhaustively rather than symbolically, is
/// `tests/enumeration.rs`: all four standing obligations hold over 991,389
/// concretely-enumerated small trees in ~5.2s under plain `cargo test`. The
/// harnesses below are kept -- not deleted, not weakened to something that
/// would pass trivially -- as an accurate, `#[kani::unwind]`-documented
/// statement of what a symbolic proof of the real `aggregate` would need to
/// check; they simply do not terminate (within the bound above) against
/// this kernel's `conditional` type with this Kani/CBMC version today.
/// Reproduce with, e.g.:
/// `cargo kani -Z unstable-options --harness proof_worst_of_evidence --unwind 3 --harness-timeout 300s`.
///
/// What to try next, not attempted here because it needs its own careful
/// review of what it does to the actual verified property: Kani function
/// stubbing (`-Z stubbing`, `#[kani::stub(...)]`) to replace
/// `Vec<String>`/`String`'s sort, dedup, and comparison with a
/// semantically-equivalent bounded implementation *for the duration of the
/// proof only*, leaving the production `conditional: Option<Vec<String>>`
/// field exactly as-is. That is a proof-harness-local change (same spirit
/// as the existing `SymTree`/`SymLeaf` shim), not a production-type change,
/// so it does not carry the content-loss problem the count idea above does
/// -- but it has not been attempted or measured, so no claim is made about
/// whether it would actually terminate.
///
/// ### First investigation (superseded above, kept for the record)
///
/// The original stall (all three harnesses run past an hour with **no**
/// verdict, not even a timeout) was root-caused to `VerdictNode::statuses:
/// BTreeSet<StatusKind>` by bisecting to single operations: a single
/// `Claimable` leaf with an always-empty `statuses` and no recursion
/// verified in ~1.1s (2942 checks, 0 failures); the same leaf built through
/// `SymLeaf::into_node` (which may insert one concrete `StatusKind::Stale`,
/// gated by one symbolic `bool`) did not complete in over 2 minutes; a root
/// `Container` with exactly one `Claimable` child and no statuses/
/// conditional at all did not complete in over 5 minutes. `--unwind 3`'s
/// own trace showed why: CBMC unwound `<BTreeMap<K,V,A> as
/// Clone>::clone::clone_subtree` on every `aggregate_raw` call regardless
/// of actual set size. `--unwind 2`/`--unwind 3`, `--object-bits 8` vs. the
/// default 16, and the `kissat` solver in place of `cadical` were all tried
/// and made no material difference (the stall is in CBMC's own symbolic
/// execution/slicing, not SAT solving). See the module's git history for
/// the full original write-up; the `StatusSet` section above is now the
/// current, accurate status.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// A depth-2, <=2-children symbolic tree shim. Kani's bounded model
    /// checking needs a finite, non-recursive shape to draw `kani::any()`
    /// values for -- the real `VerdictNode` is recursive through
    /// `Vec<VerdictNode>`, which has no bound Kani can symbolically unroll
    /// without one being imposed externally. `build` converts this shim into
    /// a real `VerdictNode`, so every proof below still calls the actual
    /// production `aggregate`, never a re-implementation of it.
    #[derive(kani::Arbitrary, Clone, Copy)]
    struct SymLeaf {
        kind: NodeKind,
        conditional: bool,
        other_status: bool,
    }

    impl SymLeaf {
        fn own_evidence(&self) -> Option<Evidence> {
            match self.kind {
                NodeKind::Claimable(e) => Some(e),
                NodeKind::Container => None,
            }
        }

        fn into_node(self, children: Vec<VerdictNode>) -> VerdictNode {
            let mut statuses = StatusSet::new();
            if self.other_status {
                statuses.insert(StatusKind::Stale);
            }
            let conditional = if self.conditional {
                Some(vec!["kani-symbolic-assumption".to_string()])
            } else {
                None
            };
            VerdictNode {
                kind: self.kind,
                statuses,
                conditional,
                children,
            }
        }
    }

    #[derive(kani::Arbitrary, Clone, Copy)]
    struct SymTree {
        root: SymLeaf,
        has_child_a: bool,
        child_a: SymLeaf,
        has_child_b: bool,
        child_b: SymLeaf,
    }

    impl SymTree {
        fn build(self) -> VerdictNode {
            let mut children = Vec::new();
            if self.has_child_a {
                children.push(self.child_a.into_node(Vec::new()));
            }
            if self.has_child_b {
                children.push(self.child_b.into_node(Vec::new()));
            }
            self.root.into_node(children)
        }

        /// The min over claimable-only own-evidence across root + present
        /// children, `None` if none of them is claimable -- computed
        /// directly from the symbolic fields, not by calling `aggregate`.
        fn expected_claimable_min(&self) -> Option<Evidence> {
            let mut acc = self.root.own_evidence();
            if self.has_child_a {
                acc = match (acc, self.child_a.own_evidence()) {
                    (None, x) => x,
                    (x, None) => x,
                    (Some(a), Some(b)) => Some(a.min(b)),
                };
            }
            if self.has_child_b {
                acc = match (acc, self.child_b.own_evidence()) {
                    (None, x) => x,
                    (x, None) => x,
                    (Some(a), Some(b)) => Some(a.min(b)),
                };
            }
            acc
        }
    }

    /// Standing obligation 1 (as reworded by The-Ply-Spec.md §7's amendment):
    /// worst-of never reports evidence stronger than the weakest claimable
    /// node in the subtree, and reports exactly `Unclaimed` when there is
    /// none. Standing obligation 3 (violation-reaches-root) is the case of
    /// this where the weakest claimable node is a `Violation`.
    ///
    /// `unwind(3)`: the only loop this proof's own shape controls is
    /// `aggregate_raw`'s `for child in &node.children`, over `SymTree`'s at
    /// most 2 children -- 2 iterations plus 1 exit check needs unwind >= 3.
    /// This is the minimal *sound* bound for that loop; it does not by
    /// itself make the harness terminate (see the module doc comment --
    /// the stall is inside `BTreeSet<StatusKind>`'s own internal unwinding,
    /// governed by the same global bound, whose required depth for
    /// this proof's data is not established because no run has completed).
    #[kani::unwind(3)]
    #[kani::proof]
    fn proof_worst_of_evidence() {
        let t: SymTree = kani::any();
        let expected = t.expected_claimable_min().unwrap_or(Evidence::Unclaimed);

        let agg = aggregate(&t.build());
        assert_eq!(agg.evidence, expected);

        let any_violation = t.root.own_evidence() == Some(Evidence::Violation)
            || (t.has_child_a && t.child_a.own_evidence() == Some(Evidence::Violation))
            || (t.has_child_b && t.child_b.own_evidence() == Some(Evidence::Violation));
        if any_violation {
            assert_eq!(agg.evidence, Evidence::Violation);
        }
    }

    /// Standing obligation 2: `conditional` never disappears without its
    /// assumptions.
    ///
    /// `unwind(3)`: see `proof_worst_of_evidence` -- same `SymTree` shape,
    /// same bound, same caveat that this does not make it terminate.
    #[kani::unwind(3)]
    #[kani::proof]
    fn proof_conditional_carries_its_assumptions() {
        let t: SymTree = kani::any();
        let any_conditional = t.root.conditional
            || (t.has_child_a && t.child_a.conditional)
            || (t.has_child_b && t.child_b.conditional);

        let agg = aggregate(&t.build());
        assert_eq!(agg.conditional.is_some(), any_conditional);
        if any_conditional {
            assert!(!agg.conditional.unwrap().is_empty());
        }
    }

    /// Standing obligation 4: aggregating the same tree twice yields
    /// identical results.
    ///
    /// `unwind(3)`: see `proof_worst_of_evidence` -- same `SymTree` shape,
    /// same bound, same caveat that this does not make it terminate.
    #[kani::unwind(3)]
    #[kani::proof]
    fn proof_aggregate_is_deterministic() {
        let t: SymTree = kani::any();
        let a = aggregate(&t.build());
        let b = aggregate(&t.build());
        assert_eq!(a, b);
    }
}
