use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_test, run_verify};

#[test]
fn bounded_module_local_enum_needs_no_root_reexport() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("localenum");
    let original = fixture.read_lib_rs();
    let run = run_verify(&bin, fixture.path(), 150);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
    assert!(
        run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["severity"] != "error"),
        "{}",
        run.json
    );
    assert!(!fixture.read_lib_rs().contains("pub use gates"));
    assert!(fixture.read_lib_rs().starts_with(&original));
    // The same source scope in an inline nested module must also resolve.
    let inline = copy_fixture("localenum");
    let gates = std::fs::read_to_string(inline.path().join("src/gates.rs")).unwrap();
    inline.write_lib_rs(&format!("pub mod outer {{ pub mod gates {{ {gates} }} }}"));
    let yaml = inline.path().join("ply.yaml");
    std::fs::write(
        &yaml,
        std::fs::read_to_string(&yaml).unwrap().replace(
            "ply_fixture_localenum::gates",
            "ply_fixture_localenum::outer::gates",
        ),
    )
    .unwrap();
    let run = run_verify(&bin, inline.path(), 150);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
    // A root name collision must not capture the bounded wrapper's return.
    let collision = copy_fixture("localenum");
    collision.write_lib_rs("pub mod gates;\npub enum ToggleState { RootOnly }\n");
    let yaml = collision.path().join("ply.yaml");
    std::fs::write(
        &yaml,
        std::fs::read_to_string(&yaml)
            .unwrap()
            .replace("bounded(2), fuzz(256), test", "bounded(2)"),
    )
    .unwrap();
    let run = run_verify(&bin, collision.path(), 150);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
}

#[test]
fn bounded_private_return_import_alias_is_resolved_from_the_original_module() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("localenum");
    std::fs::write(
        fixture.path().join("src/gates.rs"),
        r#"
use std::cmp::Ordering as Choice;
pub fn decide(enabled: bool, blocked: bool) -> Choice {
    if !enabled { Choice::Less } else if blocked { Choice::Equal } else { Choice::Greater }
}
"#,
    )
    .unwrap();
    let yaml = fixture.path().join("ply.yaml");
    let text = std::fs::read_to_string(&yaml)
        .unwrap()
        .replace("bounded(2), fuzz(256), test", "bounded(2)")
        .replace("ToggleState::Disabled", "Choice::Less")
        .replace("ToggleState::Blocked", "Choice::Equal")
        .replace("ToggleState::Active", "Choice::Greater");
    std::fs::write(yaml, text).unwrap();
    let run = run_verify(&bin, fixture.path(), 150);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
}

#[test]
fn broken_local_enum_decision_has_a_behavioral_bounded_counterexample() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("localenum");
    let source = fixture.path().join("src/gates.rs");
    let correct = std::fs::read_to_string(&source).unwrap();
    std::fs::write(
        &source,
        correct.replace(
            "else { ToggleState::Active }",
            "else { ToggleState::Blocked }",
        ),
    )
    .unwrap();
    let yaml = fixture.path().join("ply.yaml");
    std::fs::write(
        &yaml,
        std::fs::read_to_string(&yaml)
            .unwrap()
            .replace("bounded(2), fuzz(256), test", "bounded(2)"),
    )
    .unwrap();
    let run = run_verify(&bin, fixture.path(), 150);
    assert_eq!(run.exit_code, Some(1), "{}", run.json);
    assert!(
        run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "K0502" && d["engine"] == "kani"),
        "{}",
        run.json
    );
    let replay = run_cargo_test(fixture.path());
    assert!(!replay.success, "{}", replay.combined_output);
    assert!(
        replay.combined_output.contains("Broken promise"),
        "{}",
        replay.combined_output
    );
    std::fs::write(source, correct).unwrap();
    let replay = run_cargo_test(fixture.path());
    assert!(replay.success, "{}", replay.combined_output);
}

#[test]
fn bounded_generic_local_return_imports_its_name_without_type_arguments() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("localenum");
    fixture.write_lib_rs("pub mod gates;\npub enum TypedOutcome { RootOnly }\n");
    std::fs::write(
        fixture.path().join("src/gates.rs"),
        r#"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedOutcome<T> { Disabled, Value { value: T } }
pub fn decide(enabled: bool, blocked: bool) -> TypedOutcome<u8> {
    if enabled && !blocked { TypedOutcome::Value { value: 7 } } else { TypedOutcome::Disabled }
}
"#,
    )
    .unwrap();
    std::fs::write(fixture.path().join("ply.yaml"), r#"ply: 1
components:
  gates:
    anchor: ply_fixture_localenum::gates
    fns:
      decide:
        checks: [bounded(2)]
        ensures:
          - '|result| matches!(result, TypedOutcome::Value { value } if *value == 7) == (enabled && !blocked)'
          - '|result| matches!(result, TypedOutcome::Disabled) == !(enabled && !blocked)'
"#).unwrap();
    let run = run_verify(&bin, fixture.path(), 150);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
}
