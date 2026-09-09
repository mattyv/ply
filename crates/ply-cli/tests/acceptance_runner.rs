//! Real Cargo/libtest coverage for named acceptance evidence.

use ply_core::diag::AcceptanceOutcome;

fn write_project(root: &std::path::Path, test_attribute: &str, expected: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests/fixtures")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"acceptance-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn value() -> u32 { 7 }\n").unwrap();
    std::fs::write(root.join("tests/fixtures/input.txt"), "7\n").unwrap();
    std::fs::write(root.join("tests/fixtures/expected.txt"), "7\n").unwrap();
    std::fs::write(
        root.join("tests/venue_response.rs"),
        format!(
            "{test_attribute}\n#[test]\nfn decimal_strings_map_to_expected_records() {{\n    let fixture = std::fs::read_to_string(\"tests/fixtures/input.txt\").unwrap();\n    print!(\"production progress:\");\n    assert_eq!(std::env::var(\"CARGO_MANIFEST_DIR\").unwrap(), env!(\"CARGO_MANIFEST_DIR\"));\n    assert_eq!(std::env::var(\"CARGO_PKG_NAME\").unwrap(), \"acceptance-app\");\n    assert_eq!(fixture.trim(), \"{expected}\");\n    assert_eq!(acceptance_app::value(), 7);\n}}\n"
        ),
    )
    .unwrap();
}

fn write_slow_project(root: &std::path::Path) {
    write_project(root, "", "7");
    std::fs::write(
        root.join("tests/venue_response.rs"),
        "#[test]\nfn decimal_strings_map_to_expected_records() {\n    std::thread::sleep(std::time::Duration::from_secs(5));\n}\n",
    )
    .unwrap();
}

fn document(test_name: &str) -> ply_core::model::Document {
    ply_core::config::load_str(&format!(
        r#"
ply: 1
components:
  mapping:
    anchor: acceptance_app
acceptance:
  decimal_response_maps:
    requirement: decimal strings map to exact records
    component: mapping
    entry: acceptance_app::value
    test:
      package: acceptance-app
      target: venue_response
      name: {test_name}
    inputs: [tests/fixtures/input.txt]
    expected: [tests/fixtures/expected.txt]
    required: true
"#
    ))
    .unwrap()
}

#[test]
fn real_runner_preserves_cwd_and_distinguishes_pass_failure_missing_and_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write_project(root, "", "7");
    let passed = ply_cli::acceptance::run(
        root,
        &document("decimal_strings_map_to_expected_records"),
        Some(30),
    )
    .unwrap();
    assert_eq!(passed[0].outcome, AcceptanceOutcome::Passed);

    write_project(root, "", "8");
    let failed = ply_cli::acceptance::run(
        root,
        &document("decimal_strings_map_to_expected_records"),
        Some(30),
    )
    .unwrap();
    assert_eq!(failed[0].outcome, AcceptanceOutcome::Failed);

    write_project(root, "", "7");
    let missing = ply_cli::acceptance::run(root, &document("another_test"), Some(30)).unwrap();
    assert_eq!(missing[0].outcome, AcceptanceOutcome::NotRun);

    write_project(root, "#[ignore]", "7");
    let ignored = ply_cli::acceptance::run(
        root,
        &document("decimal_strings_map_to_expected_records"),
        Some(30),
    )
    .unwrap();
    assert_eq!(ignored[0].outcome, AcceptanceOutcome::NotRun);

    write_slow_project(root);
    let timed_out = ply_cli::acceptance::run(
        root,
        &document("decimal_strings_map_to_expected_records"),
        Some(1),
    )
    .unwrap();
    assert_eq!(timed_out[0].outcome, AcceptanceOutcome::Timeout);

    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"acceptance-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[[ test ]]\nname = 'venue_response'\npath = \"tests/venue_response.rs\"\nharness =  false\n",
    )
    .unwrap();
    let custom = ply_cli::acceptance::run(
        root,
        &document("decimal_strings_map_to_expected_records"),
        Some(30),
    )
    .unwrap();
    assert_eq!(custom[0].outcome, AcceptanceOutcome::ToolError);
    assert!(
        custom[0].detail.contains("harness = false"),
        "the manifest gate, not a later build accident, must refuse a custom harness: {}",
        custom[0].detail
    );
}

#[test]
fn member_specific_cargo_configuration_applies_to_the_acceptance_build() {
    let dir = tempfile::tempdir().unwrap();
    let member = dir.path().join("member");
    write_project(&member, "", "7");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"member\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(member.join(".cargo")).unwrap();
    std::fs::write(
        member.join(".cargo/config.toml"),
        "[build]\nrustflags = [\"--cfg\", \"acceptance_member_config\"]\n",
    )
    .unwrap();
    std::fs::write(
        member.join("src/lib.rs"),
        "#[cfg(acceptance_member_config)] pub fn value() -> u32 { 7 }\n#[cfg(not(acceptance_member_config))] pub fn value() -> u32 { 8 }\n",
    )
    .unwrap();

    let result = ply_cli::acceptance::run(
        &member,
        &document("decimal_strings_map_to_expected_records"),
        Some(30),
    )
    .unwrap();
    assert_eq!(
        result[0].outcome,
        AcceptanceOutcome::Passed,
        "acceptance must build under the same member configuration as native Cargo: {}",
        result[0].detail
    );
}

#[test]
fn a_nested_tests_pass_cannot_complete_the_selected_acceptance_test() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_project(root, "", "7");
    std::fs::write(
        root.join("tests/helper.rs"),
        "#[test]\nfn decimal_strings_map_to_expected_records() {}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tests/venue_response.rs"),
        r#"
#[test]
fn decimal_strings_map_to_expected_records() {
    let status = std::process::Command::new("cargo")
        .args([
            "test",
            "--test",
            "helper",
            "--",
            "--exact",
            "decimal_strings_map_to_expected_records",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    std::process::exit(0);
}
"#,
    )
    .unwrap();

    let result = ply_cli::acceptance::run(
        root,
        &document("decimal_strings_map_to_expected_records"),
        Some(30),
    )
    .unwrap();
    assert_eq!(
        result[0].outcome,
        AcceptanceOutcome::ToolError,
        "an unrelated nested summary is not completion evidence for the selected test: {}",
        result[0].detail
    );
}
