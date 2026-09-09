//! The command-level zero-record regression: local contract evidence and
//! finite application evidence remain independent, and the latter runs the
//! production parser over committed raw bytes.

use std::path::{Path, PathBuf};
use std::process::Command;

fn cargo_ply() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("cargo-ply")
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn verify(root: &Path) -> (std::process::ExitStatus, serde_json::Value) {
    let output = Command::new(cargo_ply())
        .args(["verify", ".", "--json", "--engine-timeout", "30"])
        // The outer workspace build already fetched these exact serde
        // dependencies. This fixture must prove acceptance execution, not
        // depend on the registry being reachable during the test.
        .env("CARGO_NET_OFFLINE", "true")
        .current_dir(root)
        .output()
        .expect("cargo-ply runs");
    let json = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "verify did not return JSON: {error}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, json)
}

#[test]
fn required_acceptance_detects_decimal_loss_without_upgrading_contract_evidence() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acceptance_decimal");
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture, temp.path());

    let (status, clean) = verify(temp.path());
    assert!(status.success(), "clean fixture should pass: {clean:#}");
    assert_eq!(clean["root"]["verdict"], "unclaimed");
    assert_eq!(clean["acceptance"][0]["outcome"], "passed");

    let source_path = temp.path().join("src/lib.rs");
    let source = std::fs::read_to_string(&source_path).unwrap();
    let start = source.find("fn parse_price").unwrap();
    let broken = format!(
        "{}fn parse_price(text: &str) -> Option<i64> {{\n    text.parse::<i64>().ok().and_then(|value| value.checked_mul(10_000))\n}}\n",
        &source[..start]
    );
    std::fs::write(source_path, broken).unwrap();

    let (status, failed) = verify(temp.path());
    assert!(!status.success(), "required acceptance failure must fail");
    assert_eq!(failed["root"]["verdict"], "unclaimed");
    assert_eq!(failed["acceptance"][0]["outcome"], "failed");
    assert!(
        failed["acceptance"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("decimal_strings_map_to_expected_records")
    );
}

#[test]
fn linked_acceptance_is_selected_rebased_and_never_silently_dropped() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acceptance_decimal");
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    copy_tree(&fixture, &child);
    let manifest = std::fs::read_to_string(child.join("Cargo.toml"))
        .unwrap()
        .replace("\n[workspace]\n", "\n");
    std::fs::write(child.join("Cargo.toml"), manifest).unwrap();
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"child\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("ply.yaml"),
        "ply: 1\ncomponents:\n  core:\n    anchor: acceptance_decimal\n",
    )
    .unwrap();
    let mut child_yaml = std::fs::read_to_string(child.join("ply.yaml")).unwrap();
    child_yaml = child_yaml.replacen(
        "acceptance:\n",
        "  unrelated:\n    anchor: some_other_crate\nacceptance:\n",
        1,
    );
    child_yaml.push_str(
        "  unrelated_requirement:\n    requirement: an unrelated child component does something\n    component: unrelated\n    entry: acceptance_decimal::map_response\n    test:\n      package: acceptance-decimal\n      target: venue_response\n      name: decimal_strings_map_to_expected_records\n    inputs: [tests/fixtures/venue-response.json]\n    expected: [tests/fixtures/venue-response.expected.json]\n    required: true\n",
    );
    std::fs::write(child.join("ply.yaml"), child_yaml).unwrap();

    let (status, envelope) = verify(temp.path());
    assert!(
        status.success(),
        "selected acceptance should pass: {envelope:#}"
    );
    assert_eq!(envelope["acceptance"].as_array().unwrap().len(), 1);
    assert_eq!(
        envelope["acceptance"][0]["id"],
        "child/ply.yaml::decimal_response_maps"
    );
    assert_eq!(envelope["acceptance"][0]["component"], "core");
    assert_eq!(
        envelope["acceptance"][0]["inputs"][0],
        "child/tests/fixtures/venue-response.json"
    );
    assert!(
        envelope["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "W0543"
                && diagnostic["title"]
                    .as_str()
                    .unwrap()
                    .contains("unrelated_requirement"))
    );

    let child_yaml = std::fs::read_to_string(child.join("ply.yaml"))
        .unwrap()
        .replacen(
            "inputs:\n      - tests/fixtures/venue-response.json",
            "inputs: [../outside.json]",
            1,
        );
    std::fs::write(child.join("ply.yaml"), child_yaml).unwrap();
    let (status, invalid) = verify(temp.path());
    assert!(!status.success());
    assert_eq!(invalid["acceptance"].as_array().unwrap().len(), 1);
    assert_eq!(invalid["acceptance"][0]["outcome"], "not_run");
    assert!(invalid["diagnostics"].as_array().unwrap().iter().any(|d| {
        d["code"] == "E0212" && d["node_id"] == "child/ply.yaml::decimal_response_maps"
    }));
    assert!(
        !invalid["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "E0211"),
        "a mapped acceptance configuration error must not collapse into generic E0211: {invalid:#}"
    );
}

#[test]
fn independent_linked_workspace_paths_are_rebased_to_the_root_document() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acceptance_decimal");
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    copy_tree(&fixture, &child);
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[workspace]\nmembers = []\nexclude = [\"child\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("ply.yaml"),
        "ply: 1\ncomponents:\n  core:\n    anchor: acceptance_decimal\n",
    )
    .unwrap();

    let (status, envelope) = verify(temp.path());
    assert!(
        status.success(),
        "independent child should pass: {envelope:#}"
    );
    assert_eq!(
        envelope["acceptance"][0]["id"],
        "child/ply.yaml::decimal_response_maps"
    );
    assert_eq!(
        envelope["acceptance"][0]["inputs"][0],
        "child/tests/fixtures/venue-response.json"
    );
    assert_eq!(
        envelope["acceptance"][0]["expected"][0],
        "child/tests/fixtures/venue-response.expected.json"
    );
}
