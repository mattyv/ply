//! `cargo ply check`'s crate tier (The-Ply-Spec.md §5.3, first paragraph)
//! against a workspace whose *top* crate has no library target at all --
//! only a `[[bin]]`, the shape `tests/fixtures/archtier` cannot exercise
//! (every crate there declares an explicit `[lib] name = ...`) and exactly
//! the shape this repo's own top crate, `ply-cli`, really has.
//!
//! Before the fix, `lib_target_name` returned `None` for a bin-only
//! package, so it never entered `lib_name_by_id`, so every dependency
//! *originating* from it was silently dropped from the graph -- a false
//! clean, not a crash. `tests/fixtures/archtierbin`'s `crate_top` is
//! bin-only and really depends on `crate_lib`; `crate_dual` carries both a
//! `[lib]` and a `[[bin]]`, so a dependency on it must be identified by its
//! lib name, never its bin name.
//!
//! Same helper-avoidance reason as `arch_crate_tier_command.rs`:
//! `ply_e2e::copy_fixture` requires a `ply-attrs` path dependency to
//! rewrite, which this fixture has no reason to declare.

use std::path::Path;

use ply_e2e::{build_cargo_ply, repo_root};

/// Collapses whitespace so an assertion is exact about the *words* without
/// depending on the ~92-column wrap.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

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

/// A fresh copy of `tests/fixtures/archtierbin`, so a test that edits its
/// `ply.yaml` never touches the checked-in fixture.
fn copy_archtierbin() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(&repo_root().join("tests/fixtures/archtierbin"), dir.path());
    dir
}

fn write_yaml(fixture: &Path, yaml: &str) {
    std::fs::write(fixture.join("ply.yaml"), yaml).unwrap();
}

fn run(fixture: &Path, extra: &[&str]) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let mut args: Vec<&str> = vec!["check", fixture.to_str().unwrap()];
    args.extend_from_slice(extra);
    let out = std::process::Command::new(&cargo_ply)
        .args(&args)
        .output()
        .expect("spawning cargo-ply check");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const TOP_AND_LIB_YAML: &str = "\
ply: 1

components:
  top:
    anchor: ply_fixture_archtierbin_top
  lib:
    anchor: archtierbin_lib
";

/// Test 1 (required): a binary-only top crate really depends on
/// `crate_lib` (a real `cargo metadata` edge), the two are different
/// declared components, and no `->` edge permits it -- `A0401`, naming
/// both crates, and exit non-zero. `crate_dual` is not declared as a
/// component in this document at all, so `top`'s equally real dependency
/// on it is simply out of scope here (an undeclared crate is ignored,
/// same as an ordinary external dependency) -- this test isolates the one
/// pair it names.
///
/// This is the fixture's own checked-in `ply.yaml` verbatim (no
/// `write_yaml` override needed): the reproduction the task asked for,
/// permanently in the suite.
#[test]
fn a_binary_only_top_crates_dependency_is_a0401_and_exits_nonzero() {
    let fixture = copy_archtierbin();
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("no envelope: {e}\n{stdout}"));
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(
        diagnostics.len(),
        1,
        "the undeclared `dual` crate must not also show up: {diagnostics:#?}"
    );
    let d = &diagnostics[0];
    assert_eq!(d["code"], "A0401");
    assert_eq!(d["severity"], "error");
    let title = unwrapped(d["title"].as_str().unwrap());
    assert!(title.contains("archtierbin_lib"), "{title}");
    assert!(title.contains("`top`"), "{title}");
    assert!(title.contains("`lib`"), "{title}");
    // The identity a bin-only crate is named by is its normalised package
    // name, never its bin target's own name.
    assert!(
        title.contains("ply_fixture_archtierbin_top"),
        "must name the bin-only crate by its normalised package name: {title}"
    );
    assert!(
        !title.contains("archtierbin-top-cli"),
        "must not name the bin-only crate by its *binary* target name: {title}"
    );
}

/// Test 2 (required): the same real dependency, now permitted by a
/// declared edge -- zero diagnostics, exit 0. A checker that always fires
/// on a binary-only crate is as worthless as one that never does.
#[test]
fn a_declared_edge_permits_the_binary_crates_dependency_and_the_run_is_clean() {
    let fixture = copy_archtierbin();
    write_yaml(
        fixture.path(),
        &format!("{TOP_AND_LIB_YAML}edges:\n  - \"top -> lib\"\n"),
    );
    let (code, stdout, stderr) = run(fixture.path(), &[]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("No problems found in the document."),
        "{stdout}"
    );
}

/// Test 3 (required): `crate_dual` carries both a `[lib]` and a `[[bin]]`.
/// A real dependency on it must be classified by its lib name
/// (`archtierbin_dual`), never its bin name (`archtierbin-dual-cli`) --
/// here `lib` is left undeclared so only the `top`/`dual` pair is in
/// scope.
#[test]
fn a_dependency_on_a_dual_lib_and_bin_crate_is_identified_by_its_lib_name() {
    let fixture = copy_archtierbin();
    write_yaml(
        fixture.path(),
        "ply: 1\n\ncomponents:\n  top:\n    anchor: ply_fixture_archtierbin_top\n  dual:\n    anchor: archtierbin_dual\n",
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    let title = unwrapped(diagnostics[0]["title"].as_str().unwrap());
    assert!(title.contains("archtierbin_dual"), "{title}");
    assert!(
        !title.contains("archtierbin-dual-cli"),
        "must not name the dual crate by its bin target: {title}"
    );
}
