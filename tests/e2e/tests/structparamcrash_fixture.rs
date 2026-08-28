//! A crash in a function whose only parameter is a struct loses its witness
//! (docs/review-structs-enums.md's "Also fix" list, 2026-08-28): `width`
//! panics whenever `end < start`, and Ply used to discard that crash's
//! witness (reporting a tool error) because the shrunk input describes
//! `Window`'s two *fields*, never one value per declared parameter --
//! a count mismatch the receiver path was already taught to carry through
//! in the same window this shape was found in.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_crash_behind_a_struct_parameter_keeps_its_witness() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("structparamcrash");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`width` panics whenever `end < start` -- proptest will find it, and it must be \
         reported as a real violation with a witness, never a lost tool error: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let violation = diagnostics
        .iter()
        .find(|d| d["node_id"] == "structparamcrash::width" && d["severity"] == "error")
        .unwrap_or_else(|| panic!("no violation diagnostic: {}", run.json));
    assert_eq!(
        violation["code"], "W0541",
        "a struct parameter has no Rust-source renderer yet, so this must be the honest \
         witness-only violation, not a fabricated per-field reading: {}",
        run.json
    );
    assert_ne!(
        violation["code"], "X0901",
        "the crash's witness must not be discarded as a tool error: {}",
        run.json
    );
    let raw = violation["counterexample"]["inputs"]["params_raw"]
        .as_str()
        .unwrap_or_else(|| panic!("no `params_raw` witness carried through: {}", run.json));
    assert!(
        !raw.is_empty(),
        "the witness must actually carry the shrunk field values, not an empty string: {}",
        run.json
    );
}
