use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use ply_core::config::derive_links;
use ply_render::model::parse_document;
use ply_render::svg::{RenderOptions, render_svg_with_state_and_links};

/// `--depth` is 1-indexed (top-level boxes are level 1, per §7.1), so 0
/// names no real level and a non-numeric value isn't a level at all. Both
/// get a plain-language message naming what's wrong and what to do, rather
/// than clap's default `invalid digit found in string` (which never says
/// what a depth *is*, let alone a valid one).
fn parse_depth(s: &str) -> Result<usize, String> {
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

/// Minimal static renderer: `ply.yaml` -> SVG. Proves the §7.1 visual
/// grammar is total. Not a GUI, not the future canvas — see The-Ply-Spec.md §7.1.
#[derive(Parser)]
#[command(name = "ply-render")]
struct Cli {
    /// Path to a ply.yaml (or *.ply.yaml) document.
    input: PathBuf,

    /// Output path for the SVG. Defaults to stdout.
    #[arg(short = 'o', long = "out")]
    out: Option<PathBuf>,

    /// Collapse components nested N or more levels deep (top-level = 1) into
    /// one box each, folding their contents. §7.1: depth 1 shows only
    /// top-level boxes, their interiors folded. Omit for the default:
    /// fully expanded, unchanged.
    #[arg(long = "depth", value_parser = parse_depth)]
    depth: Option<usize>,

    /// Render this component (dotted path allowed, e.g. `ingest.book`)
    /// fully expanded; every other component collapses at the point it
    /// diverges from the path down to it. §7.1 mirrors `tree --focus`.
    #[arg(long = "focus")]
    focus: Option<String>,

    /// Collapse this component (dotted path allowed; repeat the flag for
    /// more than one). Everything not named here renders exactly as the
    /// fully-expanded default would — the inverse selection to `--focus`.
    #[arg(long = "collapse")]
    collapse: Vec<String>,

    /// Write the text form instead of the drawing: the same facts, every
    /// one of them, including the ones the drawing only shows on hover.
    /// For reading in a terminal, piping into another tool, or handing to
    /// a model — none of which can hover.
    #[arg(long = "text")]
    text: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let yaml = match std::fs::read_to_string(&cli.input) {
        Ok(y) => y,
        Err(e) => {
            eprintln!("error: could not read {}: {e}", cli.input.display());
            return ExitCode::FAILURE;
        }
    };

    let doc = match parse_document(&yaml) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "error: {} did not parse as ply.yaml: {e}",
                cli.input.display()
            );
            return ExitCode::FAILURE;
        }
    };

    // The folding flags narrow a *drawing* to fit a screen. The text form
    // has no screen to fit and always states the whole document, so a run
    // that asks for both is asking for something that does not exist.
    // Ignoring the flag silently would be worse than refusing: the reader
    // would believe they had been handed a narrowed view.
    if cli.text && (cli.depth.is_some() || cli.focus.is_some() || !cli.collapse.is_empty()) {
        let named = if cli.depth.is_some() {
            "--depth"
        } else if cli.focus.is_some() {
            "--focus"
        } else {
            "--collapse"
        };
        eprintln!(
            "error: --text writes out the whole document, so it cannot be combined with \
             --depth, --focus or --collapse. Those fold parts of the drawing away to fit a \
             screen; the text form has no screen to fit. Drop {named} to get the text, or drop \
             --text to get a folded drawing."
        );
        return ExitCode::FAILURE;
    }

    // §7.1's derive-links brief: a component links to another document
    // when that document's own top-level anchor sits under this one's,
    // resolved from real crate directories before either drawn form below.
    let source_root = cli.input.parent().unwrap_or(std::path::Path::new("."));
    let link_set = derive_links(&doc, source_root);
    for finding in &link_set.findings {
        eprintln!(
            "note: {} {}: {}",
            finding.severity, finding.code, finding.message
        );
    }

    if cli.text {
        // Same read the drawing does -- the text form states everything the
        // drawing shows, shapes included.
        let text = ply_render::transcript::render_transcript_with_state_and_links(
            &doc,
            Some(&ply_render::harness::resolve_state_fields_with_links(
                source_root,
                &doc,
                Some(&link_set.links),
            )),
            Some(&link_set.links),
        );
        return match cli.out {
            Some(path) => match std::fs::write(&path, text) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: could not write {}: {e}", path.display());
                    ExitCode::FAILURE
                }
            },
            None => {
                print!("{text}");
                ExitCode::SUCCESS
            }
        };
    }

    let options = RenderOptions {
        depth: cli.depth,
        focus: cli.focus,
        collapse: cli.collapse,
    };
    // §7.1's rule for `state:` -- the document names the fields, the code
    // says what they are -- so read them from the crate each component's
    // anchor points at before drawing. A document with no code under it
    // resolves nothing and draws the type name alone.
    let state_fields = ply_render::harness::resolve_state_fields_with_links(
        source_root,
        &doc,
        Some(&link_set.links),
    );
    let svg = match render_svg_with_state_and_links(&doc, &options, &state_fields, &link_set.links)
    {
        Ok(svg) => svg,
        Err(e) => {
            eprintln!("error: {} could not be rendered: {e}", cli.input.display());
            return ExitCode::FAILURE;
        }
    };

    // A selection that selects nothing is worth saying out loud. On a flat
    // document `--depth 1` and `--focus x` produce exactly the default
    // drawing, and silence there reads as "the flag did nothing visible, so
    // something is broken" -- a smoke test on a real project recorded it as
    // a bug before deciding it was correct behaviour (2026-08-28). The
    // check is the honest one: render the default too, and compare. It
    // costs one extra layout pass and cannot disagree with what was drawn.
    if options.depth.is_some() || options.focus.is_some() || !options.collapse.is_empty() {
        let plain = render_svg_with_state_and_links(
            &doc,
            &RenderOptions::default(),
            &state_fields,
            &link_set.links,
        );
        if plain.as_deref().ok() == Some(svg.as_str()) {
            eprintln!(
                "note: this drawing is identical to the one with no --depth/--focus/--collapse \
                 at all. Nothing in {} nests deeply enough for that selection to fold anything \
                 away, so the flag had nothing to do -- not an error, and not a sign the flag \
                 was ignored.",
                cli.input.display()
            );
        }
    }

    match cli.out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, svg) {
                eprintln!("error: could not write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
        None => {
            print!("{svg}");
        }
    }

    ExitCode::SUCCESS
}
