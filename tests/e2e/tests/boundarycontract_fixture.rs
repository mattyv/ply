//! D5's second branch reached from an unclaimed callee (The-Ply-Spec.md
//! §5.5): a contract declared for the callee in `ply.yaml` is assumed, the
//! callee is stood in for rather than descended into, and the caller's
//! verdict is `conditional` -- real evidence about the contract, resting on
//! a named assumption that is owed evidence until something exercises it.
//!
//! Before this landed, `ply-core`'s `FnClaim` had no `ensures` field at all,
//! so serde silently ate the declaration (vetting 004 finding 7) and the
//! same fixture reported a plain, unqualified `bounded(2)` -- a proof that
//! had quietly inlined the real body with nothing recording that it had.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_declared_contract_for_an_unclaimed_callee_earns_a_conditional_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("boundarycontract");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "bounded(2)",
        "the proof is real -- it is the assumption that is conditional, not the evidence: {}",
        run.json
    );

    let fn_node = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(fn_node["id"], "tiered_fee", "envelope: {}", run.json);
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        statuses.contains(&"conditional"),
        "a verdict that assumed a contract must carry the `conditional` status (D5/D6): {fn_node}"
    );
    assert!(
        statuses.contains(&"owed-evidence"),
        "an assumption nothing has exercised is owed evidence, not settled (§5.5): {fn_node}"
    );
    let root_statuses: Vec<&str> = run.json["root"]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        root_statuses.contains(&"conditional"),
        "`conditional` propagates upward as a status (D6) -- the trust story must be visible \
         without expanding every fn: {}",
        run.json
    );

    // `legacy_rate` declares a contract and no checks: it is a boundary
    // contract declaration, not a claim, so it earns no node.
    let fn_nodes = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    assert_eq!(fn_nodes.len(), 1, "envelope: {}", run.json);

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "W0511", "envelope: {}", run.json);
    let title = diag["title"].as_str().unwrap();
    assert!(
        title.contains("legacy_rate"),
        "the assumption must name the callee it stands in for: {title}"
    );
    assert!(
        title.contains("owed evidence"),
        "and must say the assumption is owed evidence, not settled: {title}"
    );
    let assumptions = diag["assumptions"].as_array().unwrap();
    assert_eq!(assumptions.len(), 1, "diag: {diag}");
    assert_eq!(assumptions[0]["kind"], "assumed_contract");
    assert_eq!(assumptions[0]["fn"], "legacy_rate");
    assert_eq!(
        assumptions[0]["verdict"], "unclaimed",
        "nothing has checked `legacy_rate` -- saying so is what keeps the assumption honest: {diag}"
    );

    // The generated harness must stand in for the callee, never call it.
    let generated = std::fs::read_to_string(fixture.path().join("src/ply_generated.rs")).unwrap();
    assert!(
        generated.contains("#[kani::stub(legacy_rate, ply_stub_legacy_rate)]"),
        "generated harness:\n{generated}"
    );
}
