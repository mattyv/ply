//! N1 regression (docs/review-caveats.md): a crate that already declares
//! its own `[workspace]` table still gets the harness registered as a
//! member of that same workspace while the run needs it
//! (`harness_crate::ensure_workspace_member`), rather than being routed
//! onto the standalone-harness path used for crates that have no
//! `[workspace]` of their own. Same seeded bug as `plain`/`wsmember`, so
//! all three fixtures earn the identical verdict and only the mechanism
//! underneath differs.
//!
//! What this test pins hardest is what the *user* is left holding. The
//! registration edits a file they own, so it lasts exactly as long as the
//! run: afterwards their `Cargo.toml` is byte-for-byte what they wrote, and
//! the harness -- no longer a member of anything -- has been given its own
//! `[workspace]` table so the failing test Ply just generated is still
//! runnable. Both halves matter. Restoring the manifest without standing
//! the harness back up would orphan it, and a counterexample you cannot run
//! is a counterexample you have to take on trust.

use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_build, run_cargo_test, run_verify};

#[test]
fn a_crate_that_already_has_workspace_keeps_the_original_registered_member_mechanism() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("existingworkspace");
    let pristine_manifest = std::fs::read_to_string(fixture.path().join("Cargo.toml")).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "envelope: {}",
        run.json
    );
    assert_eq!(
        run.json["diagnostics"][0]["counterexample"]["inputs"]["x"], "7",
        "must catch the genuinely seeded bug"
    );

    // The run is over, so the user's manifest is the user's again. Byte
    // equality, not "the entry is gone": whitespace and key order they
    // wrote count too.
    let cargo_toml = std::fs::read_to_string(fixture.path().join("Cargo.toml")).unwrap();
    assert_eq!(
        cargo_toml, pristine_manifest,
        "verify must leave the crate's own Cargo.toml exactly as it found it"
    );

    // The harness was a member for the duration and is not one now, so it
    // has to have been stood back up as its own workspace root -- otherwise
    // it belongs to nothing and cannot be built.
    let harness_dir = fixture
        .path()
        .join("target/ply/fuzz/ply-fixture-existingworkspace-ply-harness");
    let harness_text = std::fs::read_to_string(harness_dir.join("Cargo.toml")).unwrap();
    assert!(
        harness_text.lines().any(|l| l.trim() == "[workspace]"),
        "a harness that is no longer a member must stand on its own, or the generated test \
         cannot be run at all:\n{harness_text}"
    );

    let build = run_cargo_build(fixture.path());
    assert!(build.success, "{}", build.combined_output);
    let test = run_cargo_test(&harness_dir);
    assert!(
        !test.success,
        "the rendered cex test must fail before the fix:\n{}",
        test.combined_output
    );

    // --- Apply the fix and confirm the same crate now earns a clean pass. ---
    let fixed = fixture
        .read_lib_rs()
        .replace("if x == 7 { x + 1 } else { x }", "x");
    fixture.write_lib_rs(&fixed);

    let run2 = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run2.json["root"]["verdict"], "fuzzed(256)",
        "envelope: {}",
        run2.json
    );
    assert_eq!(run2.json["diagnostics"].as_array().unwrap().len(), 0);

    assert_eq!(
        std::fs::read_to_string(fixture.path().join("Cargo.toml")).unwrap(),
        pristine_manifest,
        "the second run must leave it alone too -- once is luck, twice is the guard"
    );

    let build2 = run_cargo_build(fixture.path());
    assert!(build2.success, "{}", build2.combined_output);
    let test2 = run_cargo_test(&harness_dir);
    assert!(test2.success, "{}", test2.combined_output);
}
