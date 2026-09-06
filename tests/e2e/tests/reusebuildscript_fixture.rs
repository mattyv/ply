//! A recorded result is only as good as the hash beside it, and that hash
//! has to cover the code the check actually runs (The-Ply-Spec.md §5.2a).
//!
//! The first-party source set collected everything under `src/` and nothing
//! else. A build script is code the build runs, and what it emits reaches
//! the checked crate: `cargo:rustc-env` makes `env!(..)` compile to whatever
//! the script said. So rewriting `build.rs` changed what the checked
//! function returns with every line under `src/` byte-identical -- the
//! fingerprint did not move, and a stored pass was served over a function
//! that now breaks its promise.
//!
//! Detecting the `env!` macro did not help: that widens the scope to the
//! whole crate, and the whole crate was the same incomplete set of files.
//!
//! Reported by external review, 2026-09-06.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn rewriting_the_build_script_re_earns_the_claim_and_finds_the_bug() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusebuildscript");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    let claim = &first.json["root"]["children"][0]["children"][0];
    assert_eq!(claim["id"], "answer", "envelope: {}", first.json);
    assert_eq!(
        claim["verdict"], "fuzzed(64)",
        "the promise holds on a cold run: {}",
        first.json
    );

    let path = fixture.path().join("build.rs");
    let script = std::fs::read_to_string(&path).unwrap();
    let broken = script.replace("PLY_FIXTURE_ANSWER=7", "PLY_FIXTURE_ANSWER=8");
    assert_ne!(script, broken, "the build script must have been rewritten");
    std::fs::write(&path, &broken).unwrap();

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let claim = &second.json["root"]["children"][0]["children"][0];
    assert_eq!(
        claim["reused"],
        serde_json::Value::Null,
        "the recorded result was about a function that compiled against 7 and now compiles \
         against 8, so it says nothing about the code as it stands -- a build script is code \
         the build runs: {}",
        second.json
    );
    assert_eq!(
        claim["verdict"], "violation",
        "and re-running must find the broken promise the new value really introduces: {}",
        second.json
    );
}
