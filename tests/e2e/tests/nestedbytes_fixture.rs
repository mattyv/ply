use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_test, run_verify};

#[test]
fn production_nested_borrowed_bytes_earn_bounded_and_fuzz_evidence() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("nestedbytes");
    let run = run_verify(&bin, fixture.path(), 150);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(4)", "{}", run.json);
    assert!(
        run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["code"] == "K0510"),
        "{}",
        run.json
    );
}

#[test]
fn broken_nested_byte_rules_have_reproducible_bounded_counterexamples() {
    let bin = build_cargo_ply();
    for (name, broken) in [
        ("reject_all", "false".to_string()),
        ("accept_all", "true".to_string()),
        ("miss_empty", "if keys.is_empty() { return false; } let mut i=0; while i<keys.len() { let mut j=0; while j<i { if keys[i]==keys[j] { return false; } j+=1; } i+=1; } true".to_string()),
        ("miss_nonadjacent", "if keys.is_empty() { return false; } let mut i=0; while i<keys.len() { if keys[i].is_empty() { return false; } if i>0 && keys[i]==keys[i-1] { return false; } i+=1; } true".to_string()),
    ] {
        let fixture = copy_fixture("nestedbytes");
        let source = fixture.path().join("src/lib.rs");
        let correct = std::fs::read_to_string(&source).unwrap();
        std::fs::write(&source, format!("pub fn valid_key_bytes(keys: &[&[u8]]) -> bool {{ {broken} }}\n")).unwrap();
        let yaml = fixture.path().join("ply.yaml");
        std::fs::write(&yaml, std::fs::read_to_string(&yaml).unwrap().replace("bounded(4), fuzz(256)", "bounded(4)")).unwrap();
        let run = run_verify(&bin, fixture.path(), 150);
        assert_eq!(run.exit_code, Some(1), "{name}: {}", run.json);
        let diag = run.json["diagnostics"].as_array().unwrap().iter()
            .find(|d| d["code"] == "K0502").unwrap_or_else(|| panic!("{name}: {}", run.json));
        assert!(diag["counterexample"]["cargo_test"].is_string(), "{name}: {diag}");
        let replay = run_cargo_test(fixture.path());
        assert!(!replay.success, "{name}: {}", replay.combined_output);
        assert!(replay.combined_output.contains("Broken promise"), "{name}: {}", replay.combined_output);
        // Keep the generated test/module declaration while restoring the real body.
        let with_replay = std::fs::read_to_string(&source).unwrap();
        let body_end = with_replay.find('\n').unwrap();
        std::fs::write(&source, correct + &with_replay[body_end..]).unwrap();
        let replay = run_cargo_test(fixture.path());
        assert!(replay.success, "{name}: {}", replay.combined_output);
    }
}

#[test]
fn broken_nested_byte_fuzz_has_a_native_replay() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("nestedbytes");
    let source = fixture.path().join("src/lib.rs");
    std::fs::write(
        &source,
        "pub fn valid_key_bytes(keys: &[&[u8]]) -> bool { true }\n",
    )
    .unwrap();
    let yaml = fixture.path().join("ply.yaml");
    std::fs::write(
        &yaml,
        std::fs::read_to_string(&yaml)
            .unwrap()
            .replace("bounded(4), fuzz(256)", "fuzz(256)"),
    )
    .unwrap();
    let run = run_verify(&bin, fixture.path(), 60);
    assert_eq!(run.exit_code, Some(1), "{}", run.json);
    assert!(
        run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["counterexample"]["cargo_test"].is_string()),
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
}

#[test]
fn nested_domain_keeps_other_parameters_independent() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("nestedbytes");
    std::fs::write(fixture.path().join("src/lib.rs"),
        "pub fn valid_key_bytes(keys_count: usize, __ply_keys_count: usize, keys: &[&[u8]]) -> bool { true }\n").unwrap();
    std::fs::write(
        fixture.path().join("ply.yaml"),
        r#"ply: 1
components:
  nestedbytes:
    anchor: ply_fixture_nestedbytes
    fns:
      valid_key_bytes:
        checks: [bounded(2)]
        ensures:
          - '|result| *result == (keys_count == keys.len() && __ply_keys_count == keys.len())'
"#,
    )
    .unwrap();
    let run = run_verify(&bin, fixture.path(), 60);
    assert_eq!(run.json["root"]["verdict"], "violation", "{}", run.json);
    assert!(
        run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "K0502"),
        "{}",
        run.json
    );
}

