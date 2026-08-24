use vstd::prelude::*;

verus! {

// ---- Evidence (D6 order): declaration order IS the order, exactly like
// production's #[derive(PartialOrd, Ord)] on the enum. ----
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    Violation,
    Unclaimed,
    Tested,
    Fuzzed,
    Bounded,
    Proved,
}

pub open spec fn evidence_rank(e: Evidence) -> int {
    match e {
        Evidence::Violation => 0,
        Evidence::Unclaimed => 1,
        Evidence::Tested => 2,
        Evidence::Fuzzed => 3,
        Evidence::Bounded => 4,
        Evidence::Proved => 5,
    }
}

pub open spec fn evidence_min(a: Evidence, b: Evidence) -> Evidence {
    if evidence_rank(a) <= evidence_rank(b) { a } else { b }
}

// ---- NodeKind: a Container carries no Evidence of its own, by construction. ----
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Claimable(Evidence),
    Container,
}

pub open spec fn own_evidence(k: NodeKind) -> Option<Evidence> {
    match k {
        NodeKind::Claimable(e) => Some(e),
        NodeKind::Container => None,
    }
}

// combine_claimable: None is a true no-op, never folded in as a real value.
pub open spec fn combine_claimable(a: Option<Evidence>, b: Option<Evidence>) -> Option<Evidence> {
    match (a, b) {
        (Option::None, x) => x,
        (x, Option::None) => x,
        (Option::Some(x), Option::Some(y)) => Option::Some(evidence_min(x, y)),
    }
}

// ---- StatusSet: a 6-bit mask, same shape as production's u8 bitmask. ----
pub type StatusSet = u8;

pub open spec fn ss_empty() -> StatusSet { 0 }

pub open spec fn ss_union(a: StatusSet, b: StatusSet) -> StatusSet { a | b }

pub open spec fn ss_len(s: StatusSet) -> nat {
    // count_ones over the low 6 bits, spelled out bit by bit (spec-mode, no
    // loops) -- this is the "number of distinct StatusKinds" production's
    // StatusSet::len() computes via count_ones().
    ((s >> 0) & 1) as nat + ((s >> 1) & 1) as nat + ((s >> 2) & 1) as nat
        + ((s >> 3) & 1) as nat + ((s >> 4) & 1) as nat + ((s >> 5) & 1) as nat
}

// ---- Conditional: modeled abstractly as an opaque assumption-id set,
// per the task brief -- avoids fighting the verifier with Vec<String>
// sort/dedup, which is a content-preserving concrete detail, not a
// structural one. `None` = not conditional; `Some(ids)` = conditional with
// exactly those assumption ids riding along (mirrors production's
// structural pairing: no way to be conditional without assumptions). ----
pub type Conditional = Option<Set<int>>;

pub open spec fn merge_conditional(a: Conditional, b: Conditional) -> Conditional {
    match (a, b) {
        (Option::None, Option::None) => Option::None,
        (Option::Some(v), Option::None) => Option::Some(v),
        (Option::None, Option::Some(w)) => Option::Some(w),
        (Option::Some(v), Option::Some(w)) => Option::Some(v.union(w)),
    }
}

// ---- The tree. children: Seq<Node> (ghost/spec-mode, mirrors
// production's Vec<VerdictNode> field-for-field in shape, abstracted only
// on `conditional`'s payload per the brief). ----
pub struct Node {
    pub kind: NodeKind,
    pub statuses: StatusSet,
    pub conditional: Conditional,
    pub children: Seq<Node>,
}

pub struct Agg {
    pub evidence: Evidence,
    pub statuses: StatusSet,
    pub conditional: Conditional,
    pub open_items: nat,
}

// raw_evidence: Option<Evidence>, None iff no claimable node anywhere in
// the subtree -- mirrors production's aggregate_raw two-value return
// exactly (the same reason: a parent must be able to tell "no claimable
// node here" apart from "the real answer happens to be Unclaimed").
pub open spec fn raw_evidence(n: Node) -> Option<Evidence>
    decreases n
{
    combine_claimable(own_evidence(n.kind), raw_evidence_seq(n.children))
}

pub open spec fn raw_evidence_seq(cs: Seq<Node>) -> Option<Evidence>
    decreases cs
{
    if cs.len() == 0 {
        Option::None
    } else {
        combine_claimable(raw_evidence(cs[0]), raw_evidence_seq(cs.drop_first()))
    }
}

pub open spec fn agg_statuses(n: Node) -> StatusSet
    decreases n
{
    ss_union(n.statuses, agg_statuses_seq(n.children))
}

