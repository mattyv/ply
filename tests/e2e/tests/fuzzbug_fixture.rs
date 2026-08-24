//! M4 acceptance: a seeded bug shrunk to a minimal counterexample by the
//! `fuzz` check, rendered through the *same* `contract_rt` renderer the
//! Kani (`bounded`) path uses (D7's "two consumers, one renderer") as a red
//! `#[test]` that names the contract -- then passes after the fix.

use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_test, run_verify};

#[test]
fn fuzz_check_shrinks_a_seeded_bug_and_renders_it_as_a_red_test_then_passes_after_fix() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("fuzzbug");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(run.json["root"]["verdict"], "violation", "envelope: {}", run.json);
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "P0502");
    assert_eq!(diag["engine"], "proptest");
    assert_eq!(diag["counterexample"]["inputs"]["x"], "7", "proptest must shrink to the minimal seeded input");
    assert!(diag["counterexample"]["cargo_test"].is_string(), "must carry a cargo_test artifact path");

    let cex_path = fixture.path().join("src/ply_generated_cex.rs");
    assert!(cex_path.is_file(), "expected a generated ply_cex_seeded_bug_* test file");
    assert!(std::fs::read_to_string(&cex_path).unwrap().contains("ply_cex_seeded_bug_"));

    let test_run = run_cargo_test(fixture.path());
    assert!(!test_run.success, "the rendered cex test must fail before the fix:\n{}", test_run.combined_output);
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

    // --- Apply the fix: remove the seeded bug branch. ---
    let fixed = fixture.read_lib_rs().replace("if x == 7 { x + 1 } else { x }", "x");
    assert_ne!(fixed, fixture.read_lib_rs(), "fix must actually change the source");
    fixture.write_lib_rs(&fixed);

    let run2 = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(run2.json["root"]["verdict"], "fuzzed(256)", "envelope: {}", run2.json);
    assert_eq!(run2.json["diagnostics"].as_array().unwrap().len(), 0, "envelope: {}", run2.json);

    // The rendered test's own fixed input (x=7) now satisfies the contract
    // unconditionally, so the *same* test file now passes -- the oracle's
    // other half.
    let test_run2 = run_cargo_test(fixture.path());
    assert!(test_run2.success, "the cex test must pass after the fix:\n{}", test_run2.combined_output);
    assert!(test_run2.combined_output.contains("ply_cex_seeded_bug_"));
}
