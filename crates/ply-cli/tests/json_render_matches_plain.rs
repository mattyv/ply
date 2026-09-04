//! `cargo ply render <dir>` and `cargo ply --json render <dir>` must draw
//! the same picture for the same input: every visual client (VS Code,
//! JetBrains, the browser viewer) reads the JSON envelope's `svg` field,
//! never the plain SVG path, so the two can never be allowed to disagree
//! about what the document actually contains.
//!
//! They are not expected to be byte-identical: the JSON envelope always
//! carries a `data-element-id` attribute and a short "completed evidence"
//! tooltip line on every element (even a purely declared one), which the
//! plain path never attaches at all -- that is the JSON path doing its own
//! job, not a defect. What must agree is the actual *picture*: the same
//! boxes at the same size, because they were laid out from the same
//! resolved `state:` fields.
//!
//! They once did not agree at all: the JSON path never read a component's
//! declared `state:` type off real source, so its `svg` was roughly half
//! the size of the plain path's for a document with real state to resolve,
//! and its tooltips said state was unresolved when the code was right
//! there.

use std::process::Command;

fn cargo_ply() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("cargo-ply")
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn render_plain(doc: &std::path::Path) -> String {
    let out = Command::new(cargo_ply())
        .args(["ply", "render"])
        .arg(doc)
        .output()
        .expect("running cargo-ply render");
    assert!(
        out.status.success(),
        "plain render of {} failed: {}",
        doc.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("plain render emitted valid UTF-8")
}

fn render_json_svg(doc: &std::path::Path) -> String {
    let out = Command::new(cargo_ply())
        .args(["ply", "render"])
        .arg(doc)
        .arg("--json")
        .output()
        .expect("running cargo-ply render --json");
    assert!(
        out.status.success(),
        "json render of {} failed: {}",
        doc.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("render --json emitted valid JSON");
    json["svg"]
        .as_str()
        .expect("the envelope carries an `svg` field")
        .to_string()
}

/// The `width="..."` / `height="..."` a browser lays the drawing out at --
/// identical values mean identical boxes, which for a document with `state:`
/// fields is only possible when both paths drew the same resolved rows.
fn svg_dims(svg: &str) -> (f64, f64) {
    fn attr(svg: &str, name: &str) -> f64 {
        let needle = format!("{name}=\"");
        let start = svg
            .find(&needle)
            .unwrap_or_else(|| panic!("every drawing states `{name}`"))
            + needle.len();
        let rest = &svg[start..];
        rest[..rest.find('"').unwrap()].parse().unwrap()
    }
    (attr(svg, "width"), attr(svg, "height"))
}

#[test]
fn the_json_envelopes_svg_lays_out_the_same_picture_as_the_plain_render() {
    // This document has real code underneath it and declares `state:` in
    // several places (see `ply.yaml` at the repo root), so it is the case
    // that actually exercises state resolution -- a document with nothing
    // to resolve could pass this test even if resolution were silently
    // broken for everyone who has code to read.
    let doc = repo_root().join("ply.yaml");
    let plain = render_plain(&doc);
    let json_svg = render_json_svg(&doc);

    assert_eq!(
        svg_dims(&plain),
        svg_dims(&json_svg),
        "the plain render and the JSON envelope's svg must lay out the same boxes at the \
         same size for the same input -- a size mismatch here means one of them drew rows \
         (most likely `state:` rows) the other did not"
    );

    let resolved_sentence =
        "each name was found in the code, and each shape read off what that field really is";
    let unresolved_sentence = "there is no code here to read them from";
    let plain_resolved = plain.matches(resolved_sentence).count();
    let json_resolved = json_svg.matches(resolved_sentence).count();
    assert!(
        plain_resolved > 0,
        "the premise: this document's state actually resolves in the plain render"
    );
    assert_eq!(
        plain_resolved, json_resolved,
        "every component whose state resolved in the plain render must resolve exactly the \
         same way in the JSON envelope -- plain resolved {plain_resolved}, json resolved \
         {json_resolved}"
    );
    assert_eq!(
        json_svg.matches(unresolved_sentence).count(),
        0,
        "every state:-declaring component in this document has real code to read, so none of \
         them should say otherwise in the JSON envelope"
    );

    // A regression guard against the exact shape of the historic bug: the
    // JSON path silently dropping every `state:` row made its svg roughly
    // half the size of the plain path's. The JSON path legitimately carries
    // a little more than the plain one (the per-element evidence
    // attributes described above), so it is never expected to be smaller.
    assert!(
        json_svg.len() >= plain.len(),
        "the JSON envelope's svg ({} bytes) is smaller than the plain render's ({} bytes) -- \
         it should only ever carry as much or slightly more, never less",
        json_svg.len(),
        plain.len()
    );
}

#[test]
fn a_document_with_no_crate_under_it_still_draws_the_type_name_and_says_so_honestly() {
    // §7.1's rule for `state:` is that the document names the type and the
    // fields, and the *code* says what those fields are. `vetting/` has no
    // crate of its own, so nothing under this document's `state:` blocks
    // can resolve -- the honest drawing says exactly that, in both forms,
    // rather than silently going blank or inventing a fields list the
    // document alone cannot back up.
    // Written here rather than borrowed from `vetting/`: this test broke once
    // already because it leaned on `vetting/003`, and that document later
    // started declaring its field shapes, so it stopped being a names-only
    // document and the sentence under test stopped applying to it. A fixture
    // the test owns cannot be reworded out from under it by an unrelated
    // change to an example.
    let dir = std::env::temp_dir().join(format!("ply-nocode-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let doc = dir.join("ply.yaml");
    std::fs::write(
        &doc,
        "ply: 1\ncomponents:\n  thing:\n    anchor: thing\n    state:\n      of: Held\n      show: [a, b]\n",
    )
    .expect("write the names-only document");
    let plain = render_plain(&doc);
    let json_svg = render_json_svg(&doc);

    let honest_sentence = "a bare name carries no shape";
    assert!(
        plain.contains(honest_sentence),
        "the plain render of a document with no crate under it must still say plainly that \
         its state could not be resolved"
    );
    assert!(
        json_svg.contains(honest_sentence),
        "the JSON envelope's svg must say the same honest thing the plain render does -- a \
         document with no code to read from is exactly the case that must never be drawn as \
         though it resolved"
    );
    assert_eq!(
        svg_dims(&plain),
        svg_dims(&json_svg),
        "even with nothing to resolve, both forms must lay out the same boxes at the same size"
    );
}
