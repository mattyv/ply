//! D5's third branch (The-Ply-Spec.md §5.5, amended 2026-08-25 after vetting
//! 004): when a `bounded` check's function calls a callee no contract
//! describes, Ply refuses to descend into that callee. Before this rule
//! existed, Kani inlined the real body -- on a small helper that means the
//! caller earns a `bounded(2)` whose meaning silently includes code nobody
//! vouched for; on vetting 004's two-year-old `BTreeMap` lookup it means an
//! 11-minute search that reports nothing at all.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_callee_with_no_contract_is_refused_by_name_never_silently_inlined() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("unclaimedcallee");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert_eq!(
        verdict, "unclaimed",
        "a `bounded` check that would have to descend into an unclaimed callee earns no \
         evidence -- it must never report a proof whose meaning includes a body no contract \
         describes: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "W0512", "envelope: {}", run.json);
    let title = diag["title"].as_str().unwrap();
    assert!(
        title.contains("legacy_rate"),
        "the diagnostic must name the callee that was not descended into -- naming only the \
         caller tells a reader nothing they can act on (§8's non-result rule): {title}"
    );
    assert!(
        title.contains("tiered_fee"),
        "and it must name the caller whose check this was: {title}"
    );
    assert!(
        diag["counterexample"].is_null(),
        "nothing was checked, so there is no witness: {diag}"
    );
    assert!(
        !diag["fixes"].as_array().unwrap().is_empty(),
        "§8: a non-result diagnostic SHOULD carry the concrete options a repair needs: {diag}"
    );

    // No Kani harness should have been written at all: the refusal is a
    // call-graph decision, taken before any engine starts.
    assert!(
        !fixture.path().join("src/ply_generated.rs").exists(),
        "the refusal must happen before harness codegen, not after a Kani run"
    );

    assert_eq!(
        run.exit_code,
        Some(1),
        "absence of evidence fails the run by default (§1, §6): {}",
        run.json
    );
}
