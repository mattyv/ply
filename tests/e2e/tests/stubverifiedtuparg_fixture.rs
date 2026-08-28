//! D2 (adversarial review, 2026-08-26): a same-crate callee whose contract
//! Ply cannot build a stand-in for -- here, `g` destructures its parameter
//! with a tuple pattern, which `build_contract_fn` (crates/ply-core/src/
//! harness.rs) only ever accepts as a plain identifier (E0304) -- used to
//! fall through `boundary_plan`'s `if let ... && ...` chain with no `else`
//! and leave nothing behind: no stub, no refusal, no diagnostic, just Kani
//! quietly inlining `g`'s real body into `f`'s proof. That is exactly what
//! §5.5's "always stubbed, never inlined" sentence says can no longer
//! happen, for either of D5's first two branches. This fixture is the
//! reviewer's `tuparg` shape, kept permanently: `g` itself cannot be
//! checked either (its own anchor fails to build for the same reason), and
//! `f`'s call into it must now be refused by name rather than silently
//! descended into.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_same_crate_contracted_callee_ply_cannot_stub_is_refused_by_name_never_inlined() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverifiedtuparg");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let fn_nodes = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let f = fn_nodes
        .iter()
        .find(|n| n["id"] == "f")
        .unwrap_or_else(|| panic!("no `f` node in envelope: {}", run.json));
    let g = fn_nodes
        .iter()
        .find(|n| n["id"] == "g")
        .unwrap_or_else(|| panic!("no `g` node in envelope: {}", run.json));

    assert_eq!(
        g["verdict"], "unclaimed",
        "g's own tuple-pattern parameter means Ply cannot even build g's own proof harness, \
         let alone a stand-in for it -- g earns no evidence either: {}",
        run.json
    );
    assert_eq!(
        f["verdict"], "unclaimed",
        "f calls a callee Ply cannot build a stub for -- before this fix Kani silently inlined \
         g's real body here instead, which must never happen again: {}",
        run.json
    );
    assert_ne!(
        f["verdict"], "bounded(2)",
        "a false clean verdict from silently inlining the unstubbable callee's body is exactly \
         the defect this fixture guards against: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let w0512 = diagnostics
        .iter()
        .find(|d| d["code"] == "W0512" && d["node_id"] == "stubverifiedtuparg::f")
        .unwrap_or_else(|| panic!("no W0512 diagnostic naming f's own refusal: {}", run.json));
    let title = w0512["title"].as_str().unwrap();
    assert!(
        title.contains('g'),
        "the diagnostic must name the callee Ply could not build a stand-in for -- naming only \
         the caller tells a reader nothing they can act on: {title}"
    );

    assert_eq!(
        run.exit_code,
        Some(1),
        "unclaimed is an absence of evidence and fails the run by default (§1, §6): {}",
        run.json
    );
}
