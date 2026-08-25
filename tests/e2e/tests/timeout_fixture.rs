//! §5.4c's MUST: a timeout is never reported as a violation. This fixture
//! reproduces the scale spike's own confound (an iterator-chain body that
//! times out CBMC even at length 1) and must report status `timeout`,
//! carrying no witness -- never `violation`.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn timeout_is_reported_as_timeout_never_as_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("timeout");

    // A short cap: this fixture's whole point is that it times out
    // regardless of budget (the confound is generic trait-dispatch
    // unwinding, not a slow-but-finite SAT instance), so a short cap keeps
    // the suite fast without weakening the assertion.
    let run = run_verify(&cargo_ply, fixture.path(), 30);

    assert_eq!(
        run.json["root"]["verdict"], "timeout",
        "envelope: {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "K0601");
    assert!(
        diag["counterexample"].is_null(),
        "a timeout must never carry a counterexample: {}",
        run.json
    );
    assert_ne!(
        diag["severity"], "error",
        "a timeout is a warning, never an error-level violation"
    );
    // The wording is EXPECTED to mention "violation" -- honestly, in a
    // negating clause ("never as a violation") -- so the check that
    // matters is on the structured fields (verdict, code, counterexample),
    // asserted above, not on the absence of the word in prose.
    assert_eq!(
        diag["code"], "K0601",
        "must use the timeout code, never K0502 (the violation code)"
    );

    // No cex test should ever be generated for a timeout.
    assert!(!fixture.path().join("src/ply_generated_cex.rs").exists());

    // §1's absence-of-evidence principle, §6's exit table (2026-08-25):
    // this run checked nothing. Until now it exited 0, because `K0601` is a
    // warning and §6's table had no row for "checked nothing" -- which is
    // how vetting 004's 7m13s run of two evidence-free claims came back
    // green in CI.
    assert_eq!(
        run.exit_code,
        Some(1),
        "a run whose only check timed out has no evidence in it, and must not exit 0: {}",
        run.json
    );
}

/// `--fail-on=error` is §6's documented opt-out: it restores the older,
/// looser behaviour for a codebase mid-adoption where absences are expected
/// and tracked elsewhere. It must be an opt-out and nothing else -- if it
/// changed anything besides the pass/fail line, it would be a third mode
/// nobody asked for.
#[test]
fn fail_on_error_is_the_documented_opt_out_from_the_new_default() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("timeout");

    let output = std::process::Command::new(&cargo_ply)
        .args([
            "verify",
            fixture.path().to_str().unwrap(),
            "--json",
            "--engine-timeout",
            "30",
            "--fail-on",
            "error",
        ])
        .output()
        .expect("spawning cargo-ply verify --fail-on error");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["root"]["verdict"], "timeout", "envelope: {json}");
    assert_eq!(
        output.status.code(),
        Some(0),
        "`--fail-on=error` relaxes the default back to error-severity only: {stdout}"
    );
}
