use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use ply_core::visual::svg::RenderOptions;
use ply_core::visual::{parse_depth, render_document};

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
    let options = RenderOptions {
        depth: cli.depth,
        focus: cli.focus,
        collapse: cli.collapse,
    };
    let mut notice = |m: &str| eprintln!("{m}");
    let svg = match render_document(&cli.input, &options, &mut notice) {
        Ok(svg) => svg,
        Err(e) => {
            eprintln!("error: {e}");
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
        None => print!("{svg}"),
    }
    ExitCode::SUCCESS
}
