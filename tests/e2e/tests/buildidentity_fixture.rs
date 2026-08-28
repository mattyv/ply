//! The blocker docs/review-silent-narrowing.md §6 names: Ply's own version
//! is one of the inputs that decides whether a stored result may be carried
//! forward instead of re-checked (The-Ply-Spec.md §5.2a input 11, D14), and
//! it had read `0.1.0` for the whole life of this project -- the hand-edited
//! `version` field in `Cargo.toml`, which no commit ever moved. A fixed
//! defect in Ply therefore never reached anyone who had already run it: a
//! stored result matched the same constant either side of the fix and was
//! carried forward, diagnostics and all.
//!
//! `PLY_VERSION` is now `env!("PLY_BUILD_ID")`, computed by `build.rs` from
//! a hash of Ply's own first-party source (`crates/ply-core/src`,
//! `crates/ply-cli/src`, both crates' `Cargo.toml`, and the workspace
//! `Cargo.lock`) rather than a string a person has to remember to bump. This
//! is the decisive test: it builds `cargo-ply` from a private copy of
//! Ply's own source (never this checkout), records a result, edits the
//! copy's own source, rebuilds, and confirms the second build does not
//! reuse the first one's result -- a real rebuild standing in for "a bug in
//! Ply got fixed", never a hand-substituted version string.

use ply_e2e::{copy_fixture, copy_ply_source, run_verify};

#[test]
fn a_result_stored_by_one_build_is_not_carried_forward_by_a_build_with_different_behaviour() {
    let ply_source = copy_ply_source();
    let cargo_ply_v1 = ply_source.build();
    // `build()` always returns the same path inside this copy's own
    // `target/` -- rebuilding overwrites it in place, so the *file this
    // path names* is not the v1 binary any more once `build()` runs again.
    // Snapshot the bytes now, before that happens, or the byte-comparison
    // below would silently compare a file to its own later self.
    let v1_bytes = std::fs::read(&cargo_ply_v1).unwrap();

    let fixture = copy_fixture("resultreuse");
    let first = run_verify(&cargo_ply_v1, fixture.path(), 300);
    assert_eq!(
        first.exit_code,
        Some(0),
        "the first run must earn and record a real result, or nothing below tests anything: {}",
        first.json
    );
    let fns = &first.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[0]["reused"],
        serde_json::Value::Null,
        "a first run has nothing to reuse yet: {}",
        first.json
    );

    // A real second build from real changed source -- not a hand-edited
    // version string. Any token-level change to Ply's own first-party
    // source must move `PLY_BUILD_ID`; this one is deliberately behaviour-
    // neutral (a comment), which is the honest, conservative cost §5.2a
    // documents for hashing raw source rather than a token stream: Ply's
    // own identity over-invalidates on a comment-only edit the same way
    // its own "whole-crate" fallback mode does for a checked crate.
    let record_rs = ply_source.ply_core_src().join("record.rs");
    let original = std::fs::read_to_string(&record_rs).unwrap();
    std::fs::write(
        &record_rs,
        format!("{original}\n// a fix landed here, in spirit -- 2026-08-28\n"),
    )
    .unwrap();
    // `build.rs`'s own `rerun-if-changed` is mtime-based (Cargo has no
    // content-based rerun trigger), and this edit and the first build can
    // land inside the same filesystem timestamp tick when a test runs this
    // fast -- so bump the mtime explicitly rather than trusting it moved on
    // its own. This is a test-harness concern, not a defect in `build.rs`:
    // a real edit-then-rebuild by a person is never microseconds apart.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&record_rs)
        .unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(2))
        .unwrap();
    drop(file);

    let cargo_ply_v2 = ply_source.build();
    let v2_bytes = std::fs::read(&cargo_ply_v2).unwrap();
    // Compared as lengths + a short digest, never as the raw `Vec<u8>`
    // itself -- an `assert_ne!` on two multi-megabyte binaries prints both
    // in full on failure, which is its own kind of unreadable disclosure.
    assert!(
        v1_bytes.len() != v2_bytes.len() || v1_bytes != v2_bytes,
        "the two builds must actually differ (both were {} bytes), or this test proves nothing",
        v1_bytes.len()
    );

    let second = run_verify(&cargo_ply_v2, fixture.path(), 300);
    let fns2 = &second.json["root"]["children"][0]["children"];
    assert_eq!(fns2[0]["id"], "safe_increment", "envelope: {}", second.json);
    assert_eq!(
        fns2[0]["reused"],
        serde_json::Value::Null,
        "a build with different behaviour must not carry forward a result the previous build \
         earned -- the stored fingerprint has to disagree with today's `PLY_BUILD_ID`, or a fix \
         to Ply never reaches anyone who already ran it (docs/review-silent-narrowing.md §6): {}",
        second.json
    );
    assert_eq!(
        fns2[0]["verdict"], "bounded(2)",
        "and it must still earn a real verdict, not merely fail to reuse: {}",
        second.json
    );

    let not_carried = second.json["not_carried_forward"].as_array().unwrap();
    assert!(
        !not_carried.is_empty(),
        "a run that could not carry a stored result forward must say so, naming which input \
         moved (§5.2a): {}",
        second.json
    );
}

/// The other half of the same property, using the same two builds: a run
/// of the *same* binary the first run used must still reuse. Warm reuse is
/// a real, measured property (0.028s vs 11.8s in this project's own
/// history) and a build-identity fix that broke it would be the feature
/// deleted with extra steps.
#[test]
fn two_runs_of_the_same_build_still_reuse() {
    let ply_source = copy_ply_source();
    let cargo_ply = ply_source.build();
    let fixture = copy_fixture("resultreuse");

    let first = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(first.exit_code, Some(0), "envelope: {}", first.json);

    let second = run_verify(&cargo_ply, fixture.path(), 300);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[0]["reused"], true,
        "the same binary, run twice, must still reuse -- an identity that changed on every run \
         regardless of content would destroy the feature this fixes: {}",
        second.json
    );
}
