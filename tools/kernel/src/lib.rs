//! The pure verdict kernel of the future `cargo ply`: evidence levels,
//! statuses, and the worst-of aggregation rule (SPEC.md §7, D5, D6). No I/O,
//! no external crates, no code that anchors to real source -- that belongs
//! to `ply-model`/`ply-core` later. `ply-model`'s own doc comment already
//! calls this split out explicitly ("verdicts, statuses, and fingerprints
//! need anchored code and are out of scope for this crate"); this crate is
//! that carve-out, kept dependency-free so its four standing invariants
//! (below) can be checked exhaustively and, once Kani is installed, proved.
//!
//! ## Scope and conservative readings
//!
//! SPEC.md §7 describes a full tree node as
//! `{ id, kind, anchor, content_hash, verdict, statuses, worst_descendant,
//! open_items }`. This kernel models only the part that is pure computation
//! over already-known verdicts: a node's own [`Evidence`] and [`StatusKind`]
//! set, plus [`aggregate`], which computes `worst_descendant`/`statuses`/
//! `open_items` from a tree of such nodes. `id`/`kind`/`anchor`/
//! `content_hash` are identity and provenance concerns that need a real
//! codebase to anchor to; they are out of scope here and belong to
//! `ply-model`/`ply-core`.
//!
//! Where SPEC.md is silent on an exact mechanism, the choices below are
//! documented at the point they're made, each citing the § or D-number that
//! motivates it.

use std::collections::BTreeSet;

/// SPEC.md D6: "Verdicts aggregate upward as worst-of over the evidence
/// order `violation < unclaimed < tested < fuzzed < bounded < proved`."
/// Declaration order below *is* that order, so `#[derive(PartialOrd, Ord)]`
/// gives the comparison for free -- "worst" is simply the smaller value.
///
/// Conservative reading (spec silent): §5.4c's `fuzz(n)` and `bounded(k)`
/// checks carry a numeric parameter (case count / loop bound). D6's order is
/// defined only over the six *kinds* of evidence, never over two claims of
/// the same kind with different n/k (e.g. is `fuzzed(1024)` stronger than
/// `fuzzed(64)`?). The kernel compares kinds only and carries no n/k payload;
/// a parameter-aware tie-break, if ever wanted, is a model-layer concern
/// built on top of this order, not a change to it.
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

/// SPEC.md §0 / D6: "Statuses ... do not sit in that [evidence] order; they
/// propagate upward as flags and open-item counts alongside it." This is the
/// full status vocabulary named in §0's glossary row for `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusKind {
    /// D5: rests on an assumed (not locally proved) contract. Carries an
    /// assumptions list on the node that sets it -- see [`VerdictNode::assumptions`].
    Conditional,
    Stale,
    WeakSpec,
    Unsupported,
    EngineMissing,
    Timeout,
    Inconclusive,
}

/// One node of a verdict tree (SPEC.md §7), reduced to exactly what
/// [`aggregate`] needs: this node's own evidence, its own status flags, the
/// assumptions backing a `Conditional` status (D5 -- "a conditional verdict
/// carries an assumptions list"), and its children.
///
/// `evidence` and `statuses` are this node's *own* claim, before folding in
/// any child. A composite node with no direct claim of its own (a component
/// with no fn claim attached to it directly) is represented the same way a
/// real one would be: `Evidence::Unclaimed`, empty `statuses`. SPEC.md does
/// not say composite nodes are exempt from the worst-of fold (D6 states the
/// rule once, without a composite-node carve-out), so `aggregate` folds
/// every node's own evidence uniformly, root and leaves alike -- the
/// literal, uniform reading of "verdicts aggregate upward as worst-of."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictNode {
    pub evidence: Evidence,
    pub statuses: BTreeSet<StatusKind>,
    /// D5's assumptions list. Only meaningful when `statuses` contains
    /// `Conditional`; the kernel does not enforce that pairing on input (a
    /// node could set `Conditional` with an empty list, or list assumptions
    /// without the flag) -- it simply reads this field when folding a
    /// `Conditional` status upward. A stricter model-layer validation could
    /// reject the mismatched cases; that validation is out of scope for a
    /// pure aggregation kernel.
    pub assumptions: Vec<String>,
    pub children: Vec<VerdictNode>,
}

