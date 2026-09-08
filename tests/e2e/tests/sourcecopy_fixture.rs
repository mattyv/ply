//! The source copy several tests build from has to be a copy Ply actually
//! builds from -- and the way it stops being one is always the same.
//!
//! Ply embeds files that live outside the crate embedding them: the JSON
//! schema, the spec, and now the skills. Each is reached with a relative
//! path that climbs out of its own crate, so each has to exist at the same
//! relative depth in the copy or the build fails. That list was maintained
//! by hand, and it has now gone stale twice -- once when four crates joined
//! the workspace (fixed by reading the members out of the manifest instead
//! of listing them) and once when the skills were embedded.
//!
//! Both times the only symptom was `cargo build (Ply source copy) failed`
//! from a test about something else entirely, which names nothing and sends
//! whoever reads it looking in the wrong place. So rather than add `skills`
//! to a list and wait for the third time, this walks the real source, finds
//! every file embedded from outside its own crate, and fails naming the ones
//! the copy would not carry.

use std::path::{Path, PathBuf};

use ply_e2e::{copy_ply_source, repo_root};

/// Every `include_str!` / `include_bytes!` target that climbs out of the
/// crate holding it, as a repo-root-relative path.
fn files_embedded_from_outside_their_crate(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for crate_dir in std::fs::read_dir(root.join("crates")).expect("crates/ exists") {
        let crate_dir = crate_dir.unwrap().path();
        if !crate_dir.join("Cargo.toml").exists() {
            continue;
        }
        for src in rust_files(&crate_dir.join("src")) {
            let text = std::fs::read_to_string(&src).unwrap();
            for literal in embedded_paths(&text) {
                // `include_str!` resolves relative to the file doing the
                // including, not the crate root or the working directory.
                let resolved = normalise(&src.parent().unwrap().join(&literal));
                if resolved.starts_with(&crate_dir) {
                    continue; // inside its own crate: the copy carries it already
                }
                let rel = resolved
                    .strip_prefix(root)
                    .unwrap_or_else(|_| {
                        panic!(
                            "{} embeds {literal}, which is outside the repo",
                            src.display()
                        )
                    })
                    .to_string_lossy()
                    .to_string();
                out.push(rel);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The string literal from every `include_str!("..")` / `include_bytes!("..")`.
fn embedded_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for macro_name in ["include_str!(", "include_bytes!("] {
        let mut rest = text;
        while let Some(at) = rest.find(macro_name) {
            rest = &rest[at + macro_name.len()..];
            let Some(open) = rest.find('"') else { break };
            let Some(close) = rest[open + 1..].find('"') else {
                break;
            };
            out.push(rest[open + 1..open + 1 + close].to_string());
            rest = &rest[open + 1 + close..];
        }
    }
    out
}

/// Resolves `..` textually. The real files exist, but the copy's do not
/// yet, so this cannot go through `canonicalize`.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[test]
fn every_file_ply_embeds_from_outside_its_crate_survives_the_source_copy() {
    let root = repo_root();
    let embedded = files_embedded_from_outside_their_crate(&root);
    assert!(
        !embedded.is_empty(),
        "found no cross-crate embedded files at all, so this test is checking nothing -- \
         either the scan broke or `include_str!` stopped being how Ply carries the schema \
         and the spec"
    );

    let copy = copy_ply_source();
    let missing: Vec<&String> = embedded
        .iter()
        .filter(|rel| !copy.root().join(rel).exists())
        .collect();

    assert!(
        missing.is_empty(),
        "the source copy is missing {} file(s) that a build of Ply reads:\n  {}\n\nA build \
         from this copy fails with `cargo build (Ply source copy) failed` and nothing else, \
         which is why this is checked here instead. Add the directory to `copy_ply_source`.",
        missing.len(),
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
