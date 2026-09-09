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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Runs independent item indices with at most `jobs` active calls to `run`.
/// Results are returned in the same order as `items`, regardless of which
/// worker finishes first. A panic belongs to the item that caused it and does
/// not discard results from other workers.
///
/// `cancelled` is checked before a worker takes another item. Once set, no
/// unassigned work starts. The caller remains responsible for making an
/// already-running `run` observe the same token when prompt process cleanup
/// is required.
pub fn run_jobs<T, F>(
    items: &[usize],
    jobs: usize,
    cancelled: &AtomicBool,
    run: F,
) -> Vec<(usize, std::result::Result<T, String>)>
where
    T: Send,
    F: Fn(usize) -> T + Sync,
{
    if items.is_empty() || cancelled.load(Ordering::SeqCst) {
        return Vec::new();
    }
    let worker_count = jobs.max(1).min(items.len());
    let next = AtomicUsize::new(0);
    let (sender, receiver) = std::sync::mpsc::channel();

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let run = &run;
            let next = &next;
            scope.spawn(move || {
                loop {
                    if cancelled.load(Ordering::SeqCst) {
                        break;
                    }
                    let position = next.fetch_add(1, Ordering::SeqCst);
                    let Some(&item) = items.get(position) else {
                        break;
                    };
                    if cancelled.load(Ordering::SeqCst) {
                        break;
                    }
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(item)))
                            .map_err(panic_message);
                    if sender.send((position, item, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
    });

    let completed: BTreeMap<usize, (usize, std::result::Result<T, String>)> = receiver
        .into_iter()
        .map(|(position, item, result)| (position, (item, result)))
        .collect();
    completed.into_values().collect()
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "worker panicked without a message".to_string()
    }
}

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
    // `node_ids` is consulted for one thing only: a deterministic tie-break
    // key, so two independent nodes always place in the same order (see the
    // module doc). An index with no name still needs placing -- it is in the
    // domain, and a node this dropped would be a claim nobody ever verified.
    //
    // It used to index directly and panic. Ply found that by fuzzing this
    // function (`domain = {15}` against an empty `node_ids`, 2026-09-04).
    // Real callers build both lists from the same source so they always
    // agree, which is why nothing caught it. Declaring the agreement as a
    // precondition was tried and rejected: it is true, and it threw away
    // 1025 of 1195 generated inputs, so the function earned no evidence at
    // all. Every node being accounted for is the property worth keeping.
    let key = |i: usize| node_ids.get(i).cloned().unwrap_or_default();
    let mut ready: BTreeSet<(String, usize)> = domain
        .iter()
        .filter(|&&i| indegree[&i] == 0)
        .map(|&i| (key(i), i))
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
                        ready.insert((key(j), j));
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

