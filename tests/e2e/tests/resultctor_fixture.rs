//! A violation reported on correct code (docs/review-structs-enums.md
//! finding 2, 2026-08-28): `Range::new` is the ordinary fallible-constructor
//! shape (`Result<Self, E>`, rejecting `lo > hi`). Ply used to recognise
//! only a bare-`Self`-returning constructor, so `Range` fell through to
//! direct field construction, built the exact state `new` exists to
//! forbid, and reported `well_formed` -- correct code -- as broken.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_result_returning_constructor_is_recognised_and_used() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultctor");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        verdict != "violation",
        "`well_formed` is true of every `Range` the constructor can build -- a violation here \
         can only mean Ply built one `Range::new` would have rejected: {}",
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
            .all(|d| d["node_id"] != "resultctor::well_formed" || d["severity"] != "error"),
        "no error-severity diagnostic belongs on correct code: {}",
        run.json
    );
    // The public-fields-assumption disclosure (W0522) must NOT fire here --
    // its presence would mean Ply fell through to rule 2 (direct field
    // construction) instead of using `Range::new`.
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "resultctor::well_formed" || d["code"] != "W0522"),
        "a `W0522` public-fields disclosure here would mean the constructor was not used: {}",
        run.json
    );
}
