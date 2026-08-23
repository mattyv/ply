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
    assert_eq!(
        d.message,
        "\"bounded(0)\" is not a valid check: the number is how many times loops are unrolled \
         during the proof, and it must be between 1 and 64 — a bound of 0 would prove nothing \
         (fn slot)"
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
    assert_eq!(
        d.message,
        "\"app::Foo<T>\" cannot be used as an anchor path: generics, lifetimes, and \
         trait-qualified paths are not accepted — use a plain module::item path \
         (component ring, anchor)"
    );
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
    assert_eq!(
        d.message,
        "mutate has nothing to catch its planted bugs: add a test or fuzz check beside it — \
         mutation testing works by deliberately breaking the code and checking those checks \
         notice (fn slot)"
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
    assert_eq!(
        d.message,
        "\"a b\" is not an edge: expected \"a -> b\" (a may call b) or \"a ~> b : Type\" \
         (data flows from a to b) (edges)"
    );
}

#[test]
fn duplicate_unresolved_id_targets_the_shared_id() {
    let diags = diagnostics_for("tests/fixtures/duplicate_unresolved_id.ply.yaml");
    let d = diags
        .iter()
        .find(|d| d.code == "E0205")
        .expect("expected an E0205 diagnostic");
    assert_eq!(d.target, Target::UnresolvedId(5));
    assert_eq!(
        d.message,
        "unresolved id 5 is used twice (fn slot and registry): each open decision needs its \
         own number"
    );
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
    assert_eq!(
        d.message,
        "ambiguous component reference \"shared\": it could mean alpha.shared or beta.shared \
         — write the dotted form (e.g. alpha.shared) to say which"
    );
}
