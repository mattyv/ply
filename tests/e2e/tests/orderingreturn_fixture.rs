//! docs/reach-measurement-2.md / The-Ply-Spec.md §5.4b (measured
//! 2026-09-01): a function whose *return* type Ply's codegen never itself
//! constructs -- `std::cmp::Ordering`, produced only by the real call --
//! used to be refused outright before either engine ever ran, even though
//! neither engine's codegen ever names or constructs a return type.
//! `tests/fixtures/orderingreturn` is the measured reproduction, run for
//! real against both engines: a clean run earns genuine evidence on each,
//! and a broken promise about the same type is caught on each.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_function_returning_an_unmodelled_type_earns_real_evidence_on_both_engines() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("orderingreturn");

    let run = run_verify(&cargo_ply, fixture.path(), 180);

    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "the contract holds for its whole domain -- no diagnostic should appear: {}",
        run.json
    );
    assert_eq!(
        run.json["root"]["verdict"], "bounded(2)",
        "both `fuzz(64)` and `bounded(2)` are declared and both must pass -- `bounded` is the \
         stronger evidence and must be what the worst-of-nothing-failed verdict reports: {}",
        run.json
    );

    // The catching direction is not optional: the same contract, over the
    // same unmodelled return type, must actually catch a false promise
    // about it -- on both engines, since both were newly unblocked here.
    let original = fixture.read_lib_rs();
    let broken = original.replace(
        "#[ply::ensures(|result| *result != Ordering::Greater || a > b)]",
        "#[ply::ensures(|result| !result.is_lt())]",
    );
    assert_ne!(
        broken, original,
        "replacement text did not match the fixture source verbatim"
    );
    fixture.write_lib_rs(&broken);

    let run2 = run_verify(&cargo_ply, fixture.path(), 180);
    assert_eq!(
        run2.json["root"]["verdict"], "violation",
        "`compare(0, 1)` returns `Less`, so `!result.is_lt()` is false -- both engines must \
         catch this: {}",
        run2.json
    );

    let diagnostics = run2.json["diagnostics"].as_array().unwrap();
    let engines: std::collections::BTreeSet<&str> = diagnostics
        .iter()
        .map(|d| d["engine"].as_str().unwrap())
        .collect();
    assert!(
        engines.contains("proptest"),
        "the fuzz engine must independently catch this: {}",
        run2.json
    );
    assert!(
        engines.contains("kani"),
        "the bounded engine must independently catch this too -- it was blocked by the same \
         gate: {}",
        run2.json
    );
    for diag in diagnostics {
        let counterexample = &diag["counterexample"];
        assert!(
            !counterexample.is_null(),
            "a violation must carry a real witness, from both engines: {}",
            run2.json
        );
    }
}
