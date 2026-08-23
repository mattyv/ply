//! Exercises the actual `ply-check` binary (not just the library), since
//! the exit-code contract (Ply-Spec.md §6: 0 clean, 1 violations, 2 tool error)
//! and "never emits SVG" live in `main.rs`, not in `run_checks`.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ply-check"))
        .args(args)
        .output()
        .expect("ply-check should run")
}

#[test]
fn clean_fixture_exits_zero_with_no_output() {
    let out = run(&["../../vetting/001-spsc-disruptor.ply.yaml"]);
    assert!(out.status.success(), "expected exit 0, got: {out:?}");
    assert!(
        out.stdout.is_empty(),
        "clean run should print nothing, got: {out:?}"
    );
}

#[test]
fn violating_fixture_exits_nonzero_and_names_the_code() {
    let out = run(&["tests/fixtures/mutate_without_test_or_fuzz.ply.yaml"]);
    assert!(!out.status.success(), "expected nonzero exit, got: {out:?}");
    assert_eq!(
        out.status.code(),
        Some(1),
        "violations should exit 1, got: {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The exact plain-language wording (§ "diagnostics read as plain
    // language"), not just a substring check for the code — a future edit
    // that quietly reverts to jargon must fail here.
    assert_eq!(
        stdout.trim_end(),
        "E0504: mutate has nothing to catch its planted bugs: add a test or fuzz check beside \
         it — mutation testing works by deliberately breaking the code and checking those \
         checks notice (fn slot)"
    );
    assert!(
        !stdout.contains("<svg"),
        "check mode must never emit SVG, got: {stdout}"
    );
}

#[test]
fn missing_file_is_a_tool_error() {
    let out = run(&["tests/fixtures/does_not_exist.ply.yaml"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing file should exit 2 (tool error), got: {out:?}"
    );
}
