//! Cross-document links (The-Ply-Spec.md §7.1's derive-links brief):
//! a component in one document links to another when that document's own
//! top-level anchor equals, or sits under, the linking component's anchor —
//! discovered from real crate directories, never from a declared key.
//!
//! `ply_core::config::derive_links` is unit-tested directly (the four
//! refusal rules, and the self-reference degenerate case); this file
//! covers what the *renderer* does with a link once one is resolved: the
//! ordering trap (a linked-but-otherwise-empty component must never draw
//! hollow, as though there were nothing inside it) and the invariant that
//! everything the target document declares is actually drawn, checked by
//! walking the real SVG output rather than the data that fed it.

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

/// Every `"{n} component(s) · {m} fn(s) — {path}"` contents line actually
/// drawn in `svg`, parsed back out of the real markup rather than assumed
/// Every path a drawing says it took content from, read back out of the
/// finished SVG rather than from the link index that produced it -- so a
/// test can re-open those files itself and check the drawing against them.
///
/// A linked box carries `anchor \u{2014} path` on its anchor line (see
/// `linked_source_line`). Ordinary boxes carry a bare anchor and are
/// skipped by the same split that finds these.
fn drawn_link_paths(svg: &str) -> Vec<String> {
    let mut out = Vec::new();
    // `.skip(1)`: split yields everything *before* the first anchor as its
    // first item, and that prefix contains the workspace tooltip, which has
    // em-dashes of its own. Without this the helper reports the title as a
    // linked path.
    for chunk in svg.split("<text class=\"component-anchor\"").skip(1) {
        let Some(gt) = chunk.find('>') else { continue };
        let Some(close) = chunk[gt..].find("</text>") else {
            continue;
        };
        let text = &chunk[gt + 1..gt + close];
        if let Some((_, path)) = text.split_once(" \u{2014} ") {
            out.push(path.to_string());
        }
    }
    out
}

/// §7.1's trap: a derived-link box with no declared interior of its own
/// must rank *above* the hollow rule. Get the ordering backwards and the
/// box draws dashed ("nothing to zoom into yet") when the truth is the
/// opposite -- there is plenty inside, just written in another file, and
/// it is drawn here.
#[test]
fn a_linked_component_draws_the_other_documents_interior_in_place() {
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
    let svg = render_svg_with_state_and_links(
        &doc,
        &RenderOptions::default(),
        &state_fields,
        &link_set.links,
    )
    .unwrap();

    assert!(
        // Not a bare `!contains("hollow-box")`: that string also names the
        // CSS *rule* every render defines, linked or not. What must be
        // absent is the class actually being *applied* to a box.
        !svg.contains(" hollow-box\""),
        "a linked box must never draw hollow (dashed):\n{svg}"
    );
    // The linked document's interior is drawn here, in place: its nested
    // component and its promise both appear, rather than a box telling the
    // reader to go and open another file. Changed 2026-09-05 -- it drew
    // collapsed until then, which is why the root document's own drawing
    // showed five of `core`'s parts when the real file had twenty-one.
    assert!(
        svg.contains(">nested<"),
        "the linked document's nested component must be drawn here:\n{svg}"
    );
    assert!(
        svg.contains(">go<"),
        "the linked document's promise must be drawn here:\n{svg}"
    );
    // ...and the box still says which file all of that is written in, or a
    // reader has no way to know none of it is declared in the document
    // they are looking at.
    let paths = drawn_link_paths(&svg);
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert!(paths[0].ends_with("inner_lib/ply.yaml"), "{}", paths[0]);

    // The text form's whole contract is that it states everything the
    // drawing shows (transcript.rs's own module doc) -- so the same link
    // must show up there too, in the same words the drawing's tooltip uses.
    let transcript = ply_render::transcript::render_transcript_with_state_and_links(
        &doc,
        Some(&state_fields),
        Some(&link_set.links),
    );
    assert!(
        transcript
            .contains("linked — 1 component and 1 function are written down in a different file"),
        "{transcript}"
    );
    assert!(transcript.contains("inner_lib/ply.yaml"), "{transcript}");
    assert!(
        !transcript.contains("hollow — promises nothing yet"),
        "{transcript}"
    );
}

