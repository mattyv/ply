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
/// document says it must not. The approximate source finding is advisory
/// by default, but the message still has to name the function and the file
/// -- "component parse touches component exec" sends the reader hunting.
#[test]
fn a_forbidden_call_between_two_modules_is_advisory_by_default() {
    let fixture = copy_modtier();
    let (code, stdout, stderr) = run(fixture.path());
    let all = unwrapped(&format!("{stdout}\n{stderr}"));

    assert_eq!(code, 0, "an approximate finding is advisory:\n{all}");
    assert!(
        all.contains("A0402"),
        "the advisory code must be stable:\n{all}"
    );
    assert!(
        all.contains("parse.rs"),
        "the report has to name the file the call is in:\n{all}"
    );
    assert!(all.contains("run"), "and the function it calls:\n{all}");
}

/// `strict: true` is the component's explicit choice to make the same
/// approximate crossing fail the command. The strict form has its own
/// registered code, so `cargo ply explain` never lies about whether the
/// code in hand can fail a run.
#[test]
fn strict_turns_the_same_forbidden_call_into_an_error() {
    let fixture = copy_modtier();
    let yaml = fixture.path().join("ply.yaml");
    let text = std::fs::read_to_string(&yaml).unwrap();
    std::fs::write(
        &yaml,
        text.replace(
            "  parse:\n    anchor: modtier::parse",
            "  parse:\n    anchor: modtier::parse\n    strict: true",
        ),
    )
    .unwrap();

    let (code, stdout, stderr) = run(fixture.path());
    let all = unwrapped(&format!("{stdout}\n{stderr}"));
    assert_ne!(code, 0, "strict makes this finding fail the run:\n{all}");
    assert!(
        all.contains("A0420"),
        "the strict code must be stable:\n{all}"
    );
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
