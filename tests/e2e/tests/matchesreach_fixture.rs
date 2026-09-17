use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn matches_guard_helpers_have_complete_mutation_scope_and_bounded_evidence() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("matchesreach");
    let run = run_verify(&bin, fixture.path(), 150);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(
        run.json["root"]["verdict"], "bounded(4)·spec-strong",
        "{}",
        run.json
    );
    assert!(
        !run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "W0530"),
        "{}",
        run.json
    );
    let mutants: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.path().join("mutants.out/mutants.json")).unwrap(),
    )
    .unwrap();
    assert!(
        mutants
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["function"]["function_name"] == "permitted"),
        "{mutants}"
    );
}
