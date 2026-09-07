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

    // The planting has to reach the helper, and the report has to say so --
    // a survivor count that includes the helper under the words "its own
    // body" sends a reader to the one body the survivors are least likely
    // to be in.
    let weak = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "W0502")
        .unwrap_or_else(|| panic!("no weak-spec diagnostic in {}", run.json));
    let title = weak["title"].as_str().unwrap_or("");
    assert!(
        title.contains("doubled_then_capped"),
        "the logic this claim runs lives in `doubled_then_capped`, so that is where the \
         deliberate bugs have to be planted and what the report has to name: {title}"
    );

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        !verdict.contains("spec-strong"),
        "the arithmetic this claim runs is wrong in every case, and `spec-strong` says \
         mutation testing planted bugs and the checks caught them all -- reporting it here \
         means the planting never looked at the code the check runs: {}",
        run.json
    );
}

/// The same shape, with one `vec!` in the wrapper.
///
/// A macro Ply cannot expand makes the reach walk widen: every edit in the
/// crate must re-run the check, because what the macro calls is unknown.
/// Until 2026-09-07 widening also *emptied* the list of bodies the walk had
/// already identified, so the deliberate bugs went back to the wrapper's own
/// lines and the report still said "its own body" -- the very defect the
/// test above pins, reached by another road. Measured before the fix on this
/// fixture: nine planted bugs spanning the helper, down to two in the
/// wrapper alone.
#[test]
fn a_macro_in_the_wrapper_does_not_shrink_the_planting_back_to_the_wrapper() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("helperspec");

    let src = fixture.read_lib_rs();
    let widened = src.replace(
        "    doubled_then_capped(x)\n",
        "    let batch = vec![x];\n    doubled_then_capped(batch[0])\n",
    );
    assert_ne!(src, widened, "the wrapper must have gained a macro");
    // Same broken helper as above: wrong in every case, still under the cap.
    let broken = widened.replace("let doubled = x.saturating_mul(2);", "let doubled = x / 2;");
    assert_ne!(widened, broken, "the helper body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let diags = run.json["diagnostics"].as_array().unwrap();

    let weak = diags
        .iter()
        .find(|d| d["code"] == "W0502")
        .unwrap_or_else(|| panic!("no weak-spec diagnostic in {}", run.json));
    assert!(
        weak["title"]
            .as_str()
            .unwrap_or("")
            .contains("doubled_then_capped"),
        "a macro the walk cannot read says nothing about the helper beside it, so the \
         planting must still reach `doubled_then_capped`: {}",
        weak["title"]
    );

    // And the run has to admit the list was partial, naming what stopped it.
    let note = diags
        .iter()
        .find(|d| d["code"] == "W0530")
        .unwrap_or_else(|| panic!("no partial-scope note in {}", run.json));
    let title = note["title"].as_str().unwrap_or("");
    assert!(
        title.contains("vec!") && title.contains("doubled_then_capped"),
        "the note has to say which bodies were mutated and what Ply could not read: {title}"
    );
}
