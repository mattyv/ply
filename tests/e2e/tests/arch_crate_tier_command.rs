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

// ---- Review findings 2, 3, 4 (docs/review-architecture-tier.md) ----

/// Finding 2a: a component's anchor names a crate that does not exist
/// anywhere in this workspace's real dependency graph -- a rename, or a
/// typo. Before the fix this component silently owned nothing at all,
/// exit 0; now `A0410` names the component and the crate it cannot find.
#[test]
fn an_anchor_naming_a_nonexistent_crate_is_a0410() {
    let fixture = copy_archtier();
    write_yaml(
        fixture.path(),
        "ply: 1\n\ncomponents:\n  a:\n    anchor: archtier_a_typo\n  b:\n    anchor: archtier_b\n",
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "A0410")
        .unwrap_or_else(|| panic!("{diagnostics:#?}"));
    let title = unwrapped(d["title"].as_str().unwrap());
    assert!(title.contains("archtier_a_typo"), "{title}");
    assert!(title.contains("`a`"), "{title}");
}

/// Finding 2b: two components anchored at the literal same crate. Ply
/// keeps the first declaration; the second must say it owns nothing
/// rather than leave its `deny:` rule silently inert.
#[test]
fn two_components_anchored_at_the_same_crate_is_a0411() {
    let fixture = copy_archtier();
    write_yaml(
        fixture.path(),
        "ply: 1\n\ncomponents:\n  a_public:\n    anchor: archtier_a\n  a_internal:\n    anchor: archtier_a\n  b:\n    anchor: archtier_b\nedges:\n  - \"b -> a_public\"\ndeny:\n  - \"b -> a_internal\"\n",
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "A0411")
        .unwrap_or_else(|| panic!("{diagnostics:#?}"));
    assert_eq!(d["node_id"], "a_internal");
    let title = unwrapped(d["title"].as_str().unwrap());
    assert!(title.contains("a_public"), "{title}");
    // The ban attached to the shadowed component must never silently fire
    // against the crate the *first* component actually owns.
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "A0405"),
        "a ban on a shadowed component must not fire: {diagnostics:#?}"
    );
}

/// Finding 2c: a component anchored at a *module* inside a crate another
/// component already claims whole. The crate tier only reads an anchor's
/// first `::`-segment, so `archtier_a::ratemod` collides with plain
/// `archtier_a` exactly the way a literal duplicate does -- same code
/// path, different surface shape.
#[test]
fn a_module_anchored_component_collides_with_a_crate_anchored_one_is_a0411() {
    let fixture = copy_archtier();
    write_yaml(
        fixture.path(),
        "ply: 1\n\ncomponents:\n  a:\n    anchor: archtier_a\n  a_ratemod:\n    anchor: archtier_a::ratemod\n  b:\n    anchor: archtier_b\nedges:\n  - \"b -> a\"\n",
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "A0411")
        .unwrap_or_else(|| panic!("{diagnostics:#?}"));
    assert_eq!(d["node_id"], "a_ratemod");
}

/// Finding 2d: an edge naming a component that does not exist at all is
/// silently inert today -- `matching_deny`/`permitted` simply find no
/// match, and the run reads as clean. `A0413` says so instead of staying
/// quiet about a typo.
#[test]
fn a_deny_rule_naming_a_nonexistent_component_is_a0413() {
    let fixture = copy_archtier();
    write_yaml(
        fixture.path(),
        &format!("{BASE_YAML}edges:\n  - \"b -> a\"\ndeny:\n  - \"b -> nosuchcomponent\"\n"),
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "A0413")
        .unwrap_or_else(|| panic!("{diagnostics:#?}"));
    let title = unwrapped(d["title"].as_str().unwrap());
    assert!(title.contains("nosuchcomponent"), "{title}");
}

/// Finding 3: the coverage line's denominator. `crate_c` is real (it's
/// `crate_a`'s own dependency in this fixture) but, with the nested `c`
/// component removed from the document entirely, no declared component
/// claims it -- exactly the shape `tests/e2e` is in this repo's own
/// `ply.yaml` today. The coverage sentence must name it rather than stay
/// silent the way a wildcard `deny:` (which only ever means "any
/// *declared* component") would.
#[test]
fn an_undeclared_workspace_crate_is_counted_and_named_in_coverage() {
    let fixture = copy_archtier();
    // `b -> a` declared so the *other* real dependency in this fixture
    // (`b` on `a`) stays clean -- isolates the denominator disclosure from
    // any unrelated `A0401`. `a`'s own real dependency on `c` is simply out
    // of scope now that nothing declares `c` (§5.3: an undeclared crate is
    // out of scope, not a violation), which is exactly the silence finding
    // 3 asks to be made visible.
    write_yaml(
        fixture.path(),
        "ply: 1\n\ncomponents:\n  a:\n    anchor: archtier_a\n  b:\n    anchor: archtier_b\nedges:\n  - \"b -> a\"\n",
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let cov = json["coverage"]["checked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["tier"] == "architecture")
        .unwrap();
    let detail = unwrapped(cov["detail"].as_str().unwrap());
    assert!(
        detail.contains("2 of 3 crates in this workspace belong to a declared component"),
        "{detail}"
    );
    assert!(detail.contains("archtier_c"), "{detail}");
}

/// Finding 4: a dependency that exists only as a `dev-dependencies` entry
/// is not enforced (§5.3 is about code that ships), but must be disclosed
/// rather than silently dropped -- the review's own finding that the
/// printed sentence ("no crate here depends on another") goes false the
/// moment a dev-dependency crosses an undeclared boundary.
#[test]
fn a_dev_dependency_crossing_is_disclosed_but_not_enforced() {
    let fixture = copy_archtier();
    // `b` gets a *dev*-dependency on the nested `a.c` component's crate,
    // with no edge permitting it and no normal dependency involved at all
    // -- isolates the disclosure from `b`'s existing real dependency on
    // `a` (declared clean via the edge below).
    let crate_b_toml = fixture.path().join("crate_b/Cargo.toml");
    let mut content = std::fs::read_to_string(&crate_b_toml).unwrap();
    content.push_str(
        "\n[dev-dependencies]\narchtier_c = { package = \"ply-fixture-archtier-c\", path = \"../crate_c\" }\n",
    );
    std::fs::write(&crate_b_toml, content).unwrap();
    write_yaml(
        fixture.path(),
        &format!("{BASE_YAML}edges:\n  - \"b -> a\"\n"),
    );
    let (code, stdout, stderr) = run(fixture.path(), &["--json"]);
    assert_eq!(
        code, 0,
        "a dev-only crossing must not be enforced: {stdout}\n{stderr}"
    );
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        json["diagnostics"].as_array().unwrap().is_empty(),
        "{:#?}",
        json["diagnostics"]
    );
    let cov = json["coverage"]["checked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["tier"] == "architecture")
        .unwrap();
    let detail = unwrapped(cov["detail"].as_str().unwrap());
    assert!(
        detail.contains("1 more crosses a declared boundary only as a test or build dependency"),
        "{detail}"
    );
}
