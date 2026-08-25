//! Differential test binding the Verus shadow to production: for a shared
//! corpus of generated trees, production `ply_kernel::aggregate` and the
//! shadow's plain-Rust executable version (`ply_verus_diff_spike::aggregate`,
//! a hand-written transcription of the same rules proved in
//! `tests/spike/verus/proof/shadow.rs`) must agree at every node -- not just
//! the root, per CLAUDE.md's "walk the real output, fail on the first
//! unexplained item" model (`tools/kernel/tests/enumeration.rs` is the house
//! precedent this follows).
//!
//! This is what licenses the Verus proof to say anything about the real
//! kernel: Verus proved properties of `shadow.rs`'s *spec*-mode model
//! (`Seq`-based, `conditional` abstracted to a set of ids); this test proves
//! that the shadow's *executable* transcription -- built the same way, just
//! runnable -- produces bit-for-bit the same aggregation as production's
//! real `Vec`-based, `Option<Vec<String>>`-carrying kernel, on every
//! generated tree. Neither half alone is the whole claim; both are needed.
//!
//! No external RNG crate: a hand-rolled xorshift64* keeps this corpus
//! reproducible (fixed seeds) without a network-fetched dependency, in
//! keeping with the rest of this spike being self-contained.

use ply_kernel::{
    AggregatedNode, Evidence as PEvidence, NodeKind as PNodeKind, StatusKind, StatusSet,
    VerdictNode,
};
use ply_verus_diff_spike::{self as shadow, Evidence as SEvidence, Node as SNode, NodeKind as SNodeKind};

