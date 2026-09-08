//! Regression for cargo-mutants' no-arrow whole-body description for an
//! implicit-unit function (external review of `9beea6b`).

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn deleting_an_implicit_unit_helper_is_reported_as_a_survivor() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("unitreturnmutate");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let weak = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "W0502")
        .unwrap_or_else(|| panic!("no weak-spec diagnostic in {}", run.json));
    let title = weak["title"].as_str().unwrap_or("");
    assert!(
        title.contains("replace reset with ()"),
        "deleting the unit-return helper survived, but the report omitted that mutant: {title}"
    );
}
