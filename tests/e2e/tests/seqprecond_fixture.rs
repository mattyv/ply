//! Adversarial review, docs/review-caveats.md N2, second half: the earlier
//! calls Ply's own bounded sequence makes before the checked call must
//! honour that method's own precondition too, not only the final call's
//! arguments. `Thing::set` can never break its own postcondition
//! (`result <= 10`) on a call that respects its own precondition
//! (`k <= 10`) -- a violation here can only mean Ply called `set` with an
//! argument its own contract forbids, as an earlier step in the sequence.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn earlier_sequence_calls_never_violate_the_checked_methods_own_precondition() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("seqprecond");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        verdict != "violation",
        "`Thing::set` cannot break `result <= 10` on any call that honours its own \
         `k <= 10` precondition -- a violation here means an earlier step in Ply's own \
         generated sequence called `set` out of contract: {}",
        run.json
    );
    assert!(
        verdict.starts_with("fuzzed"),
        "a promise that holds on every in-contract call is a real pass: {}",
        run.json
    );
}
