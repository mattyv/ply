//! ARCHITECTURE.md shows two pictures of Ply's own structure and tells the
//! reader they are rendered from the `ply.yaml` documents rather than drawn
//! by hand. That sentence is a claim, and a committed SVG is exactly the
//! kind of artifact that stops being true quietly: someone adds a crate or a
//! promise, the spec gains a box, the picture in the document keeps showing
//! the old shape, and the page now lies about the thing it exists to
//! describe.
//!
//! So the claim is checked. This renders the real documents and compares
//! them against the committed files, byte for byte.

use std::path::PathBuf;

use ply_render::model::parse_document;
use ply_render::svg::{RenderOptions, render_svg_with_options};

/// `tools/render/tests/` -> the repository root.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("tools/render lives two levels below the repository root")
        .to_path_buf()
}

/// ARCHITECTURE.md embeds two drawings, and each is checked the same way.
///
/// The workspace document says which crates exist and who may depend on
/// whom; the library document says what `ply-core` promises about its own
/// functions. They are separate files because a function claim resolves
/// against one crate's `src/lib.rs` and a workspace root has none -- so the
/// page carries both, and both can go stale.
#[test]
fn the_architecture_diagrams_match_the_specs_they_claim_to_be_rendered_from() {
    let root = repo_root();
    for (yaml_path, svg_path) in [
        ("ply.yaml", "docs/ply-self.svg"),
        ("crates/ply-core/ply.yaml", "docs/ply-core-self.svg"),
    ] {
        let yaml = std::fs::read_to_string(root.join(yaml_path))
            .unwrap_or_else(|e| panic!("reading {yaml_path}: {e}"));
        let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{yaml_path} must parse: {e}"));
        let fresh = render_svg_with_options(&doc, &RenderOptions::default())
            .unwrap_or_else(|e| panic!("{yaml_path} must render: {e}"));

        let committed = std::fs::read_to_string(root.join(svg_path)).unwrap_or_else(|e| {
            panic!(
                "ARCHITECTURE.md embeds {svg_path} and it could not be read ({e}). Regenerate \
                 it from the tools workspace with:\n  cargo run --release -p ply-render -- \
                 ../{yaml_path} -o ../{svg_path}"
            )
        });

        assert_eq!(
            committed, fresh,
            "{svg_path} no longer matches what {yaml_path} renders to, so the diagram in \
             ARCHITECTURE.md is showing a structure this repository no longer has. \
             Regenerate it from the tools workspace, then look at the result before \
             committing it:\n  cargo run --release -p ply-render -- ../{yaml_path} -o \
             ../{svg_path}"
        );
    }
}

/// The committed text forms beside each vetting scenario, checked the same
/// way and for the same reason as the drawings next to them.
///
/// A saved transcript is exactly the artifact this project distrusts: it is
/// generated output, it is easy to read as authoritative, and it goes stale
/// silently. The point of committing it anyway is that a change to the
/// wording shows up in review as a diff a person can read — the seal
/// sentence naming the wrong rule for months would have been visible in one
/// of these. That only holds while the file is regenerated whenever the
/// renderer changes, which is what this test is for.
#[test]
fn the_committed_text_forms_still_match_what_the_documents_render_to() {
    let root = repo_root();
    for (yaml_path, text_path) in [
        (
            "vetting/001-spsc-disruptor.ply.yaml",
            "vetting/001-spsc-disruptor.txt",
        ),
        (
            "vetting/002-ingest-pipeline.ply.yaml",
            "vetting/002-ingest-pipeline.txt",
        ),
        (
            "vetting/003-trading-system.ply.yaml",
            "vetting/003-trading-system-full.txt",
        ),
        ("ply.yaml", "docs/ply-self.txt"),
        ("crates/ply-core/ply.yaml", "docs/ply-core-self.txt"),
        // The unchecked-legacy scenario, which had a committed drawing and no
        // committed text — while the README and this module both claimed one
        // sat beside every scenario. Making the sentence true beats softening
        // it, and this is the scenario where the text form's own reason for
        // existing (telling a written empty checks list from an inherited
        // one) does the most work (external review, 2026-08-30).
        (
            "vetting/004-legacy-extension/feature/ply.yaml",
            "vetting/004-legacy-extension.txt",
        ),
    ] {
        let yaml = std::fs::read_to_string(root.join(yaml_path))
            .unwrap_or_else(|e| panic!("reading {yaml_path}: {e}"));
        let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{yaml_path} must parse: {e}"));
        let fresh = ply_render::transcript::render_transcript(&doc);

        let committed = std::fs::read_to_string(root.join(text_path)).unwrap_or_else(|e| {
            panic!(
                "{text_path} could not be read ({e}). Regenerate it from the tools workspace \
                 with:\n  cargo run -q -p ply-render -- ../{yaml_path} --text -o ../{text_path}"
            )
        });

        assert_eq!(
            committed, fresh,
            "{text_path} no longer matches what {yaml_path} renders to. These files are \
             committed so that a change to the words shows up in review as a readable diff; a \
             stale one does the opposite, presenting last month's wording as current. \
             Regenerate it from the tools workspace, then read the diff before committing \
             it:\n  cargo run -q -p ply-render -- ../{yaml_path} --text -o ../{text_path}"
        );
    }
}
