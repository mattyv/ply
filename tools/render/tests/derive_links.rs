//! Cross-document links (The-Ply-Spec.md §7.1's derive-links brief):
//! a component in one document links to another when that document's own
//! top-level anchor equals, or sits under, the linking component's anchor —
//! discovered from real crate directories, never from a declared key.
//!
//! `ply_core::config::derive_links` is unit-tested directly (the four
//! refusal rules, and the self-reference degenerate case); this file
//! covers what the *renderer* does with a link once one is resolved: the
//! ordering trap (a linked-but-otherwise-empty component must draw
//! collapsed, never hollow) and the invariant that what gets drawn always
//! matches the target document, checked by walking the real SVG output
//! rather than the data that fed it.

use ply_core::config::derive_links;
use ply_render::model::{Component, parse_document};
use ply_render::svg::{RenderOptions, render_svg_with_state_and_links};
use std::path::Path;

/// A minimal real crate on disk: `Cargo.toml` plus `src/lib.rs`, which is
/// all `ply_core::harness::workspace_library_crates` needs to find it, and
/// its own `ply.yaml` alongside.
fn write_crate(dir: &Path, crate_name: &str, ply_yaml: &str) {
    let crate_dir = dir.join(crate_name);
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        format!("[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    std::fs::write(crate_dir.join("src/lib.rs"), "").unwrap();
    std::fs::write(crate_dir.join("ply.yaml"), ply_yaml).unwrap();
}

/// An independent recount, written from scratch rather than calling the
/// renderer's own (private) `count_subtree` — the oracle a "does the
/// drawing match reality" test needs is not the code path being checked.
fn count_descendants(comp: &Component) -> (usize, usize) {
    let mut components = comp.components.len();
    let mut fns = comp.fns.len();
    for child in comp.components.values() {
        let (c, f) = count_descendants(child);
        components += c;
        fns += f;
    }
    (components, fns)
}

/// Every `"{n} component(s) · {m} fn(s) — {path}"` contents line actually
/// drawn in `svg`, parsed back out of the real markup rather than assumed
/// from the format string that wrote it.
fn drawn_link_lines(svg: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    for chunk in svg.split("<text class=\"component-anchor\"") {
        let Some(gt) = chunk.find('>') else { continue };
        let Some(close) = chunk[gt..].find("</text>") else {
            continue;
        };
        let text = &chunk[gt + 1..gt + close];
        let Some((left, path)) = text.split_once(" \u{2014} ") else {
            continue;
        };
        let Some((n_part, m_part)) = left.split_once(" \u{b7} ") else {
            continue;
        };
        let Some(n) = n_part.split_whitespace().next().and_then(|s| s.parse().ok()) else {
            continue;
        };
        let Some(m) = m_part.split_whitespace().next().and_then(|s| s.parse().ok()) else {
            continue;
        };
        out.push((n, m, path.to_string()));
    }
    out
}

/// §7.1's trap: a derived-link box with no declared interior of its own
/// must rank *above* the hollow rule. Get the ordering backwards and the
/// box draws dashed ("nothing to zoom into yet") when the truth is the
/// opposite -- there is plenty inside, just in another file.
#[test]
fn a_linked_hollow_component_draws_collapsed_not_hollow() {
    let dir = tempfile::tempdir().unwrap();
    write_crate(
        dir.path(),
        "inner_lib",
        "ply: 1\ncomponents:\n  inner:\n    anchor: inner_lib\n    components:\n      nested:\n        anchor: inner_lib::nested\n        fns:\n          go:\n            checks: [bounded(2)]\n",
    );
    let outer_text = "ply: 1\ncomponents:\n  core:\n    anchor: inner_lib\n";
    std::fs::write(dir.path().join("ply.yaml"), outer_text).unwrap();
    let doc = parse_document(outer_text).unwrap();

    let link_set = derive_links(&doc, dir.path());
    assert!(link_set.findings.is_empty(), "{:?}", link_set.findings);
    assert!(link_set.links.contains_key("core"));

    let state_fields = ply_core::harness::resolve_state_fields(dir.path(), &doc);
    let svg =
        render_svg_with_state_and_links(&doc, &RenderOptions::default(), &state_fields, &link_set.links)
            .unwrap();

    assert!(
        // Not a bare `!contains("hollow-box")`: that string also names the
        // CSS *rule* every render defines, linked or not. What must be
        // absent is the class actually being *applied* to a box.
        !svg.contains(" hollow-box\""),
        "a linked box must never draw hollow (dashed):\n{svg}"
    );
    assert!(
        svg.contains("collapsed-stack"),
        "a linked box must draw the collapsed stack cue:\n{svg}"
    );
    let lines = drawn_link_lines(&svg);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, 1, "1 nested component"); // `nested`
    assert_eq!(lines[0].1, 1, "1 fn (`go`, inside `nested`)");
    assert!(lines[0].2.ends_with("inner_lib/ply.yaml"), "{}", lines[0].2);

    // The text form's whole contract is that it states everything the
    // drawing shows (transcript.rs's own module doc) -- so the same link
    // must show up there too, in the same words the drawing's tooltip uses.
    let transcript = ply_render::transcript::render_transcript_with_state_and_links(
        &doc,
        Some(&state_fields),
        Some(&link_set.links),
    );
    assert!(
        transcript.contains("linked — 1 component and 1 function live in a different file"),
        "{transcript}"
    );
    assert!(transcript.contains("inner_lib/ply.yaml"), "{transcript}");
    assert!(
        !transcript.contains("hollow — promises nothing yet"),
        "{transcript}"
    );
}

