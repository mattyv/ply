//! A promise about what a method *changes* must be checkable, and must
//! catch the bugs a rule about the whole value cannot see.
//!
//! Round 3 of the A/B vetting planted three bugs in this exact type. Only one
//! broke `available <= capacity`; the other two left it perfectly true, and
//! nothing in Ply could state them. Across two stateful scenarios four of six
//! bugs were that shape, which is the measured gap this closes.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// A refill of nothing that silently tops the bucket back up. It preserves
/// the whole-value rule exactly -- proved so in
/// `tests/spike/verus-component`, where the invariant proof passes at 6
/// verified, 0 errors with this bug present.
#[test]
fn a_refill_of_nothing_that_silently_refills_is_caught() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    let broken = src.replace(
        "        let room = self.capacity - self.available;",
        "        if tokens == 0 {\n            self.available = self.capacity;\n            return;\n        }\n        let room = self.capacity - self.available;",
    );
    assert_ne!(src, broken, "the refill body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`refill` promises to add exactly what was asked for, and a refill of nothing now \
         fills the bucket -- a whole-value rule cannot see this, which is why the promise \
         exists: {}",
        run.json
    );
}

/// A take that succeeds one token short. This one a whole-value rule can
/// reach, and the promise must reach it too -- a new instrument that loses
/// what the old one caught is not an improvement.
#[test]
fn a_take_that_succeeds_one_token_short_is_caught() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    let broken = src.replace(
        "        if self.available >= tokens {",
        "        if self.available + 1 >= tokens {",
    );
    assert_ne!(src, broken, "the sufficiency test must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`try_take` promises it succeeds exactly when there are enough tokens: {}",
        run.json
    );
}

/// The honest half: the untouched fixture earns its evidence rather than
/// reporting a refusal. Without this, both tests above would pass against a
/// tool that refused every claim.
#[test]
fn the_untouched_bucket_earns_evidence_for_both_of_its_mutators() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "a method that changes something has to be checkable, not refused: {}",
        run.json
    );
}
