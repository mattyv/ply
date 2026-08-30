//! Regression e2e for the external review of 2026-08-30: a timeout hid a
//! real, already-observed failure.
//!
//! `test` and `fuzz` share one cargo subprocess and one deadline. Both
//! classifiers checked `run.timed_out` before their own failure lists, so a
//! slow sibling still running at the kill relabelled a violation that had
//! already been reported — and the diagnostic then said, in as many words,
//! "reported as `timeout`, never as a violation". The failing test name was
//! sitting in the captured output; the classifier order threw it away.
//!
//! Why this is worse than a mislabel: the suggested fix for a timeout is to
//! raise the budget, and raising it happens to work — the example fails
//! either way once fuzzing gets to finish. So the bug hid itself behind a
//! remedy that appeared to succeed, and the developer never learns their
//! example is wrong.
//!
//! The exit code stayed non-zero throughout, so a gate keyed to exit alone
//! held. This is about the evidence being false, not about a green light.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_failure_that_was_actually_observed_beats_a_timeout_on_a_sibling_check() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("slowfailingexample");

    // Short enough that fuzzing 32 cases of a 400ms function cannot finish,
    // long enough that the example has failed and been reported first.
    let run = run_verify(&cargo_ply, fixture.path(), 12);

    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let verdict = fn_node["verdict"].as_str().unwrap_or("");
    assert_eq!(
        verdict, "violation",
        "the worked example failed and was reported before the deadline. Calling this a \
         timeout discards a fact Ply already had, and sends the reader to raise a budget \
         instead of to a wrong assertion: {}",
        run.json
    );

    // Scoped to the check that actually observed the failure. `fuzz` really
    // did run out of clock with no failure of its own, so its timeout
    // diagnostic is honest and must stay -- the defect was never "a timeout
    // was mentioned", it was the check holding a reported failure denying
    // that a violation existed.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let test_denies_it = diagnostics.iter().any(|d| {
        d["check"].as_str() == Some("test")
            && d["title"]
                .as_str()
                .is_some_and(|t| t.contains("never as a violation"))
    });
    assert!(
        !test_denies_it,
        "the `test` check observed its example fail, and its diagnostic still says there is \
         no violation. That sentence is the defect, not the label: {}",
        run.json
    );
    let names_the_failure = diagnostics.iter().any(|d| {
        d["check"].as_str() == Some("test")
            && d["title"]
                .as_str()
                .is_some_and(|t| t.contains("ply_example_slow_identity"))
    });
    assert!(
        names_the_failure,
        "the surviving diagnostic must name the example that failed, or the reader is told a \
         violation exists without being told which assertion produced it: {}",
        run.json
    );

    assert_ne!(
        run.exit_code,
        Some(0),
        "a violation must not exit 0: {}",
        run.json
    );
}
