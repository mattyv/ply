//! docs/reach-measurement-2.md: a contract naming a type nowhere in the
//! checked fn's own signature -- `std::cmp::Ordering`, named only inside the
//! `#[ply::ensures]` text -- used to fail the generated harness to compile
//! (`error[E0433]: cannot find type Ordering in this scope`), because the
//! generated module only ever imported the checked fn's own path and the
//! types its parameters/receiver walk found, never a name the contract
//! text alone refers to. `tests/fixtures/ensuresimport` reproduces this
//! without touching the separate return-type gate at all: `Ordering` here
//! is neither a parameter nor the return type, just a name the
//! postcondition reads.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_contract_naming_a_type_outside_the_signature_compiles_and_catches_a_false_promise() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("ensuresimport");

    let run = run_verify(&cargo_ply, fixture.path(), 90);
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the generated harness must actually compile and run real cases -- a contract naming \
         `Ordering` only in its own text, never in the signature, must not stop this fn from \
         earning real evidence: {}",
        run.json
    );

    // The catching direction is not optional: the same contract, referring
    // to the same outside type, must actually catch a false promise about
    // it. `a >= b` disagrees with `a <= b` on almost every unequal pair, so
    // fuzzing catches it with overwhelming probability well inside 64 cases.
    let original = fixture.read_lib_rs();
    let broken = original.replace(
        "pub fn is_le(a: u32, b: u32) -> bool {\n    a <= b\n}",
        "pub fn is_le(a: u32, b: u32) -> bool {\n    a >= b\n}",
    );
    assert_ne!(
        broken, original,
        "replacement text did not match the fixture source verbatim"
    );
    fixture.write_lib_rs(&broken);

    let run2 = run_verify(&cargo_ply, fixture.path(), 90);
    assert_eq!(
        run2.json["root"]["verdict"], "violation",
        "a>=b disagrees with a<=b on almost every unequal pair -- fuzzing must catch it: {}",
        run2.json
    );
}