/// The result of [`aggregate`] at one tree position: this node's subtree
/// (itself plus every descendant) folded into `worst_descendant`, unioned
/// `statuses`, unioned `assumptions`, and a total `open_items` count.
/// Mirrors the input tree's shape via `children`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedNode {
    /// SPEC.md §7's `worst_descendant`: the worst-of (D6) over this node's
    /// own evidence and every descendant's own evidence.
    pub evidence: Evidence,
    /// Union of this node's own `statuses` and every descendant's own
    /// `statuses` -- D6: "statuses ... propagate upward as flags."
    pub statuses: BTreeSet<StatusKind>,
    /// Sorted, deduplicated union of `assumptions` from every node in the
    /// subtree (self included) whose *own* `statuses` contains `Conditional`
    /// (D5). This is standing obligation 2: a `Conditional` flag never
    /// reaches an ancestor without the assumptions that justify it riding
    /// along with it.
    pub assumptions: Vec<String>,
    /// SPEC.md §7's `open_items`, folded as a count. Conservative reading
    /// (spec silent on the exact arithmetic): §7 lists "unresolved markers,
    /// weak specs, conditional or stale verdicts" as the kinds of thing that
    /// count, without saying whether one node with two flags counts once or
    /// twice. This kernel counts every individual status flag on every node
    /// in the subtree (so a node carrying both `Conditional` and `Stale`
    /// contributes 2, and the same `StatusKind` recurring on three nodes
    /// contributes 3) -- "how many distinct things need attention," the more
    /// information-preserving reading. Unresolved markers (`ply.yaml`
    /// registry entries, §5.6) have no representation in this kernel's node
    /// type -- they anchor to code the kernel never sees -- so they are not
    /// part of this count; a model layer that also tracks them would add
    /// its own count to this one.
    pub open_items: usize,
    pub children: Vec<AggregatedNode>,
}

