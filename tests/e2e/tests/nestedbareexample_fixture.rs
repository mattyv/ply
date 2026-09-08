//! A nested function's own bare example supplies the only input admitted
//! by its precondition. The ordinary example is deliberately tautological;
//! the contract is false at that input. If the literal extractor compares
//! only `rate_limiter::wait_millis` with the bare `wait_millis`, every
//! generated boundary case is rejected and the broken promise looks clean.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_nested_functions_bare_example_is_checked_against_its_contract() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nestedbareexample");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "the bare example's tuple is admissible and breaks the declared promise: {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["node_id"] == "nestedbareexample::rate_limiter::wait_millis"
                && diagnostic["code"] == "R0502"
        }),
        "the generated direct contract case must report the violation: {}",
        run.json
    );
}
