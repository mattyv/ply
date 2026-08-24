//! Regression e2e for the 2026-08-24 M4 review's D6: §5.4c's MUST -- "MUST
//! NOT emit a `violation` without a witness". When the fuzz check fails but
//! Ply cannot recover the failing input (here: the function panics before
//! the postcondition is ever evaluated, so the harness never prints its
//! marker), the adapter built the right diagnostic -- an `X0901` tool error
//! carrying no counterexample -- and then labelled the node `violation`
//! regardless.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_failure_with_no_recoverable_witness_is_a_tool_error_not_a_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("panicbug");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "X0901", "envelope: {}", run.json);
    assert!(
        diag["counterexample"].is_null(),
        "there is no witness here: {diag}"
    );

    assert_ne!(
        run.json["root"]["verdict"], "violation",
        "§5.4c MUST: a `violation` without a witness is exactly the report this project exists to \
         prevent -- there is no failing input in this envelope: {}",
        run.json
    );
    assert_eq!(
        run.json["root"]["verdict"], "tool_error",
        "envelope: {}",
        run.json
    );

    let title = diag["title"].as_str().unwrap();
    assert!(
        title.contains("could not find the line its own generated harness prints"),
        "the diagnostic must name the cause in the terms a fix needs (§8): {title}"
    );
    assert!(
        title.contains("panicking"),
        "the diagnostic must name the likeliest cause a user can act on: {title}"
    );
    assert!(
        !diag["fixes"].as_array().unwrap().is_empty(),
        "§8: a non-result diagnostic SHOULD carry fixes: {diag}"
    );
    // Non-zero; see badexample_fixture.rs on why the exact code is not
    // pinned (§6 reserves 2 for a tool error; the CLI still returns 1).
    assert_ne!(run.exit_code, Some(0));
}
