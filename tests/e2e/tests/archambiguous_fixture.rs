//! Finding 6 (docs/review-architecture-tier.md): two different crates in
//! one workspace can legally build a library with the same name. Crate
//! identity resolved by library name alone collapses them into one, and a
//! dependency graph edge naming that identity cannot be trusted to mean
//! either crate in particular -- the review's own ws8 reproduction found
//! this reported a dependency that did not exist, in both directions
//! (a false `A0401`/`A0405`, and an inert ban going unnoticed).
//!
//! `tests/fixtures/archambiguous`: `left` and `right` both build a library
//! called `shared`; `user_l` really depends on `left`, `user_r` really
//! depends on `right` -- neither depends on the other. `ply.yaml` anchors
//! `leftside` at `shared` and bans `user_r -> leftside`. `user_r` does not
//! actually depend on `leftside` at all, so that ban must never fire, and
//! Ply must say the identity is ambiguous rather than guess.

use std::path::Path;

use ply_e2e::{build_cargo_ply, repo_root};

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

fn copy_archambiguous() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(
        &repo_root().join("tests/fixtures/archambiguous"),
        dir.path(),
    );
    dir
}

fn run(fixture: &Path) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let out = std::process::Command::new(&cargo_ply)
        .args(["check", fixture.to_str().unwrap(), "--json"])
        .output()
        .expect("spawning cargo-ply check");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn an_ambiguous_library_name_is_a0412_and_never_a_false_crossing() {
    let fixture = copy_archambiguous();
    let (code, stdout, stderr) = run(fixture.path());
    assert_eq!(code, 1, "stdout: {stdout}\nstderr: {stderr}");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("no envelope: {e}\n{stdout}"));
    let diagnostics = json["diagnostics"].as_array().unwrap();

    assert!(
        !diagnostics
            .iter()
            .any(|d| d["code"] == "A0401" || d["code"] == "A0405"),
        "user_r does not really depend on leftside -- an ambiguous identity must never be \
         silently attributed to either real crate: {diagnostics:#?}"
    );
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "A0412")
        .unwrap_or_else(|| panic!("{diagnostics:#?}"));
    let title = unwrapped(d["title"].as_str().unwrap());
    assert!(title.contains("shared"), "{title}");
    assert!(title.contains("leftside"), "{title}");
}
