//! §9's engine-absence matrix, one entry of it, and the case that showed
//! §1's absence-of-evidence rule was implemented over the wrong thing
//! (adversarial review of the post-004 fixes, D2).
//!
//! `weakspec` declares `checks: [fuzz(64), mutate]`. With cargo-mutants
//! masked, the fuzz check still earns a real `fuzzed(64)` -- so the *verdict*
//! is real evidence, and the absence lands as a status beside it. The first
//! version of the fail-by-default rule read verdict strings only, so this run
//! exited 0: a declared check with no engine behind it, reported clean,
//! against §6's own exit-3 row.

use std::os::unix::fs::PermissionsExt;

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify_with_env};

/// A `cargo` that fails only `cargo mutants ...` and forwards everything
/// else to the real one -- the harness crate still has to build.
fn masked_cargo_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let real = std::env::var("CARGO").expect("cargo sets CARGO for its own test processes");
    let shim = dir.path().join("cargo");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"mutants\" ]; then\n\
             \x20 echo 'cargo-mutants masked for this test' >&2\n\
             \x20 exit 1\n\
             fi\n\
             exec {real} \"$@\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

#[test]
fn a_declared_mutate_check_with_no_engine_is_an_absence_not_a_clean_run() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("weakspec");
    let shim = masked_cargo_dir();
    let path = format!(
        "{}:{}",
        shim.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let run = run_verify_with_env(&cargo_ply, fixture.path(), Some(150), &[("PATH", path)]);

    // The fuzz check really ran, so the verdict is real evidence. The
    // absence is the `mutate` check, and it is recorded as a status.
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the fuzz check is untouched by a missing mutation engine: {}",
        run.json
    );
    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        statuses.contains(&"engine-missing"),
        "a `mutate` check with no cargo-mutants behind it is a missing engine, and must say so \
         by name -- `inconclusive` reads as \"the engine ran and settled nothing\", which is a \
         different fact and the one that used to exit 0: {fn_node}"
    );
    assert!(
        !statuses.contains(&"weak-spec"),
        "nothing survived, because nothing ever ran: {fn_node}"
    );

    let codes: Vec<&str> = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert!(
        codes.contains(&"W0110"),
        "the missing engine keeps its own warning (§3): {}",
        run.json
    );

    assert_eq!(
        run.exit_code,
        Some(3),
        "§6: 3 is missing engine for an explicitly requested check -- `mutate` was requested, and \
         no engine performed it (§1: an absence is a name, not a slot): {}",
        run.json
    );
}
