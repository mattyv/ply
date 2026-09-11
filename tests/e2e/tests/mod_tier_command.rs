//! The module tier end to end: a rule between two modules of ONE crate,
//! checked by `cargo ply check` against the real source.
//!
//! This is the case the crate tier cannot reach. `tests/fixtures/modtier`
//! is a single package, so its Cargo dependency list is empty and says
//! nothing about whether the parser calls into the executor. Only reading
//! the source answers that, and until this tier existed Ply would report
//! such a document as clean while the forbidden call sat in plain sight.

use std::path::Path;

use ply_e2e::{build_cargo_ply, repo_root};

/// Collapses whitespace so an assertion is exact about the *words* without
/// depending on the column wrap.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == "Cargo.lock" {
            continue;
        }
        let (s, d) = (entry.path(), dst.join(&name));
        if s.is_dir() {
            std::fs::create_dir_all(&d).unwrap();
            copy_dir_recursive(&s, &d);
        } else {
            std::fs::copy(&s, &d).unwrap();
        }
    }
}

fn copy_modtier() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(&repo_root().join("tests/fixtures/modtier"), dir.path());
    dir
}

fn run(fixture: &Path) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let out = std::process::Command::new(&cargo_ply)
        .args(["ply", "check", "."])
        .current_dir(fixture)
        .output()
        .expect("run cargo ply check");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// **The acceptance case.** `parse` calls `exec::run` directly, and the
/// document says it must not. The run has to fail, and the message has to
/// name the function and the file -- "component parse touches component
/// exec" sends the reader hunting.
#[test]
fn a_forbidden_call_between_two_modules_of_one_crate_is_reported_and_fails() {
    let fixture = copy_modtier();
    let (code, stdout, stderr) = run(fixture.path());
    let all = unwrapped(&format!("{stdout}\n{stderr}"));

    assert_ne!(
        code, 0,
        "a call the document forbids must not exit 0:\n{stdout}\n{stderr}"
    );
    assert!(
        all.contains("parse.rs"),
        "the report has to name the file the call is in:\n{all}"
    );
    assert!(all.contains("run"), "and the function it calls:\n{all}");
}

/// The permitted calls must not be reported. Both modules really do call
/// `shared::normalise`, and the document allows it; a tier that flagged
/// those would be unusable on any real repository.
#[test]
fn the_calls_the_document_allows_are_not_reported() {
    let fixture = copy_modtier();
    let (_, stdout, stderr) = run(fixture.path());
    let all = unwrapped(&format!("{stdout}\n{stderr}"));
    // Precisely: no *violation* line may be about the permitted calls.
    let violations: Vec<&str> = all.split("A0402").skip(1).collect();
    for v in &violations {
        let line = v.split("A04").next().unwrap_or(v);
        assert!(
            !line.contains("shared"),
            "a permitted call was reported as forbidden:\n{line}"
        );
    }
    assert_eq!(
        violations.len(),
        1,
        "exactly the one real violation:\n{all}"
    );
    // And the report must stay readable: one line per module for what the
    // scan could not follow, not one per `.len()`.
    let noise = all.matches("method call").count();
    assert!(
        noise <= 3,
        "the limits of the scan must be stated once per module, not per call site \
         ({noise} times here):\n{all}"
    );
}