/// The invariant the design pass asked for: every cross-document link's
/// drawn counts match the target document -- checked as a sweep over every
/// link the model produced, not a spot-check on one pair, in the style of
/// `every_painted_element_resolves_a_style_rule` in this same crate's
/// `render.rs`.
#[test]
fn every_cross_document_links_drawn_counts_match_its_target_document() {
    let dir = tempfile::tempdir().unwrap();
    write_crate(
        dir.path(),
        "lib_a",
        "ply: 1\ncomponents:\n  a:\n    anchor: lib_a\n    fns:\n      one:\n        checks: [bounded(2)]\n      two:\n        checks: [bounded(2)]\n",
    );
    write_crate(
        dir.path(),
        "lib_b",
        "ply: 1\ncomponents:\n  b:\n    anchor: lib_b\n    components:\n      sub:\n        anchor: lib_b::sub\n        components:\n          leaf:\n            anchor: lib_b::sub::leaf\n            fns:\n              three:\n                checks: [bounded(2)]\n",
    );
    let outer_text =
        "ply: 1\ncomponents:\n  first:\n    anchor: lib_a\n  second:\n    anchor: lib_b\n";
    std::fs::write(dir.path().join("ply.yaml"), outer_text).unwrap();
    let doc = parse_document(outer_text).unwrap();

    let link_set = derive_links(&doc, dir.path());
    assert!(link_set.findings.is_empty(), "{:?}", link_set.findings);
    assert_eq!(link_set.links.len(), 2, "{:?}", link_set.links.keys().collect::<Vec<_>>());

    let state_fields = ply_core::harness::resolve_state_fields(dir.path(), &doc);
    let svg =
        render_svg_with_state_and_links(&doc, &RenderOptions::default(), &state_fields, &link_set.links)
            .unwrap();
    let drawn = drawn_link_lines(&svg);
    assert_eq!(drawn.len(), 2, "{drawn:?}");

    // The sweep: every drawn line's path is re-read from disk, completely
    // independently of `link_set`, and recounted from scratch.
    for (n, m, path) in drawn {
        let text = std::fs::read_to_string(dir.path().join(&path))
            .unwrap_or_else(|e| panic!("drawn path {path:?} did not read back: {e}"));
        let target = parse_document(&text).unwrap();
        let (_, top) = target
            .components
            .iter()
            .next()
            .unwrap_or_else(|| panic!("{path:?} declares no top-level component"));
        let (expected_n, expected_m) = count_descendants(top);
        assert_eq!(
            (n, m),
            (expected_n, expected_m),
            "drawn counts for {path:?} do not match a fresh recount of the file itself"
        );
    }
}
