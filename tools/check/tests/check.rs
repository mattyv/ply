//! `ply-check`'s document-local rules (SPEC.md §5.1a, §5.1, §5.6) — the
//! subset of `cargo ply check` that needs no anchored Rust code.

use ply_check::run_checks;
use ply_model::parse_document;

fn diagnostics_for(path: &str) -> Vec<ply_check::Diagnostic> {
    let yaml = std::fs::read_to_string(path).unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    run_checks(&doc)
}

const KNOWN_CODES: &[&str] = &["E0203", "E0205", "E0206", "E0304", "E0504"];

/// The `ply-render` fixtures render cleanly, which already proves they parse
/// and resolve; this asserts they are *also* free of every document-local
/// violation `ply-check` knows about, without duplicating the YAML.
#[test]
fn clean_render_fixtures_produce_no_diagnostics() {
    for path in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../render/tests/fixtures/full.ply.yaml",
        "../render/tests/fixtures/qualified_refs.ply.yaml",
    ] {
        let diags = diagnostics_for(path);
        assert!(diags.is_empty(), "{path} should be clean, got: {diags:?}");
    }
}

#[test]
fn mutate_without_test_or_fuzz_is_e0504() {
    let diags = diagnostics_for("tests/fixtures/mutate_without_test_or_fuzz.ply.yaml");
    assert!(
        diags.iter().any(|d| d.code == "E0504" && d.message.contains("fn slot")),
        "expected E0504 naming `fn slot`, got: {diags:?}"
    );
}

#[test]
fn bad_check_syntax_is_e0203() {
    let diags = diagnostics_for("tests/fixtures/bad_check_syntax.ply.yaml");
    assert!(
        diags.iter().any(|d| d.code == "E0203" && d.message.contains("bounded(0)")),
        "expected E0203 naming the out-of-range check string, got: {diags:?}"
    );
}

#[test]
fn bad_edge_syntax_is_e0203() {
    let diags = diagnostics_for("tests/fixtures/bad_edge_syntax.ply.yaml");
    assert!(
        diags.iter().any(|d| d.code == "E0203"),
        "expected E0203 for the malformed edge string, got: {diags:?}"
    );
}

#[test]
fn bad_path_form_is_e0304() {
    let diags = diagnostics_for("tests/fixtures/bad_path_form.ply.yaml");
    assert!(
        diags.iter().any(|d| d.code == "E0304" && d.message.contains("Foo<T>")),
        "expected E0304 naming the generic anchor, got: {diags:?}"
    );
}

#[test]
fn duplicate_unresolved_id_is_e0205() {
    let diags = diagnostics_for("tests/fixtures/duplicate_unresolved_id.ply.yaml");
    assert!(
        diags.iter().any(|d| d.code == "E0205" && d.message.contains('5')),
        "expected E0205 naming id 5, got: {diags:?}"
    );
}

/// §5.1a rule 6 is already implemented (and tested) in `ply-render`; this
/// confirms the same fixture is flagged document-locally, independent of
/// whether anything ever renders it.
#[test]
fn ambiguous_bare_reference_is_e0206() {
    let diags = diagnostics_for("../render/tests/fixtures/ambiguous_ref.ply.yaml");
    assert!(
        diags.iter().any(|d| d.code == "E0206" && d.message.contains("shared")),
        "expected E0206 naming the ambiguous token, got: {diags:?}"
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
    ] {
        for d in diagnostics_for(path) {
            assert!(
                KNOWN_CODES.contains(&d.code),
                "{path} produced an unregistered diagnostic code: {d:?}"
            );
        }
    }
}
