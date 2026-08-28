//! A violation reported on correct code, second shape
//! (docs/review-structs-enums.md finding 2, 2026-08-28): `Window` is
//! declared in one file, its constructor written in another -- an ordinary
//! way to organise a Rust crate. Ply used to search for a constructor only
//! in the type's own declaring file, so this layout was indistinguishable
//! from "no constructor at all", fell through to direct field construction,
//! and reported `well_formed` -- correct code -- as broken.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_constructor_in_a_different_file_from_its_type_is_found() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("splitimplctor");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        verdict != "violation",
        "`well_formed` is true of every `Window` the constructor can build, and the \
         constructor's own precondition (written in a different file) forbids `start > end` -- \
         a violation here can only mean the cross-file constructor was not found: {}",
        run.json
    );
    assert!(
        verdict.starts_with("fuzzed"),
        "correct code, checked honestly, is a real pass: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "splitimplctor::well_formed" || d["severity"] != "error"),
        "no error-severity diagnostic belongs on correct code: {}",
        run.json
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "splitimplctor::well_formed" || d["code"] != "W0522"),
        "a `W0522` public-fields disclosure here would mean the cross-file constructor was not \
         found and Ply fell through to direct field construction instead: {}",
        run.json
    );
}
