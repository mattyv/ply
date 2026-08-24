//! Shared e2e test support: copies a fixture into a scratch tempdir (with
//! its `ply` dependency path rewritten to absolute, since fixtures declare
//! it relative to their checked-in location under `tests/fixtures/`), builds
//! `cargo-ply`, and runs it against the copy. Every acceptance test in
//! `tests/e2e/tests/` uses this instead of touching the checked-in fixtures.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// The repo root -- `tests/e2e/..` is `tests/`, `../..` is the repo root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Builds `cargo-ply` (debug profile) and returns its path. Always builds
/// fresh via `cargo build -p ply-cli` rather than assuming a prior build --
/// slower, but never silently tests a stale binary.
pub fn build_cargo_ply() -> PathBuf {
    let root = repo_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["build", "-p", "ply-cli"])
        .status()
        .expect("spawning `cargo build -p ply-cli`");
    assert!(status.success(), "cargo build -p ply-cli failed");
    root.join("target/debug/cargo-ply")
}

/// A fixture copied into a fresh tempdir, with its `ply-attrs` path
/// dependency rewritten to an absolute path so the copy builds
/// independently of its original location under `tests/fixtures/`.
pub struct FixtureCopy {
    pub dir: TempDir,
}

impl FixtureCopy {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn lib_rs_path(&self) -> PathBuf {
        self.path().join("src/lib.rs")
    }

    pub fn read_lib_rs(&self) -> String {
        std::fs::read_to_string(self.lib_rs_path()).unwrap()
    }

    pub fn write_lib_rs(&self, content: &str) {
        std::fs::write(self.lib_rs_path(), content).unwrap();
    }
}

/// Copies `tests/fixtures/<name>` into a new tempdir (skipping `target/` and
/// any `Cargo.lock`, both regenerable) and rewrites the `ply-attrs` path
/// dependency to an absolute path.
pub fn copy_fixture(name: &str) -> FixtureCopy {
    let src = repo_root().join("tests/fixtures").join(name);
    assert!(src.is_dir(), "no such fixture: {}", src.display());
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(&src, dir.path());

    let cargo_toml_path = dir.path().join("Cargo.toml");
    let cargo_toml = std::fs::read_to_string(&cargo_toml_path).unwrap();
    let ply_attrs_abs = repo_root().join("crates/ply-attrs");
    let rewritten = cargo_toml.replace(
        "path = \"../../../crates/ply-attrs\"",
        &format!("path = \"{}\"", ply_attrs_abs.display()),
    );
    assert_ne!(
        cargo_toml, rewritten,
        "fixture Cargo.toml did not contain the expected relative ply-attrs path"
    );
    std::fs::write(&cargo_toml_path, rewritten).unwrap();

    FixtureCopy { dir }
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

pub struct VerifyRun {
    pub json: serde_json::Value,
    pub exit_code: Option<i32>,
}

/// Runs `cargo-ply verify <fixture_dir> --json --engine-timeout <secs>` and
/// parses its stdout as the §8 envelope.
pub fn run_verify(cargo_ply: &Path, fixture_dir: &Path, engine_timeout_secs: u32) -> VerifyRun {
    let output = Command::new(cargo_ply)
        .args([
            "verify",
            fixture_dir.to_str().unwrap(),
            "--json",
            "--engine-timeout",
            &engine_timeout_secs.to_string(),
        ])
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "cargo-ply verify did not print valid JSON: {e}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    VerifyRun {
        json,
        exit_code: output.status.code(),
    }
}

pub struct CargoTestRun {
    pub success: bool,
    pub combined_output: String,
}

/// Runs `cargo test --lib` in `crate_dir` and returns whether it succeeded
/// plus the combined stdout+stderr (for substring assertions on failure
/// output).
pub fn run_cargo_test(crate_dir: &Path) -> CargoTestRun {
    let output = Command::new("cargo")
        .current_dir(crate_dir)
        .args(["test", "--lib"])
        .output()
        .expect("spawning cargo test");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    CargoTestRun {
        success: output.status.success(),
        combined_output: combined,
    }
}
