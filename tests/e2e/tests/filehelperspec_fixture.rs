//! `spec-strong` must be measured over the code the check runs, whichever
//! file that code sits in (2026-09-08, external review of `7820a4b`).
//!
//! Its sibling `helperspec_fixture.rs` pins the same rule for a helper in an
//! inline `mod maths { .. }`. That fix named the helper `maths::helper`,
//! which is exactly how `cargo mutants` names an inline one -- and exactly
//! how it does *not* name a helper in `src/maths.rs`, where the module comes
//! from the file path and the reported owner is the bare `helper`. So the
//! ordinary way to write a helper kept the false clean the inline fix
//! closed: nothing was ever planted in it, and the run still reported that
//! every planted bug was caught.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// Break the helper so every answer is wrong while the bound still holds.
/// Only mutation testing can see that, and only if it plants where the logic
/// is -- in a file the claimed function does not live in.
///
/// **This is the outcome a user cares about, not the test that discriminates
/// the fix:** measured on the unfixed selector, this still passed, because
/// the two survivors in the wrapper alone are already enough to withhold
/// strength. The test below it is the one that goes red without the fix.
/// Both are kept -- one pins what a reader sees, the other pins why.
#[test]
fn a_bug_in_a_helper_in_its_own_file_is_not_reported_as_spec_strong() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("filehelperspec");

    let maths = fixture.path().join("src/maths.rs");
    let src = std::fs::read_to_string(&maths).unwrap();
    let broken = src.replace("let doubled = x.saturating_mul(2);", "let doubled = x / 2;");
    assert_ne!(src, broken, "the helper body must have been rewritten");
    std::fs::write(&maths, &broken).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        !verdict.contains("spec-strong"),
        "every answer this claim computes is wrong, and `spec-strong` says deliberate bugs \
         were planted and the checks caught them all -- reporting it here means nothing was \
         ever planted in the file the logic lives in: {}",
        run.json
    );

    let weak = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "W0502")
        .unwrap_or_else(|| panic!("no weak-spec diagnostic in {}", run.json));
    assert!(
        weak["title"]
            .as_str()
            .unwrap_or("")
            .contains("doubled_then_capped"),
        "and the report has to name the body the surviving bugs are in, or it sends the \
         reader to the one place they are least likely to be: {weak}"
    );
}

/// The direct evidence, on the untouched fixture: the deliberate bugs have
/// to actually land in the file the logic lives in.
///
/// This is the assertion that separates the fix from its absence. The
/// promise here is bound-shaped on purpose and cannot catch a wrong answer
/// under the cap, so survivors are expected either way -- what changed is
/// *where they are*. Before the fix the selector matched no function in
/// `src/maths.rs`, so every survivor sat in the two-line wrapper and the
/// helper was never touched at all.
#[test]
fn the_deliberate_bugs_reach_the_file_the_logic_lives_in() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("filehelperspec");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let weak = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "W0502")
        .unwrap_or_else(|| panic!("no weak-spec diagnostic in {}", run.json));
    let title = weak["title"].as_str().unwrap_or("");
    assert!(
        title.contains("src/maths.rs"),
        "not one deliberate bug was planted in the file holding every line of logic this \
         claim runs, so whatever the run says about strength is measured over the wrapper \
         alone: {title}"
    );
}
