mod verify;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use verify::VerifyOptions;

/// cargo-ply -- the Ply CLI. This M3 thin slice implements only `verify`
/// (plus the global `--json`), per the M3 brief's explicit scope.
#[derive(Parser)]
#[command(name = "cargo-ply", bin_name = "cargo-ply")]
struct Cli {
    /// Emit the §8 JSON envelope instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run checks via engines and write cex artifacts (§6).
    Verify {
        /// Path to the crate directory containing `ply.yaml`.
        path: PathBuf,
        #[arg(long, default_value_t = 60)]
        engine_timeout: u32,
    },
}

fn main() -> anyhow::Result<()> {
    // `cargo ply verify ...` invokes this binary as `cargo-ply ply verify
    // ...` (cargo's external-subcommand convention re-passes the
    // subcommand name) -- drop a leading "ply" so this also works as a
    // direct invocation (`cargo-ply verify ...`, what the e2e tests use).
    let mut raw: Vec<String> = std::env::args().collect();
    if raw.len() > 1 && raw[1] == "ply" {
        raw.remove(1);
    }
    let cli = Cli::parse_from(raw);

    match cli.command {
        Commands::Verify { path, engine_timeout } => {
            let opts = VerifyOptions { engine_timeout_secs: engine_timeout };
            let envelope = verify::verify_crate(&path, &opts)?;
            if cli.json {
                println!("{}", envelope.to_json_pretty());
            } else {
                print_human(&envelope);
            }
            std::process::exit(exit_code_for(&envelope));
        }
    }
}

fn print_human(envelope: &ply_core::diag::Envelope) {
    fn walk(node: &ply_core::diag::Node, depth: usize) {
        println!("{}{} — {}", "  ".repeat(depth), node.id, node.verdict);
        for child in &node.children {
            walk(child, depth + 1);
        }
    }
    walk(&envelope.root, 0);
    for d in &envelope.diagnostics {
        println!("[{}] {} — {}", d.code, d.node_id, d.title);
    }
}

fn exit_code_for(envelope: &ply_core::diag::Envelope) -> i32 {
    let has_violation = envelope
        .diagnostics
        .iter()
        .any(|d| d.severity == "error");
    if has_violation {
        1
    } else {
        0
    }
}
