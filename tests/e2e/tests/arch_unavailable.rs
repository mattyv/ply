//! docs/review-architecture-tier.md, finding 1: when the architecture
//! tier cannot get the real crate dependency graph at all, `cargo ply
//! check` used to print "No problems found in the document." and exit 0,
//! with the failure buried as a sentence inside the coverage report. Now
//! it is `A0409`, an error-severity diagnostic, so the run fails loudly
//! and the reassuring line never prints.
//!
//! Three reproductions, all against a mutated copy of
//! `tests/fixtures/archtier` (never the checked-in fixture itself):
//! a broken manifest anywhere in the workspace, a real package dependency
//! cycle (the review's headline case -- exactly the shape a `deny:` rule
//! is usually written to catch), and `cargo` missing from `PATH`.

use std::path::Path;

use ply_e2e::{build_cargo_ply, repo_root};

fn copy_dir_recursive(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let file_name = entry.file_name();
        if file_name == "target" || file_name == "Cargo.lock" {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&file_name);
        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path).unwrap();
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}

fn copy_archtier() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(&repo_root().join("tests/fixtures/archtier"), dir.path());
    dir
}

fn run(fixture: &Path, path_override: Option<&str>) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let mut cmd = std::process::Command::new(&cargo_ply);
    cmd.args(["check", fixture.to_str().unwrap(), "--json"]);
    if let Some(path) = path_override {
        cmd.env("PATH", path);
    }
    let out = cmd.output().expect("spawning cargo-ply check");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The one thing that must be true across all three reproductions: the
/// run never reads as clean. `"No problems found"` must never print when
/// the architecture tier could not run at all -- printing it first, with
/// the failure buried afterward, is exactly the defect finding 1 names.
fn assert_not_a_clean_run(code: i32, stdout: &str, stderr: &str) {
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        !stdout.contains("No problems found in the document."),
        "a run that could not check architecture must never read as clean: {stdout}"
    );
    let json: serde_json::Value =
        serde_json::from_str(stdout).unwrap_or_else(|e| panic!("no envelope: {e}\n{stdout}"));
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|d| d["code"] == "A0409"),
        "{diagnostics:#?}"
    );
    assert!(diagnostics.iter().find(|d| d["code"] == "A0409").unwrap()["severity"] == "error");
}

/// (a) A bad version requirement in any manifest in the workspace makes
/// `cargo metadata` fail outright.
#[test]
fn a_broken_manifest_anywhere_in_the_workspace_is_a0409_not_a_clean_run() {
    let fixture = copy_archtier();
    let cargo_toml = fixture.path().join("crate_b/Cargo.toml");
    let mut content = std::fs::read_to_string(&cargo_toml).unwrap();
    content.push_str("bogus = \"!!!\"\n");
    std::fs::write(&cargo_toml, content).unwrap();

    let (code, stdout, stderr) = run(fixture.path(), None);
    assert_not_a_clean_run(code, &stdout, &stderr);
}

/// (b, the review's headline reproduction) A real package dependency
/// cycle: `crate_a` already depends on `crate_c` (the fixture's own
/// containment pair); adding the dependency back the other way round
/// makes it a genuine cycle, which `cargo metadata` refuses to produce a
/// graph for at all -- the exact shape a `ply.yaml` boundary rule usually
/// exists to catch.
#[test]
fn a_real_package_dependency_cycle_is_a0409_not_a_clean_run() {
    let fixture = copy_archtier();
    let cargo_toml = fixture.path().join("crate_c/Cargo.toml");
    let mut content = std::fs::read_to_string(&cargo_toml).unwrap();
    content.push_str(
        "\n[dependencies]\narchtier_a = { package = \"ply-fixture-archtier-a\", path = \"../crate_a\" }\n",
    );
    std::fs::write(&cargo_toml, content).unwrap();

    let (code, stdout, stderr) = run(fixture.path(), None);
    assert_not_a_clean_run(code, &stdout, &stderr);
}

/// (c) `cargo` missing from `PATH` -- a container without a toolchain, or
/// a CI step ordering mistake. Only this subprocess's `PATH` is touched
/// (via `Command::env`), never the test process's own environment, so
/// this is safe to run alongside every other parallel test.
#[test]
fn cargo_missing_from_path_is_a0409_not_a_clean_run() {
    let fixture = copy_archtier();
    let (code, stdout, stderr) = run(fixture.path(), Some("/nonexistent-path-for-this-test"));
    assert_not_a_clean_run(code, &stdout, &stderr);
}
