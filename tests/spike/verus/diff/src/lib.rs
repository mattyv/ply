//! The Verus shadow's **executable** version: a plain-Rust, hand-written
//! transcription of the same rules proved (by structural induction) in
//! `tests/spike/verus/proof/shadow.rs`. This crate has no vstd/Verus
//! dependency at all -- it exists purely so `cargo test` can run it and
//! `tests/differential.rs` can check it against the real `ply-kernel`
//! crate's `aggregate` on a shared corpus of generated trees, which is what
//! licenses the Verus proof to speak for the production code (see
//! tests/spike/verus/FINDINGS.md, "What binds the proof to production").
//!
//! One representational difference from the proof, by design, matching the
//! proof's own documented modeling decision: `conditional` here carries an
//! **id** (`u64`) rather than free-form assumption text, exactly mirroring
//! the proof's `Set<int>` abstraction of `Conditional`. The differential
//! test bridges this back to production's real `Option<Vec<String>>` by
//! generating each production tree's assumption text as `format!("assumption-{id}")`
//! from the very same id, so comparing "the set of ids recoverable from
//! production's text" against "this crate's id set" is an exact content
//! check, not merely a presence check.
//!
//! Every other field matches production `tools/kernel/src/lib.rs` exactly in
//! shape: the six-variant `Evidence` order, the `Claimable`/`Container`
//! split, the `StatusSet` bitmask, and the two-pass `raw_evidence`/`aggregate`
//! structure (an `Option<Evidence>` accumulator that is `None` exactly when
//! no claimable node has been seen yet, never treated as if `Unclaimed` were
//! a foldable placeholder -- the same bug class `tools/kernel`'s own doc
//! comment records `tests/enumeration.rs` having caught once already).

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Evidence {
    Violation,
    Unclaimed,
    Tested,
    Fuzzed,
    Bounded,
    Proved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Claimable(Evidence),
    Container,
}

/// A 6-bit mask, same bit-per-`StatusKind` layout as production's
/// `StatusSet` (declaration order = bit index): bit 0 = Stale, 1 =
/// WeakSpec, 2 = Unsupported, 3 = EngineMissing, 4 = Timeout, 5 =
/// Inconclusive. Kept as a bare `u8` (not re-deriving production's own
/// type) so the differential test can compare the raw bits directly.
pub type StatusSet = u8;

pub const fn ss_empty() -> StatusSet {
    0
}

pub const fn ss_union(a: StatusSet, b: StatusSet) -> StatusSet {
    a | b
}

pub const fn ss_len(s: StatusSet) -> usize {
    (s & 0b0011_1111).count_ones() as usize
}

/// `None` = not conditional; `Some(ids)` = conditional, carrying the
/// abstract assumption-ids this node's own claim rests on (see the module
/// doc comment for how the differential test bridges this back to
/// production's real assumption text).
pub type Conditional = Option<BTreeSet<u64>>;

fn merge_conditional(a: Conditional, b: Conditional) -> Conditional {
    match (a, b) {
        (None, None) => None,
        (Some(v), None) => Some(v),
        (None, Some(w)) => Some(w),
        (Some(mut v), Some(w)) => {
            v.extend(w);
            Some(v)
        }
    }
}

pub struct Node {
    pub kind: NodeKind,
    pub statuses: StatusSet,
    pub conditional: Conditional,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agg {
    pub evidence: Evidence,
    pub statuses: StatusSet,
    pub conditional: Conditional,
    pub open_items: usize,
}

fn own_evidence(k: NodeKind) -> Option<Evidence> {
    match k {
        NodeKind::Claimable(e) => Some(e),
        NodeKind::Container => None,
    }
}

/// `None` is a true no-op here, never folded in as if it were a real
/// evidence value -- see the module doc comment and production's own
/// `combine_claimable` doc comment for the exact bug this guards against.
fn combine_claimable(a: Option<Evidence>, b: Option<Evidence>) -> Option<Evidence> {
    match (a, b) {
        (None, x) => x,
        (x, None) => x,
        (Some(x), Some(y)) => Some(x.min(y)),
    }
}

fn own_open_items(n: &Node) -> usize {
    ss_len(n.statuses) + if n.conditional.is_some() { 1 } else { 0 }
}

/// The recursive core, structured exactly like production's
/// `aggregate_raw`: returns the subtree's raw claimable-only evidence
/// accumulator *alongside* the public `Agg`, so a parent can tell "no
/// claimable descendant at all" apart from "the real answer is Unclaimed".
fn aggregate_raw(n: &Node) -> (Option<Evidence>, Agg) {
    let mut raw = own_evidence(n.kind);
    let mut statuses = n.statuses;
    let mut conditional = n.conditional.clone();
    let mut open_items = own_open_items(n);

    for child in &n.children {
        let (child_raw, child_agg) = aggregate_raw(child);
        raw = combine_claimable(raw, child_raw);
        statuses = ss_union(statuses, child_agg.statuses);
        conditional = merge_conditional(conditional, child_agg.conditional.clone());
        open_items += child_agg.open_items;
    }

    let evidence = raw.unwrap_or(Evidence::Unclaimed);
    (
        raw,
        Agg {
            evidence,
            statuses,
            conditional,
            open_items,
        },
    )
}

pub fn aggregate(n: &Node) -> Agg {
    aggregate_raw(n).1
}

#[cfg(test)]
mod self_tests {
    use super::*;

    #[test]
    fn a_single_claimable_leaf_aggregates_to_its_own_evidence() {
        let n = Node {
            kind: NodeKind::Claimable(Evidence::Proved),
            statuses: ss_empty(),
            conditional: None,
            children: vec![],
        };
        assert_eq!(aggregate(&n).evidence, Evidence::Proved);
    }

    #[test]
    fn an_empty_container_aggregates_to_unclaimed() {
        let n = Node {
            kind: NodeKind::Container,
            statuses: ss_empty(),
            conditional: None,
            children: vec![],
        };
        assert_eq!(aggregate(&n).evidence, Evidence::Unclaimed);
    }
}
