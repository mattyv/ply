//! `ply-check`'s document-local rules (The-Ply-Spec.md §5.1a, §5.1, §5.6) — the
//! subset of `cargo ply check` that needs no anchored Rust code.

use ply_core::check::{Target, run_checks};
use ply_core::model::parse_document;

fn diagnostics_for(path: &str) -> Vec<ply_core::check::Diagnostic> {
    let yaml = std::fs::read_to_string(path).unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    run_checks(&doc)
}

const KNOWN_CODES: &[&str] = &[
    "E0202", "E0203", "E0205", "E0206", "E0207", "E0208", "E0209", "E0304", "E0504", "W0409",
    "W0410",
];

/// The `ply-render` fixtures render cleanly, which already proves they parse
/// and resolve; this asserts they are *also* free of every document-local
/// violation `ply-check` knows about, without duplicating the YAML.
#[test]
fn clean_render_fixtures_produce_no_diagnostics() {
    for path in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../render/tests/fixtures/full.ply.yaml",
        "../render/tests/fixtures/qualified_refs.ply.yaml",
        // §5.1 checks inheritance: every fn here either declares its own
        // checks or inherits a *valid* ancestor default, so nothing about
        // resolving the inheritance itself should raise a diagnostic.
        "../render/tests/fixtures/checks_inheritance.ply.yaml",
        // docs/plans/external-elements.md: a well-formed external, named by
        // both flow edges and an `entry:`.
        "../render/tests/fixtures/externals.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
    ] {
        let diags = diagnostics_for(path);
        assert!(diags.is_empty(), "{path} should be clean, got: {diags:?}");
    }
}

/// Every diagnostic message a user can meet must read as plain language:
/// what is wrong and why it matters, no bare spec ranges (the code still
/// prefixes it, via `Diagnostic`'s `Display`, but that's checked in `cli.rs`
/// against the real binary output). These pin the exact wording so a future
/// edit can't quietly regress back to jargon.
#[test]
fn mutate_without_test_or_fuzz_is_e0504() {
    let diags = diagnostics_for("tests/fixtures/mutate_without_test_or_fuzz.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0504")
        .expect("expected an E0504 diagnostic");
    assert_eq!(
        d.message,
        "mutate has nothing to catch its planted bugs: add a test or fuzz check beside it — \
         mutation testing works by deliberately breaking the code and checking those checks \
         notice (fn slot)"
    );
}

/// §5.1 checks inheritance: a fn with no `checks` of its own inherits an
/// ancestor component's default (§5.1's "optional default checks for all
/// fns in scope"). `E0504` must be evaluated against that *effective* list,
/// not the fn's own (empty) one — otherwise a broken component default
/// (`checks: [mutate]` with no `test`/`fuzz` beside it) silently reaches
/// every inheriting fn with no warning at all.
#[test]
fn mutate_in_an_inherited_default_is_e0504_on_the_inheriting_fn() {
    let diags = diagnostics_for("tests/fixtures/mutate_inherited_default_is_broken.ply.yaml");
    let fn_diag = diags
        .iter()
        .find(|d| {
            d.code == "E0504"
                && d.target
                    == Target::Fn {
                        component_path: "audit".to_string(),
                        fn_name: "verify".to_string(),
                    }
        })
        .unwrap_or_else(|| panic!("expected an E0504 on fn verify, got: {diags:?}"));
    assert_eq!(
        fn_diag.message,
        "mutate has nothing to catch its planted bugs: add a test or fuzz check beside it — \
         mutation testing works by deliberately breaking the code and checking those checks \
         notice (fn verify, checks inherited from component audit)"
    );
    // The component's own declared default is independently broken too —
    // that diagnostic still fires at the component, unchanged by this fix.
    let component_diag = diags
        .iter()
        .find(|d| d.code == "E0504" && d.target == Target::Component("audit".to_string()))
        .unwrap_or_else(|| panic!("expected an E0504 on component audit, got: {diags:?}"));
    assert_eq!(
        component_diag.message,
        "mutate has nothing to catch its planted bugs: add a test or fuzz check beside it — \
         mutation testing works by deliberately breaking the code and checking those checks \
         notice (component audit)"
    );
}

