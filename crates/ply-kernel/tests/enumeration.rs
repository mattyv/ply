//! Exhaustive enumeration of small verdict trees, checking `aggregate`
//! against an independent oracle on every node of every tree -- the
//! "`every_painted_element_resolves_a_style_rule`" style CLAUDE.md asks for:
//! walk the real output, fail on the first handful of counterexamples, print
//! the offending tree so the failure names the actual defect.
//!
//! ## What is enumerated, and what is only represented
//!
//! **Exhaustive over shape.** Every tree with **1 to 4 total nodes**, **depth
//! <= 3**, **<= 3 children per node**, each node's own (kind, status shape)
//! drawn independently from a 21-option space: 7 [`NodeKind`] shapes (6
//! `Claimable` evidence levels + `Container`) x 3 status shapes (no status,
//! conditional, flagged). 991,389 trees. Nothing is sampled and nothing is
//! skipped -- for those bounds this dimension really is every case.
//!
//! Capping *total node count* at 4 (rather than letting every one of <=3
//! children independently be a full <=3-children subtree at every one of 3
//! levels) is what keeps the space "low millions": the latter is not merely
//! large, it is combinatorially infeasible at any nontrivial config-space
//! size -- three levels of unconstrained 3-way branching alone produces more
//! than 10^14 trees even with only 2 configs per node, because the count at
//! depth d appears cubed (via the branching factor) in the count at depth
//! d+1. Capping the total node budget at 4 instead still reaches both
//! structural extremes that matter for these invariants: a depth-3 chain
//! (root -> child -> grandchild -> great-grandchild, exercising multi-level
//! fold-through) and a width-3 branch (root with 3 leaf children, exercising
//! fold-across-siblings).
//!
//! The `NodeKind` dimension is what The-Ply-Spec.md §7's amendment added: a
//! `Container` config carries no evidence of its own by construction, so
//! this corpus covers both "claimable node with real evidence" and
//! "container with none" at every position in every enumerated shape,
//! including containers with zero, one, or several claimable descendants.
//!
//! **Representative over payload.** Which *concrete* flags a flagged node
//! carries, and which *concrete* text a conditional node's assumption list
//! holds, are not enumerated -- they are chosen by the node's pre-order
//! position, cycling with period 2 (`decorate_by_position`). So the corpus
//! contains 3 of the 7 [`StatusKind`]s and 2 assumption texts, not all of
//! them and not all strings.
//!
//! That is a real reduction, and calling the result "exhaustive" without
//! arguing it would be overclaiming by quotient. The argument -- per-bit
//! uniformity of [`StatusSet`], and how far content-independence of the
//! assumption merge does and does not hold -- is written out in
//! **`docs/kernel-honesty-cleanups.md`**, together with the mutation runs
//! that measure it and the one payload combination this corpus still does
//! not reach (a single node carrying both a status flag and a conditional).
//! It lives there rather than here because it runs to a couple of pages with
//! an evidence table, and burying that above the code would put the test
//! itself past where anyone scrolls. Read it before widening or narrowing
//! the payload cycles below: it is what licenses the word "exhaustive"
//! anywhere near this file.
//!
//! Until 2026-08-25 the payloads were *one* status kind and *one* assumption
//! text, and the header asserted that reduction was safe rather than arguing
//! it. It was not safe: three separate one-line breakages of the real kernel
//! -- dropping `sort()`, dropping half the list in `merge_conditional`, and
//! keeping only the first non-empty status set in the upward union -- all
//! left this test green. The period-2 cycles are what kill them, at the same
//! 991,389 trees and the same runtime.
use ply_kernel::{
    AggregatedNode, Evidence, NodeKind, StatusKind, StatusSet, VerdictNode, aggregate,
};
use std::collections::BTreeSet;

const ALL_EVIDENCE: [Evidence; 6] = [
    Evidence::Violation,
    Evidence::Unclaimed,
    Evidence::Tested,
    Evidence::Fuzzed,
    Evidence::Bounded,
    Evidence::Proved,
];

const MAX_NODES: usize = 4;
const MAX_DEPTH: u32 = 3;
const MAX_CHILDREN: usize = 3;