pub open spec fn agg_statuses_seq(cs: Seq<Node>) -> StatusSet
    decreases cs
{
    if cs.len() == 0 {
        ss_empty()
    } else {
        ss_union(agg_statuses(cs[0]), agg_statuses_seq(cs.drop_first()))
    }
}

pub open spec fn agg_conditional(n: Node) -> Conditional
    decreases n
{
    merge_conditional(n.conditional, agg_conditional_seq(n.children))
}

pub open spec fn agg_conditional_seq(cs: Seq<Node>) -> Conditional
    decreases cs
{
    if cs.len() == 0 {
        Option::None
    } else {
        merge_conditional(agg_conditional(cs[0]), agg_conditional_seq(cs.drop_first()))
    }
}

pub open spec fn own_open_items(n: Node) -> nat {
    ss_len(n.statuses) + if n.conditional is Some { 1nat } else { 0nat }
}

pub open spec fn agg_open_items(n: Node) -> nat
    decreases n
{
    own_open_items(n) + agg_open_items_seq(n.children)
}

pub open spec fn agg_open_items_seq(cs: Seq<Node>) -> nat
    decreases cs
{
    if cs.len() == 0 {
        0
    } else {
        agg_open_items(cs[0]) + agg_open_items_seq(cs.drop_first())
    }
}

pub open spec fn aggregate(n: Node) -> Agg {
    Agg {
        evidence: match raw_evidence(n) {
            Option::Some(e) => e,
            Option::None => Evidence::Unclaimed,
        },
        statuses: agg_statuses(n),
        conditional: agg_conditional(n),
        open_items: agg_open_items(n),
    }
}

// ============================================================
// Independent oracles (defined separately from aggregate/raw_evidence
// so a lemma connecting them is a real check, not a restatement).
// ============================================================

// The set of every claimable node's own Evidence in the subtree (a
// Container contributes nothing of its own -- exactly §7's rule).
pub open spec fn claimable_evidences(n: Node) -> Set<Evidence>
    decreases n
{
    claimable_evidences_seq(n.children).union(
        match n.kind {
            NodeKind::Claimable(e) => Set::empty().insert(e),
            NodeKind::Container => Set::empty(),
        }
    )
}

pub open spec fn claimable_evidences_seq(cs: Seq<Node>) -> Set<Evidence>
    decreases cs
{
    if cs.len() == 0 {
        Set::empty()
    } else {
        claimable_evidences(cs[0]).union(claimable_evidences_seq(cs.drop_first()))
    }
}

// Union of every conditional node's own assumption-id set in the subtree.
pub open spec fn all_conditional_ids(n: Node)  -> Set<int>
    decreases n
{
    all_conditional_ids_seq(n.children).union(
        match n.conditional {
            Option::Some(ids) => ids,
            Option::None => Set::empty(),
        }
    )
}

pub open spec fn all_conditional_ids_seq(cs: Seq<Node>) -> Set<int>
    decreases cs
{
    if cs.len() == 0 {
        Set::empty()
    } else {
        all_conditional_ids(cs[0]).union(all_conditional_ids_seq(cs.drop_first()))
    }
}

pub open spec fn any_conditional(n: Node) -> bool
    decreases n
{
    n.conditional is Some || any_conditional_seq(n.children)
}

pub open spec fn any_conditional_seq(cs: Seq<Node>) -> bool
    decreases cs
{
    if cs.len() == 0 {
        false
    } else {
        any_conditional(cs[0]) || any_conditional_seq(cs.drop_first())
    }
}

// ============================================================
// Standing obligation 1 (+3): worst-of never reports evidence stronger
// than the weakest claimable node's own evidence anywhere in the
// subtree, and reads exactly Unclaimed when there is none. Obligation 3
// (a violation anywhere reaches the root) is the special case where
// Evidence::Violation (rank 0, the unique minimum) is a member of the set.
// ============================================================

