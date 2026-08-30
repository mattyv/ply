//! Regression e2e for the external review of 2026-08-30: the compiler probe
//! ran in the caller's own working directory instead of the crate being
//! verified.
//!
//! Rustup resolves a toolchain from the current directory, and the engines
//! this tool drives run against the target crate -- so a probe taken
//! anywhere else records a compiler that never touched the code being
//! checked. That recorded compiler is a fingerprint input (The-Ply-Spec.md
//! §5.2a): it decides whether a stored result may be carried forward
//! instead of re-earned. Both directions were demonstrated in the real fix
//! -- a genuine toolchain change went unnoticed because the fingerprint
//! tracked the caller's directory instead, and an unrelated directory move
//! alone looked like a toolchain change. This test reproduces the sharper
//! of the two: stale evidence quietly surviving a real compiler change,
//! which is worse than an unnecessary re-check because nothing about the
//! output says anything happened at all.
//!
//! The fixture is verified from a directory that is neither the fixture's
//! own nor pinned to anything -- standing in for "wherever the user's shell
//! happened to be" -- while only the fixture's own pinned toolchain changes
//! between the two runs. `RUSTUP_TOOLCHAIN` is stripped from the child's
//! environment before either run: `cargo test` itself runs under a
//! rustup-proxied `cargo`, which sets that variable for everything it
//! spawns, and its presence would force a single toolchain regardless of
//! directory -- overriding the exact directory-based resolution this test
//! exists to exercise, and masking the bug either way. A plain shell
//! invoking `cargo-ply verify` directly never has this variable set, so
//! removing it here restores the ordinary case rather than engineering
//! around it.

use std::path::Path;
use std::process::Command;

use ply_e2e::{build_cargo_ply, copy_fixture};

struct Run {
    json: serde_json::Value,
    exit_code: Option<i32>,
}

/// `cargo-ply verify <fixture_dir> --json`, run with an explicit, unrelated
/// working directory and no inherited `RUSTUP_TOOLCHAIN` -- see the module
/// doc for why both are necessary to observe this bug at all.
fn run_verify_from(cargo_ply: &Path, fixture_dir: &Path, caller_cwd: &Path) -> Run {
    let output = Command::new(cargo_ply)
        .args([
            "verify",
            fixture_dir.to_str().unwrap(),
            "--json",
            "--engine-timeout",
            "120",
        ])
        .current_dir(caller_cwd)
        .env_remove("RUSTUP_TOOLCHAIN")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "cargo-ply verify did not print valid JSON: {e}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    Run {
        json,
        exit_code: output.status.code(),
    }
}

#[test]
fn a_real_toolchain_change_in_the_target_crate_is_not_missed_because_the_caller_sat_elsewhere() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("toolchainprobe");
    // Never the fixture's own directory, and never pinned to anything
    // itself -- a probe that reads *this* directory instead of the
    // fixture's would report the same, fixed answer across both runs no
    // matter what the fixture's own pin says.
    let caller_cwd = tempfile::tempdir().unwrap();

    std::fs::write(
        fixture.path().join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.97.1\"\n",
    )
    .unwrap();
    let first = run_verify_from(&cargo_ply, fixture.path(), caller_cwd.path());
    assert_eq!(
        first.exit_code,
        Some(0),
        "the first run must earn and record a real result, or nothing below tests anything: {}",
        first.json
    );
    let fn_node = &first.json["root"]["children"][0]["children"][0];
    assert_eq!(fn_node["id"], "safe_increment", "envelope: {}", first.json);
    assert_eq!(
        fn_node["verdict"], "tested",
        "the fixture's own promise is true and `test` should earn it cleanly: {}",
        first.json
    );
    assert_eq!(
        fn_node["reused"],
        serde_json::Value::Null,
        "a first run has nothing to reuse yet: {}",
        first.json
    );

    // A real change to the crate's own compiler -- not to the caller's
    // directory, which never moves across these two runs. Removing the pin
    // falls back to this machine's default toolchain, which earlier
    // (1.97.1, pinned) already differs from.
    std::fs::remove_file(fixture.path().join("rust-toolchain.toml")).unwrap();

    let second = run_verify_from(&cargo_ply, fixture.path(), caller_cwd.path());
    let fn_node2 = &second.json["root"]["children"][0]["children"][0];
    assert_eq!(
        fn_node2["id"], "safe_increment",
        "envelope: {}",
        second.json
    );
    assert_eq!(
        fn_node2["reused"],
        serde_json::Value::Null,
        "the crate's own compiler genuinely changed between these two runs, and a stored result \
         earned under the old one must not be carried forward under the new one -- reusing it \
         here means the probe recorded the caller's directory instead of the crate's, and a real \
         toolchain change went completely unnoticed: {}",
        second.json
    );

    let not_carried = second.json["not_carried_forward"].as_array().unwrap();
    let names_the_compiler = not_carried.iter().any(|entry| {
        entry["node_id"].as_str() == Some("toolchainprobe::safe_increment")
            && entry["because"].as_array().is_some_and(|reasons| {
                reasons
                    .iter()
                    .any(|r| r.as_str() == Some("the compiler and the build target"))
            })
    });
    assert!(
        names_the_compiler,
        "a run that could not carry a result forward because the compiler changed must say so, \
         naming that input specifically -- not just fail to reuse for some unstated reason: {}",
        second.json
    );
}
