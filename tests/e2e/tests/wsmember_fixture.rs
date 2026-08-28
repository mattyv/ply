//! N1 acceptance (docs/review-caveats.md): `cargo ply verify` against a
//! crate that is a *member* of an ordinary multi-crate workspace, run from
//! inside that member's own directory (`alpha`) -- the second layout the
//! review found unrunnable, whose only workaround (giving `alpha` its own
//! `[workspace]` table) broke `cargo build` for the whole `wsmember`
//! workspace ("multiple workspace roots found in the same workspace").
//!
//! Asserts the run catches the fixture's genuinely seeded bug, that
//! neither `alpha/Cargo.toml` nor the workspace root's `Cargo.toml` was
//! touched, and -- the falsification the task demands -- that `cargo
//! build` at the *workspace root* still builds every member afterwards,
//! shelled out for real.

use ply_e2e::{build_cargo_ply, copy_fixture_tree, run_cargo_build, run_cargo_test, run_verify};

#[test]
fn verify_runs_on_a_workspace_member_and_the_parent_workspace_still_builds() {
    let cargo_ply = build_cargo_ply();
    let tree = copy_fixture_tree("wsmember");
    let root = tree.path();
    let alpha = root.join("alpha");

    let root_toml_before = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let alpha_toml_before = std::fs::read_to_string(alpha.join("Cargo.toml")).unwrap();
    assert!(
        !alpha_toml_before.lines().any(|l| l.trim() == "[workspace]"),
        "alpha must start with no [workspace] table of its own -- it belongs to the parent's"
    );

    // The parent workspace builds cleanly before Ply ever runs -- so any
    // later failure is provably Ply's doing, not a broken fixture.
    let build_before = run_cargo_build(root);
    assert!(
        build_before.success,
        "the wsmember workspace must build before verify runs at all:\n{}",
        build_before.combined_output
    );

    let run = run_verify(&cargo_ply, &alpha, 120);
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "Ply must produce a real verdict on a workspace member, not crash: envelope {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    assert_eq!(
        diagnostics[0]["counterexample"]["inputs"]["x"], "7",
        "must catch the genuinely seeded bug, not merely produce a green tree"
    );

    // Neither manifest was touched: no [workspace] table appeared in
    // alpha's own Cargo.toml, and the workspace root's own members list is
    // byte-for-byte unchanged.
    let alpha_toml_after = std::fs::read_to_string(alpha.join("Cargo.toml")).unwrap();
    assert!(
        !alpha_toml_after.lines().any(|l| l.trim() == "[workspace]"),
        "verify must never give a workspace member its own [workspace] table:\n{alpha_toml_after}"
    );
    let root_toml_after = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert_eq!(
        root_toml_before, root_toml_after,
        "verify must never touch the parent workspace's own Cargo.toml"
    );

    // The falsification the task demands: shell out and prove the parent
    // workspace -- alpha, beta, and their shared Cargo.lock -- still
    // builds, for real, after this run touched one member.
    let build_after = run_cargo_build(root);
    assert!(
        build_after.success,
        "cargo build at the workspace root must still succeed after verify ran against one \
         member (docs/review-caveats.md N1: the old workaround broke exactly this):\n{}",
        build_after.combined_output
    );

    // `cargo test` inside alpha runs the rendered cex test D7 wrote into
    // its own `src/`, which must fail for the right reason (the promise
    // really is still broken) -- not for an infrastructure reason.
    let test_alpha = run_cargo_test(&alpha);
    assert!(
        !test_alpha.success,
        "the rendered cex test must fail before the fix:\n{}",
        test_alpha.combined_output
    );
    assert!(test_alpha.combined_output.contains("postcondition"));

    // --- Apply the fix and confirm the same member now earns a clean pass. ---
    let lib_path = alpha.join("src/lib.rs");
    let fixed = std::fs::read_to_string(&lib_path)
        .unwrap()
        .replace("if x == 7 { x + 1 } else { x }", "x");
    std::fs::write(&lib_path, &fixed).unwrap();

    let run2 = run_verify(&cargo_ply, &alpha, 120);
    assert_eq!(
        run2.json["root"]["verdict"], "fuzzed(256)",
        "envelope: {}",
        run2.json
    );
    assert_eq!(run2.json["diagnostics"].as_array().unwrap().len(), 0);

    let build2 = run_cargo_build(root);
    assert!(build2.success, "{}", build2.combined_output);
    let test2 = run_cargo_test(&alpha);
    assert!(test2.success, "{}", test2.combined_output);
}
