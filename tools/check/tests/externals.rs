//! docs/plans/external-elements.md — externals: document-local validation.
//! Externals share the component reference namespace but appear in `~>`
//! flow edges (and a fn's `entry:` list) only: a `->` call edge or a `deny`
//! pattern touching one is refused, a flow needs at least one workspace
//! endpoint, `entry:` must name a declared external, and a declared-but-
//! unreferenced external is flagged so a typo in `externals:` or `entry:`
//! never reads as silently fine.

use ply_core::check::{Target, run_checks};
use ply_core::model::parse_document;

fn diagnostics_for(path: &str) -> Vec<ply_core::check::Diagnostic> {
    let yaml = std::fs::read_to_string(path).unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    run_checks(&doc)
}

/// A `->` call edge naming an external: Ply can never verify a call into
/// code it cannot see, so this is refused — the message must say why and
/// point at the two forms that ARE allowed (`~>`, `entry:`).
#[test]
fn external_in_call_edge_is_e0207() {
    let diags = diagnostics_for("tests/fixtures/external_in_call_edge.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0207")
        .expect("expected an E0207 diagnostic");
    assert_eq!(d.target, Target::EdgeIndex(0));
    assert_eq!(
        d.message,
        "\"gateway -> venue\" is not allowed: venue is external (declared under \
         `externals:`), and Ply can never verify a call into code it cannot see — use a \
         data-flow edge (\"venue ~> other : Type\") to show data crossing this boundary, or \
         \"entry: [venue]\" on the function venue can reach"
    );
}

/// Same rule, `deny` side: Ply cannot enforce a ban on a system it cannot
/// observe, so a deny pattern naming an external is refused the same way.
#[test]
fn external_in_deny_is_e0207() {
    let diags = diagnostics_for("tests/fixtures/external_in_deny.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0207")
        .expect("expected an E0207 diagnostic");
    assert_eq!(d.target, Target::DenyIndex(0));
    assert_eq!(
        d.message,
        "\"* -> venue\" is not allowed: venue is external (declared under `externals:`), and \
         Ply cannot enforce a ban on a system it cannot observe — use a data-flow edge \
         (\"venue ~> other : Type\") to show data crossing this boundary, or \"entry: \
         [venue]\" on the function venue can reach"
    );
}

/// `external ~> external`: nothing of this codebase is on either end, so
/// there is nothing for the flow to declare about a boundary this codebase
/// actually has.
#[test]
fn external_to_external_flow_is_e0208() {
    let diags = diagnostics_for("tests/fixtures/external_to_external.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0208")
        .expect("expected an E0208 diagnostic");
    assert_eq!(d.target, Target::EdgeIndex(0));
    assert_eq!(
        d.message,
        "\"venue ~> clock : TimeSync\" connects two externals with nothing of this codebase \
         between them: a data-flow edge needs at least one real component as an endpoint — \
         Ply draws externals to show where this codebase meets the outside world, not to \
         describe the outside world talking to itself"
    );
}

/// `entry:` naming an external that was never declared: the most likely
/// cause is a typo, so the message points at the fix (declare it, or check
/// the spelling) rather than just saying "unknown".
#[test]
fn entry_naming_unknown_external_is_e0209() {
    let diags = diagnostics_for("tests/fixtures/entry_names_unknown_external.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0209")
        .expect("expected an E0209 diagnostic");
    assert_eq!(
        d.target,
        Target::Fn {
            component_path: "oms".to_string(),
            fn_name: "Oms::submit".to_string(),
        }
    );
    assert_eq!(
        d.message,
        "entry: names \"venue\", but no external called \"venue\" is declared — add it under \
         `externals:`, or check the spelling against the names declared there (fn Oms::submit)"
    );
}

/// An external declared but named by no `~>` edge and no `entry:` list:
/// nothing in the document says how it connects, which is exactly the kind
/// of silent gap a reviewer needs flagged rather than discovering by
/// squinting at the picture.
#[test]
fn unreferenced_external_is_w0410() {
    let diags = diagnostics_for("tests/fixtures/external_declared_unused.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "W0410")
        .expect("expected a W0410 diagnostic");
    assert!(d.is_advisory(), "W0410 must be advisory, not a hard error");
    assert_eq!(d.target, Target::External("venue".to_string()));
    assert_eq!(
        d.message,
        "external \"venue\" is declared but never used: it is not named by any `~>` edge or \
         any function's `entry:` list, so nothing in this document says how it connects — add \
         an edge or an entry:, or remove it if it is no longer needed"
    );
}

/// An external whose name collides with a component's: they share one
/// reference namespace, so this is the existing duplicate-name error.
#[test]
fn external_name_colliding_with_a_component_is_e0202() {
    let diags = diagnostics_for("tests/fixtures/external_duplicates_component.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0202")
        .expect("expected an E0202 diagnostic");
    assert_eq!(
        d.message,
        "\"venue\" is declared twice: both as a component and as an external — externals \
         share the component reference namespace, so every name must be unique across both"
    );
}

/// A well-formed external — named by both a flow edge and an `entry:` —
/// produces no diagnostics at all.
#[test]
fn clean_external_produces_no_diagnostics() {
    let diags = diagnostics_for("tests/fixtures/clean_external.ply.yaml");
    assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
}
