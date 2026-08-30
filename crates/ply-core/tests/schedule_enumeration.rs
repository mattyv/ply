//! Exhaustive enumeration over small call graphs, checking
//! [`ply_core::schedule::order`] against an independent oracle -- the
//! `every_painted_element_resolves_a_style_rule` style CLAUDE.md asks for:
//! walk the real output, fail on the first handful of counterexamples, print
//! the offending graph so the failure names the actual defect.
//!
//! This corpus replaces `tools/schedule/tests/enumeration.rs`'s
//! `plan_orders_callees_before_callers_and_batches_cycles_together`, which
//! enumerated a *different* scheduler (one that layers a cycle's dependents
//! into batches after the cycle, as though they were cleanly orderable).
//! `order`'s contract is deliberately more conservative -- see
//! `ply_core::schedule`'s module doc comment for why -- so this oracle is
//! rederived from that contract, not copied from the old one.
//!
//! ## What is enumerated
//!
//! Every directed "callee calls caller" edge relation on **4 nodes**
//! (self-loops included: 4*4 = 16 possible `(callee, caller)` edges, so
//! 2^16 = **65,536** edge masks) x every **subset of those 4 nodes as the
//! domain** (2^4 = **16** domain masks). Total **65,536 * 16 = 1,048,576**
//! combinations. Edges are generated over the full 4-node universe
//! regardless of domain membership -- including edges whose callee lies
//! *outside* the domain -- specifically to exercise the domain-restriction
//! behaviour `order`'s own doc comment warns is easy to get wrong (a node
//! outside the domain can never be placed, so anything depending on it,
//! even transitively, is permanently blocked exactly like a real cycle).
//!
//! ## The independent oracle
//!
//! `order`'s real implementation is a Kahn's-algorithm indegree count.
//! This oracle instead computes, from scratch, for each edge mask:
//!
//! 1. Forward reachability between all 4 nodes (fixed-point relaxation over
//!    a plain adjacency matrix).
//! 2. The full graph's nontrivial strongly-connected components from that
//!    reachability (self-loop, or mutual reachability with another node,
//!    counts as "nontrivial" -- a genuine cycle).
//! 3. A "poison set": every node in a nontrivial SCC, unioned (per domain)
//!    with every node simply not in the domain -- a domain-external node
//!    can never be placed, so it poisons anything depending on it exactly
//!    as a cycle would.
//! 4. The oracle's tainted set is the domain nodes reachable from the
//!    poison set by following edges forward (callee -> caller, one or more
//!    hops): a domain node depending on a poisoned callee is itself never
//!    placeable, and so poisons its own callers in turn.
//!
//! This is a different algorithm from Kahn's indegree counting (SCC +
//! reachability vs. iterative indegree decrement), so it is a genuine
//! independent check, not a restatement of the code under test.
use ply_core::schedule::order;
use std::collections::{BTreeMap, BTreeSet};

const N: usize = 4;
/// 4 nodes x 4 nodes = 16 possible directed (callee, caller) edges
/// (self-loops included), so 2^16 edge masks.
const EDGE_SLOTS: usize = N * N;

fn edge_slots() -> Vec<(usize, usize)> {
    let mut slots = Vec::with_capacity(EDGE_SLOTS);
    for callee in 0..N {
        for caller in 0..N {
            slots.push((callee, caller));
        }
    }
    slots
}

fn build_edges(edges_mask: u32, slots: &[(usize, usize)]) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (i, &(callee, caller)) in slots.iter().enumerate() {
        if edges_mask & (1 << i) != 0 {
            edges.entry(callee).or_default().insert(caller);
        }
    }
    edges
}

fn build_domain(domain_mask: u32) -> BTreeSet<usize> {
    (0..N).filter(|&i| domain_mask & (1 << i) != 0).collect()
}

fn node_ids() -> Vec<String> {
    (0..N).map(|i| format!("n{i}")).collect()
}

