//! Link refusal uses the same JSON diagnostics path as every other finding.

use std::process::Command;

fn cargo_ply() -> std::path::PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("cargo-ply")
}

fn linked_workspace(child_yaml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("inner_lib");
    std::fs::create_dir_all(child.join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"inner_lib\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        child.join("Cargo.toml"),
        "[package]\nname = \"inner_lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(child.join("src/lib.rs"), "pub fn seven() -> u32 { 7 }\n").unwrap();
    std::fs::write(child.join("ply.yaml"), child_yaml).unwrap();
    std::fs::write(
        dir.path().join("ply.yaml"),
        "ply: 1\ncomponents:\n  core:\n    anchor: inner_lib\n",
    )
    .unwrap();
    dir
}

fn json_verify(dir: &std::path::Path) -> std::process::Output {
    Command::new(cargo_ply())
        .args(["verify", ".", "--json", "--fail-on", "error"])
        .current_dir(dir)
        .output()
        .expect("cargo-ply runs")
}

#[test]
fn ambiguity_fails_with_e0211_inside_the_json_envelope() {
    let dir = linked_workspace(
        "ply: 1\ncomponents:\n  selected:\n    anchor: inner_lib\n    fns:\n      seven: {checks: []}\n  unrelated:\n    anchor: inner_lib::other\n    fns:\n      eight: {checks: []}\n",
    );

    let output = json_verify(dir.path());
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "verify must return its JSON envelope even when the link is ambiguous: {e}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });

    assert!(!output.status.success());
    assert!(
        envelope["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "E0211"
                    && diagnostic["severity"] == "error"
                    && diagnostic["node_id"] == "core"
            })
    );
}

#[test]
fn anchor_drift_is_advisory_and_still_returns_json() {
    let dir =
        linked_workspace("ply: 1\ncomponents:\n  implementation:\n    anchor: somewhere_else\n");

    let output = json_verify(dir.path());
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "verify must return its JSON envelope when a link no longer forms: {e}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });

    assert!(output.status.success());
    assert!(
        envelope["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "W0532"
                    && diagnostic["severity"] == "warning"
                    && diagnostic["node_id"] == "core"
            })
    );
}