/// The invariant the design pass asked for, tightened on 2026-09-05 from
/// "the counts match" to "every declared thing is actually drawn": a sweep
/// over every link the model produced, not a spot-check on one pair, in
/// the style of `every_painted_element_resolves_a_style_rule` in this same
/// crate's `render.rs`.
#[test]
fn every_cross_document_link_draws_everything_its_target_document_declares() {
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
    assert_eq!(
        link_set.links.len(),
        2,
        "{:?}",
        link_set.links.keys().collect::<Vec<_>>()
    );

    let state_fields = ply_core::harness::resolve_state_fields(dir.path(), &doc);
    let svg = render_svg_with_state_and_links(
        &doc,
        &RenderOptions::default(),
        &state_fields,
        &link_set.links,
    )
    .unwrap();
    let drawn = drawn_link_paths(&svg);
    assert_eq!(drawn.len(), 2, "{drawn:?}");

    // The sweep: every path the drawing says it took content from is
    // re-opened from disk, independently of `link_set`, and every single
    // thing that file declares must actually appear in the drawing.
    //
    // Stronger than the count this replaced. A count can match while the
    // wrong things are drawn, and a linked box that quietly showed some of
    // a file's parts is exactly the failure this whole change exists to
    // fix -- the root document drew five of `core`'s twenty-one for three
    // days and every test stayed green, because nothing walked the target
    // and asked "is this one here?".
    for path in drawn {
        let text = std::fs::read_to_string(dir.path().join(&path))
            .unwrap_or_else(|e| panic!("drawn path {path:?} did not read back: {e}"));
        let target = parse_document(&text).unwrap();
        let (_, top) = target
            .components
            .iter()
            .next()
            .unwrap_or_else(|| panic!("{path:?} declares no top-level component"));
        let mut expected: Vec<String> = Vec::new();
        fn collect(comp: &Component, out: &mut Vec<String>) {
            for (name, child) in &comp.components {
                out.push(name.clone());
                collect(child, out);
            }
            out.extend(comp.fns.keys().cloned());
        }
        collect(top, &mut expected);
        assert!(
            !expected.is_empty(),
            "{path:?} declares nothing, so this sweep would pass vacuously"
        );
        for item in expected {
            assert!(
                svg.contains(&format!(">{item}<")),
                "{path:?} declares {item:?}, but the drawing that claims to \
                 show that file never draws it:\n{svg}"
            );
        }
    }
}

/// A link only ever stands in for an interior nobody declared locally. A
/// component that already writes its own real fn or nested component
/// keeps drawing exactly that, even when its anchor also resolves to
/// another document that would otherwise link.
#[test]
fn a_component_with_real_local_content_ignores_a_resolvable_link() {
    let dir = tempfile::tempdir().unwrap();
    write_crate(
        dir.path(),
        "inner_lib",
        "ply: 1\ncomponents:\n  inner:\n    anchor: inner_lib\n    fns:\n      go:\n        checks: [bounded(2)]\n",
    );
    let outer_text = "ply: 1\ncomponents:\n  core:\n    anchor: inner_lib\n    fns:\n      own_fn:\n        checks: [bounded(2)]\n";
    std::fs::write(dir.path().join("ply.yaml"), outer_text).unwrap();
    let doc = parse_document(outer_text).unwrap();

    // The link still *resolves* -- deriving it is unconditional -- but the
    // renderer must not act on it here.
    let link_set = derive_links(&doc, dir.path());
    assert!(link_set.links.contains_key("core"));

    let state_fields = ply_core::harness::resolve_state_fields(dir.path(), &doc);
    let svg = render_svg_with_state_and_links(
        &doc,
        &RenderOptions::default(),
        &state_fields,
        &link_set.links,
    )
    .unwrap();

    assert!(
        svg.contains("own_fn"),
        "the component's own declared fn must still be drawn:\n{svg}"
    );
    assert!(
        // As in the ordering-trap test: check the class is actually
        // *applied* to an element, not merely defined in the stylesheet
        // every render carries regardless.
        !svg.contains("class=\"collapsed-stack"),
        "a component with real local content must not switch to the linked/collapsed \
         rendering:\n{svg}"
    );
    assert!(drawn_link_paths(&svg).is_empty(), "{svg}");
}
