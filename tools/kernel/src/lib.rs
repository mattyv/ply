//! The pure verdict kernel of the future `cargo ply`: evidence levels,
//! statuses, and the worst-of aggregation rule (SPEC.md §7, D5, D6). No I/O,
//! no external crates, no code that anchors to real source -- that belongs
//! to `ply-model`/`ply-core` later. `ply-model`'s own doc comment already
//! calls this split out explicitly ("verdicts, statuses, and fingerprints
//! need anchored code and are out of scope for this crate"); this crate is
//! that carve-out, kept dependency-free so its four standing invariants
//! (below) can be checked exhaustively and, once Kani is installed, proved.
//!
//! ## Scope
//!
//! SPEC.md §7 describes a full tree node as
//! `{ id, kind, anchor, content_hash, verdict, statuses, worst_descendant,
//! open_items }`. This kernel models only the part that is pure computation
//! over already-known verdicts: a node's own claim ([`NodeKind`]), its own
//! [`StatusKind`] set and conditional assumptions, plus [`aggregate`], which
//! computes `worst_descendant`/`statuses`/`open_items` from a tree of such
//! nodes. `id`/`anchor`/`content_hash` are identity and provenance concerns
//! that need a real codebase to anchor to; they are out of scope here and
//! belong to `ply-model`/`ply-core`.
//!
//! SPEC.md §7's paragraph "Aggregation rules the verdict kernel
//! (`tools/kernel`) checks exhaustively" is now the normative source for the
//! four rules below; earlier revisions of this file carried conservative
//! readings for two of them (own-evidence-for-containers, and
//! conditional/assumptions pairing) that the amendment has since settled --
//! see the doc comments on [`NodeKind`] and [`VerdictNode::conditional`].

use std::collections::BTreeSet;

/// SPEC.md D6 / §7: "The evidence order compares the six kinds" --
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
/// `#[cfg_attr(kani, derive(kani::Arbitrary))]` mirrors SPEC.md D2's own
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

/// SPEC.md §7: "Only claimable items contribute their own evidence. fns fold
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

/// SPEC.md §0 / §7: "Statuses ... do not sit in that [evidence] order; they
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

/// One node of a verdict tree (SPEC.md §7), reduced to exactly what
/// [`aggregate`] needs: this node's own claim ([`NodeKind`]), its own status
/// flags, its own conditional assumptions (if any), and its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictNode {
    pub kind: NodeKind,
    pub statuses: BTreeSet<StatusKind>,
    /// SPEC.md §7 / D5: "`conditional` structurally carries its assumptions:
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
    /// SPEC.md §7's `worst_descendant`: the worst-of (D6) over every
    /// *claimable* node's own evidence in the subtree. `Evidence::Unclaimed`
    /// exactly when the subtree contains no claimable node at all (§7: "A
    /// container with no claimable descendants reads `unclaimed`").
    pub evidence: Evidence,
    /// Union of this node's own `statuses` and every descendant's own
    /// `statuses` -- D6/§7: "statuses ... propagate upward as flags."
    pub statuses: BTreeSet<StatusKind>,
    /// Sorted, deduplicated union of the assumptions carried by every node
    /// in the subtree (self included) whose own [`VerdictNode::conditional`]
    /// is `Some(_)`. `None` exactly when no node in the subtree is
    /// conditional -- this is standing obligation 2: a conditional status
    /// never reaches an ancestor without the assumptions that justify it.
    pub conditional: Option<Vec<String>>,
    /// SPEC.md §7's `open_items`, folded as a count: "`open_items` counts
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
/// defaulted to `Evidence::Unclaimed` for display (SPEC.md §7: "A container
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
    let mut statuses: BTreeSet<StatusKind> = node.statuses.clone();
    let mut conditional = node.conditional.clone();
    let mut open_items = node.statuses.len() + if node.conditional.is_some() { 1 } else { 0 };
    let mut children = Vec::with_capacity(node.children.len());

    for child in &node.children {
        let (child_raw, child_agg) = aggregate_raw(child);
        raw_evidence = combine_claimable(raw_evidence, child_raw);
        statuses.extend(child_agg.statuses.iter().copied());
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
        AggregatedNode { evidence, statuses, conditional, open_items, children },
    )
}

/// Fold a verdict tree into its per-node aggregated results (SPEC.md §7,
/// D6). Pure: no I/O, no randomness, no shared mutable state -- calling it
/// twice on equal inputs always yields equal outputs (standing obligation
/// 4). Every collection used for aggregated state (`BTreeSet`, plus a
/// sorted+deduplicated `Vec` for conditional assumptions) is
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
            statuses: BTreeSet::new(),
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
    /// during the SPEC.md §7 rework: a container must not seed its own fold
    /// with a placeholder `Unclaimed` and then `.min()` it against a real,
    /// stronger claimable child -- that wrongly produces `Unclaimed` here
    /// instead of `Tested`.
    #[test]
    fn a_container_with_one_claimable_child_reports_the_childs_evidence() {
        let tree = VerdictNode {
            kind: NodeKind::Container,
            statuses: BTreeSet::new(),
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
            statuses: BTreeSet::new(),
            conditional: None,
            children: vec![
                leaf(NodeKind::Claimable(Evidence::Violation)),
                leaf(NodeKind::Claimable(Evidence::Tested)),
            ],
        };
        let agg = aggregate(&tree);
        assert_eq!(agg.evidence, Evidence::Violation, "a violation anywhere must reach the root");
    }

    #[test]
    fn conditional_on_a_child_propagates_with_its_assumptions() {
        let mut conditional_child = leaf(NodeKind::Claimable(Evidence::Bounded));
        conditional_child.conditional = Some(vec!["parser::parse fuzzed(256)".to_string()]);

        let tree = VerdictNode {
            kind: NodeKind::Container,
            statuses: BTreeSet::new(),
            conditional: None,
            children: vec![conditional_child],
        };
        let agg = aggregate(&tree);
        assert_eq!(agg.conditional, Some(vec!["parser::parse fuzzed(256)".to_string()]));
    }
}

/// Kani proof harnesses for the same four standing obligations, using
/// `kani::any()`-generated symbolic trees. Entirely `#[cfg(kani)]`-gated:
/// under plain `cargo build`/`cargo test` this module does not exist at all
/// (the `kani` crate is not a Cargo dependency -- `cargo kani` supplies it as
/// a compiler-provided pseudo-crate, per SPEC.md D9's "engines run as
/// subprocesses ... never linked as libraries"), so there is nothing here
/// for plain cargo to fail to compile. Run with `cargo kani` once Kani is
/// installed (not attempted in this session).
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
            let mut statuses = BTreeSet::new();
            if self.other_status {
                statuses.insert(StatusKind::Stale);
            }
            let conditional = if self.conditional {
                Some(vec!["kani-symbolic-assumption".to_string()])
            } else {
                None
            };
            VerdictNode { kind: self.kind, statuses, conditional, children }
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

    /// Standing obligation 1 (as reworded by SPEC.md §7's amendment):
    /// worst-of never reports evidence stronger than the weakest claimable
    /// node in the subtree, and reports exactly `Unclaimed` when there is
    /// none. Standing obligation 3 (violation-reaches-root) is the case of
    /// this where the weakest claimable node is a `Violation`.
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
    #[kani::proof]
    fn proof_aggregate_is_deterministic() {
        let t: SymTree = kani::any();
        let a = aggregate(&t.build());
        let b = aggregate(&t.build());
        assert_eq!(a, b);
    }
}
