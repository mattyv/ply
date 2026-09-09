//! §5.4b's preferred bounded shape, proved end to end (2026-08-25).
//!
//! Vetting 004 s7 claimed exactly this function and got:
//!
//! ```text
//! "code": "V0505", "node_id": "withdrawal::carded_fee_cents",
//! "title": "Ply cannot check `carded_fee_cents`: parameter(s) card_bps:
//!  Unsupported(\"[u32 ; 4]\") use a type neither the bounded (Kani) nor the
//!  fuzz (proptest) codegen builds inputs for. ..."
//! ```
//!
//! -- for the shape §5.4b tells generated harnesses to "reach for first".

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_array_parameter_earns_a_real_bounded_verdict_with_no_unwind_annotation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("arraycard");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "bounded(2)",
        "an array parameter must earn evidence, not `unsupported`: {}",
        run.json
    );
    assert!(
        run.json["diagnostics"].as_array().unwrap().is_empty(),
        "envelope: {}",
        run.json
    );

    assert!(
        !fixture.path().join("src/ply_generated.rs").exists(),
        "the proof ran, but its scratch module survived in the user's crate"
    );
}