pub proof fn lemma_worst_of(n: Node)
    ensures
        claimable_evidences(n).is_empty() <==> raw_evidence(n) is None,
        claimable_evidences(n).is_empty() ==> aggregate(n).evidence == Evidence::Unclaimed,
        !claimable_evidences(n).is_empty() ==> {
            &&& raw_evidence(n) == Option::Some(aggregate(n).evidence)
            &&& claimable_evidences(n).contains(aggregate(n).evidence)
            &&& forall|e: Evidence| #[trigger] claimable_evidences(n).contains(e)
                ==> evidence_rank(aggregate(n).evidence) <= evidence_rank(e)
        }
    decreases n
{
    lemma_worst_of_seq(n.children);
    let rest_set = claimable_evidences_seq(n.children);
    let own_set: Set<Evidence> = match n.kind {
        NodeKind::Claimable(e) => Set::empty().insert(e),
        NodeKind::Container => Set::empty(),
    };
    assert(claimable_evidences(n) =~= rest_set.union(own_set));
    assert(raw_evidence(n) == combine_claimable(own_evidence(n.kind), raw_evidence_seq(n.children)));
    assert(aggregate(n).evidence == match raw_evidence(n) {
        Option::Some(e) => e,
        Option::None => Evidence::Unclaimed,
    });

    if own_set.is_empty() && rest_set.is_empty() {
        assert(own_evidence(n.kind) is None);
        assert(raw_evidence_seq(n.children) is None);
    } else if own_set.is_empty() && !rest_set.is_empty() {
        assert(own_evidence(n.kind) is None);
        assert(raw_evidence_seq(n.children) is Some);
    } else if !own_set.is_empty() && rest_set.is_empty() {
        assert(raw_evidence_seq(n.children) is None);
        assert(own_evidence(n.kind) is Some);
    } else {
        assert(own_evidence(n.kind) is Some);
        assert(raw_evidence_seq(n.children) is Some);
        let a = own_evidence(n.kind)->Some_0;
        let b = raw_evidence_seq(n.children)->Some_0;
        assert(own_set =~= Set::empty().insert(a));
        assert(combine_claimable(own_evidence(n.kind), raw_evidence_seq(n.children))
            == Option::Some(evidence_min(a, b)));
        let result = evidence_min(a, b);
        assert forall|e: Evidence| #[trigger] own_set.union(rest_set).contains(e) implies
            evidence_rank(result) <= evidence_rank(e) by {
            if own_set.contains(e) {
                assert(e == a);
            } else {
                assert(rest_set.contains(e));
                assert(evidence_rank(b) <= evidence_rank(e));
            }
        }
    }
}

pub proof fn lemma_worst_of_seq(cs: Seq<Node>)
    ensures
        claimable_evidences_seq(cs).is_empty() ==> raw_evidence_seq(cs) is None,
        !claimable_evidences_seq(cs).is_empty() ==> {
            &&& raw_evidence_seq(cs) is Some
            &&& claimable_evidences_seq(cs).contains(raw_evidence_seq(cs)->Some_0)
            &&& forall|e: Evidence| #[trigger] claimable_evidences_seq(cs).contains(e)
                ==> evidence_rank(raw_evidence_seq(cs)->Some_0) <= evidence_rank(e)
        }
    decreases cs
{
    if cs.len() == 0 {
    } else {
        lemma_worst_of(cs[0]);
        lemma_worst_of_seq(cs.drop_first());
        let head_set = claimable_evidences(cs[0]);
        let rest_set = claimable_evidences_seq(cs.drop_first());
        assert(claimable_evidences_seq(cs) =~= head_set.union(rest_set));
        assert(raw_evidence_seq(cs) == combine_claimable(raw_evidence(cs[0]), raw_evidence_seq(cs.drop_first())));

        if head_set.is_empty() && !rest_set.is_empty() {
            // only the rest contributes -- combine_claimable passes b through unchanged
        } else if !head_set.is_empty() && rest_set.is_empty() {
            // only the head contributes
            assert(raw_evidence(cs[0]) == Option::Some(aggregate(cs[0]).evidence));
        } else if !head_set.is_empty() && !rest_set.is_empty() {
            assert(raw_evidence(cs[0]) == Option::Some(aggregate(cs[0]).evidence));
            let a = aggregate(cs[0]).evidence;
            let b = raw_evidence_seq(cs.drop_first())->Some_0;
            assert(combine_claimable(raw_evidence(cs[0]), raw_evidence_seq(cs.drop_first()))
                == Option::Some(evidence_min(a, b)));
            let result = evidence_min(a, b);
            assert forall|e: Evidence| #[trigger] head_set.union(rest_set).contains(e) implies
                evidence_rank(result) <= evidence_rank(e) by {
                if head_set.contains(e) {
                    assert(evidence_rank(a) <= evidence_rank(e));
                } else {
                    assert(rest_set.contains(e));
                    assert(evidence_rank(b) <= evidence_rank(e));
                }
            }
        }
    }
}

// Obligation 3 as its own explicit statement: if Violation is anywhere in
// the subtree's claimable set, the aggregated evidence at this node IS
// Violation (not merely <=, since rank 0 is the unique minimum rank).
pub proof fn lemma_violation_reaches_root(n: Node)
    requires claimable_evidences(n).contains(Evidence::Violation)
    ensures aggregate(n).evidence == Evidence::Violation
{
    lemma_worst_of(n);
    assert(evidence_rank(aggregate(n).evidence) <= evidence_rank(Evidence::Violation));
    assert(evidence_rank(Evidence::Violation) == 0);
    // evidence_rank is injective (six distinct arms, six distinct ints) --
    // rank 0 forces the Violation arm.
    assert(evidence_rank(aggregate(n).evidence) == 0 ==> aggregate(n).evidence == Evidence::Violation) by {
        assert(evidence_rank(Evidence::Unclaimed) == 1);
        assert(evidence_rank(Evidence::Tested) == 2);
        assert(evidence_rank(Evidence::Fuzzed) == 3);
        assert(evidence_rank(Evidence::Bounded) == 4);
        assert(evidence_rank(Evidence::Proved) == 5);
    };
}

