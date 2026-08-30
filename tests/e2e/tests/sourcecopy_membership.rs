//! The private copy of Ply's own source tree has to be a workspace cargo
//! will load. It used to name the directories to copy by hand, so adding a
//! crate to the real workspace silently produced a copy whose root manifest
//! pointed at members that were not there -- and the only symptom was
//! `cargo build ... failed` from a test about build identity, three layers
//! from the cause.
//!
//! This is the invariant instead: whatever the root manifest declares as a
//! member, the copy has a manifest for.

use std::path::Path;

/// Reads the workspace member patterns straight out of the root manifest,
/// independently of the copy routine under test -- deliberately its own
/// small reader rather than a shared helper, so that a bug in the routine
/// cannot make this check agree with it.
fn declared_members(root: &Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let after = manifest
        .split_once("members")
        .expect("root manifest has no `members` key")
        .1;
    let list = after
        .split_once('[')
        .expect("`members` is not followed by a list")
        .1
        .split_once(']')
        .expect("`members` list is never closed")
        .0;

    let mut out = Vec::new();
    for pattern in list.split('"').skip(1).step_by(2) {
        match pattern.strip_suffix("/*") {
            Some(parent) => {
                for entry in std::fs::read_dir(root.join(parent)).unwrap() {
                    let entry = entry.unwrap();
                    if entry.path().join("Cargo.toml").exists() {
                        out.push(format!(
                            "{}/{}",
                            parent,
                            entry.file_name().to_string_lossy()
                        ));
                    }
                }
            }
            None => out.push(pattern.to_string()),
        }
    }
    out.sort();
    out
}

#[test]
fn the_source_copy_carries_every_crate_the_workspace_declares() {
    let repo = ply_e2e::repo_root();
    let copy = ply_e2e::copy_ply_source();

    let declared = declared_members(&repo);
    assert!(
        declared.len() > 1,
        "read no workspace members from the real manifest -- the test is broken, not the copy"
    );

    let missing: Vec<&String> = declared
        .iter()
        .filter(|m| !copy.root().join(m).join("Cargo.toml").exists())
        .collect();

    assert!(
        missing.is_empty(),
        "the copy of Ply's source is missing manifests the workspace root declares as \
         members, so cargo will refuse to load it: {missing:?}"
    );
}
