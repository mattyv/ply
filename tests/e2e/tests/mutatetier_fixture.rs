//! M4 acceptance: `mutate` declared with no `test`/`fuzz` entry in the same
//! checks list is `E0504` (D12) -- caught as a config error before any
//! engine runs, not attempted and left to fail confusingly.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn mutate_without_a_kill_signal_is_e0504() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("mutatetier");

    // Fast: E0504 is caught in config validation, before any engine (Kani,
    // proptest, cargo-mutants) is ever invoked -- a short timeout is enough
    // and also proves nothing slow ran.
    let run = run_verify(&cargo_ply, fixture.path(), 20);
    assert_eq!(run.json["root"]["verdict"], "unclaimed", "envelope: {}", run.json);
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "E0504");
    assert_eq!(diag["severity"], "error");
    let title = diag["title"].as_str().unwrap();
    assert!(title.contains("mutate"), "{title}");
    assert!(title.contains("test") && title.contains("fuzz"), "must name both remedies: {title}");
    let fixes = diag["fixes"].as_array().unwrap();
    assert!(!fixes.is_empty(), "§8: a non-result diagnostic SHOULD populate fixes: {}", run.json);

    // No harness crate should ever have been created for this fn: E0504 is
    // a config-time refusal, not something that runs and then fails.
    assert!(!fixture.path().join("target/ply/fuzz").exists());
}
