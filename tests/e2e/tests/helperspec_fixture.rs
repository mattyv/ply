//! `spec-strong` must be measured over the code the check actually runs.
//!
//! Mutation testing planted deliberate bugs only in the *claimed function's
//! own lines*. Ply's own writing guide tells authors to lift logic into a
//! helper and keep the claimed function thin, so on the shape Ply teaches,
//! the planting touched the one body that does the least -- and reported
//! that nothing survived.
//!
//! Found 2026-09-07 by handing a real task to an agent that followed the
//! guide and then planting a bug in its helper: the wait shrank on every
//! attempt and the run still came back `fuzzed(256)·spec-strong`.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// The claimed function is a shell over `doubled_then_capped`. Break the
/// helper so the answer is wrong while the bound still holds, and the run
/// must not call its own checks strong.
#[test]
fn a_bug_in_the_helper_the_check_runs_is_not_reported_as_spec_strong() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("helperspec");

    let src = fixture.read_lib_rs();
    // Halves instead of doubling: every answer is wrong, and every one is
    // still `<= 100`, so the bound-shaped promise cannot see it. Only
    // mutation testing can, and only if it plants where the logic is.
    let broken = src.replace("let doubled = x.saturating_mul(2);", "let doubled = x / 2;");
    assert_ne!(src, broken, "the helper body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        !verdict.contains("spec-strong"),
        "the arithmetic this claim runs is wrong in every case, and `spec-strong` says \
         mutation testing planted bugs and the checks caught them all -- reporting it here \
         means the planting never looked at the code the check runs: {}",
        run.json
    );
}
