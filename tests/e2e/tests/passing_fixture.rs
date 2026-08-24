//! A contract that holds earns a clean `bounded(k)` verdict -- not only the
//! falsified case needs exercising.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn passing_fixture_earns_clean_bounded_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("passing");

    let run = run_verify(&cargo_ply, fixture.path(), 90);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "envelope: {}", run.json);
    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "a clean verdict must carry no diagnostics: {}",
        run.json
    );
    assert_eq!(run.exit_code, Some(0), "clean verify must exit 0");

    // No cex test should be generated for a fn with no violation.
    assert!(!fixture.path().join("src/ply_generated_cex.rs").exists());
}
