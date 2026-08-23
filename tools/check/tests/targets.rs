//! §7.1 "finding" row: `ply-render` needs a structured attachment target on
//! every diagnostic, not just prose, so it knows *what to draw red*. This
//! pins the target `ply-check` computes for each of the existing fixtures —
//! written before `Diagnostic` grows a `target` field, so it starts red on
//! a compile error (the field doesn't exist yet), then on a mismatch once
//! it does.

use ply_check::{Target, run_checks};
use ply_model::parse_document;

fn diagnostics_for(path: &str) -> Vec<ply_check::Diagnostic> {
    let yaml = std::fs::read_to_string(path).unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    run_checks(&doc)
}

#[test]
fn bad_check_syntax_targets_the_offending_fn() {
    let diags = diagnostics_for("tests/fixtures/bad_check_syntax.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0203")
        .expect("expected an E0203 diagnostic");
    assert_eq!(
        d.target,
        Target::Fn {
            component_path: "ring".to_string(),
            fn_name: "slot".to_string()
        }
    );
}

#[test]
fn bad_path_form_targets_the_offending_component() {
    let diags = diagnostics_for("tests/fixtures/bad_path_form.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0304")
        .expect("expected an E0304 diagnostic");
    assert_eq!(d.target, Target::Component("ring".to_string()));
}

#[test]
fn mutate_without_test_or_fuzz_targets_the_offending_fn() {
    let diags = diagnostics_for("tests/fixtures/mutate_without_test_or_fuzz.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0504")
        .expect("expected an E0504 diagnostic");
    assert_eq!(
        d.target,
        Target::Fn {
            component_path: "ring".to_string(),
            fn_name: "slot".to_string()
        }
    );
}

#[test]
fn bad_edge_syntax_targets_the_edge_by_index() {
    let diags = diagnostics_for("tests/fixtures/bad_edge_syntax.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0203")
        .expect("expected an E0203 diagnostic");
    // The fixture's `edges:` list has exactly one entry, "a b".
    assert_eq!(d.target, Target::EdgeIndex(0));
}

#[test]
fn duplicate_unresolved_id_targets_the_shared_id() {
    let diags = diagnostics_for("tests/fixtures/duplicate_unresolved_id.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0205")
        .expect("expected an E0205 diagnostic");
    assert_eq!(d.target, Target::UnresolvedId(5));
}

/// §5.1a rule 6 ambiguity on a deny endpoint (not exercised by any fixture
/// in this crate) must still resolve to `DenyIndex`, not `Fn`/`Component` —
/// the offending item is the deny rule string, not either component it names.
#[test]
fn ambiguous_deny_endpoint_targets_the_deny_by_index() {
    let yaml = r#"
ply: 1
components:
  alpha:
    anchor: app::alpha
    components:
      shared:
        anchor: app::alpha::shared
  beta:
    anchor: app::beta
    components:
      shared:
        anchor: app::beta::shared
deny:
  - "shared -> alpha"
"#;
    let doc = parse_document(yaml).expect("doc should parse");
    let diags = run_checks(&doc);
    let d = diags
        .iter()
        .find(|d| d.code == "E0206")
        .expect("expected an E0206 diagnostic");
    assert_eq!(d.target, Target::DenyIndex(0));
}