/// Groups an already topologically ordered set into dependency-ready waves.
/// Every node in a wave depends only on nodes in earlier waves, so members of
/// one wave may overlap without letting a caller observe an unfinished
/// callee. Relative order inside each wave follows `ordered`.
pub fn waves(ordered: &[usize], edges: &BTreeMap<usize, BTreeSet<usize>>) -> Vec<Vec<usize>> {
    let ordered_set: BTreeSet<usize> = ordered.iter().copied().collect();
    let mut levels: BTreeMap<usize, usize> = ordered.iter().map(|&item| (item, 0)).collect();
    for &item in ordered {
        let next_level = levels.get(&item).copied().unwrap_or(0) + 1;
        if let Some(callers) = edges.get(&item) {
            for &caller in callers {
                if ordered_set.contains(&caller) {
                    levels
                        .entry(caller)
                        .and_modify(|level| *level = (*level).max(next_level));
                }
            }
        }
    }
    let wave_count = levels.values().copied().max().map_or(0, |level| level + 1);
    let mut result = vec![Vec::new(); wave_count];
    for &item in ordered {
        result[levels.get(&item).copied().unwrap_or(0)].push(item);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    /// Ids that deliberately do **not** sort in index order -- `n0..nN`
    /// would make a tie-break on index look identical to one on id, and
    /// that blind spot was real until review found it on 2026-08-30.
    /// These sort to indices `[2, 1, 3, 0]`.
    fn ids(n: usize) -> Vec<String> {
        ["d", "b", "a", "c"][..n]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Ply found this by fuzzing its own scheduler: `domain = {15}` against
    /// an empty `node_ids` panicked, because the index was used to look up a
    /// tie-break key without checking it was in range.
    ///
    /// Real callers build both from the same list, so they always agree --
    /// which is exactly why nothing caught it and why it stayed unwritten.
    /// It is fixed rather than declared as a precondition because the two
    /// answers are not equal: a precondition here would be true and would
    /// make the function unfuzzable (measured: 1025 of 1195 generated inputs
    /// thrown away, proptest gave up, no evidence at all), while every node
    /// still being accounted for is the property that actually matters. A
    /// node this dropped would be a claim nobody ever verified, and nothing
    /// else in the system would say so.
    #[test]
    fn an_index_with_no_name_is_still_placed_rather_than_panicking() {
        let domain: BTreeSet<usize> = [15].into_iter().collect();
        let (placed, tainted) = order(&domain, &[], &BTreeMap::new());
        assert_eq!(
            placed.len() + tainted.len(),
            domain.len(),
            "every node in the domain is either ordered or tainted, and 15 is in the domain \
             whether or not anything named it"
        );
        assert_eq!(placed, vec![15]);
    }

    /// And the tie-break stays deterministic when a name is missing: the
    /// same inputs must give the same order every run, or a verdict could
    /// change without the code changing.
    #[test]
    fn a_missing_name_still_breaks_ties_the_same_way_every_run() {
        let domain: BTreeSet<usize> = [0, 7].into_iter().collect();
        let first = order(&domain, &ids(1), &BTreeMap::new());
        let again = order(&domain, &ids(1), &BTreeMap::new());
        assert_eq!(first, again);
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
        // Ids "d", "b", "a" -- so id order is node 2, then 1, then 0, the
        // exact reverse of index order. An implementation breaking ties on
        // index would place `[0, 1, 2]` and pass with `n0..n2` ids.
        let (placed, tainted) = order(&domain, &ids(3), &BTreeMap::new());
        assert_eq!(placed, vec![2, 1, 0]);
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

    #[test]
    fn two_workers_really_overlap_and_one_worker_does_not() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let rendezvous = Arc::new(Barrier::new(2));
        let cancelled = AtomicBool::new(false);
        let results = run_jobs(&[0, 1], 2, &cancelled, {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let rendezvous = Arc::clone(&rendezvous);
            move |item| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                rendezvous.wait();
                active.fetch_sub(1, Ordering::SeqCst);
                item
            }
        });
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(results.len(), 2);

        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let results = run_jobs(&[0, 1], 1, &cancelled, |item| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            active.fetch_sub(1, Ordering::SeqCst);
            item
        });
        assert_eq!(peak.load(Ordering::SeqCst), 1);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn worker_completion_order_never_changes_result_order() {
        let cancelled = AtomicBool::new(false);
        let results = run_jobs(&[3, 1, 2], 3, &cancelled, |item| {
            std::thread::sleep(std::time::Duration::from_millis((4 - item) as u64 * 5));
            item * 10
        });
        assert_eq!(
            results
                .into_iter()
                .map(|(item, result)| (item, result.expect("worker completed")))
                .collect::<Vec<_>>(),
            vec![(3, 30), (1, 10), (2, 20)]
        );
    }

    #[test]
    fn one_worker_panicking_is_attached_to_its_item_and_does_not_drop_others() {
        let cancelled = AtomicBool::new(false);
        let results = run_jobs(&[0, 1, 2], 2, &cancelled, |item| {
            assert_ne!(item, 1, "controlled worker crash");
            item
        });
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].1.as_ref().unwrap(), &0);
        assert!(results[1].1.is_err());
        assert_eq!(results[2].1.as_ref().unwrap(), &2);
    }

    #[test]
    fn cancellation_stops_assigning_new_work() {
        let cancelled = AtomicBool::new(false);
        let results = run_jobs(&[0, 1, 2], 1, &cancelled, |item| {
            cancelled.store(true, Ordering::SeqCst);
            item
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 0);
    }

    #[test]
    fn dependency_waves_release_callers_only_after_all_callees() {
        // 0 and 1 are independent callees; 2 waits for both; 3 is unrelated.
        let order = vec![0, 1, 2, 3];
        let edges = BTreeMap::from([(0, BTreeSet::from([2])), (1, BTreeSet::from([2]))]);
        assert_eq!(waves(&order, &edges), vec![vec![0, 1, 3], vec![2]]);
    }
}