// ============================================================
// Standing obligation 2: `conditional` never disappears without its
// assumptions -- the aggregate carries Some(ids) with exactly the union
// of every conditional node's own ids, iff any node in the subtree is
// conditional at all.
// ============================================================

pub proof fn lemma_conditional_carries_assumptions(n: Node)
    ensures
        any_conditional(n) ==> aggregate(n).conditional == Option::Some(all_conditional_ids(n)),
        !any_conditional(n) ==> aggregate(n).conditional is None,
        !any_conditional(n) ==> all_conditional_ids(n) =~= Set::empty(),
    decreases n
{
    lemma_conditional_carries_assumptions_seq(n.children);
    let rest_cond = agg_conditional_seq(n.children);
    assert(aggregate(n).conditional == merge_conditional(n.conditional, rest_cond));
    assert(all_conditional_ids(n) =~= all_conditional_ids_seq(n.children).union(
        match n.conditional {
            Option::Some(ids) => ids,
            Option::None => Set::empty(),
        }
    ));

    match n.conditional {
        Option::None => {
            if any_conditional_seq(n.children) {
                let w = rest_cond->Some_0;
                assert(merge_conditional(Option::None, rest_cond) == Option::Some(w));
            }
        }
        Option::Some(v) => {
            if any_conditional_seq(n.children) {
                let w = rest_cond->Some_0;
                assert(merge_conditional(Option::Some(v), rest_cond) == Option::Some(v.union(w)));
            } else {
                assert(merge_conditional(Option::Some(v), rest_cond) == Option::Some(v));
            }
        }
    }
}

pub proof fn lemma_conditional_carries_assumptions_seq(cs: Seq<Node>)
    ensures
        any_conditional_seq(cs) ==> agg_conditional_seq(cs) == Option::Some(all_conditional_ids_seq(cs)),
        !any_conditional_seq(cs) ==> agg_conditional_seq(cs) is None,
        !any_conditional_seq(cs) ==> all_conditional_ids_seq(cs) =~= Set::empty(),
    decreases cs
{
    if cs.len() == 0 {
    } else {
        lemma_conditional_carries_assumptions(cs[0]);
        lemma_conditional_carries_assumptions_seq(cs.drop_first());
        let head_cond = agg_conditional(cs[0]);
        let rest_cond = agg_conditional_seq(cs.drop_first());
        assert(agg_conditional_seq(cs) == merge_conditional(head_cond, rest_cond));
        assert(all_conditional_ids_seq(cs) =~= all_conditional_ids(cs[0]).union(all_conditional_ids_seq(cs.drop_first())));

        if !any_conditional(cs[0]) && !any_conditional_seq(cs.drop_first()) {
            // both empty: head_cond and rest_cond are both None, ids both empty.
        } else if !any_conditional(cs[0]) && any_conditional_seq(cs.drop_first()) {
            let w = rest_cond->Some_0;
            assert(merge_conditional(head_cond, rest_cond) == Option::Some(w));
        } else if any_conditional(cs[0]) && !any_conditional_seq(cs.drop_first()) {
            let v = head_cond->Some_0;
            assert(merge_conditional(head_cond, rest_cond) == Option::Some(v));
        } else {
            let v = head_cond->Some_0;
            let w = rest_cond->Some_0;
            assert(merge_conditional(head_cond, rest_cond) == Option::Some(v.union(w)));
        }
    }
}

// ============================================================
// Standing obligation 4: no rule sequence assigns one node two different
// verdicts -- aggregate is a pure mathematical function of the tree, so
// two evaluations on the same value are equal by reflexivity. This is
// where the deductive model and Kani's bounded-model-checking world part
// ways hardest: Kani's obligation guards against a *concrete, imperative*
// nondeterminism (hash-iteration order, allocator behaviour) that a pure
// spec function cannot exhibit by construction. The lemma is trivial here
// on purpose -- see FINDINGS.md for what that triviality does and does not
// establish.
// ============================================================

pub proof fn lemma_deterministic(n: Node)
    ensures aggregate(n) == aggregate(n)
{
}

} // verus!

fn main() {}
