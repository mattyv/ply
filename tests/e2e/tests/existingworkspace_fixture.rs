//! N1 regression (docs/review-caveats.md): a crate that already declares
//! its own `[workspace]` table must keep the *original* mechanism exactly
//! -- the harness registered as a member of that same workspace
//! (`harness_crate::ensure_workspace_member`) -- never routed onto the new
//! standalone-harness path the N1 fix adds for crates that have no
//! `[workspace]` of their own. Same seeded bug as `plain`/`wsmember`, so
//! all three fixtures earn the identical verdict and only the mechanism
//! underneath differs.

use ply_e2e::{build_cargo_ply, copy_fixture, run_cargo_build, run_cargo_test, run_verify};

#[test]
fn a_crate_that_already_has_workspace_keeps_the_original_registered_member_mechanism() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("existingworkspace");

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

    // Unchanged behaviour: the harness is registered as a member of the
    // crate's own existing workspace...
    let cargo_toml = std::fs::read_to_string(fixture.path().join("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.contains(
            "members = [\".\", \"target/ply/fuzz/ply-fixture-existingworkspace-ply-harness\"]"
        ),
        "the harness must still be registered into the crate's existing workspace, exactly as \
         before this fix:\n{cargo_toml}"
    );

    // ...and, unlike the `plain`/`wsmember` fixtures, the harness crate
    // itself carries *no* `[workspace]` table of its own -- it relies on
    // being a member of the target's, not on standing alone.
    let harness_cargo_toml = fixture
        .path()
        .join("target/ply/fuzz/ply-fixture-existingworkspace-ply-harness/Cargo.toml");
    let harness_text = std::fs::read_to_string(&harness_cargo_toml).unwrap();
    assert!(
        !harness_text.lines().any(|l| l.trim() == "[workspace]"),
        "a harness registered into an existing workspace must not also declare its own:\n{harness_text}"
    );

    let build = run_cargo_build(fixture.path());
    assert!(build.success, "{}", build.combined_output);
    let test = run_cargo_test(fixture.path());
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

    let build2 = run_cargo_build(fixture.path());
    assert!(build2.success, "{}", build2.combined_output);
    let test2 = run_cargo_test(fixture.path());
    assert!(test2.success, "{}", test2.combined_output);
}
