//! Regression for the external review of 2026-08-30: `build.rs` hashed
//! Ply's own Rust sources and manifests into `PLY_BUILD_ID`, but never
//! `schema/ply.schema.json` -- the file `ply-core` embeds with
//! `include_str!` and validates every document against. Editing the schema
//! therefore changed what the binary accepts and rejects while `--version`
//! kept reporting an unchanged build identity, which is exactly the silent
//! narrowing this identity exists to catch (The-Ply-Spec.md §5.2a input 11):
//! a stored result earned under the old schema could look current under a
//! binary that no longer agrees with it.
//!
//! A full behavioural proof already exists for the sibling case of this bug
//! -- moving Ply's own `.rs` sources -- in
//! `tests/e2e/tests/buildidentity_fixture.rs`, which rebuilds a private
//! copy of Ply's own source tree twice and compares the resulting
//! identities. Repeating that for the schema too is honest, but it pays for
//! two more full workspace builds (minutes, not seconds) to prove something
//! that is really a property of three lines of `build.rs` text, not of the
//! hashing mechanism itself -- the existing e2e already exercises that
//! mechanism end to end. So this test reads `build.rs` at the source level
//! and asserts the schema path is part of what gets hashed and part of what
//! triggers a rebuild. That is a cheaper claim than "the identity changes
//! when the schema changes", but it is the same claim at a level that costs
//! milliseconds: `build.rs` cannot compute a different digest without
//! reading a different set of bytes, and this pins exactly which bytes are
//! in that set. It fails the instant the schema line (or its rebuild
//! trigger) is removed, which is the only way this defect comes back.

use std::path::Path;

/// Returns the byte range of `build.rs` that is *actually hashed* --
/// everything before the digest is finalized. A mention of the schema path
/// after this point (in a trailing comment, say) would not move
/// `PLY_BUILD_ID` at all, so it must not satisfy this test.
fn hashed_region(source: &str) -> &str {
    let finalize_at = source.find("hasher.finalize()").unwrap_or_else(|| {
        panic!(
            "build.rs no longer calls hasher.finalize() -- rewrite this test against whatever \
             replaced it"
        )
    });
    &source[..finalize_at]
}

/// True if `needle` appears within `window` bytes after some occurrence of
/// `marker`. Used to confirm a `rerun-if-changed` line names the schema
/// without depending on the exact formatting of the `println!` call that
/// builds it (the path is assembled via `.join(...).display()`, not typed
/// out verbatim next to the word `rerun-if-changed`).
fn near_after(haystack: &str, marker: &str, needle: &str, window: usize) -> bool {
    haystack.match_indices(marker).any(|(idx, _)| {
        let start = idx + marker.len();
        let end = (start + window).min(haystack.len());
        haystack[start..end].contains(needle)
    })
}

#[test]
fn build_rs_hashes_the_embedded_schema_into_the_build_identity() {
    let build_rs_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs");
    let source = std::fs::read_to_string(&build_rs_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", build_rs_path.display()));

    assert!(
        hashed_region(&source).contains("schema/ply.schema.json"),
        "build.rs must hash schema/ply.schema.json into PLY_BUILD_ID: ply-core embeds that file \
         and validates every document against it, so editing it changes what the tool accepts \
         and rejects. Without this, --version reports an unchanged identity across a real \
         behaviour change, and a stored result survives a schema edit that should have \
         invalidated it."
    );

    assert!(
        near_after(&source, "rerun-if-changed", "schema", 200),
        "build.rs hashes the schema but must also tell Cargo to rerun when it changes -- \
         Cargo's rerun triggers are opt-in per path, so an incremental build that never touched \
         anything named would keep serving the old digest baked in from a stale build.rs run: {}",
        source
    );
}
