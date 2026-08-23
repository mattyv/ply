//! Exhaustive enumeration of small verdict trees, checking `aggregate`
//! against an independent oracle on every node of every tree -- the
//! "`every_painted_element_resolves_a_style_rule`" style CLAUDE.md asks for:
//! walk the real output, fail on the first handful of counterexamples, print
//! the offending tree so the failure names the actual defect.
//!
//! ## The enumeration bound (stated per the task brief)
//!
//! Every tree with **1 to 4 total nodes**, **depth <= 3**, **<= 3 children
//! per node**, where each node's own (kind, statuses, conditional) is drawn
//! from a 21-option representative config space: 7 [`NodeKind`] shapes (6
//! `Claimable` evidence levels + `Container`) x 3 representative status
//! shapes (no status, conditional, another status).
//!
//! The status reduction is deliberate: The-Ply-Spec.md D6/§7 says statuses "do not
//! sit in [the evidence] order" and all propagate the same way (union
//! upward, count upward) *except* `conditional`, which alone carries an
//! extra assumptions-list obligation (D5, standing obligation 2). So one
//! stand-in "another status" (mapped to `Stale`) represents the other five
//! kinds (`Stale`/`WeakSpec`/`Unsupported`/`EngineMissing`/`Timeout`/
//! `Inconclusive`), which fold identically under union+count; a node
//! carrying two flags at once is deliberately left out of this
//! representative set -- the union/count fold is checked per flag
//! independently, and every multi-node tree in this corpus already
//! exercises combining two *different* flags across two *different* nodes.
//!
//! The `NodeKind` dimension is what The-Ply-Spec.md §7's amendment added: a
//! `Container` config carries no evidence of its own by construction, so
//! this corpus now covers both "claimable node with real evidence" and
//! "container with none" at every position in every enumerated shape,
//! including containers with zero, one, or several claimable descendants.
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
//! fold-across-siblings), while keeping the corpus at roughly 991K trees --
//! verified below to run in seconds even in an unoptimized debug build.
use ply_kernel::{AggregatedNode, Evidence, NodeKind, StatusKind, VerdictNode, aggregate};
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
/// Every conditional node in this corpus carries this exact assumption
/// text, so the expected aggregated assumptions list is always either
/// `None` or exactly `Some([ASSUMED_CONTRACT])` -- a crisp oracle, not just
/// "non-empty".
const ASSUMED_CONTRACT: &str = "assumed contract";

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
    fn node(&self, children: Vec<VerdictNode>) -> VerdictNode {
        let mut statuses = BTreeSet::new();
        let conditional = match self.status {
            StatusShape::None => None,
            StatusShape::Conditional => Some(vec![ASSUMED_CONTRACT.to_string()]),
            StatusShape::Other => {
                statuses.insert(StatusKind::Stale);
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

fn naive_statuses_union(node: &VerdictNode) -> BTreeSet<StatusKind> {
    let mut s = node.statuses.clone();
    for c in &node.children {
        s.extend(naive_statuses_union(c));
    }
    s
}

fn naive_open_items(node: &VerdictNode) -> usize {
    let own = node.statuses.len() + if node.conditional.is_some() { 1 } else { 0 };
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

    let expects_conditional = naive_has_conditional(node);
    let expected_conditional: Option<Vec<String>> = if expects_conditional {
        Some(vec![ASSUMED_CONTRACT.to_string()])
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
    if agg.statuses != expected_statuses {
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
    let trees = all_trees(MAX_NODES, &configs);
    assert_eq!(
        trees.len(),
        991_389,
        "enumeration bound changed shape -- update this crate's doc comments too if intentional"
    );

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