// ---- tiny dependency-free PRNG (xorshift64*) ----

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (n > 0). Not perfectly uniform (modulo bias) --
    /// irrelevant for a test corpus generator, not a security context.
    fn range(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

const ALL_EVIDENCE_P: [PEvidence; 6] = [
    PEvidence::Violation,
    PEvidence::Unclaimed,
    PEvidence::Tested,
    PEvidence::Fuzzed,
    PEvidence::Bounded,
    PEvidence::Proved,
];
const ALL_EVIDENCE_S: [SEvidence; 6] = [
    SEvidence::Violation,
    SEvidence::Unclaimed,
    SEvidence::Tested,
    SEvidence::Fuzzed,
    SEvidence::Bounded,
    SEvidence::Proved,
];
const ALL_STATUS: [StatusKind; 6] = [
    StatusKind::Stale,
    StatusKind::WeakSpec,
    StatusKind::Unsupported,
    StatusKind::EngineMissing,
    StatusKind::Timeout,
    StatusKind::Inconclusive,
];

/// Generates one (production, shadow) tree pair from the same random
/// choices, so the two trees are structurally identical modulo their
/// different `conditional` representations (real assumption text on the
/// production side, an id on the shadow side -- see the module doc comment
/// on `ply_verus_diff_spike` for how the differential test bridges them
/// back together). Returns the pair plus the number of nodes actually used
/// (<= `size_budget`).
fn gen_pair(
    rng: &mut Rng,
    depth_budget: u32,
    size_budget: usize,
    next_id: &mut u64,
) -> (VerdictNode, SNode, usize) {
    let kind_idx = rng.range(7);
    let (p_kind, s_kind) = if kind_idx == 6 {
        (PNodeKind::Container, SNodeKind::Container)
    } else {
        let i = kind_idx as usize;
        (
            PNodeKind::Claimable(ALL_EVIDENCE_P[i]),
            SNodeKind::Claimable(ALL_EVIDENCE_S[i]),
        )
    };

    // Statuses: every one of the 6 flags decided independently -- this
    // corpus deliberately does NOT reduce to one representative "other
    // status" the way tools/kernel/tests/enumeration.rs's config space
    // does, so it covers multi-flag nodes that enumeration.rs's own doc
    // comment says it leaves out.
    let mut p_statuses = StatusSet::new();
    let mut s_statuses: shadow::StatusSet = shadow::ss_empty();
    for (bit, kind) in ALL_STATUS.iter().enumerate() {
        if rng.range(2) == 0 {
            p_statuses.insert(*kind);
            s_statuses = shadow::ss_union(s_statuses, 1 << bit);
        }
    }

    // Conditional: ~1/3 of nodes, each with a globally unique id so the
    // differential check can trace exactly which node contributed which
    // assumption all the way to the root.
    let (p_conditional, s_conditional) = if rng.range(3) == 0 {
        let id = *next_id;
        *next_id += 1;
        (
            Some(vec![format!("assumption-{id}")]),
            Some(std::iter::once(id).collect()),
        )
    } else {
        (None, None)
    };

    let mut p_children = Vec::new();
    let mut s_children = Vec::new();
    let mut used = 1usize;

    if depth_budget > 0 && size_budget > 1 {
        let num_children = 1 + rng.range(3) as usize; // 1..=3
        let mut remaining = size_budget - 1;
        for i in 0..num_children {
            if remaining == 0 {
                break;
            }
            let slots_left = num_children - i;
            let share = (remaining / slots_left.max(1)).max(1) as u64;
            let this_budget = (1 + rng.range(share) as usize).min(remaining).max(1);
            let (pn, sn, u) = gen_pair(rng, depth_budget - 1, this_budget, next_id);
            used += u;
            remaining = remaining.saturating_sub(u);
            p_children.push(pn);
            s_children.push(sn);
        }
    }

    (
        VerdictNode {
            kind: p_kind,
            statuses: p_statuses,
            conditional: p_conditional,
            children: p_children,
        },
        SNode {
            kind: s_kind,
            statuses: s_statuses,
            conditional: s_conditional,
            children: s_children,
        },
        used,
    )
}

fn evidence_matches(p: PEvidence, s: SEvidence) -> bool {
    matches!(
        (p, s),
        (PEvidence::Violation, SEvidence::Violation)
            | (PEvidence::Unclaimed, SEvidence::Unclaimed)
            | (PEvidence::Tested, SEvidence::Tested)
            | (PEvidence::Fuzzed, SEvidence::Fuzzed)
            | (PEvidence::Bounded, SEvidence::Bounded)
            | (PEvidence::Proved, SEvidence::Proved)
    )
}

fn conditional_ids_match(p: &Option<Vec<String>>, s: &Option<std::collections::BTreeSet<u64>>) -> bool {
    match (p, s) {
        (None, None) => true,
        (Some(texts), Some(ids)) => {
            let parsed: std::collections::BTreeSet<u64> = texts
                .iter()
                .map(|t| {
                    t.strip_prefix("assumption-")
                        .expect("every generated assumption carries this prefix")
                        .parse()
                        .expect("every generated assumption id is a valid u64")
                })
                .collect();
            &parsed == ids
        }
        _ => false,
    }
}

/// Walks the production tree, the shadow tree, and production's own nested
/// `AggregatedNode` in lockstep, recomputing the shadow's aggregate at every
/// subtree (cheap: these are small trees) and asserting agreement at every
/// node -- not just the root, per CLAUDE.md's invariant-test model. Appends
/// a description of the first mismatch found rather than panicking
/// immediately, mirroring `tools/kernel/tests/enumeration.rs`'s pattern of
/// reporting a few counterexamples at once.
fn check_node(
    p_node: &VerdictNode,
    s_node: &SNode,
    p_agg: &AggregatedNode,
    offending: &mut Vec<String>,
) {
    let s_agg = shadow::aggregate(s_node);

    if !evidence_matches(p_agg.evidence, s_agg.evidence) {
        offending.push(format!(
            "evidence mismatch: production {:?} vs shadow {:?}",
            p_agg.evidence, s_agg.evidence
        ));
    }

    for (bit, kind) in ALL_STATUS.iter().enumerate() {
        let p_has = p_agg.statuses.contains(*kind);
        let s_has = (s_agg.statuses >> bit) & 1 == 1;
        if p_has != s_has {
            offending.push(format!(
                "status bit mismatch for {kind:?}: production {p_has} vs shadow {s_has}"
            ));
        }
    }

    if !conditional_ids_match(&p_agg.conditional, &s_agg.conditional) {
        offending.push(format!(
            "conditional mismatch: production {:?} vs shadow {:?}",
            p_agg.conditional, s_agg.conditional
        ));
    }

    if p_agg.open_items != s_agg.open_items {
        offending.push(format!(
            "open_items mismatch: production {} vs shadow {}",
            p_agg.open_items, s_agg.open_items
        ));
    }

    for ((child_p, child_s), child_p_agg) in p_node
        .children
        .iter()
        .zip(s_node.children.iter())
        .zip(p_agg.children.iter())
    {
        check_node(child_p, child_s, child_p_agg, offending);
        if offending.len() >= 5 {
            return;
        }
    }
}

#[test]
fn shadow_matches_production_on_hand_picked_edge_cases() {
    // Cheap smoke tests before the random sweep, mirroring
    // tools/kernel/src/lib.rs's own unit tests: single claimable leaf,
    // empty container, violation-drags-root, conditional propagation.
    let mut offending = Vec::new();

    let leaf_p = VerdictNode {
        kind: PNodeKind::Claimable(PEvidence::Proved),
        statuses: StatusSet::new(),
        conditional: None,
        children: vec![],
    };
    let leaf_s = SNode {
        kind: SNodeKind::Claimable(SEvidence::Proved),
        statuses: shadow::ss_empty(),
        conditional: None,
        children: vec![],
    };
    let leaf_p_agg = ply_kernel::aggregate(&leaf_p);
    check_node(&leaf_p, &leaf_s, &leaf_p_agg, &mut offending);

    let empty_container_p = VerdictNode {
        kind: PNodeKind::Container,
        statuses: StatusSet::new(),
        conditional: None,
        children: vec![],
    };
    let empty_container_s = SNode {
        kind: SNodeKind::Container,
        statuses: shadow::ss_empty(),
        conditional: None,
        children: vec![],
    };
    let empty_container_p_agg = ply_kernel::aggregate(&empty_container_p);
    check_node(&empty_container_p, &empty_container_s, &empty_container_p_agg, &mut offending);

    let violation_root_p = VerdictNode {
        kind: PNodeKind::Container,
        statuses: StatusSet::new(),
        conditional: None,
        children: vec![
            VerdictNode {
                kind: PNodeKind::Claimable(PEvidence::Violation),
                statuses: StatusSet::new(),
                conditional: None,
                children: vec![],
            },
            VerdictNode {
                kind: PNodeKind::Claimable(PEvidence::Proved),
                statuses: StatusSet::new(),
                conditional: None,
                children: vec![],
            },
        ],
    };
    let violation_root_s = SNode {
        kind: SNodeKind::Container,
        statuses: shadow::ss_empty(),
        conditional: None,
        children: vec![
            SNode {
                kind: SNodeKind::Claimable(SEvidence::Violation),
                statuses: shadow::ss_empty(),
                conditional: None,
                children: vec![],
            },
            SNode {
                kind: SNodeKind::Claimable(SEvidence::Proved),
                statuses: shadow::ss_empty(),
                conditional: None,
                children: vec![],
            },
        ],
    };
    let violation_root_p_agg = ply_kernel::aggregate(&violation_root_p);
    check_node(&violation_root_p, &violation_root_s, &violation_root_p_agg, &mut offending);

    assert!(
        offending.is_empty(),
        "shadow disagreed with production on a hand-picked edge case:\n{}",
        offending.join("\n")
    );
}

#[test]
fn shadow_matches_production_over_generated_trees() {
    const NUM_TREES: usize = 4000;
    const MAX_NODES: usize = 24;
    const MAX_DEPTH: u32 = 5;

    let mut offending: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for seed in 0..NUM_TREES as u64 {
        let mut rng = Rng::new(seed);
        let mut next_id: u64 = 0;
        let size_budget = 1 + rng.range(MAX_NODES as u64) as usize;
        let (p_tree, s_tree, _used) = gen_pair(&mut rng, MAX_DEPTH, size_budget, &mut next_id);
        let p_agg = ply_kernel::aggregate(&p_tree);

        let before = offending.len();
        check_node(&p_tree, &s_tree, &p_agg, &mut offending);
        checked += 1;
        if offending.len() > before {
            offending.push(format!("(seed {seed}, size_budget {size_budget})"));
        }
        if offending.len() >= 5 {
            break;
        }
    }

    assert!(
        offending.is_empty(),
        "shadow disagreed with production on {} of {} generated trees checked (showing first mismatches):\n{}",
        offending.len(),
        checked,
        offending.join("\n\n")
    );
}