#[test]
fn smallest_bound_and_merged_inline_contracts_are_checked_natively() {
    let bin = build_cargo_ply();
    let zero = copy_fixture("nestedbytes");
    let yaml = zero.path().join("ply.yaml");
    std::fs::write(
        &yaml,
        std::fs::read_to_string(&yaml)
            .unwrap()
            .replace("bounded(4), fuzz(256)", "bounded(1)"),
    )
    .unwrap();
    let run = run_verify(&bin, zero.path(), 60);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(1)", "{}", run.json);

    let fixture = copy_fixture("nestedbytes");
    let source = fixture.path().join("src/lib.rs");
    let original = std::fs::read_to_string(&source).unwrap();
    let inline = r#"#[ply::requires(!keys.is_empty())]
#[ply::ensures(|result| *result == (!keys.is_empty() && keys.iter().all(|key| !key.is_empty()) && (0..keys.len()).all(|i| (0..i).all(|j| keys[i] != keys[j]))))]
"#;
    let source_before = inline.to_string() + &original;
    std::fs::write(&source, &source_before).unwrap();
    let yaml = fixture.path().join("ply.yaml");
    let merged = r#"ply: 1
components:
  nestedbytes:
    anchor: ply_fixture_nestedbytes
    fns:
      valid_key_bytes:
        checks: [bounded(2), fuzz(32)]
        ensures:
          - '|result| !keys.is_empty()'
"#;
    std::fs::write(&yaml, merged).unwrap();
    let run = run_verify(&bin, fixture.path(), 90);
    assert_eq!(run.exit_code, Some(0), "{}", run.json);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), source_before);
    std::fs::write(&yaml, merged.replace("!keys.is_empty()", "keys.is_empty()")).unwrap();
    let run = run_verify(&bin, fixture.path(), 90);
    assert_eq!(run.json["root"]["verdict"], "violation", "{}", run.json);
    assert!(fixture
        .path()
        .join("target/ply/witness/valid_key_bytes.json")
        .is_file());
    std::fs::write(&yaml, merged).unwrap();
    let run = run_verify(&bin, fixture.path(), 90);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
    let replay = run_cargo_test(fixture.path());
    assert!(replay.success, "{}", replay.combined_output);
}

#[test]
fn native_snapshots_and_other_borrowed_inputs_preserve_the_signature() {
    let bin = build_cargo_ply();
    for (body, requires, ensures, expected) in [
        (
            "pub fn valid_key_bytes(__ply_old_0: usize, keys: &[&[u8]]) -> bool { true }",
            "true",
            "|result| *result == (__ply_old_0 == old(keys.len()))",
            "violation",
        ),
        (
            "pub fn valid_key_bytes(keys: &[&[u8]], x: &u8) -> u8 { *x }",
            "*x <= 1",
            "|result| *result == *x",
            "bounded(2)",
        ),
    ] {
        let fixture = copy_fixture("nestedbytes");
        std::fs::write(fixture.path().join("src/lib.rs"), body).unwrap();
        std::fs::write(fixture.path().join("ply.yaml"), format!(
            "ply: 1\ncomponents:\n  nestedbytes:\n    anchor: ply_fixture_nestedbytes\n    fns:\n      valid_key_bytes:\n        checks: [bounded(2)]\n        requires:\n          - '{requires}'\n        ensures:\n          - '{ensures}'\n")).unwrap();
        let run = run_verify(&bin, fixture.path(), 60);
        assert_eq!(run.json["root"]["verdict"], expected, "{}", run.json);
        if expected == "violation" {
            assert!(
                run.json["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["code"] == "K0502"),
                "{}",
                run.json
            );
            let replay = run_cargo_test(fixture.path());
            assert!(
                !replay.success && replay.combined_output.contains("Broken promise"),
                "{}",
                replay.combined_output
            );
        }
    }
}

#[test]
fn receiverless_associated_inline_byte_function_is_checked_in_its_shadow() {
    let bin = build_cargo_ply();
    let fixture = copy_fixture("nestedbytes");
    let source = r#"pub struct Keys;
impl Keys {
    #[ply::ensures(|result| *result == !keys.is_empty())]
    pub fn validate(keys: &[&[u8]]) -> bool { !keys.is_empty() }
}
"#;
    std::fs::write(fixture.path().join("src/lib.rs"), source).unwrap();
    std::fs::write(
        fixture.path().join("ply.yaml"),
        r#"ply: 1
components:
  nestedbytes:
    anchor: ply_fixture_nestedbytes
    fns:
      'Keys::validate':
        checks: [bounded(2)]
"#,
    )
    .unwrap();
    let run = run_verify(&bin, fixture.path(), 90);
    assert_eq!(run.json["root"]["verdict"], "bounded(2)", "{}", run.json);
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("src/lib.rs")).unwrap(),
        source
    );
}
