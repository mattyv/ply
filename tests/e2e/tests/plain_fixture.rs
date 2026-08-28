//! N1 acceptance (docs/review-caveats.md): `cargo ply verify` against the
//! exact layout `cargo new --lib` produces -- no `[workspace]` table
//! anywhere in the crate's own `Cargo.toml`. Before this fix, this call
//! never got as far as checking anything: `harness_crate::ensure_workspace_member`
//! bailed out with a raw `anyhow::Error` and a stack trace the instant a
//! `fuzz`/`test` check needed the generated harness crate.
//!
//! Asserts, in order: the run actually produces a verdict (not a crash); it
//! catches the fixture's genuinely seeded bug; Ply never wrote a
//! `[workspace]` table into the user's own `Cargo.toml` to make that
//! happen; and the user's `cargo build`/`cargo test` still succeed
//! afterwards -- shelled out for real, not read back from Ply's own report
//! (the task's explicit falsification requirement).

use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_build, run_cargo_test, run_verify};

#[test]
fn verify_runs_on_an_ordinary_crate_with_no_workspace_table_and_catches_the_seeded_bug() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("plain");

    // The crate genuinely has no `[workspace]` table before Ply touches it
    // -- the exact precondition the old code bailed out on.
    let cargo_toml_before = fixture.path().join("Cargo.toml").to_owned();
    let before_text = std::fs::read_to_string(&cargo_toml_before).unwrap();
    assert!(
        !before_text.lines().any(|l| l.trim() == "[workspace]"),
        "fixture must start with no [workspace] table, the exact shape this fixture exists to \
         cover:\n{before_text}"
    );

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "Ply must produce a real verdict on an ordinary crate, not crash before checking \
         anything: envelope {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["engine"], "proptest");
    assert_eq!(
        diag["counterexample"]["inputs"]["x"], "7",
        "must catch the genuinely seeded bug, not merely produce a green tree"
    );

    // Ply must not have added a `[workspace]` table to the user's own
    // Cargo.toml to make this work (docs/review-caveats.md N1: that was
    // the only workaround, and it breaks a multi-crate project's build).
    let after_text = std::fs::read_to_string(&cargo_toml_before).unwrap();
    assert!(
        !after_text.lines().any(|l| l.trim() == "[workspace]"),
        "verify must never write a [workspace] table into the user's own manifest:\n{after_text}"
    );

    // The harness crate exists and is its own isolated workspace root
    // instead (the mechanism that replaces workspace-member registration
    // for a crate that never had one).
    let harness_cargo_toml = fixture
        .path()
        .join("target/ply/fuzz/ply-fixture-plain-ply-harness/Cargo.toml");
    assert!(
        harness_cargo_toml.is_file(),
        "expected a generated harness crate at {}",
        harness_cargo_toml.display()
    );
    let harness_text = std::fs::read_to_string(&harness_cargo_toml).unwrap();
    assert!(
        harness_text.lines().any(|l| l.trim() == "[workspace]"),
        "the harness crate must be its own workspace root when the target crate has none:\n{harness_text}"
    );

    // The falsification the task demands: shell out and prove the user's
    // own crate still *builds* (Ply's edits did not corrupt the manifest or
    // the workspace resolution). `cargo build` never runs the rendered cex
    // test, only compiles, so it must succeed even now.
    let build = run_cargo_build(fixture.path());
    assert!(
        build.success,
        "cargo build must still succeed in the user's crate after verify:\n{}",
        build.combined_output
    );
    // `cargo test --lib` *does* run the rendered cex test D7 wrote into
    // this crate's own `src/`, and that test asserts the still-broken
    // promise directly -- it must fail, for the right reason, exactly as
    // the fuzzbug fixture's own oracle expects (§9's cex validity oracle).
    let test = run_cargo_test(fixture.path());
    assert!(
        !test.success,
        "the rendered cex test must fail before the fix:\n{}",
        test.combined_output
    );
    assert!(
        test.combined_output.contains("postcondition"),
        "failure output must name what a postcondition is (newbie bar):\n{}",
        test.combined_output
    );

    // --- Apply the fix and confirm the same crate now earns a clean pass. ---
    let fixed = fixture
        .read_lib_rs()
        .replace("if x == 7 { x + 1 } else { x }", "x");
    assert_ne!(
        fixed,
        fixture.read_lib_rs(),
        "fix must actually change the source"
    );
    fixture.write_lib_rs(&fixed);

    let run2 = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run2.json["root"]["verdict"], "fuzzed(256)",
        "envelope: {}",
        run2.json
    );
    assert_eq!(run2.json["diagnostics"].as_array().unwrap().len(), 0);

    let build2 = run_cargo_build(fixture.path());
    assert!(build2.success, "{}", build2.combined_output);
    let test2 = run_cargo_test(fixture.path());
    assert!(test2.success, "{}", test2.combined_output);
}
