//! The-Ply-Spec.md §7.1 "finding (tool-computed, not declared)" row: a document
//! that parses cleanly but fails `ply-check`'s document-local rules must
//! not render as if nothing were wrong (demos/fault-injection.md, fault 3:
//! `decode`'s check loosened to `bounded(0)` drew as confident green `B0`).
//!
//! This is the invariant, not a spot-check on one fixture: for every
//! finding `ply-check` reports on every fixture below, the rendered SVG
//! must either mark a real drawn item red with that finding's code in its
//! tooltip, or count it in the workspace-title fallback (for a finding
//! whose target never gets drawn at all). A construct added later that
//! slips through unflagged fails this test on its own fixture, without
//! anyone having to add a new spot-check for it.

use ply_render::model::parse_document;
use ply_render::svg::render_svg;
use std::collections::BTreeMap;

/// Every wrapper class the renderer uses to mark an offending item red.
/// Deliberately distinct from the plain item classes (`fn-chip-box`,
/// `component-box`, ...) so `every_painted_element_resolves_a_style_rule`
/// still resolves every class on its own, and so a finding never changes
/// the rendering of a document that has none (byte-identical goldens).
const FINDING_CLASSES: &[&str] = &[
    "fn-chip-box-finding",
    "component-box-finding",
    "edge-line-finding",
    "deny-line-finding",
    "unresolved-pin-finding",
    "registry-pin-finding",
];

fn code_counts<I: IntoIterator<Item = String>>(codes: I) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for c in codes {
        *m.entry(c).or_insert(0) += 1;
    }
    m
}

/// Every fixture `ply-check` already has coverage for, plus the
/// fault-injection demo this feature exists to fix. Each must report at
/// least one finding — otherwise it's not exercising this test at all.
const FIXTURES: &[&str] = &[
    "../check/tests/fixtures/bad_check_syntax.ply.yaml",
    "../check/tests/fixtures/bad_edge_syntax.ply.yaml",
    "../check/tests/fixtures/bad_path_form.ply.yaml",
    "../check/tests/fixtures/duplicate_unresolved_id.ply.yaml",
    "../check/tests/fixtures/mutate_without_test_or_fuzz.ply.yaml",
    "../../demos/fault3.ply.yaml",
];

