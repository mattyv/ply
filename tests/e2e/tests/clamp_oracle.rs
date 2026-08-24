//! §9's cex validity oracle, for real: `cargo ply verify` on the clamp
//! fixture must render a `ply_cex_clamp_*` test that FAILS under plain
//! `cargo test` for the right reason (states the contract, names
//! "postcondition") before the fix, and PASSES -- the same test -- after.

use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_test, run_verify};

#[test]
fn rendered_cex_test_fails_for_noncrashing_ensures_violation_then_passes_after_fix() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("clamp");

    // --- Before the fix: verify must find a violation with a witness. ---
    let run = run_verify(&cargo_ply, fixture.path(), 90);
    assert_eq!(run.json["root"]["verdict"], "violation", "envelope: {}", run.json);
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "K0502");
    assert!(diag["counterexample"]["cargo_test"].is_string(), "must carry a cargo_test artifact path");
    assert!(diag["counterexample"]["kani_witness"].is_string(), "must carry kani_witness (D7 rename), never kani_playback");

    let cex_path = fixture.path().join("src/ply_generated_cex.rs");
    assert!(cex_path.is_file(), "expected a generated ply_cex_clamp_* test file");
    let cex_source = std::fs::read_to_string(&cex_path).unwrap();
    assert!(cex_source.contains("ply_cex_clamp_"), "test name must start with ply_cex_ (housekeeping ownership marker)");

    // Run cargo test in the fixture: must FAIL, and fail for the RIGHT
    // reason -- naming the contract text and the word "postcondition", not
    // just going red for any reason (an overflow panic, say).
    let test_run = run_cargo_test(fixture.path());
    assert!(!test_run.success, "the cex test must fail before the fix:\n{}", test_run.combined_output);
    assert!(
        test_run.combined_output.contains("postcondition"),
        "failure output must name what a postcondition is (newbie bar):\n{}",
        test_run.combined_output
    );
    assert!(
        test_run.combined_output.contains("result == x"),
        "failure output must state the actual contract text:\n{}",
        test_run.combined_output
    );
    assert!(
        !test_run.combined_output.contains("attempt to add with overflow"),
        "must fail on the contract assertion, never on an incidental overflow inside the check:\n{}",
        test_run.combined_output
    );

    // --- Apply the fix: contract -> `result == x.min(100)`. ---
    let fixed = fixture.read_lib_rs().replace("*result == x)", "*result == x.min(100))");
    assert_ne!(fixed, fixture.read_lib_rs(), "fix must actually change the source");
    fixture.write_lib_rs(&fixed);

    let run2 = run_verify(&cargo_ply, fixture.path(), 90);
    assert_eq!(run2.json["root"]["verdict"], "bounded(2)", "envelope: {}", run2.json);
    assert_eq!(run2.json["diagnostics"].as_array().unwrap().len(), 0);

    // The SAME rendered test must now pass (the oracle's other half).
    let test_run2 = run_cargo_test(fixture.path());
    assert!(test_run2.success, "the cex test must pass after the fix:\n{}", test_run2.combined_output);
    assert!(
        test_run2.combined_output.contains("ply_cex_clamp_"),
        "the SAME test name must be the one that now passes, not a different artifact:\n{}",
        test_run2.combined_output
    );
}
