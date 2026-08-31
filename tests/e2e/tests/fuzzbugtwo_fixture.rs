//! Defect 3 (2026-08-30, "a test that reproduces this" is false when two
//! functions fail): `write_generated_test` used to overwrite
//! `ply_generated_cex.rs` wholesale on every call, and `verify` called it
//! once per fn whose promise broke -- so with two broken fns in the same
//! run, the terminal printed "Ply wrote a test that reproduces this" twice,
//! but the file on disk only ever held the *last* fn's test. A user who
//! trusted the first line and ran `cargo test` for that fn got no such
//! test at all.

use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_test, run_verify};

#[test]
fn both_broken_fns_in_one_run_keep_a_surviving_test_each() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("fuzzbugtwo");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let p0502: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["code"] == "P0502")
        .collect();
    assert_eq!(
        p0502.len(),
        2,
        "both fns must be reported broken: {}",
        run.json
    );

    // Every diagnostic that promises a cargo_test path must actually have
    // its own test survive in the generated file -- not just the last
    // one's.
    let cex_path = fixture.path().join("src/ply_generated_cex.rs");
    assert!(cex_path.is_file(), "expected a generated cex test file");
    let cex_source = std::fs::read_to_string(&cex_path).unwrap();
    assert!(
        cex_source.contains("ply_cex_seeded_bug_a_"),
        "seeded_bug_a's test must survive alongside seeded_bug_b's, not be overwritten:\n{cex_source}"
    );
    assert!(
        cex_source.contains("ply_cex_seeded_bug_b_"),
        "seeded_bug_b's test must survive alongside seeded_bug_a's, not be overwritten:\n{cex_source}"
    );

    // Both rendered tests must actually be present and runnable -- not
    // merely mentioned -- and both must fail (the fix has not landed yet).
    let test_run = run_cargo_test(fixture.path());
    assert!(
        !test_run.success,
        "both rendered cex tests must fail before the fix:\n{}",
        test_run.combined_output
    );
    assert!(
        test_run.combined_output.contains("ply_cex_seeded_bug_a_"),
        "seeded_bug_a's test must actually run: {}",
        test_run.combined_output
    );
    assert!(
        test_run.combined_output.contains("ply_cex_seeded_bug_b_"),
        "seeded_bug_b's test must actually run: {}",
        test_run.combined_output
    );
}