/// Fold a verdict tree into its per-node aggregated results (SPEC.md §7,
/// D6). Pure: no I/O, no randomness, no shared mutable state -- calling it
/// twice on equal inputs always yields equal outputs (standing obligation
/// 4). Every collection used for aggregated state (`BTreeSet`, plus a
/// sorted+deduplicated `Vec` for assumptions) is order-independent by
/// construction, specifically to avoid the classic footgun where a
/// hash-based set's iteration order can differ between two otherwise-equal
/// instances (Rust's `HashSet`/`HashMap` seed their hasher per instance) --
/// that would make two `AggregatedNode` values compare unequal, or print
/// differently, for no reason tied to the data itself.
pub fn aggregate(node: &VerdictNode) -> AggregatedNode {
    let mut evidence = node.evidence;
    let mut statuses: BTreeSet<StatusKind> = node.statuses.clone();
    let mut assumptions: Vec<String> = if node.statuses.contains(&StatusKind::Conditional) {
        node.assumptions.clone()
    } else {
        Vec::new()
    };
    let mut open_items = node.statuses.len();
    let mut children = Vec::with_capacity(node.children.len());

    for child in &node.children {
        let agg = aggregate(child);
        // D6 worst-of: `Violation` is declared weakest, so the worst value
        // in the evidence order is the smaller one -- `.min()` is the fold
        // that "aggregates upward as worst-of" actually means. (An earlier
        // version of this line used `.max()`, watched
        // `aggregate_matches_naive_oracle_over_every_small_tree` in
        // tests/enumeration.rs fail with a printed counterexample tree
        // naming exactly this, then was fixed to `.min()`.)
        evidence = evidence.min(agg.evidence);
        statuses.extend(agg.statuses.iter().copied());
        assumptions.extend(agg.assumptions.iter().cloned());
        open_items += agg.open_items;
        children.push(agg);
    }

    assumptions.sort();
    assumptions.dedup();

    AggregatedNode {
        evidence,
        statuses,
        assumptions,
        open_items,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(evidence: Evidence) -> VerdictNode {
        VerdictNode {
            evidence,
            statuses: BTreeSet::new(),
            assumptions: Vec::new(),
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
    fn a_single_leaf_aggregates_to_its_own_evidence() {
        let agg = aggregate(&leaf(Evidence::Proved));
        assert_eq!(agg.evidence, Evidence::Proved);
        assert!(agg.statuses.is_empty());
        assert!(agg.assumptions.is_empty());
        assert_eq!(agg.open_items, 0);
    }

    #[test]
    fn a_violated_child_drags_a_proved_root_down_to_violation() {
        let tree = VerdictNode {
            evidence: Evidence::Proved,
            statuses: BTreeSet::new(),
            assumptions: Vec::new(),
            children: vec![leaf(Evidence::Violation), leaf(Evidence::Tested)],
        };
        let agg = aggregate(&tree);
        assert_eq!(agg.evidence, Evidence::Violation, "a violation anywhere must reach the root");
    }

    #[test]
    fn conditional_on_a_child_propagates_with_its_assumptions() {
        let mut conditional_child = leaf(Evidence::Bounded);
        conditional_child.statuses.insert(StatusKind::Conditional);
        conditional_child.assumptions = vec!["parser::parse fuzzed(256)".to_string()];

        let tree = VerdictNode {
            evidence: Evidence::Proved,
            statuses: BTreeSet::new(),
            assumptions: Vec::new(),
            children: vec![conditional_child],
        };
        let agg = aggregate(&tree);
        assert!(agg.statuses.contains(&StatusKind::Conditional));
        assert_eq!(agg.assumptions, vec!["parser::parse fuzzed(256)".to_string()]);
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
        evidence: Evidence,
        conditional: bool,
        other_status: bool,
    }

    impl SymLeaf {
        fn into_node(self, children: Vec<VerdictNode>) -> VerdictNode {
            let mut statuses = BTreeSet::new();
            let mut assumptions = Vec::new();
            if self.conditional {
                statuses.insert(StatusKind::Conditional);
                assumptions.push("kani-symbolic-assumption".to_string());
            }
            if self.other_status {
                statuses.insert(StatusKind::Stale);
            }
            VerdictNode {
                evidence: self.evidence,
                statuses,
                assumptions,
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
    }

    /// Standing obligations 1 and 3: worst-of never reports evidence
    /// stronger than the weakest own-evidence in the subtree, and in
    /// particular a `Violation` anywhere reaches the root.
    #[kani::proof]
    fn proof_worst_of_evidence() {
        let t: SymTree = kani::any();
        let mut expected = t.root.evidence;
        if t.has_child_a {
            expected = expected.min(t.child_a.evidence);
        }
        if t.has_child_b {
            expected = expected.min(t.child_b.evidence);
        }

        let agg = aggregate(&t.build());
        assert_eq!(agg.evidence, expected);

        let any_violation = t.root.evidence == Evidence::Violation
            || (t.has_child_a && t.child_a.evidence == Evidence::Violation)
            || (t.has_child_b && t.child_b.evidence == Evidence::Violation);
        if any_violation {
            assert_eq!(agg.evidence, Evidence::Violation);
        }
    }

    /// Standing obligation 2: `Conditional` never disappears without its
    /// assumptions.
    #[kani::proof]
    fn proof_conditional_carries_its_assumptions() {
        let t: SymTree = kani::any();
        let any_conditional = t.root.conditional
            || (t.has_child_a && t.child_a.conditional)
            || (t.has_child_b && t.child_b.conditional);

        let agg = aggregate(&t.build());
        assert_eq!(agg.statuses.contains(&StatusKind::Conditional), any_conditional);
        if any_conditional {
            assert!(!agg.assumptions.is_empty());
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