/// The two assumption texts a `StatusShape::Conditional` node can carry,
/// selected by the node's pre-order position (see `decorate_by_position`).
///
/// Two, not one, because with a single text every merge is a merge of equal
/// values: `merge_conditional`'s "keep both lists" arm and `sort()` are both
/// unobservable, and mutating either away leaves this test green (measured
/// -- see docs/kernel-honesty-cleanups.md). Two texts make the union, the
/// dedup, and the sort each observable.
///
/// They are deliberately in *descending* lexicographic order, so a node at
/// an even position followed by one at an odd position produces
/// `["zeta ...", "alpha ..."]` before sorting -- concatenation order and
/// sorted order disagree, which is what makes a missing `sort()` visible.
const ASSUMED_CONTRACTS: [&str; 2] = ["zeta: assumed contract", "alpha: assumed contract"];

/// The two status-flag sets a `StatusShape::Other` node can carry, selected
/// by the node's pre-order position (see `decorate_by_position`).
///
/// The second is a *pair* so that some node in the corpus carries two flags
/// at once: without it no node ever contributes more than one open item, and
/// The-Ply-Spec.md §7's own sentence -- "a node carrying two statuses
/// contributes 2" -- is never exercised. Distinct kinds across the two
/// entries are what make the upward union combine two different bits rather
/// than re-union the same one.
const FLAG_SETS: [&[StatusKind]; 2] = [
    &[StatusKind::Stale],
    &[StatusKind::WeakSpec, StatusKind::Timeout],
];

#[derive(Clone, Copy)]
enum StatusShape {
    None,
    Conditional,
    Other,
}

#[derive(Clone, Copy)]
struct Config {
    kind: NodeKind,
    status: StatusShape,
}

/// 7 node-kind shapes (6 `Claimable` evidence levels + `Container`) x 3
/// representative status shapes = 21 configs.
fn all_configs() -> Vec<Config> {
    let mut kinds: Vec<NodeKind> = ALL_EVIDENCE
        .iter()
        .map(|&e| NodeKind::Claimable(e))
        .collect();
    kinds.push(NodeKind::Container);

    let mut out = Vec::new();
    for &kind in &kinds {
        for status in [
            StatusShape::None,
            StatusShape::Conditional,
            StatusShape::Other,
        ] {
            out.push(Config { kind, status });
        }
    }
    out
}

impl Config {
    /// Builds the node with a *placeholder* payload: the right shape (no
    /// flags / conditional / flagged), with the concrete flag set and
    /// assumption text filled in afterwards by `decorate_by_position`, which
    /// needs the node's position in the finished tree to choose them.
    fn node(&self, children: Vec<VerdictNode>) -> VerdictNode {
        let mut statuses = StatusSet::new();
        let conditional = match self.status {
            StatusShape::None => None,
            StatusShape::Conditional => Some(Vec::new()),
            StatusShape::Other => {
                statuses.insert(PLACEHOLDER_FLAG);
                None
            }
        };
        VerdictNode {
            kind: self.kind,
            statuses,
            conditional,
            children,
        }
    }
}

/// Marks a `StatusShape::Other` node before `decorate_by_position` replaces
/// it with the real flag set for that position. Any variant would do; it
/// never survives into an enumerated tree.
const PLACEHOLDER_FLAG: StatusKind = StatusKind::Stale;

/// Walks a finished tree in pre-order and gives each flagged or conditional
/// node the payload for its position: flag set `FLAG_SETS[i % 2]`,
/// assumption text `ASSUMED_CONTRACTS[i % 2]`, for pre-order index `i`.
///
/// Position, not config, chooses the payload because doing it the other way
/// -- adding "two flags" and "the other assumption" as further `Config`
/// variants -- multiplies the config space, and the corpus grows as
/// (configs)^(nodes). This pass leaves the tree count at exactly 991,389.
///
/// The trade it makes: each (shape, position) pair gets exactly one payload,
/// so payloads are represented rather than enumerated. A period of 2 over
/// trees of up to 4 nodes is enough to reach what the fold actually needs to
/// be seen doing -- both same-payload pairs (positions 0 and 2) and
/// different-payload pairs (0 and 1), in parent-with-child and
/// sibling-with-sibling roles. What it still cannot reach is listed under
/// "What the reduction still does not cover" in
/// docs/kernel-honesty-cleanups.md.
fn decorate_by_position(node: &mut VerdictNode, next_index: &mut usize) {
    let i = *next_index;
    *next_index += 1;

    if !node.statuses.is_empty() {
        let mut statuses = StatusSet::new();
        for &flag in FLAG_SETS[i % FLAG_SETS.len()] {
            statuses.insert(flag);
        }
        node.statuses = statuses;
    }
    if node.conditional.is_some() {
        node.conditional = Some(vec![
            ASSUMED_CONTRACTS[i % ASSUMED_CONTRACTS.len()].to_string(),
        ]);
    }

    for child in &mut node.children {
        decorate_by_position(child, next_index);
    }
}

