//! `cargo ply check`'s crate tier (The-Ply-Spec.md §5.3, first paragraph)
//! end to end: the real crate dependency graph, from `cargo metadata`,
//! checked against declared components and `edges:`/`deny:`.
//!
//! This needs a fixture with more than one crate -- `tests/fixtures/clamp`
//! and friends are single-crate by construction, so they can never carry a
//! real cross-crate dependency for this tier to classify.
//! `tests/fixtures/archtier` is a tiny workspace of three crates for
//! exactly this: `crate_b` really depends on `crate_a` (an ordinary
//! cross-component dependency), and `crate_a` really depends on `crate_c`,
//! which `ply.yaml` declares as `crate_a`'s own nested component (so that
//! pair exercises containment).
//!
//! This does not use `ply_e2e::copy_fixture`: that helper requires every
//! fixture's `Cargo.toml` to carry a `ply-attrs` path dependency to
//! rewrite, which this fixture has no reason to declare (there is nothing
//! here for `cargo ply verify` to check -- only `check`'s crate tier).

use std::path::Path;

use ply_e2e::{build_cargo_ply, repo_root};

/// Collapses whitespace so an assertion is exact about the *words* (the
/// point of the newbie bar) without depending on the ~92-column wrap.
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

/// A fresh copy of `tests/fixtures/archtier`, so a test that edits its
/// `ply.yaml` never touches the checked-in fixture.
fn copy_archtier() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(&repo_root().join("tests/fixtures/archtier"), dir.path());
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

const BASE_YAML: &str = "\
ply: 1

components:
  a:
    anchor: archtier_a
    components:
      c:
        anchor: archtier_c
  b:
    anchor: archtier_b
";

/// §5.3, first paragraph, plain default-deny case: `crate_b` really depends
/// on `crate_a` (a real `cargo metadata` edge) and no `->` edge says
/// component `b` may depend on component `a` -- `A0401`, and nothing else:
/// the *other* real dependency in this fixture (`a` on its own nested `c`)
/// is silently permitted by containment, so it must not also show up here.
#[test]
fn an_undeclared_cross_component_crate_dependency_is_a0401_and_exits_nonzero() {
    let fixture = copy_archtier();
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("no envelope: {e}\n{stdout}"));
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(
        diagnostics.len(),
        1,
        "containment must not also be flagged: {diagnostics:#?}"
    );
    let d = &diagnostics[0];
    assert_eq!(d["code"], "A0401");
    assert_eq!(d["severity"], "error");
    let title = unwrapped(d["title"].as_str().unwrap());
    assert!(title.contains("archtier_a"), "{title}");
    assert!(title.contains("archtier_b"), "{title}");
    assert!(title.contains("`a`"), "{title}");
    assert!(title.contains("`b`"), "{title}");
    assert!(
        title.contains("no `->` edge in this document says"),
        "{title}"
    );
}

/// The same real dependency, now permitted by a declared edge: zero
/// diagnostics, exit 0 -- a checker that always fires is as worthless as
/// one that never does.
#[test]
fn a_declared_edge_permits_it_and_the_run_is_clean() {
    let fixture = copy_archtier();
    write_yaml(
        fixture.path(),
        &format!("{BASE_YAML}edges:\n  - \"b -> a\"\n"),
    );
    let (code, stdout, stderr) = run(fixture.path(), &[]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("No problems found in the document."),
        "{stdout}"
    );
}

/// A `deny:` rule is checked against the real graph independent of whether
/// an edge permits it: `A0405` fires even though the edge above would
/// otherwise leave this clean -- which is what makes it a different fact
/// from `A0401`, not the same finding under another name.
#[test]
fn a_deny_rule_violated_by_the_real_dependency_is_a0405() {
    let fixture = copy_archtier();
    write_yaml(
        fixture.path(),
        &format!("{BASE_YAML}edges:\n  - \"b -> a\"\ndeny:\n  - \"b -> a\"\n"),
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|d| d["code"] == "A0405"),
        "{diagnostics:#?}"
    );
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "A0401"),
        "the edge permits it, so A0401 must not also fire: {diagnostics:#?}"
    );
}

/// An explicit edge from a component to its own nested descendant is
/// redundant -- containment already permits it -- so it is `W0409`, an
/// advisory finding that does not fail the run.
#[test]
fn a_redundant_edge_to_a_nested_component_is_w0409_and_does_not_fail_the_run() {
    let fixture = copy_archtier();
    // `b -> a` is declared too, so the *other* real dependency in this
    // fixture stays permitted and does not also show up as `A0401` --
    // this test isolates the one redundant-edge finding.
    write_yaml(
        fixture.path(),
        &format!("{BASE_YAML}edges:\n  - \"b -> a\"\n  - \"a -> a.c\"\n"),
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(
        code, 0,
        "a W-code must not fail the run: {stdout}\n{stderr}"
    );
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0]["code"], "W0409");
    assert_eq!(diagnostics[0]["severity"], "warning");
}
