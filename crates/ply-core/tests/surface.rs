//! What `audit` and `worklist` can read out of a crate's own source
//! (The-Ply-Spec.md §5.3, §5.4a, §5.6, §5.7).
//!
//! Every one of these constructs is written by a human in Rust and read
//! back by a listing command. The tests are over the *facts recovered*, not
//! over the shape of the walk: an escape whose reason is lost, or a marker
//! whose id is lost, is a listing that cannot be acted on.

use std::path::Path;

use ply_core::surface;

fn crate_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

/// §5.6: "`ply worklist` lists every marker (macro or `ply.yaml` registry)
/// with its span, enclosing component, and blocking status." The span and
/// the enclosing function are what this side of it recovers.
#[test]
fn an_unresolved_marker_carries_its_id_note_line_and_enclosing_fn() {
    let dir = crate_with(&[(
        "src/lib.rs",
        "pub fn discount(pct: u32) -> u32 {\n    \
         ply::unresolved!(147, \"employee discount undecided\");\n}\n",
    )]);
    let found = surface::scan_crate(dir.path());
    assert_eq!(found.markers.len(), 1, "{:#?}", found.markers);
    let m = &found.markers[0];
    assert_eq!(m.id, Some(147));
    assert_eq!(m.note.as_deref(), Some("employee discount undecided"));
    assert_eq!(m.enclosing_fn.as_deref(), Some("discount"));
    assert_eq!(m.file, "src/lib.rs");
    assert_eq!(m.line, 2);
}

/// A marker inside a nested module still belongs to a function a reader can
/// go and find, so the name is the path they would type.
#[test]
fn a_marker_in_a_nested_module_is_named_by_its_path() {
    let dir = crate_with(&[(
        "src/lib.rs",
        "pub mod pricing {\n    pub fn quote() -> u32 {\n        \
         ply::unresolved!(151, \"settlement rounding rule TBD\");\n    }\n}\n",
    )]);
    let found = surface::scan_crate(dir.path());
    assert_eq!(
        found.markers[0].enclosing_fn.as_deref(),
        Some("pricing::quote")
    );
}

/// Every `.rs` file under `src/`, not only `lib.rs`: a decision nobody has
/// made does not become less open by living in another file.
#[test]
fn markers_are_found_in_every_source_file_not_only_lib_rs() {
    let dir = crate_with(&[
        ("src/lib.rs", "pub mod fees;\n"),
        (
            "src/fees.rs",
            "pub fn tier() -> u32 {\n    ply::unresolved!(9, \"tier table TBD\");\n}\n",
        ),
    ]);
    let found = surface::scan_crate(dir.path());
    assert_eq!(found.markers.len(), 1);
    assert_eq!(found.markers[0].file, "src/fees.rs");
}

/// §5.3's per-item escape. The reason is the whole value of the record: an
/// exemption with no reason is an exemption nobody can review.
#[test]
fn a_profile_escape_carries_what_it_suppresses_the_reason_and_the_item() {
    let dir = crate_with(&[(
        "src/lib.rs",
        "#[ply::allow(no_panics, reason = \"bounds are checked by the caller\")]\n\
         pub fn fast_path(i: usize) -> u32 { [1, 2][i] }\n",
    )]);
    let found = surface::scan_crate(dir.path());
    assert_eq!(found.escapes.len(), 1, "{:#?}", found.escapes);
    let e = &found.escapes[0];
    assert_eq!(e.item, "fast_path");
    assert_eq!(e.suppressed, "no_panics");
    assert_eq!(
        e.reason.as_deref(),
        Some("bounds are checked by the caller")
    );
    assert_eq!(e.line, 1);
}

/// §5.7: a body the model wrote carries `#[ply::derived(spec_hash = "...")]`.
#[test]
fn a_derived_body_carries_its_spec_hash() {
    let dir = crate_with(&[(
        "src/lib.rs",
        "#[ply::derived(spec_hash = \"b3f9ac\")]\npub fn quote() -> u32 { 1 }\n",
    )]);
    let found = surface::scan_crate(dir.path());
    assert_eq!(found.derived.len(), 1);
    assert_eq!(found.derived[0].item, "quote");
    assert_eq!(found.derived[0].spec_hash.as_deref(), Some("b3f9ac"));
}

/// §5.4a: "A `#[ply::pure]` helper called from any contract is a trust
/// surface, not a free pass." Finding the call is what makes the listing
/// possible at all.
#[test]
fn a_contract_expression_reports_the_helpers_it_calls() {
    assert_eq!(
        surface::contract_helpers("bps_ok(bps) && amount <= 100"),
        vec!["bps_ok".to_string()]
    );
    assert_eq!(
        surface::contract_helpers("|result| *result <= cap_for(tier)"),
        vec!["cap_for".to_string()]
    );
}

/// Two constructs that look like helper calls and are not: `old(expr)` is
/// §5.4a's own two-state primitive, and a capitalised name is a type or
/// enum-variant constructor (§5.5's convention). Listing either as a
/// trusted helper would put noise on the one surface that must stay
/// readable.
#[test]
fn old_and_constructors_are_not_helpers() {
    assert!(surface::contract_helpers("|result| *result == old(x) + 1").is_empty());
    assert!(surface::contract_helpers("|result| result == Some(3)").is_empty());
}

/// A contract Ply cannot parse yields no helpers rather than a panic: the
/// expression subset is checked elsewhere (`E0501`), and `audit` listing
/// nothing is better than `audit` falling over on a document `verify` would
/// have reported properly.
#[test]
fn an_unparseable_contract_yields_no_helpers() {
    assert!(surface::contract_helpers("this is not rust ((").is_empty());
}

/// The same contract clause can be written twice — once as a
/// `#[ply::requires]` attribute and once in `ply.yaml` — and the two
/// spellings do not match textually: the attribute comes back from the
/// parser token-spaced (`bps_ok (bps)`) while the document holds what the
/// user typed. Comparing the raw strings made `audit` list one assumption
/// as two, which is how a list of things a codebase trusts starts
/// overstating itself.
#[test]
fn two_spellings_of_one_expression_are_recognised_as_the_same_clause() {
    assert!(surface::same_expression("bps_ok (bps)", "bps_ok(bps)"));
    assert!(surface::same_expression("a  &&   b", "a && b"));
    assert!(!surface::same_expression("tick > 0", "tick >= 0"));
    // A difference inside a string literal is a real difference, which is
    // why this compares parsed tokens rather than stripping whitespace.
    assert!(!surface::same_expression("s == \"a b\"", "s == \"ab\""));
    // Neither side parses: fall back to the strings themselves rather than
    // calling two unrelated clauses equal.
    assert!(surface::same_expression("((", "(("));
    assert!(!surface::same_expression("((", "))"));
}

/// A crate with no `src/` at all is not a crash: `audit` runs on whatever
/// it was pointed at.
#[test]
fn a_crate_with_no_source_scans_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let found = surface::scan_crate(Path::new(dir.path()));
    assert!(found.markers.is_empty());
    assert!(found.escapes.is_empty());
    assert!(found.derived.is_empty());
}
