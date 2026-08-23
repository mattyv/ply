use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use ply_render::model::parse_document;
use ply_render::svg::render_svg;

/// Minimal static renderer: `ply.yaml` -> SVG. Proves the §7.1 visual
/// grammar is total. Not a GUI, not the future canvas — see SPEC.md §7.1.
#[derive(Parser)]
#[command(name = "ply-render")]
struct Cli {
    /// Path to a ply.yaml (or *.ply.yaml) document.
    input: PathBuf,

    /// Output path for the SVG. Defaults to stdout.
    #[arg(short = 'o', long = "out")]
    out: Option<PathBuf>,
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
            eprintln!("error: {} did not parse as ply.yaml: {e}", cli.input.display());
            return ExitCode::FAILURE;
        }
    };

    let svg = match render_svg(&doc) {
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