/// Independent reachability oracle: a plain adjacency matrix relaxed to its
/// fixed point, never touching `ply_core::schedule`'s internals. `reach[i][j]`
/// is true iff `j` is reachable from `i` by following one or more `(callee,
/// caller)` edges forward (`i == j` always true, reflexively).
fn oracle_reach(edges_mask: u32, slots: &[(usize, usize)]) -> [[bool; N]; N] {
    let mut adj = [[false; N]; N];
    for (i, &(callee, caller)) in slots.iter().enumerate() {
        if edges_mask & (1 << i) != 0 {
            adj[callee][caller] = true;
        }
    }
    let mut reach = [[false; N]; N];
    for (i, row) in reach.iter_mut().enumerate() {
        row[i] = true;
    }
    // Plain triple-index matrix relaxation reads clearer than an iterator
    // chain here (both the row and column being tested change meaning at
    // every nesting level), so the range-loop lint is silenced rather than
    // routed around.
    #[allow(clippy::needless_range_loop)]
    loop {
        let mut changed = false;
        for i in 0..N {
            for j in 0..N {
                if reach[i][j] {
                    for k in 0..N {
                        if adj[j][k] && !reach[i][k] {
                            reach[i][k] = true;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    reach
}

/// A node with a genuine self-loop edge (`(i, i)` present in the mask) is a
/// one-node cycle in its own right, even though plain mutual-reachability
/// with *another* node cannot detect it (`reach[i][i]` is always true
/// reflexively, whether or not a real self-loop edge exists). Computed
/// straight from the mask, independent of `reach`.
fn has_self_loop(edges_mask: u32, slots: &[(usize, usize)], i: usize) -> bool {
    slots
        .iter()
        .enumerate()
        .any(|(idx, &(c, r))| c == i && r == i && edges_mask & (1 << idx) != 0)
}

/// The oracle's tainted set for a given domain: see the module doc comment
/// above for the derivation (SCC + domain-external nodes as poison sources,
/// then forward reachability).
fn oracle_tainted(
    edges_mask: u32,
    slots: &[(usize, usize)],
    domain: &BTreeSet<usize>,
) -> [bool; N] {
    let reach = oracle_reach(edges_mask, slots);

    let mut poison = [false; N];
    for i in 0..N {
        let nontrivial_scc = (0..N).any(|j| j != i && reach[i][j] && reach[j][i])
            || has_self_loop(edges_mask, slots, i);
        if nontrivial_scc || !domain.contains(&i) {
            poison[i] = true;
        }
    }

    let mut tainted = poison;
    for i in 0..N {
        if poison[i] {
            for j in 0..N {
                if reach[i][j] {
                    tainted[j] = true;
                }
            }
        }
    }
    std::array::from_fn(|i| tainted[i] && domain.contains(&i))
}

#[test]
fn order_places_callees_before_callers_and_taints_every_cycle_dependent() {
    let slots = edge_slots();
    let ids = node_ids();
    let total_edge_masks: u64 = 1u64 << EDGE_SLOTS;
    let total_domain_masks: u64 = 1u64 << N;
    assert_eq!(
        total_edge_masks, 65_536,
        "enumeration bound changed shape -- update this file's doc comment too if intentional"
    );
    assert_eq!(
        total_domain_masks, 16,
        "enumeration bound changed shape -- update this file's doc comment too if intentional"
    );

    let mut offending: Vec<String> = Vec::new();

    'outer: for edges_mask in 0..total_edge_masks {
        let edges_mask = edges_mask as u32;
        let edges = build_edges(edges_mask, &slots);

        for domain_mask in 0..total_domain_masks {
            let domain_mask = domain_mask as u32;
            let domain = build_domain(domain_mask);
            let expected_tainted = oracle_tainted(edges_mask, &slots, &domain);

            let (placed, tainted) = order(&domain, &ids, &edges);
            let (placed_again, tainted_again) = order(&domain, &ids, &edges);

            // Determinism (standing obligation 4).
            if placed != placed_again || tainted != tainted_again {
                offending.push(format!(
                    "determinism: order() gave two different results for edges {edges_mask:#06x}, domain {domain_mask:#06x}"
                ));
            }

            // Nothing outside the domain is ever placed or tainted.
            for &i in placed.iter() {
                if !domain.contains(&i) {
                    offending.push(format!(
                        "domain leak: edges {edges_mask:#06x}, domain {domain_mask:#06x}: node {i} placed but not in domain"
                    ));
                }
            }
            for &i in tainted.iter() {
                if !domain.contains(&i) {
                    offending.push(format!(
                        "domain leak: edges {edges_mask:#06x}, domain {domain_mask:#06x}: node {i} tainted but not in domain"
                    ));
                }
            }

            // placed and tainted are disjoint, and their union is exactly the domain.
            let placed_set: BTreeSet<usize> = placed.iter().copied().collect();
            for &i in tainted.iter() {
                if placed_set.contains(&i) {
                    offending.push(format!(
                        "overlap: edges {edges_mask:#06x}, domain {domain_mask:#06x}: node {i} is both placed and tainted"
                    ));
                }
            }
            for &i in domain.iter() {
                if !placed_set.contains(&i) && !tainted.contains(&i) {
                    offending.push(format!(
                        "coverage: edges {edges_mask:#06x}, domain {domain_mask:#06x}: node {i} is neither placed nor tainted"
                    ));
                }
            }

            // The tainted set matches the independent oracle exactly.
            for (i, &expected) in expected_tainted.iter().enumerate() {
                let actual = tainted.contains(&i);
                if actual != expected {
                    offending.push(format!(
                        "tainted mismatch: edges {edges_mask:#06x}, domain {domain_mask:#06x}, node {i}: order() says tainted={actual}, oracle says {expected}"
                    ));
                }
            }

            // Ordering: an edge (callee, caller) with both endpoints placed
            // must place callee strictly before caller.
            let position: BTreeMap<usize, usize> = placed
                .iter()
                .enumerate()
                .map(|(pos, &id)| (id, pos))
                .collect();
            for (i, &(callee, caller)) in slots.iter().enumerate() {
                if edges_mask & (1 << i) == 0 {
                    continue;
                }
                if let (Some(&pc), Some(&pr)) = (position.get(&callee), position.get(&caller))
                    && pc >= pr
                {
                    offending.push(format!(
                        "order violation: edges {edges_mask:#06x}, domain {domain_mask:#06x}: callee {callee} (pos {pc}) not before caller {caller} (pos {pr})"
                    ));
                }
            }

            if offending.len() >= 5 {
                break 'outer;
            }
        }
    }

    assert!(
        offending.is_empty(),
        "order() disagreed with the independent oracle (showing up to 5 of {} found):\n{}",
        offending.len(),
        offending.join("\n\n")
    );
}

/// Ties break on node id, not `Vec`/`BTreeSet` insertion order: two
/// independent, simultaneously-ready nodes must always place the smaller
/// id first. Regression-shaped rather than exhaustive -- the exhaustive
/// test above already runs `order` twice per input and requires equal
/// output, which catches nondeterminism from any source; this test
/// additionally pins which direction the tie-break must go.
#[test]
fn ties_among_ready_nodes_break_on_node_id() {
    let domain: BTreeSet<usize> = [0, 1].into_iter().collect();
    let ids = node_ids();
    let edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let (placed, tainted) = order(&domain, &ids, &edges);
    assert!(tainted.is_empty());
    assert_eq!(
        placed,
        vec![0, 1],
        "two independently-ready nodes must place the smaller id first"
    );
}