#[test]
fn every_finding_is_visibly_flagged_or_counted_at_the_title() {
    for fixture in FIXTURES {
        let yaml = std::fs::read_to_string(fixture)
            .unwrap_or_else(|e| panic!("{fixture}: could not read fixture: {e}"));
        let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture} should parse: {e}"));

        let findings = ply_core::check::run_checks(&doc);
        assert!(
            !findings.is_empty(),
            "{fixture}: expected at least one finding — fixture no longer exercises this test"
        );

        // The central requirement: a document with findings still renders.
        // Refusing to draw would hide exactly the problem this feature
        // exists to surface (fault-injection demo).
        let svg = render_svg(&doc).unwrap_or_else(|e| {
            panic!("{fixture}: a document with findings must still render, got: {e}")
        });
        let xml = roxmltree::Document::parse(&svg)
            .unwrap_or_else(|e| panic!("{fixture}: rendered SVG must be well-formed: {e}"));

        // Every red-flagged item's tooltip must lead with "FINDING <code>: ".
        let mut visible_codes: Vec<String> = Vec::new();
        for node in xml.descendants().filter(|n| n.is_element()) {
            let Some(class) = node.attribute("class") else {
                continue;
            };
            // The component-box-finding rect now also carries a `ceiling-*`
            // class alongside it (§7.1 "declared ceiling"), so a finding
            // class can no longer be an exact match against the whole
            // `class` attribute — check membership in its space-separated
            // tokens instead.
            let Some(class) = class
                .split_whitespace()
                .find(|c| FINDING_CLASSES.contains(c))
            else {
                continue;
            };
            let title_text = node
                .ancestors()
                .find_map(|a| a.children().find(|c| c.tag_name().name() == "title"))
                .and_then(|t| t.text())
                .unwrap_or_else(|| panic!("{fixture}: red-flagged .{class} has no tooltip"));
            let after = title_text.strip_prefix("FINDING ").unwrap_or_else(|| {
                panic!(
                    "{fixture}: tooltip for .{class} does not lead with the finding: {title_text:?}"
                )
            });
            let code = after
                .split(':')
                .next()
                .unwrap_or_else(|| panic!("{fixture}: malformed finding tooltip: {title_text:?}"))
                .to_string();
            visible_codes.push(code);
        }

        // A finding whose target never gets drawn (e.g. a syntactically
        // invalid edge string, skipped entirely) attaches a count to the
        // workspace title instead.
        let fallback_text = xml
            .descendants()
            .find(|n| n.attribute("class") == Some("finding-count"))
            .and_then(|n| n.text())
            .map(str::to_string);
        let fallback_count: usize = fallback_text
            .as_deref()
            .and_then(|t| t.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);

        // Reconcile by code, not by diagnostic identity: a single
        // `UnresolvedId` finding may legitimately flag two pins (the
        // duplicate itself), so the visible count for a code may exceed
        // the number of diagnostics carrying it — that excess owes the
        // fallback nothing. What must never happen is a diagnostic that is
        // neither shown red anywhere nor counted.
        let expected = code_counts(findings.iter().map(|d| d.code.to_string()));
        let visible = code_counts(visible_codes);
        for code in visible.keys() {
            assert!(
                expected.contains_key(code),
                "{fixture}: SVG shows a red .{code:?} finding that `ply-check` never reported"
            );
        }
        let required_fallback: usize = expected
            .iter()
            .map(|(code, n)| n.saturating_sub(visible.get(code).copied().unwrap_or(0)))
            .sum();
        assert_eq!(
            required_fallback,
            fallback_count,
            "{fixture}: {} diagnostic(s) reported ({expected:?}), {} shown red ({visible:?}), \
             {} counted in the workspace-title fallback ({fallback_text:?}) — every finding must \
             be either visibly flagged or counted",
            findings.len(),
            visible.values().sum::<usize>(),
            fallback_count,
        );
        if fallback_count > 0 {
            let text = fallback_text.unwrap();
            assert!(
                text.contains("run ply-check"),
                "{fixture}: workspace-title fallback should point at `ply-check`, got: {text:?}"
            );
        }
    }
}

/// The finding tooltip embeds `Diagnostic::message` verbatim after the
/// `FINDING <code>: ` prefix (`finding_tooltip_lines` in `svg.rs`) — so the
/// plain-language rewrite of `ply-check`'s messages must show up here too,
/// not just in `ply-check`'s own tests. Pinned against the fault-injection
/// demo's `decode` fn, whose `bounded(0)` is the out-of-range case.
#[test]
fn finding_tooltip_carries_the_plain_language_message() {
    let yaml = std::fs::read_to_string("../../demos/fault3.ply.yaml").unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    let svg = render_svg(&doc).expect("fixture should render");
    // The `<title>` text is XML-escaped (`esc()` in `svg.rs`), so the
    // embedded literal quotes come back as `&quot;`.
    assert!(
        svg.contains(
            "FINDING E0203: &quot;bounded(0)&quot; is not a valid check: the number is how \
             many times loops are unrolled during the proof, and it must be between 1 and 64 \
             — a bound of 0 would prove nothing (fn decode)"
        ),
        "expected the plain-language E0203 message in the rendered tooltip, got: {svg}"
    );
}

/// Reruns the fixture set to confirm a finding never leaks into a document
/// that doesn't have one — the whole design rests on findings being purely
/// additive so vetting's clean fixtures render byte-identically.
#[test]
fn a_clean_document_gets_no_finding_markup_at_all() {
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/qualified_refs.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).expect("fixture should parse");
        assert!(
            ply_core::check::run_checks(&doc).is_empty(),
            "{fixture}: expected to be clean; this test's premise (no findings) is now false"
        );
        let svg = render_svg(&doc).expect("fixture should render");
        for class in FINDING_CLASSES {
            assert!(
                !svg.contains(&format!("class=\"{class}\"")),
                "{fixture}: clean document must draw no finding markup, found .{class}"
            );
        }
        assert!(
            !svg.contains("class=\"finding-count\""),
            "{fixture}: clean document must show no finding count at the title"
        );
    }
}
