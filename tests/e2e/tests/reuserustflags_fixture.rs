//! Fixture regression: compiler flags are part of the build and appear
//! nowhere in the source, so they belong in what the fingerprint covers
//! (The-Ply-Spec.md §5.2a). Reported by external review 2026-09-06.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify, run_verify_with_env};

#[test]
fn a_run_under_different_compiler_flags_does_not_reuse_the_old_pass() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reuserustflags");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    let claim = &first.json["root"]["children"][0]["children"][0];
    assert_eq!(claim["id"], "answer", "envelope: {}", first.json);
    assert_eq!(
        claim["verdict"], "fuzzed(64)",
        "the promise holds with no flags set: {}",
        first.json
    );

    // The same source, the same compiler, the same target -- and a build
    // that behaves differently.
    let second = run_verify_with_env(
        &cargo_ply,
        fixture.path(),
        Some(120),
        &[("RUSTFLAGS", "--cfg broken".to_string())],
    );
    let claim = &second.json["root"]["children"][0]["children"][0];
    assert_eq!(
        claim["reused"],
        serde_json::Value::Null,
        "the recorded result was earned by a build that compiled a different body, so it says \
         nothing about this one -- flags are part of the build: {}",
        second.json
    );
    assert_eq!(
        claim["verdict"], "violation",
        "and re-running under those flags must find the promise the flag really breaks: {}",
        second.json
    );
}

#[test]
fn compiler_flags_from_cargo_config_do_not_reuse_the_old_pass() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reuserustflags");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    let claim = &first.json["root"]["children"][0]["children"][0];
    assert_eq!(claim["verdict"], "fuzzed(64)", "envelope: {}", first.json);

    let cargo_dir = fixture.path().join(".cargo");
    std::fs::create_dir_all(&cargo_dir).unwrap();
    std::fs::write(
        cargo_dir.join("config.toml"),
        "[build]\nrustflags = [\"--cfg\", \"broken\"]\n",
    )
    .unwrap();

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let claim = &second.json["root"]["children"][0]["children"][0];
    assert_eq!(
        claim["reused"],
        serde_json::Value::Null,
        "Cargo now compiles a different body, so the old result must not be reused: {}",
        second.json
    );
    assert_eq!(
        claim["verdict"], "violation",
        "a fresh run must expose the promise broken by Cargo's configured flags: {}",
        second.json
    );
}
