//! Fixture regression: a build script is code the build runs, and what it
//! emits reaches the compilation, so it belongs in what the fingerprint
//! covers (The-Ply-Spec.md §5.2a). Reported by external review 2026-09-06.

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
