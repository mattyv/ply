//! Computes `PLY_BUILD_ID` and hands it to `verify.rs` as `env!("PLY_BUILD_ID")`
//! (The-Ply-Spec.md §5.2a input 11, D14: "Ply's own version").
//!
//! **What was here before, and what it cost.** `PLY_VERSION` used to be
//! `env!("CARGO_PKG_VERSION")` -- the hand-edited `version = "0.1.0"` in
//! `Cargo.toml`. Fourteen false-clean fixes landed on this branch and not
//! one of them moved that string, so every fixed build hashed identically
//! to the broken one it fixed, and a stored result from the broken build
//! kept being reused, diagnostics and all, forever
//! (docs/review-silent-narrowing.md §6: the fourteenth false clean,
//! reproduced word for word by today's binary against yesterday's stored
//! result). A hand-edited version number is not an input, it is a promise
//! nobody is enforcing.
//!
//! **What replaces it.** A build fingerprint over Ply's own first-party
//! source: every `.rs` file under `ply-core/src` and `ply-cli/src` (the two
//! crates that decide what a verdict means), each crate's own `Cargo.toml`
//! (a dependency's version *requirement* is part of what this build ships),
//! and the workspace's `Cargo.lock` (the versions those requirements
//! actually resolved to -- `cargo update` changes Ply's own behaviour the
//! same way §5.2a input 10 says it changes a checked crate's). Hashed with
//! blake3, the same hash already used for every other fingerprint input.
//!
//! **Why source content rather than a git commit.** A commit hash needs
//! `.git` to exist and needs the tree to be clean to mean anything --
//! neither holds for a release tarball, and the task that produced this
//! module explicitly named a dirty tree as a case to weigh. A build cannot
//! happen at all without this source being present and on disk exactly as
//! it will be compiled, dirty or not, tarball or clone -- so hashing it
//! directly has no "unavailable" case to fall back from, and needs no
//! special handling for either scenario. The cost is precision: this hashes
//! raw file bytes, not token streams, so a comment-only edit to Ply's own
//! source invalidates every stored result in every crate Ply has ever
//! checked, exactly the way §5.2a's own "whole-crate" fallback mode
//! over-invalidates on purpose (per that section: "coarser... and it is
//! never wrong"). Erring toward re-checking is the safe direction; erring
//! the other way is the bug this build script exists to close.
//!
//! **The failure mode this must not have.** A mechanism that silently
//! falls back to a constant when its input is unavailable reintroduces
//! exactly the bug above with extra steps. So there is no fallback here:
//! if an expected file or directory cannot be read, this build script
//! panics and the build fails outright. A build that does not know its own
//! identity must not produce a binary at all, rather than produce one that
//! lies about it.

use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set by cargo"),
    );
    // `ply-cli` -> `crates` -> the workspace root.
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("crates/ply-cli sits two directories below the workspace root")
        .to_path_buf();
    let ply_core_dir = workspace_root.join("crates/ply-core");

    let mut hasher = blake3::Hasher::new();

    hash_source_dir(&mut hasher, &manifest_dir.join("src"), &manifest_dir);
    hash_source_dir(&mut hasher, &ply_core_dir.join("src"), &ply_core_dir);
    hash_file(
        &mut hasher,
        "ply-cli/Cargo.toml",
        &manifest_dir.join("Cargo.toml"),
    );
    hash_file(
        &mut hasher,
        "ply-core/Cargo.toml",
        &ply_core_dir.join("Cargo.toml"),
    );
    // The resolved versions every dependency requirement above actually
    // compiled against -- present precisely because this is a workspace
    // build (D9), which is the only supported way to build Ply at all.
    hash_file(
        &mut hasher,
        "Cargo.lock",
        &workspace_root.join("Cargo.lock"),
    );
    // The normative schema, which `ply-core` embeds with `include_str!` and
    // validates every document against. Editing it changes what the binary
    // accepts and rejects; leaving it out let a behaviour change ship under
    // an unchanged build identity, which is the exact silent narrowing this
    // identity exists to refuse (external review, 2026-08-30, demonstrated
    // by editing the schema and watching the id hold still).
    hash_file(
        &mut hasher,
        "schema/ply.schema.json",
        &workspace_root.join("schema/ply.schema.json"),
    );
    // This script decides what all of the above means. A change to the input
    // set is a change to what the identity is worth, so it has to move the
    // identity too -- otherwise dropping an input from the digest would go
    // unrecorded by the very number meant to notice it.
    hash_file(
        &mut hasher,
        "ply-cli/build.rs",
        &manifest_dir.join("build.rs"),
    );

    let build_id = hasher.finalize().to_hex().to_string();
    println!("cargo:rustc-env=PLY_BUILD_ID={build_id}");

    // Rebuild the id whenever anything it covers changes -- and whenever
    // this script itself does, since a change to what gets hashed is
    // exactly the kind of change that must not go unnoticed.
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ply_core_dir.join("Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("Cargo.lock").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("src").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        ply_core_dir.join("src").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("schema/ply.schema.json").display()
    );
}

