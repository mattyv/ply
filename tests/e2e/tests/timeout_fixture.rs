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

    assert_eq!(run.json["root"]["verdict"], "timeout", "envelope: {}", run.json);
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "K0601");
    assert!(
        diag["counterexample"].is_null(),
        "a timeout must never carry a counterexample: {}",
        run.json
    );
    assert_ne!(diag["severity"], "error", "a timeout is a warning, never an error-level violation");
    // The wording is EXPECTED to mention "violation" -- honestly, in a
    // negating clause ("never as a violation") -- so the check that
    // matters is on the structured fields (verdict, code, counterexample),
    // asserted above, not on the absence of the word in prose.
    assert_eq!(diag["code"], "K0601", "must use the timeout code, never K0502 (the violation code)");

    // No cex test should ever be generated for a timeout.
    assert!(!fixture.path().join("src/ply_generated_cex.rs").exists());
}
