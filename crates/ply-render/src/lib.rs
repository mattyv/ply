/// Re-exported so existing call sites (`ply_render::model::...`) and tests
/// keep working unchanged. The model itself lives in the product
/// (`ply_core::model`) since Phase 1a -- this renderer consumes it rather
/// than owning a second copy of the `ply.yaml` grammar.
pub use ply_core::model;
mod layout;
pub mod svg;

use std::path::Path;

use crate::model::parse_document;
use crate::svg::{RenderOptions, render_svg_with_options};

/// One rendering run, start to finish: read the document, draw it, and say
/// so if the selection folded nothing away.
///
/// This lives in the library rather than in either caller because there are
/// now two of them -- `cargo ply render` and the standalone `ply-render`
/// binary -- and a rule that only one of them applies is a rule with a hole
/// in it. The "this folded nothing" notice in particular was written for
/// someone using the tool for the first time; it would be a poor joke for
/// it to appear from one entry point and not the other.
///
/// Returns the SVG. Notices go to `notice` (stderr for both callers), so
/// nothing this function writes can contaminate an SVG on stdout.
pub fn render_document(
    input: &Path,
    options: &RenderOptions,
    notice: &mut dyn FnMut(&str),
) -> Result<String, String> {
    let yaml = std::fs::read_to_string(input)
        .map_err(|e| format!("could not read {}: {e}", input.display()))?;
    let doc = parse_document(&yaml)
        .map_err(|e| format!("{} did not parse as ply.yaml: {e}", input.display()))?;
    let svg = render_svg_with_options(&doc, options)
        .map_err(|e| format!("{} could not be rendered: {e}", input.display()))?;

    // A selection that selects nothing is worth saying out loud. On a flat
    // document `--depth 1` and `--focus x` produce exactly the default
    // drawing, and silence there reads as "the flag did nothing visible, so
    // something is broken" -- a smoke test on a real project recorded it as
    // a bug before deciding it was correct behaviour (2026-08-28). The
    // check is the honest one: render the default too, and compare. It
    // costs one extra layout pass and cannot disagree with what was drawn.
    if options.depth.is_some() || options.focus.is_some() || !options.collapse.is_empty() {
        let plain = render_svg_with_options(&doc, &RenderOptions::default());
        if plain.as_deref().ok() == Some(svg.as_str()) {
            notice(&format!(
                "note: this drawing is identical to the one with no --depth/--focus/--collapse \
                 at all. Nothing in {} nests deeply enough for that selection to fold anything \
                 away, so the flag had nothing to do -- not an error, and not a sign the flag \
                 was ignored.",
                input.display()
            ));
        }
    }
    Ok(svg)
}

/// Writes a rendered document to `out`, or to stdout when `out` is `None`.
pub fn write_rendering(svg: &str, out: Option<&Path>) -> Result<(), String> {
    match out {
        Some(path) => std::fs::write(path, svg)
            .map_err(|e| format!("could not write {}: {e}", path.display())),
        None => {
            print!("{svg}");
            Ok(())
        }
    }
}

/// `--depth` is 1-indexed (top-level boxes are level 1, per §7.1), so 0
/// names no real level and a non-numeric value isn't a level at all. Both
/// get a plain-language message naming what's wrong and what to do, rather
/// than clap's default `invalid digit found in string` (which never says
/// what a depth *is*, let alone a valid one).
///
/// Shared by both entry points for the same reason the run itself is: a
/// message written to the newbie bar is worth having from whichever command
/// the reader happened to type.
pub fn parse_depth(s: &str) -> Result<usize, String> {
    match s.parse::<usize>() {
        Ok(0) => Err(
            "--depth 0 doesn't select anything: nesting levels start at 1 for the top-level \
             boxes — pass --depth 1 or higher, or drop --depth to render everything expanded"
                .to_string(),
        ),
        Ok(n) => Ok(n),
        Err(_) => Err(format!(
            "--depth wants a whole number of nesting levels, counting the top-level boxes as \
             1 — {s:?} is not a number"
        )),
    }
}
