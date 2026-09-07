//! Deliberate bugs cannot be planted in a package cargo-mutants was never
//! pointed at, so the report must not say they were.
//!
//! The reach walk follows path dependencies, so a helper one package away is
//! correctly named as code a claim's checks run. `cargo mutants` is invoked
//! with `-p <root package>`, and a `--re <name>` for a function in another
//! package selects nothing there. Until 2026-09-07 the run said the planting
//! covered that helper: measured on this fixture, two mutants planted, both
//! in the wrapper, and the message naming the helper anyway. That is the
//! `helperspec` defect one package out (external review of cb8e3cd).

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_helper_in_another_package_is_named_as_not_planted_in_rather_than_covered() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("crosspkgmutate");

    // Halve instead of doubling: every answer wrong, every answer still
    // under the cap, so only mutation testing could see it -- and only if it
    // reached this package, which it cannot.
    let helper = fixture.path().join("helperpkg/src/lib.rs");
    let src = std::fs::read_to_string(&helper).unwrap();
    let broken = src.replace("let doubled = x.saturating_mul(2);", "let doubled = x / 2;");
    assert_ne!(src, broken, "the helper body must have been rewritten");
    std::fs::write(&helper, &broken).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let diags = run.json["diagnostics"].as_array().unwrap();

    let weak = diags
        .iter()
        .find(|d| d["code"] == "W0502")
        .unwrap_or_else(|| panic!("no weak-spec diagnostic in {}", run.json));
    let title = weak["title"].as_str().unwrap_or("");
    assert!(
        !title.contains("doubled_then_capped"),
        "no bug was planted in `doubled_then_capped` -- it is in another package, and \
         cargo-mutants was pointed at this one -- so naming it as covered is the \
         overstatement this fixture exists to stop: {title}"
    );

    let note = diags
        .iter()
        .find(|d| d["code"] == "W0530")
        .unwrap_or_else(|| panic!("no partial-scope note in {}", run.json));
    let note_title = note["title"].as_str().unwrap_or("");
    assert!(
        note_title.contains("doubled_then_capped") && note_title.contains("helperpkg"),
        "the note has to name the function that went unplanted and the package it lives \
         in, or a reader cannot tell what the clean result leaves out: {note_title}"
    );
}
