//! Exhaustive enumeration over small call graphs, checking [`plan`] and [`may_stub`]
//! against independent oracles built fresh in this file -- never by calling
//! `ply_schedule`'s own internals (which are private besides the two functions under
//! test anyway). Two corpora, one per function under test, per this crate's task
//! brief ("Enumerate exhaustively over every directed graph up to a small node
//! bound").
//!
//! ## Corpus A -- `plan`'s dependency/cycle ordering (invariants 1, 3's
//! no-deadlock/termination half, and 4)
//!
//! Every directed graph on **4 nodes**, self-loops included: 4*4 = 16 possible
//! `(caller, callee)` edges, so 2^16 = **65,536** graphs. `plan` never reads per-fn
//! claim/result data, so no per-node config is crossed in here -- this corpus is
//! about graph *topology* alone. 4 nodes is enough to exercise both structural
//! extremes that matter: a depth-3 chain (a->b->c->d, exercising multi-level
//! ordering) and multi-node cycles up to a full 4-cycle, while keeping the corpus
//! small enough to enumerate in well under a second even in a debug build.
//!
//! ## Corpus B -- `may_stub`'s Allowed/Refused gate (invariant 2, and invariant 3's
//! "no cycle member may stub another" half)
//!
//! Every directed graph on **2 nodes** (self-loops included: 2*2 = 4 possible edges,
//! so 2^4 = 16 graphs) x every combination of a **16-config** per-node claim/result
//! space (4 proof-relevant configs: no contract / has contract but unproved / has
//! contract and failed / has contract and passed -- x 2 crate ids x 2 `is_pub`
//! values) assigned independently to each of the 2 nodes: 16^2 = 256 configs. Total
//! **16 * 256 = 4,096** graph+config combinations.
//!
//! 2 nodes is enough for this corpus's job: [`may_stub`]'s decision for an ordered
//! pair `(caller, callee)` depends only on (a) whether `caller`/`callee` are mutually
//! reachable -- representable with 2 nodes via a direct 2-cycle, or via a self-loop
//! for `caller == callee`, both structurally equivalent to any longer cycle for this
//! decision (only "same SCC or not" matters, not cycle length) -- (b) the crate/`pub`
//! relationship between the two, and (c) `callee`'s own contract/proof state. General
//! multi-node cycle *topology* (arbitrary-length cycles, chains through third nodes)
//! is already exhaustively covered by Corpus A; Corpus B's job is only to check the
//! decision table itself is implemented correctly at every input combination that
//! table actually branches on, which 2 nodes reaches in full.
use ply_schedule::{
    Batch, CallGraph, CrateId, Evidence, FnId, FnInfo, ProofResults, ProofStatus, StubDecision,
    StubRefusalReason, may_stub, plan,
};

/// Unwraps the `Vec<FnId>` payload of each [`Batch`] in order, for iterating "which
/// node is in which batch index" without repeating `.0` at every call site.
fn real_batches(batches: &[Batch]) -> impl Iterator<Item = &[FnId]> {
    batches.iter().map(|b| b.0.as_slice())
}

// --- Corpus A: plan() over every directed graph on 4 nodes ---

const N: usize = 4;
/// 4 nodes x 4 nodes = 16 possible directed edges (self-loops included), so 2^16
/// graphs. Declared as a named bound per the task brief's "state the bound ... in a
/// doc comment."
const EDGE_SLOTS: usize = N * N;

fn edge_slots() -> Vec<(FnId, FnId)> {
    let mut slots = Vec::with_capacity(EDGE_SLOTS);
    for caller in 0..N as FnId {
        for callee in 0..N as FnId {
            slots.push((caller, callee));
        }
    }
    slots
}

fn dummy_info() -> FnInfo {
    FnInfo {
        crate_id: CrateId(0),
        declares_contract: false,
        is_pub: false,
        own_evidence: Evidence::Unclaimed,
    }
}

fn build_graph(edges_mask: u32, slots: &[(FnId, FnId)]) -> CallGraph {
    let mut g = CallGraph::new();
    for id in 0..N as FnId {
        g.add_fn(id, dummy_info());
    }
    for (i, &(caller, callee)) in slots.iter().enumerate() {
        if edges_mask & (1 << i) != 0 {
            g.add_edge(caller, callee);
        }
    }
    g
}