/// All ways to write `total` as an ordered sum of `parts` positive integers.
fn compositions(total: usize, parts: usize) -> Vec<Vec<usize>> {
    if parts == 1 {
        return vec![vec![total]];
    }
    let mut out = Vec::new();
    for first in 1..=(total - (parts - 1)) {
        for mut rest in compositions(total - first, parts - 1) {
            rest.insert(0, first);
            out.push(rest);
        }
    }
    out
}

/// Cartesian product over a fixed number of child slots, each slot's
/// candidate list already computed independently.
fn cartesian(slots: &[Vec<VerdictNode>]) -> Vec<Vec<VerdictNode>> {
    match slots.first() {
        None => vec![Vec::new()],
        Some(first) => {
            let rest = cartesian(&slots[1..]);
            let mut out = Vec::with_capacity(first.len() * rest.len());
            for node in first {
                for tail in &rest {
                    let mut v = Vec::with_capacity(1 + tail.len());
                    v.push(node.clone());
                    v.extend(tail.iter().cloned());
                    out.push(v);
                }
            }
            out
        }
    }
}

/// Every tree with exactly `n` total nodes, depth <= `depth_budget` from
/// this node down, <= `MAX_CHILDREN` children per node, node configs drawn
/// from `configs`.
fn trees_with_exactly(n: usize, depth_budget: u32, configs: &[Config]) -> Vec<VerdictNode> {
    if n == 1 {
        return configs.iter().map(|c| c.node(Vec::new())).collect();
    }
    if depth_budget == 0 {
        return Vec::new();
    }
    let remaining = n - 1;
    let mut out = Vec::new();
    for num_children in 1..=remaining.min(MAX_CHILDREN) {
        for sizes in compositions(remaining, num_children) {
            let per_slot: Vec<Vec<VerdictNode>> = sizes
                .iter()
                .map(|&sz| trees_with_exactly(sz, depth_budget - 1, configs))
                .collect();
            for children in cartesian(&per_slot) {
                for cfg in configs {
                    out.push(cfg.node(children.clone()));
                }
            }
        }
    }
    out
}

fn all_trees(max_nodes: usize, configs: &[Config]) -> Vec<VerdictNode> {
    (1..=max_nodes)
        .flat_map(|n| trees_with_exactly(n, MAX_DEPTH, configs))
        .collect()
}

// --- Independent oracle: computed by a plain walk over `VerdictNode`,
// never calling `aggregate` itself, so it can't share a bug with it. ---

/// The min over *claimable-only* own-evidence in the subtree, `None` if the
/// subtree contains no claimable node at all. A `Container` contributes
/// nothing of its own; it only passes through whatever its children found.
fn naive_min_claimable_evidence(node: &VerdictNode) -> Option<Evidence> {
    let own = match node.kind {
        NodeKind::Claimable(e) => Some(e),
        NodeKind::Container => None,
    };
    node.children.iter().fold(own, |acc, child| {
        match (acc, naive_min_claimable_evidence(child)) {
            (None, x) => x,
            (x, None) => x,
            (Some(a), Some(b)) => Some(a.min(b)),
        }
    })
}

fn naive_has_conditional(node: &VerdictNode) -> bool {
    node.conditional.is_some() || node.children.iter().any(naive_has_conditional)
}

/// Every assumption text anywhere in the subtree, collected into a std
/// `BTreeSet` -- which sorts and deduplicates by a completely different
/// mechanism than `aggregate_raw`'s `Vec::extend` + `sort` + `dedup`. That
/// is the point: the oracle must not reach the same answer by running the
/// same code.
fn naive_assumptions<'a>(node: &'a VerdictNode, out: &mut BTreeSet<&'a str>) {
    if let Some(assumptions) = &node.conditional {
        out.extend(assumptions.iter().map(String::as_str));
    }
    for c in &node.children {
        naive_assumptions(c, out);
    }
}

