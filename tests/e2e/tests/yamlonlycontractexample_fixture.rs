//! Defect 1 (2026-08-30, regression, "a promise nobody checks is now
//! reported green in total silence"): a fix landed the same day narrowed
//! the warning that discloses a `ply.yaml`-only contract to fire only when
//! the function *also* carries an inline `#[ply::requires]`/
//! `#[ply::ensures]` attribute. That reopened exactly the silence the
//! warning exists to prevent, for a fn that declares `checks: [test]` with
//! an `examples:` entry: the example passes, `test` reports `tested`, and
//! the ply.yaml `ensures` clause -- naming the wrong answer -- is never
//! checked and never mentioned. Zero diagnostics, in total silence.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_yaml_contract_beside_a_passing_example_still_earns_a_diagnostic() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("yamlonlycontractexample");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let on_seven: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["node_id"] == "yamlonlycontractexample::seven")
        .collect();
    assert!(
        !on_seven.is_empty(),
        "the ply.yaml contract on `seven` must never go completely unmentioned, even though \
         the declared `examples:` entry passes on its own: {}",
        run.json
    );
    assert!(
        on_seven
            .iter()
            .any(|d| d["title"].as_str().unwrap().contains("ply.yaml")),
        "at least one diagnostic must name the ply.yaml contract that was never checked: {:#?}",
        on_seven
    );
}