/// The override rule cuts the other way too: a fn's own non-empty `checks`
/// replaces the inherited default entirely, so an inherited `test` does NOT
/// rescue a fn-level `[mutate]` that declares no `test`/`fuzz` of its own.
#[test]
fn fn_level_mutate_is_e0504_even_when_the_component_default_has_test() {
    let diags = diagnostics_for("tests/fixtures/mutate_own_list_ignores_inherited_test.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0504")
        .expect("expected an E0504 diagnostic");
    assert_eq!(
        d.target,
        Target::Fn {
            component_path: "audit".to_string(),
            fn_name: "verify".to_string(),
        }
    );
    assert_eq!(
        d.message,
        "mutate has nothing to catch its planted bugs: add a test or fuzz check beside it — \
         mutation testing works by deliberately breaking the code and checking those checks \
         notice (fn verify)"
    );
    // The component's own list ([test], no mutate) is clean on its own —
    // only the fn-level override is broken.
    assert_eq!(
        diags.iter().filter(|d| d.code == "E0504").count(),
        1,
        "expected exactly one E0504, got: {diags:?}"
    );
}

#[test]
fn bad_check_syntax_is_e0203() {
    let diags = diagnostics_for("tests/fixtures/bad_check_syntax.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0203")
        .expect("expected an E0203 diagnostic");
    assert_eq!(
        d.message,
        "\"bounded(0)\" is not a valid check: the number is how many times loops are unrolled \
         during the proof, and it must be between 1 and 64 — a bound of 0 would prove nothing \
         (fn slot)"
    );
}

#[test]
fn bad_edge_syntax_is_e0203() {
    let diags = diagnostics_for("tests/fixtures/bad_edge_syntax.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0203")
        .expect("expected an E0203 diagnostic");
    assert_eq!(
        d.message,
        "\"a b\" is not an edge: expected \"a -> b\" (a may call b) or \"a ~> b : Type\" \
         (data flows from a to b) (edges)"
    );
}

#[test]
fn bad_path_form_is_e0304() {
    let diags = diagnostics_for("tests/fixtures/bad_path_form.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0304")
        .expect("expected an E0304 diagnostic");
    assert_eq!(
        d.message,
        "\"app::Foo<T>\" cannot be used as an anchor path: generics, lifetimes, and \
         trait-qualified paths are not accepted — use a plain module::item path \
         (component ring, anchor)"
    );
}

#[test]
fn duplicate_unresolved_id_is_e0205() {
    let diags = diagnostics_for("tests/fixtures/duplicate_unresolved_id.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0205")
        .expect("expected an E0205 diagnostic");
    assert_eq!(
        d.message,
        "unresolved id 5 is used twice (fn slot and registry): each open decision needs its \
         own number"
    );
}

/// §5.1a rule 6 is already implemented (and tested) in `ply-render`; this
/// confirms the same fixture is flagged document-locally, independent of
/// whether anything ever renders it.
#[test]
fn ambiguous_bare_reference_is_e0206() {
    let diags = diagnostics_for("../render/tests/fixtures/ambiguous_ref.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0206")
        .expect("expected an E0206 diagnostic");
    assert_eq!(
        d.message,
        "ambiguous component reference \"shared\": it could mean alpha.shared or beta.shared \
         — write the dotted form (e.g. alpha.shared) to say which"
    );
}