/// Hashes every `.rs` file under `dir`, recursively, in a stable order --
/// sorted by the path relative to `root` -- so the digest depends only on
/// file names and contents, never on directory-listing order or on where
/// `root` happens to sit on disk. Each entry is length-prefixed the same
/// way `record.rs`'s own `FingerprintInputs::group` writes its fields, so
/// no file's content can be arranged to shift a byte into its neighbour's
/// path and hash the same as a genuinely different tree.
///
/// Nothing here is optional: a directory that cannot be listed, or a file
/// that cannot be read, panics rather than being skipped. Skipping it
/// silently would mean Ply's own identity no longer covers code that is
/// actually part of the build -- the same silent narrowing this whole
/// mechanism exists to refuse in the tool it hashes.
fn hash_source_dir(hasher: &mut blake3::Hasher, dir: &Path, root: &Path) {
    let mut files = collect_rs_files(dir);
    files.sort();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        put(hasher, "path", rel.as_bytes());
        let contents = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("PLY_BUILD_ID: could not read {}: {e}", path.display()));
        put(hasher, "contents", &contents);
    }
}

fn collect_rs_files(dir: &Path) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        panic!(
            "PLY_BUILD_ID: could not list {} ({e}) -- Ply cannot be built without knowing its own \
             source, so this fails the build rather than silently hashing nothing",
            dir.display()
        )
    });
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry
            .unwrap_or_else(|e| panic!("PLY_BUILD_ID: bad dir entry in {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_rs_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}

/// `label` is a stable, repo-relative name (`"Cargo.lock"`, not an absolute
/// path) so the digest depends only on content -- a clone at a different
/// path on disk must hash the same as one at the original path, the same
/// way `record.rs`'s own fingerprint never hashes an absolute path either.
fn hash_file(hasher: &mut blake3::Hasher, label: &str, path: &Path) {
    let contents = std::fs::read(path).unwrap_or_else(|e| {
        panic!(
            "PLY_BUILD_ID: could not read {} ({e}) -- Ply cannot be built without knowing its own \
             identity, so this fails the build rather than silently falling back to a constant",
            path.display()
        )
    });
    put(hasher, "file", label.as_bytes());
    put(hasher, "contents", &contents);
}

/// Same length-prefixing convention as `record.rs`'s `FingerprintInputs::group`:
/// label, byte length, bytes -- so no value can be mistaken for a field
/// boundary.
fn put(hasher: &mut blake3::Hasher, label: &str, value: &[u8]) {
    hasher.update(label.as_bytes());
    hasher.update(&[0]);
    hasher.update(value.len().to_string().as_bytes());
    hasher.update(&[0]);
    hasher.update(value);
    hasher.update(&[0]);
}
