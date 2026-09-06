//! A recorded result is only as good as the hash beside it, and that hash
//! has to cover the code the check actually runs (The-Ply-Spec.md §5.2a).
//!
//! `reusereach` proves that for a helper in the crate's own source, and
//! `reuseexamplehelper` for one only a worked example calls. This file
//! proves it for a helper in a **path dependency declared under a platform
//! predicate**. Cargo compiles `[target.'cfg(unix)'.dependencies]` exactly
//! as it compiles `[dependencies]`, and the check runs straight through it,
//! but until 2026-09-06 Ply matched only the two plain table headers. The
//! dependency's whole source tree was hashed nowhere, so rewriting it left
//! the fingerprint byte-identical and served the old pass.
//!
//! Reported by external review 2026-09-06, who asked for a fresh-versus-
//! cached check rather than a unit test of the manifest reader -- reading
//! the table correctly and actually invalidating the record are two
//! different claims, and only the second is the promise.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn platform_src(fixture: &ply_e2e::FixtureCopy) -> std::path::PathBuf {
    fixture.path().join("platform/src/lib.rs")
}

/// The reproduction. Break the body inside the platform-gated dependency;
/// the claim must be checked again, and must report the violation that is
/// really there.
#[test]
fn breaking_a_platform_gated_dependency_re_earns_the_claim_and_finds_the_bug() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusetargetdep");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    let claim = &first.json["root"]["children"][0]["children"][0];
    assert_eq!(claim["id"], "doubled", "envelope: {}", first.json);
    assert_eq!(
        claim["verdict"], "fuzzed(64)",
        "the claim holds on a cold run: {}",
        first.json
    );

    let path = platform_src(&fixture);
    let src = std::fs::read_to_string(&path).unwrap();
    let broken = src.replace(
        "pub fn scale(x: u32) -> u32 {\n    x * 2\n}",
        "pub fn scale(x: u32) -> u32 {\n    x / 2\n}",
    );
    assert_ne!(
        src, broken,
        "the dependency's body must have been rewritten"
    );
    std::fs::write(&path, &broken).unwrap();

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let claim = &second.json["root"]["children"][0]["children"][0];
    assert_eq!(
        claim["reused"],
        serde_json::Value::Null,
        "the recorded result was about a dependency body that is no longer there, so it says \
         nothing about the code as it stands now -- a table header Ply did not recognise is \
         still code Cargo compiles: {}",
        second.json
    );
    assert_eq!(
        claim["verdict"], "violation",
        "and re-running must find the bug the broken dependency really introduces -- a carried \
         forward `fuzzed(64)` here is a green verdict over code nobody checked: {}",
        second.json
    );
}
