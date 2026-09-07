//! Fixture regressions: a build script can change compiled behaviour while
//! its own source remains byte-identical. Its inputs cannot be enumerated
//! soundly, so claims in that first-party closure are never reused.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify, run_verify_with_env};

fn assert_cold_pass(run: &ply_e2e::VerifyRun) {
    let claim = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(claim["id"], "answer", "envelope: {}", run.json);
    assert_eq!(claim["verdict"], "fuzzed(64)", "envelope: {}", run.json);
}

fn assert_fresh_violation(run: &ply_e2e::VerifyRun, changed: &str) {
    let claim = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(
        claim["reused"],
        serde_json::Value::Null,
        "the build script consumed a changed {changed}, so its old result says nothing about \
         this build: {}",
        run.json
    );
    assert_eq!(
        claim["verdict"], "violation",
        "a fresh run must expose the promise the changed {changed} breaks: {}",
        run.json
    );
}

#[test]
fn a_file_consumed_by_an_unchanged_build_script_cannot_reuse_a_pass() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusebuildinputs");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    assert_cold_pass(&first);
    let lock_path = fixture.path().join("ply.lock");
    if lock_path.exists() {
        let lock: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(lock_path).unwrap()).unwrap();
        assert_eq!(
            lock["results"],
            serde_json::json!({}),
            "a build-script-backed result must not be offered for reuse later: {lock}"
        );
    }
    std::fs::write(fixture.path().join("answer.txt"), "8\n").unwrap();

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    assert_fresh_violation(&second, "file");
}

#[test]
fn an_environment_value_consumed_by_an_unchanged_build_script_cannot_reuse_a_pass() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusebuildinputs");

    let first = run_verify_with_env(
        &cargo_ply,
        fixture.path(),
        Some(120),
        &[("PLY_FIXTURE_OFFSET", "0".to_string())],
    );
    assert_cold_pass(&first);

    let second = run_verify_with_env(
        &cargo_ply,
        fixture.path(),
        Some(120),
        &[("PLY_FIXTURE_OFFSET", "1".to_string())],
    );
    assert_fresh_violation(&second, "environment value");
}
