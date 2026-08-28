//! The eleventh false pass, pinned as a permanent fixture (task 2026-08-27,
//! docs/review-strings-receivers.md finding 1): the exact reproduction from
//! the review, `checks: [test]` on a receiver method (`Calc::value`) whose
//! promise (`*result >= 1000`) is false on every input -- `v` is clamped to
//! at most 10 by the constructor. `test` generates no worked-examples test
//! (no `examples:` entries) and no direct-contract case (receiver methods
//! leave that to the sampling tier), so this function's harness module used
//! to contribute nothing at all: `cargo test`'s own filter then matched zero
//! tests, the process still exited 0, and "no failing test" was read as
//! `tested`/held -- a green pass over zero tests. This must never happen
//! again: the structural fix is that a passing result now requires positive
//! evidence that at least one case actually executed, checked once, in the
//! one place both `fuzz` and `test` share.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_false_promise_on_a_receiver_method_checked_only_by_test_is_never_a_clean_pass() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("falsereceivertest");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        verdict != "tested" && !verdict.starts_with("fuzzed") && verdict != "proved",
        "a `test` check that ran zero cases against a false promise must never read as a pass, \
         got `{verdict}`: {}",
        run.json
    );
    assert_eq!(
        verdict, "tool_error",
        "zero cases ran, so this is an honest tool error, never a violation (there is no witness) \
         and never a pass (there is no evidence): {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["node_id"] == "falsereceivertest::Calc::value" && d["code"] == "X0901")
        .unwrap_or_else(|| panic!("no X0901 tool-error diagnostic: {}", run.json));

    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("ran zero cases"),
        "the diagnostic must say plainly that nothing executed: {title}"
    );
    // The wording must not blame a build failure that never happened --
    // this harness crate compiles cleanly; there is simply nothing for
    // `test` alone to run against a receiver method with no `examples:`.
    assert!(
        !title.contains("failed to compile"),
        "the harness built fine here -- claiming a compile failure that never happened is exactly \
         the kind of wrong-cause diagnostic this task also had to fix elsewhere (finding 5): {title}"
    );
    assert!(
        title.contains("needs a value to call it on") || title.contains("receiver"),
        "the diagnostic must name the real reason: a receiver method has nothing for `test` alone \
         to run against: {title}"
    );
    assert!(
        title.contains("fuzz"),
        "the diagnostic must point at the check that actually can check this function: {title}"
    );

    assert!(
        !d["fixes"].as_array().unwrap().is_empty(),
        "a non-result diagnostic should carry a concrete fix: {d}"
    );
    assert!(
        d["counterexample"].is_null(),
        "a tool error has no witness to show: {d}"
    );

    assert_ne!(
        run.exit_code,
        Some(0),
        "a run that checked nothing must not exit clean: {}",
        run.json
    );
}
