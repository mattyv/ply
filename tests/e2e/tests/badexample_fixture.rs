//! Regression e2e for the 2026-08-24 M4 review's D1 (the review's most
//! serious finding): a harness that fails to *compile* was reported as a
//! clean `fuzzed(n)`/`tested` pass with zero diagnostics. A run that did not
//! succeed, did not time out, and named no failing test did not run at all
//! -- §8's rule for that is a tool error (`X0901`) carrying the engine's own
//! output, never a pass (there was no evidence) and never a violation
//! (§5.4c: no violation without a witness).
//!
//! This asserts what a user actually sees: no `fuzzed(64)`/`tested` verdict,
//! a diagnostic that names the build failure and carries concrete fixes, and
//! a non-zero exit.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify, run_verify_with_env};

#[test]
fn a_harness_that_fails_to_compile_is_a_tool_error_not_a_clean_pass() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("badexample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        !verdict.starts_with("fuzzed") && verdict != "tested",
        "a harness that never compiled must not earn evidence, got `{verdict}`: {}",
        run.json
    );
    assert_eq!(verdict, "tool_error", "envelope: {}", run.json);

    // The node also used to carry `evidence: { engine: "proptest", seed,
    // cases: 64 }` -- for a harness that never compiled and ran zero cases
    // (adversarial review of the post-004 fixes, D5). §1 asks a verdict to
    // name the evidence that produced it; this verdict has none, so the
    // honest envelope has no evidence block at all.
    let fn_node = &run.json["root"]["children"][0]["children"][0];
    assert!(
        fn_node["evidence"].is_null(),
        "nothing ran, so there is no run to name -- an `evidence` block here describes a fuzz \
         run that never happened: {fn_node}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.is_empty(),
        "a run that checked nothing must say so: {}",
        run.json
    );
    let build_failures: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["code"] == "X0901")
        .collect();
    assert_eq!(
        build_failures.len(),
        2,
        "both the fuzz and the test check ran zero cases, so both must report it: {}",
        run.json
    );
    for d in &build_failures {
        let title = d["title"].as_str().unwrap();
        assert!(
            title.contains("failed to compile"),
            "the diagnostic must name the cause in the terms a fix needs (§8): {title}"
        );
        assert!(
            title.contains("examples"),
            "the diagnostic must name the most likely cause a user can act on: {title}"
        );
        assert!(
            title.contains("E0308") || title.contains("mismatched types"),
            "the diagnostic must carry the engine's own distinguishing output (§5.4c): {title}"
        );
        assert!(
            !d["fixes"].as_array().unwrap().is_empty(),
            "§8: a non-result diagnostic SHOULD carry concrete fixes: {d}"
        );
        assert!(
            d["counterexample"].is_null(),
            "a tool error has no witness to show: {d}"
        );
    }
    assert_eq!(
        build_failures
            .iter()
            .filter(|d| d["check"] == "fuzz(64)")
            .count(),
        1,
        "one per declared check, each naming its own check: {}",
        run.json
    );
    assert_eq!(
        build_failures
            .iter()
            .filter(|d| d["check"] == "test")
            .count(),
        1,
        "one per declared check, each naming its own check: {}",
        run.json
    );

    // Non-zero, deliberately not pinned to a specific code: §6's exit-code
    // table reserves 2 for a tool error, and `main::exit_code_for` still
    // maps every error-severity diagnostic to 1 (M3-inherited, recorded in
    // TODO.md and docs/m4-review-closure.md rather than changed here).
    assert_ne!(
        run.exit_code,
        Some(0),
        "a run that could not check anything must not exit 0"
    );
}

/// The same fixture, run the way this project's own CI runs everything:
/// `CARGO_TERM_COLOR=always`.
///
/// Ply reads its engines' output, and that reading is line-oriented -- a
/// compiler error is found by a line beginning `error`, and attributed to a
/// function by the `-->` span beneath it. Under forced colour, cargo emits
/// those lines wrapped in escape sequences, so every one of them began with
/// `\x1b` instead. Nothing matched. Ply could not pin the failure to the
/// function that caused it and could not quote the compiler either, so it
/// fell back to "the compiler gave no specific error line" -- a sentence
/// written for a failure genuinely beyond attribution, printed for one that
/// was entirely attributable. A true sentence in the wrong place, which
/// reads exactly like the tool working.
///
/// The test above passes in a plain terminal and passed here for months.
/// This one is the environment the failure actually needed.
#[test]
fn a_build_failure_is_still_attributed_when_the_compiler_is_forced_to_use_colour() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("badexample");

    let run = run_verify_with_env(
        &cargo_ply,
        fixture.path(),
        Some(120),
        &[("CARGO_TERM_COLOR", "always".to_string())],
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let build_failures: Vec<_> = diagnostics
        .iter()
        .filter(|d| d["code"] == "X0901")
        .collect();
    assert!(
        !build_failures.is_empty(),
        "the harness still fails to compile under colour, so it is still a tool error: {}",
        run.json
    );
    for d in &build_failures {
        let title = d["title"].as_str().unwrap();
        assert!(
            !title.contains("gave no specific error line"),
            "colour must not cost Ply the compiler's own message -- that fallback is for a \
             failure nothing could attribute, and this one names a line: {title}"
        );
        assert!(
            title.contains("E0308") || title.contains("mismatched types"),
            "the compiler's distinguishing output has to survive the escape codes: {title}"
        );
    }
}
