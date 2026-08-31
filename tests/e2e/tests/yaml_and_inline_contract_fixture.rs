//! Adversarial review, 2026-08-31: "deleting the surviving warning outright
//! survives the whole suite -- nothing asserts it fires when inline
//! attributes do exist." `W0510` (the diagnostic saying a ply.yaml
//! `requires:`/`ensures:` contract is used only at a boundary, never added
//! to a fn's own checks) is exercised elsewhere only for a fn with *no*
//! inline attribute at all (`yamlonlycontract_fixture`). `envelopecontract`'s
//! `add` carries both halves -- an inline `#[ply::ensures]` and a ply.yaml
//! `ensures:` -- and no existing test asserts that `W0510` still fires for
//! it. Deleting the whole diagnostic outright left every other test green.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn the_yaml_contract_warning_still_fires_beside_a_real_inline_attribute() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("envelopecontract");
    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let w0510 = diagnostics
        .iter()
        .find(|d| d["code"] == "W0510" && d["node_id"] == "envelopecontract::add")
        .unwrap_or_else(|| {
            panic!(
                "expected W0510 to fire for `add`, which declares a ply.yaml contract \
                 alongside its own inline `#[ply::ensures]`: {}",
                run.json
            )
        });
    let title = w0510["title"].as_str().unwrap();
    assert!(
        title.contains("ply.yaml"),
        "must name the ply.yaml contract: {title}"
    );
    assert!(
        title.contains("does not check `add` against it"),
        "must say plainly that the ply.yaml contract itself is not what got checked: {title}"
    );
}
