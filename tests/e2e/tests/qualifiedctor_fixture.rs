//! A constructor in a **qualified** `impl` block, for a type used as an
//! ordinary parameter.
//!
//! The receiver path learned in 2026-08-27 that `impl super::T`,
//! `impl crate::T` and `impl Alias` name the same type as `impl T` -- found
//! after a mutator written that way was never called and the verdict
//! claimed coverage it did not have. The parameter path kept the older
//! rule, which counted an `impl` block only when its self type was a single
//! bare segment.
//!
//! So a type declared in `lib.rs` with its `impl` block in a submodule --
//! which must write `impl super::T`, there being no other spelling
//! available in that file -- had no constructor as far as Ply was
//! concerned, and Ply said so: "it has no constructor Ply can call", about
//! a type with a public `new`. Not a missing feature but a false sentence,
//! which is the worse of the two.
//!
//! Both halves are pinned here: the claim is checked at all, and the check
//! is real rather than vacuous -- weakening what the constructor guarantees
//! has to turn the verdict red, or Ply is not actually calling it.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_constructor_in_a_qualified_impl_block_is_found_and_actually_called() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("qualifiedctor");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the constructor is reachable, so the claim must be checked: {}",
        run.json
    );
    assert_eq!(run.json["diagnostics"].as_array().unwrap().len(), 0);

    // The half that matters. A green run here would be worthless if Ply
    // were not really constructing the value: break what the constructor
    // guarantees and the promise resting on it has to fail.
    let ops = fixture.path().join("src/ops.rs");
    let ops_src = std::fs::read_to_string(&ops).unwrap();
    let weakened = ops_src.replace("per_second: per_second.max(1),", "per_second,");
    assert_ne!(
        weakened, ops_src,
        "the edit this test rests on did not match anything in src/ops.rs"
    );
    std::fs::write(&ops, weakened).unwrap();

    let run2 = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run2.json["root"]["verdict"], "violation",
        "if weakening the constructor changes nothing, Ply never called it: {}",
        run2.json
    );
}
