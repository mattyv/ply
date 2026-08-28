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

/// Copies a *multi-crate* fixture tree verbatim (`wsmember`: a workspace
/// root plus member crates), rewriting the `ply-attrs` path dependency to
/// an absolute path in every `Cargo.toml` under the copy, whatever depth
/// each member sits at -- `copy_fixture` above only handles the common
/// single-crate-at-the-root shape (one fixed relative depth). Returns the
/// scratch dir itself; callers join the member subdirectory they want to
/// point `cargo-ply verify` or `cargo build`/`cargo test` at.
pub fn copy_fixture_tree(name: &str) -> TempDir {
    let src = repo_root().join("tests/fixtures").join(name);
    assert!(src.is_dir(), "no such fixture: {}", src.display());
    let dir = tempfile::tempdir().expect("tempdir");
    copy_dir_recursive(&src, dir.path());
    let ply_attrs_abs = repo_root().join("crates/ply-attrs");
    let rewrote = rewrite_ply_attrs_paths(dir.path(), &ply_attrs_abs);
    assert!(
        rewrote > 0,
        "no Cargo.toml under {} referenced crates/ply-attrs by relative path",
        src.display()
    );
    dir
}

/// Walks `dir` for every `Cargo.toml` and replaces the quoted path value on
/// any `path = "..."` line that mentions `crates/ply-attrs` with `abs`.
/// Returns how many files were rewritten, so the caller can assert it found
/// at least one (the same sanity check `copy_fixture` does with `assert_ne!`).
fn rewrite_ply_attrs_paths(dir: &Path, abs: &Path) -> u32 {
    let mut rewritten_count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            rewritten_count += rewrite_ply_attrs_paths(&path, abs);
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        if !text.contains("crates/ply-attrs") {
            continue;
        }
        let mut out = String::with_capacity(text.len());
        for line in text.lines() {
            if line.contains("crates/ply-attrs") && line.contains("path = \"") {
                let start = line.find("path = \"").unwrap() + "path = \"".len();
                let end = start + line[start..].find('"').unwrap();
                out.push_str(&line[..start]);
                out.push_str(&abs.display().to_string());
                out.push_str(&line[end..]);
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        std::fs::write(&path, out).unwrap();
        rewritten_count += 1;
    }
    rewritten_count
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
    run_verify_with_env(cargo_ply, fixture_dir, Some(engine_timeout_secs), &[])
}

/// The same run with environment overrides, and with `--engine-timeout`
/// optional: passing `None` exercises §6's shape-aware **default** budget,
/// which no test observed until 2026-08-25 (§6 said so itself). The env
/// overrides exist for the engine-absence matrix §9 asks for -- masking an
/// engine means changing what `cargo mutants --version` does, which means
/// changing `PATH`.
pub fn run_verify_with_env(
    cargo_ply: &Path,
    fixture_dir: &Path,
    engine_timeout_secs: Option<u32>,
    env: &[(&str, String)],
) -> VerifyRun {
    let mut args: Vec<String> = vec![
        "verify".into(),
        fixture_dir.to_str().unwrap().into(),
        "--json".into(),
    ];
    if let Some(secs) = engine_timeout_secs {
        args.push("--engine-timeout".into());
        args.push(secs.to_string());
    }
    let mut cmd = Command::new(cargo_ply);
    cmd.args(&args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("spawning cargo-ply verify");
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

/// Runs plain `cargo build` in `crate_dir` -- no `--lib` restriction, so
/// pointed at a *workspace root* it builds every member. Used to prove a
/// multi-crate workspace still builds after `cargo ply verify` ran against
/// one of its members (docs/review-caveats.md N1: the only prior
/// workaround broke exactly this).
pub fn run_cargo_build(crate_dir: &Path) -> CargoTestRun {
    let output = Command::new("cargo")
        .current_dir(crate_dir)
        .arg("build")
        .output()
        .expect("spawning cargo build");
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

/// A private copy of Ply's own source tree, built and edited independently
/// of this checkout -- what the decisive build-identity test needs
/// (The-Ply-Spec.md §5.2a input 11): to change what a *build* of Ply
/// hashes, without ever touching the source this test suite itself runs
/// from.
///
/// Copies everything a workspace build of `ply-cli` reads: the root
/// manifest and lockfile, every crate under `crates/`, and `schema/`
/// (`ply_core::schema` embeds `schema/ply.schema.json` via `include_str!`
/// at a path relative to `ply-core`'s own manifest, so it must exist at the
/// same relative depth in the copy). `target/` lives *inside* the copy too,
/// deliberately -- not shared with this repo's own `target/` -- so the
/// whole thing, source and every build artifact alike, is one tempdir that
/// vanishes on drop and never touches this checkout's build state.
pub struct PlySourceCopy {
    dir: TempDir,
}

impl PlySourceCopy {
    /// `crates/ply-core/src` inside the copy -- edit a file under here
    /// between two calls to `build()` to change what the next build's
    /// identity hashes, honestly: a real second build from real changed
    /// source, not a hand-substituted string.
    pub fn ply_core_src(&self) -> PathBuf {
        self.dir.path().join("crates/ply-core/src")
    }

    /// Builds `cargo-ply` from this copy (its own self-contained
    /// `target/`) and returns the binary's path. Safe to call more than
    /// once on the same copy after editing its source between calls --
    /// each call is an ordinary incremental `cargo build`.
    pub fn build(&self) -> PathBuf {
        let target_dir = self.dir.path().join("target");
        let status = Command::new("cargo")
            .current_dir(self.dir.path())
            .args(["build", "-p", "ply-cli"])
            .env("CARGO_TARGET_DIR", &target_dir)
            .status()
            .expect("spawning `cargo build -p ply-cli` in the Ply source copy");
        assert!(status.success(), "cargo build (Ply source copy) failed");
        target_dir.join("debug/cargo-ply")
    }
}

/// Copies Ply's own source tree into a fresh tempdir. See
/// [`PlySourceCopy`]'s own doc for exactly what is copied and why.
pub fn copy_ply_source() -> PlySourceCopy {
    let root = repo_root();
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["crates", "schema"] {
        let dst = dir.path().join(name);
        std::fs::create_dir_all(&dst).unwrap();
        copy_dir_recursive(&root.join(name), &dst);
    }
    for name in ["Cargo.toml", "Cargo.lock"] {
        std::fs::copy(root.join(name), dir.path().join(name)).unwrap();
    }
    // The workspace root declares `tests/e2e` as an explicit member --
    // cargo requires that manifest to exist even though building `-p
    // ply-cli` never compiles its test binaries -- so its library half
    // (never the `tests/` integration tests, which are not needed to
    // build the binary) comes along too.
    std::fs::create_dir_all(dir.path().join("tests/e2e/src")).unwrap();
    std::fs::copy(
        root.join("tests/e2e/Cargo.toml"),
        dir.path().join("tests/e2e/Cargo.toml"),
    )
    .unwrap();
    copy_dir_recursive(
        &root.join("tests/e2e/src"),
        &dir.path().join("tests/e2e/src"),
    );
    PlySourceCopy { dir }
}
