//! `ply-check`'s document-local rules (The-Ply-Spec.md §5.1a, §5.1, §5.6) — the
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
