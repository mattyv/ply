//! Ply's pure verification scheduler: the callees-before-callers ordering
//! (The-Ply-Spec.md D5, §5.5). No I/O, no anchoring to real source -- that
//! belongs to `ply-cli`, which builds the `domain`/`edges` this module takes
//! from its own call graph and consumes the result. Modeled on
//! `crates/ply-core/src/kernel.rs`'s split: one pure module, deterministic
//! collections only (`BTreeMap`/`BTreeSet`, never a hash-based collection
//! whose iteration order could vary between two equal inputs -- the same
//! footgun `kernel`'s `StatusSet` doc comment calls out for
//! `HashSet`/`HashMap`).
//!
//! ## Why this is soundness-critical
//!
//! D5's own text says Ply's scheduler -- callees verified first, a caller
//! credited with an assumed contract only if its callees' own checks passed
//! -- "is therefore the entire soundness guarantee; an implementation that
//! relaxes it is unsound and nothing downstream will notice." [`order`]'s
//! [`Placement::tainted`] set is what enforces the "nothing downstream will
//! notice" half in practice: every fn in that set is denied assumed-contract
//! credit entirely (its caller passes `bound: None` for every call, per
//! `resolve_contracted_calls` in `ply-cli`), never merely reported oddly.
//! Getting the *membership* of that set wrong -- too small -- is exactly the
//! bug class this module and its exhaustive enumeration
//! (`crates/ply-core/tests/schedule_enumeration.rs`) exist to catch.
//!
//! ## What "tainted" means, precisely
//!
//! A plain reading of "a cycle cannot be ordered" suggests the tainted set
//! is just the cycle's own members. It is not, and calling it merely
//! "cyclic" understates what is in it: because a node's turn to be placed
//! only comes once *all* of its in-domain callees have already been placed,
//! a node that transitively calls into a cycle -- without itself being part
//! of one -- never reaches its turn either. So `tainted` is the cycle
//! members **and every node that depends on one, however many calls away**.
//! Every one of them is denied assumed-contract credit, which is the
//! conservative direction: a caller three calls removed from a genuine cycle
//! gets no less scrutiny than the cycle members themselves, rather than
//! being scored as though its dependency chain were clean.
//!
//! The same fate befalls a node whose callee simply never enters `domain` at
//! all (a fn outside the bounded-eligible set this run is scheduling): it,
//! too, can never be placed, so anything depending on it is tainted exactly
//! as if it depended on a cycle. `order` treats "will never be placed" as
//! the one condition that matters, and both a genuine cycle and a
//! domain-external dependency produce it.
use std::collections::{BTreeMap, BTreeSet};

/// The result of [`order`]: which of `domain`'s nodes could be placed in a
/// callees-before-callers sequence, and which could not.
///
/// `placed` and `tainted` are always disjoint, and their union is always
/// exactly `domain` -- every domain node ends up in exactly one of the two.
pub type Placement = (Vec<usize>, BTreeSet<usize>);