/// The union of every status flag in the subtree, as a std `BTreeSet` rather
/// than a [`StatusSet`]. Same reason as `naive_assumptions`: computing the
/// union with `StatusSet`'s own bitwise `extend` would make the oracle share
/// the implementation's set algebra, so a bug in that algebra would agree
/// with itself and pass. `StatusSet::iter` is still shared -- the oracle can
/// only read a node's flags through it -- and that residue is covered by
/// `every_status_in_the_glossary_round_trips_through_the_bitset` in the
/// crate's own unit tests.
fn naive_statuses_union(node: &VerdictNode) -> BTreeSet<StatusKind> {
    let mut s: BTreeSet<StatusKind> = node.statuses.iter().collect();
    for c in &node.children {
        s.extend(naive_statuses_union(c));
    }
    s
}

fn naive_open_items(node: &VerdictNode) -> usize {
    let own_flags = node.statuses.iter().count();
    let own = own_flags + if node.conditional.is_some() { 1 } else { 0 };
    own + node.children.iter().map(naive_open_items).sum::<usize>()
}

/// Checks every node of `node`/`agg` (a matched pair from the input tree and
/// its `aggregate()` output) against the naive oracle, recursing into
/// children. Appends a human-readable mismatch plus the offending subtree to
/// `offending` rather than panicking immediately, so a run can report
/// several distinct counterexamples at once (mirrors the `unstyled` Vec
/// pattern in tools/render/tests/render.rs).
fn check_subtree(node: &VerdictNode, agg: &AggregatedNode, offending: &mut Vec<String>) {
    let expected_evidence = naive_min_claimable_evidence(node).unwrap_or(Evidence::Unclaimed);
    if agg.evidence != expected_evidence {
        offending.push(format!(
            "evidence mismatch: expected worst-of-claimable {expected_evidence:?}, got {:?}, for subtree {node:#?}",
            agg.evidence
        ));
    }

    let expected_conditional: Option<Vec<String>> = if naive_has_conditional(node) {
        let mut assumptions = BTreeSet::new();
        naive_assumptions(node, &mut assumptions);
        Some(assumptions.into_iter().map(str::to_string).collect())
    } else {
        None
    };
    if agg.conditional != expected_conditional {
        offending.push(format!(
            "conditional mismatch: expected {expected_conditional:?}, got {:?}, for subtree {node:#?}",
            agg.conditional
        ));
    }

    let expected_statuses = naive_statuses_union(node);
    if agg.statuses.iter().collect::<BTreeSet<_>>() != expected_statuses {
        offending.push(format!(
            "statuses union mismatch: expected {expected_statuses:?}, got {:?}, for subtree {node:#?}",
            agg.statuses
        ));
    }

    let expected_open_items = naive_open_items(node);
    if agg.open_items != expected_open_items {
        offending.push(format!(
            "open_items mismatch: expected {expected_open_items}, got {}, for subtree {node:#?}",
            agg.open_items
        ));
    }

    for (child, child_agg) in node.children.iter().zip(agg.children.iter()) {
        check_subtree(child, child_agg, offending);
    }
}

/// Standing obligations 1, 2, 3, and 4, checked at every node of every tree
/// in the corpus (not just the root -- a bug could plausibly show up only at
/// an interior node): worst-of-claimable evidence with the container's
/// "reads unclaimed" case (1 and 3), conditional+assumptions propagation (2,
/// via the naive oracle), and determinism (4, via a second independent
/// `aggregate()` call on the same tree). One pass, one corpus build.
#[test]
fn aggregate_matches_naive_oracle_over_every_small_tree() {
    let configs = all_configs();
    let mut trees = all_trees(MAX_NODES, &configs);
    assert_eq!(
        trees.len(),
        991_389,
        "enumeration bound changed shape -- update this crate's doc comments too if intentional"
    );
    for tree in &mut trees {
        decorate_by_position(tree, &mut 0);
    }

    let mut offending: Vec<String> = Vec::new();
    for tree in &trees {
        let agg = aggregate(tree);
        check_subtree(tree, &agg, &mut offending);

        let agg_again = aggregate(tree);
        if agg != agg_again {
            offending.push(format!(
                "determinism mismatch: aggregate() gave two different results for the same tree {tree:#?}\nfirst: {agg:#?}\nsecond: {agg_again:#?}"
            ));
        }

        if offending.len() >= 5 {
            break;
        }
    }
    assert!(
        offending.is_empty(),
        "aggregate() disagreed with the naive oracle (showing up to 5 of {} found):\n{}",
        offending.len(),
        offending.join("\n\n")
    );
}
