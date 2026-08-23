use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use ply_render::model::parse_document;
use ply_render::svg::{RenderOptions, render_svg_with_options};

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

    let options = RenderOptions {
        depth: cli.depth,
        focus: cli.focus,
        collapse: cli.collapse,
    };
    let svg = match render_svg_with_options(&doc, &options) {
        Ok(svg) => svg,
        Err(e) => {
            eprintln!("error: {} could not be rendered: {e}", cli.input.display());
            return ExitCode::FAILURE;
        }
    };

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