/// Topological order (Kahn's algorithm, deterministic: ties break on node
/// id, never on `Vec`/`BTreeSet` insertion order) over the call graph
/// restricted to `domain` (The-Ply-Spec.md §5.5's "within a crate, verify
/// claimed functions callees-before-callers"). `edges` maps a callee's index
/// to the set of caller indices that depend on it (`edges[callee] ∋
/// caller`) -- the direction a caller's own contracted calls are collected
/// in, and the direction Kahn's algorithm needs to hand out "in-degree" as
/// "how many not-yet-placed callees does this caller still wait on".
///
/// Returns the orderable nodes callees-first, and separately the ones that
/// could never be placed -- see the module doc comment for exactly what
/// that second set contains and why "a cycle cannot be ordered" is not a
/// failure of this function, it is the fact D5's second branch exists to
/// catch.
///
/// `domain` is the only set this function may ever place a node from or
/// return in the tainted set -- restricted throughout, not merely at the
/// edges. An earlier version of this function (when it lived as
/// `crates/ply-cli/src/verify.rs`'s private `topological_order`) sized
/// everything off `node_ids.len()` (every plan, reused and
/// non-bounded-eligible ones included), so a reused or fuzz-only claim with
/// in-degree 0 by default silently entered the topological order and was
/// then run through the ordered pass unconditionally (adversarial review,
/// 2026-08-26). That bug is exactly what `crates/ply-core/tests/
/// schedule_enumeration.rs`'s domain-leak check exists to catch: edges are
/// generated over the full node universe there regardless of domain
/// membership, including edges whose callee lies outside the domain, and
/// the test asserts nothing outside `domain` is ever placed or tainted.
pub fn order(
    domain: &BTreeSet<usize>,
    node_ids: &[String],
    edges: &BTreeMap<usize, BTreeSet<usize>>,
) -> Placement {
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
    let tainted: BTreeSet<usize> = domain
        .iter()
        .copied()
        .filter(|i| !placed.contains(i))
        .collect();
    (order, tainted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("n{i}")).collect()
    }

    #[test]
    fn an_empty_domain_places_and_taints_nothing() {
        let (placed, tainted) = order(&BTreeSet::new(), &ids(0), &BTreeMap::new());
        assert!(placed.is_empty());
        assert!(tainted.is_empty());
    }

    #[test]
    fn independent_nodes_place_in_id_order() {
        let domain: BTreeSet<usize> = [0, 1, 2].into_iter().collect();
        let (placed, tainted) = order(&domain, &ids(3), &BTreeMap::new());
        assert_eq!(placed, vec![0, 1, 2]);
        assert!(tainted.is_empty());
    }

    #[test]
    fn a_callee_is_placed_before_its_caller() {
        let domain: BTreeSet<usize> = [0, 1].into_iter().collect();
        // callee 0 -> caller 1
        let mut edges = BTreeMap::new();
        edges.insert(0, [1].into_iter().collect());
        let (placed, tainted) = order(&domain, &ids(2), &edges);
        assert_eq!(placed, vec![0, 1]);
        assert!(tainted.is_empty());
    }

    #[test]
    fn a_two_node_cycle_taints_both_members() {
        let domain: BTreeSet<usize> = [0, 1].into_iter().collect();
        let mut edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        edges.insert(0, [1].into_iter().collect());
        edges.insert(1, [0].into_iter().collect());
        let (placed, tainted) = order(&domain, &ids(2), &edges);
        assert!(placed.is_empty());
        assert_eq!(tainted, domain);
    }

    /// The case the module doc comment names explicitly: a node that merely
    /// *depends on* a cycle, without being part of one, is tainted too.
    #[test]
    fn a_dependent_of_a_cycle_is_tainted_even_though_it_is_not_in_the_cycle() {
        let domain: BTreeSet<usize> = [0, 1, 2].into_iter().collect();
        let mut edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        edges.insert(0, [1].into_iter().collect()); // 0 <-> 1 cycle
        edges.insert(1, [0, 2].into_iter().collect()); // 1 also calls out to 2's caller edge: 2 depends on 1
        let (placed, tainted) = order(&domain, &ids(3), &edges);
        assert!(
            placed.is_empty(),
            "node 2 transitively depends on the cycle and must not be placed"
        );
        assert_eq!(tainted, domain);
    }

    #[test]
    fn a_node_outside_the_domain_never_appears_in_either_set() {
        // Only node 0 is in the domain; node 1 (its caller) is not.
        let domain: BTreeSet<usize> = [0].into_iter().collect();
        let mut edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        edges.insert(0, [1].into_iter().collect());
        let (placed, tainted) = order(&domain, &ids(2), &edges);
        assert_eq!(placed, vec![0]);
        assert!(tainted.is_empty());
    }

    #[test]
    fn depending_on_a_domain_external_node_taints_like_a_cycle_does() {
        // Node 1 is in the domain and depends on callee 0, which is *not*
        // in the domain -- 0 can never be placed, so 1 can never be either.
        let domain: BTreeSet<usize> = [1].into_iter().collect();
        let mut edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        edges.insert(0, [1].into_iter().collect());
        let (placed, tainted) = order(&domain, &ids(2), &edges);
        assert!(placed.is_empty());
        assert_eq!(tainted, domain);
    }

    #[test]
    fn calling_twice_on_equal_input_gives_equal_output() {
        let domain: BTreeSet<usize> = [0, 1, 2].into_iter().collect();
        let mut edges: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        edges.insert(0, [1, 2].into_iter().collect());
        let a = order(&domain, &ids(3), &edges);
        let b = order(&domain, &ids(3), &edges);
        assert_eq!(a, b);
    }
}