/// §5.3 "containment implies permission": an explicit edge between a
/// component and its own descendant (either direction, at any depth) is
/// redundant — the descendant could already be reached without a declared
/// edge, the same way it calls between its own functions.
#[test]
fn parent_to_child_edge_is_w0409() {
    let diags = diagnostics_for("tests/fixtures/redundant_parent_child_edge.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "W0409")
        .expect("expected a W0409 diagnostic");
    assert_eq!(
        d.message,
        "\"edge outer -> outer.inner\" is redundant: outer.inner is inside outer, and a \
         component may always call within its own nesting line — no edge needed"
    );
}

/// Same rule, reached through a bare (unqualified) leaf reference rather
/// than the dotted form — resolved the same way §5.1a rule 6 (E0206)
/// resolves bare references, since `inner` is unique across the tree here.
#[test]
fn parent_to_child_edge_via_bare_leaf_is_w0409() {
    let yaml = r#"
ply: 1
components:
  outer:
    anchor: app::outer
    components:
      inner:
        anchor: app::outer::inner
edges:
  - "outer -> inner"
"#;
    let doc = parse_document(yaml).expect("doc should parse");
    let diags = run_checks(&doc);
    let d = diags
        .iter()
        .find(|d| d.code == "W0409")
        .expect("expected a W0409 diagnostic");
    assert_eq!(
        d.message,
        "\"edge outer -> inner\" is redundant: outer.inner is inside outer, and a component \
         may always call within its own nesting line — no edge needed"
    );
}

/// The redundancy is symmetric: a descendant explicitly calling back into
/// its own ancestor is just as redundant as the other direction.
#[test]
fn child_to_parent_edge_is_w0409() {
    let yaml = r#"
ply: 1
components:
  outer:
    anchor: app::outer
    components:
      inner:
        anchor: app::outer::inner
edges:
  - "outer.inner -> outer"
"#;
    let doc = parse_document(yaml).expect("doc should parse");
    let diags = run_checks(&doc);
    let d = diags
        .iter()
        .find(|d| d.code == "W0409")
        .expect("expected a W0409 diagnostic");
    assert_eq!(
        d.message,
        "\"edge outer.inner -> outer\" is redundant: outer.inner is inside outer, and a \
         component may always call within its own nesting line — no edge needed"
    );
}

/// A data-flow edge within one nesting line is equally redundant — §5.3
/// states the rule in terms of "an explicit edge", not "an explicit call
/// edge", and closes with "Edges are for crossings between nesting lines"
/// (again unqualified by kind).
#[test]
fn parent_to_child_flow_edge_is_w0409() {
    let yaml = r#"
ply: 1
components:
  outer:
    anchor: app::outer
    components:
      inner:
        anchor: app::outer::inner
edges:
  - "outer ~> outer.inner : app::Payload"
"#;
    let doc = parse_document(yaml).expect("doc should parse");
    let diags = run_checks(&doc);
    let d = diags
        .iter()
        .find(|d| d.code == "W0409")
        .expect("expected a W0409 diagnostic");
    assert_eq!(
        d.message,
        "\"edge outer ~> outer.inner : app::Payload\" is redundant: outer.inner is inside \
         outer, and a component may always call within its own nesting line — no edge needed"
    );
}

/// Negative case (named directly in The-Ply-Spec.md §5.3's own example): an
/// edge that crosses *into* another component's descendant — not its own —
/// is exactly what edges are for, and must not be flagged.
#[test]
fn cross_container_descendant_edge_is_not_w0409() {
    let diags = diagnostics_for("tests/fixtures/cross_container_descendant_edge.ply.yaml");
    assert!(
        !diags.iter().any(|d| d.code == "W0409"),
        "strategy -> ingest.book crosses into another component's descendant and should not \
         be flagged, got: {diags:?}"
    );
    assert!(
        diags.is_empty(),
        "fixture should otherwise be clean, got: {diags:?}"
    );
}

/// Invariant: `run_checks` never emits a code outside the set this file
/// documents test coverage for — a typo'd or forgotten code fails loudly
/// here instead of shipping silently.
#[test]
fn every_diagnostic_carries_a_known_spec_code() {
    for path in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../render/tests/fixtures/full.ply.yaml",
        "../render/tests/fixtures/qualified_refs.ply.yaml",
        "../render/tests/fixtures/ambiguous_ref.ply.yaml",
        "tests/fixtures/mutate_without_test_or_fuzz.ply.yaml",
        "tests/fixtures/bad_check_syntax.ply.yaml",
        "tests/fixtures/bad_edge_syntax.ply.yaml",
        "tests/fixtures/bad_path_form.ply.yaml",
        "tests/fixtures/duplicate_unresolved_id.ply.yaml",
        "tests/fixtures/redundant_parent_child_edge.ply.yaml",
        "tests/fixtures/cross_container_descendant_edge.ply.yaml",
        "tests/fixtures/mutate_inherited_default_is_broken.ply.yaml",
        "tests/fixtures/mutate_own_list_ignores_inherited_test.ply.yaml",
        "../render/tests/fixtures/checks_inheritance.ply.yaml",
        "tests/fixtures/external_in_call_edge.ply.yaml",
        "tests/fixtures/external_in_deny.ply.yaml",
        "tests/fixtures/external_to_external.ply.yaml",
        "tests/fixtures/entry_names_unknown_external.ply.yaml",
        "tests/fixtures/external_declared_unused.ply.yaml",
        "tests/fixtures/external_duplicates_component.ply.yaml",
        "tests/fixtures/clean_external.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
    ] {
        for d in diagnostics_for(path) {
            assert!(
                KNOWN_CODES.contains(&d.code),
                "{path} produced an unregistered diagnostic code: {d:?}"
            );
        }
    }
}
