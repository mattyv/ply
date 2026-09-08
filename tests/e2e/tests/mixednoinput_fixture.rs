//! The generated direct test tries only boundary values, all rejected by
//! this function's precondition, while fuzzing easily finds 64 admitted
//! values. The warning must describe only the empty generated test; it
//! cannot deny the sibling evidence that appears beside it.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_empty_generated_test_does_not_deny_successful_sibling_evidence() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("mixednoinput");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    let function = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(function["verdict"], "fuzzed(64)", "envelope: {}", run.json);
    assert!(
        function["statuses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|status| status == "inconclusive"),
        "the empty generated test must remain visible: {}",
        run.json
    );

    let warning = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "W0542")
        .unwrap_or_else(|| panic!("no boundary-input warning in {}", run.json));
    let title = warning["title"].as_str().unwrap();
    assert!(title.contains("generated `test` check"), "{title}");
    assert!(!title.contains("was never called"), "{title}");
    assert!(!title.contains("nothing here is proven"), "{title}");
    assert_ne!(
        run.exit_code,
        Some(0),
        "the unresolved test is not a clean run"
    );
}