/// Independent oracle: reachability computed fresh (plain BFS via an adjacency list
/// built directly from the mask), never touching `ply_schedule`'s internals.
fn oracle_reachable(edges_mask: u32, slots: &[(FnId, FnId)], start: FnId) -> [bool; N] {
    let mut reach = [false; N];
    reach[start as usize] = true;
    loop {
        let mut changed = false;
        for (i, &(caller, callee)) in slots.iter().enumerate() {
            if edges_mask & (1 << i) != 0 && reach[caller as usize] && !reach[callee as usize] {
                reach[callee as usize] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    reach
}

fn oracle_same_scc(reach: &[[bool; N]; N], a: FnId, b: FnId) -> bool {
    a == b || (reach[a as usize][b as usize] && reach[b as usize][a as usize])
}

/// Independent oracle for `plan`'s batch index of every node: longest-path layer over
/// the SCC condensation, computed from scratch (union by mutual reachability, then a
/// straightforward fixed-point relaxation over the -- acyclic by construction --
/// condensation graph; no memoized recursion, no shared code with `ply_schedule`).
fn oracle_batch_index(edges_mask: u32, slots: &[(FnId, FnId)]) -> [usize; N] {
    let reach: [[bool; N]; N] =
        std::array::from_fn(|i| oracle_reachable(edges_mask, slots, i as FnId));

    // Canonical SCC representative per node: the smallest id in its mutual-reachability
    // class.
    let scc_rep: [usize; N] = std::array::from_fn(|i| {
        (0..N)
            .find(|&j| oracle_same_scc(&reach, i as FnId, j as FnId))
            .unwrap()
    });

    // Fixed-point relaxation: layer(scc) = 0 if it calls no *other* scc, else
    // 1 + max(layer of every other scc it calls). The condensation is acyclic by
    // construction (SCCs are maximal), so this always converges in <= N passes.
    let mut layer = [0usize; N];
    for _ in 0..N {
        for (i, &(caller, callee)) in slots.iter().enumerate() {
            if edges_mask & (1 << i) == 0 {
                continue;
            }
            let sc = scc_rep[caller as usize];
            let sd = scc_rep[callee as usize];
            if sc != sd {
                layer[sc] = layer[sc].max(layer[sd] + 1);
            }
        }
    }
    std::array::from_fn(|i| layer[scc_rep[i]])
}

#[test]
fn plan_orders_callees_before_callers_and_batches_cycles_together() {
    let slots = edge_slots();
    let total_graphs: u64 = 1u64 << EDGE_SLOTS;
    assert_eq!(
        total_graphs, 65_536,
        "enumeration bound changed shape -- update this file's doc comment too if intentional"
    );

    let mut offending: Vec<String> = Vec::new();

    for mask in 0..total_graphs {
        let mask = mask as u32;
        let graph = build_graph(mask, &slots);
        let expected_index = oracle_batch_index(mask, &slots);

        let batches = plan(&graph);
        let batches_again = plan(&graph);

        // Determinism (invariant 4).
        if batches != batches_again {
            offending.push(format!(
                "determinism: plan() gave two different results for mask {mask:#06x}"
            ));
        }

        // Coverage + no duplicates: every node appears in exactly one batch.
        let mut seen = [0usize; N];
        for (batch_idx, batch) in real_batches(&batches).enumerate() {
            for &id in batch {
                seen[id as usize] += 1;
                // Ordering (invariant 1, acyclic case) + same-batch grouping
                // (invariant 3, cycle case): the produced batch index must equal the
                // oracle's dependency layer exactly.
                if batch_idx != expected_index[id as usize] {
                    offending.push(format!(
                        "batch-index mismatch for mask {mask:#06x}, node {id}: plan put it in batch {batch_idx}, oracle says {}",
                        expected_index[id as usize]
                    ));
                }
            }
        }
        for (id, &count) in seen.iter().enumerate() {
            if count != 1 {
                offending.push(format!(
                    "coverage mismatch for mask {mask:#06x}: node {id} appeared in {count} batches (want exactly 1)"
                ));
            }
        }

        if offending.len() >= 5 {
            break;
        }
    }

    assert!(
        offending.is_empty(),
        "plan() disagreed with the independent oracle (showing up to 5 of {} found):\n{}",
        offending.len(),
        offending.join("\n\n")
    );
}

// --- Corpus B: may_stub() over every directed graph on 2 nodes x every per-node config ---

const N2: usize = 2;
const EDGE_SLOTS_2: usize = N2 * N2;

#[derive(Clone, Copy, Debug)]
enum ProofConfig {
    NoContract,
    Unproved,
    Failed,
    Passed,
}
const ALL_PROOF_CONFIGS: [ProofConfig; 4] = [
    ProofConfig::NoContract,
    ProofConfig::Unproved,
    ProofConfig::Failed,
    ProofConfig::Passed,
];

#[derive(Clone, Copy, Debug)]
struct NodeConfig {
    proof: ProofConfig,
    crate_id: u32,
    is_pub: bool,
}

fn all_node_configs() -> Vec<NodeConfig> {
    let mut out = Vec::with_capacity(16);
    for &proof in &ALL_PROOF_CONFIGS {
        for crate_id in [0u32, 1u32] {
            for is_pub in [false, true] {
                out.push(NodeConfig {
                    proof,
                    crate_id,
                    is_pub,
                });
            }
        }
    }
    out
}

fn edge_slots_2() -> Vec<(FnId, FnId)> {
    let mut slots = Vec::with_capacity(EDGE_SLOTS_2);
    for caller in 0..N2 as FnId {
        for callee in 0..N2 as FnId {
            slots.push((caller, callee));
        }
    }
    slots
}

fn build_graph_2(
    edges_mask: u32,
    slots: &[(FnId, FnId)],
    configs: &[NodeConfig; N2],
) -> (CallGraph, ProofResults) {
    let mut g = CallGraph::new();
    let mut results = ProofResults::new();
    for (id, cfg) in configs.iter().enumerate() {
        let id = id as FnId;
        let declares_contract = !matches!(cfg.proof, ProofConfig::NoContract);
        g.add_fn(
            id,
            FnInfo {
                crate_id: CrateId(cfg.crate_id),
                declares_contract,
                is_pub: cfg.is_pub,
                own_evidence: Evidence::Unclaimed,
            },
        );
        let status = match cfg.proof {
            ProofConfig::NoContract | ProofConfig::Unproved => ProofStatus::NotRun,
            ProofConfig::Failed => ProofStatus::Failed,
            ProofConfig::Passed => ProofStatus::Passed,
        };
        results.record(id, status);
    }
    for (i, &(caller, callee)) in slots.iter().enumerate() {
        if edges_mask & (1 << i) != 0 {
            g.add_edge(caller, callee);
        }
    }
    (g, results)
}

/// Independent oracle for the Allowed/Refused decision, matching D5/§5.5's text
/// directly rather than mirroring `may_stub`'s implementation.
fn oracle_decision(
    caller: FnId,
    callee: FnId,
    caller_cfg: &NodeConfig,
    callee_cfg: &NodeConfig,
    same_scc: bool,
) -> StubDecision {
    if caller == callee || same_scc {
        return StubDecision::Refused(StubRefusalReason::CalleeInCycle);
    }
    if caller_cfg.crate_id != callee_cfg.crate_id && !callee_cfg.is_pub {
        return StubDecision::Refused(StubRefusalReason::CalleeCrossCrateNotReprovable);
    }
    match callee_cfg.proof {
        ProofConfig::NoContract => StubDecision::Refused(StubRefusalReason::CalleeWeakerEvidence),
        ProofConfig::Unproved => StubDecision::Refused(StubRefusalReason::CalleeUnproved),
        ProofConfig::Failed => StubDecision::Refused(StubRefusalReason::CalleeFailed),
        ProofConfig::Passed => StubDecision::Allowed,
    }
}

#[test]
fn may_stub_allows_only_when_callee_actually_passed() {
    let slots = edge_slots_2();
    let node_configs = all_node_configs();
    assert_eq!(
        node_configs.len(),
        16,
        "per-node config space changed shape -- update this file's doc comment too if intentional"
    );
    let total_graphs: u64 = 1u64 << EDGE_SLOTS_2;
    assert_eq!(total_graphs, 16);
    let total_combinations = total_graphs * (node_configs.len() as u64).pow(N2 as u32);
    assert_eq!(
        total_combinations, 4_096,
        "enumeration bound changed shape -- update this file's doc comment too if intentional"
    );

    let mut offending: Vec<String> = Vec::new();

    'outer: for mask in 0..total_graphs {
        let mask = mask as u32;
        for &cfg0 in &node_configs {
            for &cfg1 in &node_configs {
                let configs = [cfg0, cfg1];
                let (graph, results) = build_graph_2(mask, &slots, &configs);

                // same_scc from the same from-scratch style oracle as Corpus A, but
                // freshly computed here (no shared helper with `ply_schedule`).
                let reach0 = oracle_reach_2(mask, &slots, 0);
                let reach1 = oracle_reach_2(mask, &slots, 1);
                let same_scc = reach0[1] && reach1[0];

                for caller in 0..N2 as FnId {
                    for callee in 0..N2 as FnId {
                        let expected = oracle_decision(
                            caller,
                            callee,
                            &configs[caller as usize],
                            &configs[callee as usize],
                            caller != callee && same_scc,
                        );
                        let actual = may_stub(&graph, caller, callee, &results);
                        if actual != expected {
                            offending.push(format!(
                                "mask {mask:#03x}, configs {configs:?}, caller {caller}, callee {callee}: got {actual:?}, oracle says {expected:?}"
                            ));
                        }
                    }
                }

                if offending.len() >= 5 {
                    break 'outer;
                }
            }
        }
    }

    assert!(
        offending.is_empty(),
        "may_stub() disagreed with the independent oracle (showing up to 5 of {} found):\n{}",
        offending.len(),
        offending.join("\n\n")
    );
}

fn oracle_reach_2(edges_mask: u32, slots: &[(FnId, FnId)], start: FnId) -> [bool; N2] {
    let mut reach = [false; N2];
    reach[start as usize] = true;
    loop {
        let mut changed = false;
        for (i, &(caller, callee)) in slots.iter().enumerate() {
            if edges_mask & (1 << i) != 0 && reach[caller as usize] && !reach[callee as usize] {
                reach[callee as usize] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    reach
}
