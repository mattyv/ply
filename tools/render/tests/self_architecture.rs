//! ARCHITECTURE.md shows a picture of Ply's own crate structure and tells
//! the reader it is rendered from `ply.yaml` rather than drawn by hand. That
//! sentence is a claim, and a committed SVG is exactly the kind of artifact
//! that stops being true quietly: someone adds a crate, the spec gains a
//! box, the picture in the document keeps showing the old shape, and the
//! page now lies about the thing it exists to describe.
//!
//! So the claim is checked. This renders the real `ply.yaml` and compares it
//! against the committed file, byte for byte.

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

#[test]
fn the_architecture_diagram_matches_the_spec_it_claims_to_be_rendered_from() {
    let root = repo_root();
    let yaml = std::fs::read_to_string(root.join("ply.yaml")).expect("reading ply.yaml");
    let doc = parse_document(&yaml).expect("Ply's own ply.yaml must parse");
    let fresh = render_svg_with_options(&doc, &RenderOptions::default())
        .expect("Ply's own ply.yaml must render");

    let committed_path = root.join("docs/ply-self.svg");
    let committed = std::fs::read_to_string(&committed_path).unwrap_or_else(|e| {
        panic!(
            "ARCHITECTURE.md embeds {} and it could not be read ({e}). Regenerate it with:\n  \
             cargo run --release -p ply-render -- ply.yaml -o docs/ply-self.svg",
            committed_path.display()
        )
    });

    assert_eq!(
        committed, fresh,
        "docs/ply-self.svg no longer matches what ply.yaml renders to, so the diagram in \
         ARCHITECTURE.md is showing a structure this repository no longer has. Regenerate it \
         from the tools workspace, then look at the result before committing it:\n  \
         cargo run --release -p ply-render -- ply.yaml -o docs/ply-self.svg"
    );
}
