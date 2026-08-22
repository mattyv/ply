//! The `ply` command line (§14).

use clap::{Parser, Subcommand};
use ply_diag::Color;
use ply_driver::{Analysis, CheckOutcome, FmtOutcome, collect_ply_files};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "ply",
    version,
    about = "The Ply toolchain",
    long_about = "Ply: a language for LLM-driven development — local reasoning, \
                  machine-checkable correctness, counterexample-first diagnostics."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the front end: parse, resolve names, check conventions. No verification.
    Check {
        /// A file, or a directory to search for `.ply` files.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit diagnostics as JSON (the stable agent-facing surface).
        #[arg(long)]
        json: bool,
        /// Print the parsed syntax tree instead of checking.
        #[arg(long)]
        dump_ast: bool,
    },
    /// Rewrite files in the one canonical format.
    Fmt {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Report files that are not formatted instead of rewriting them.
        #[arg(long)]
        check: bool,
    },
    /// Explain a diagnostic code, e.g. `ply explain-code E0301`.
    ExplainCode { code: String },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { path, json, dump_ast } => cmd_check(path, json, dump_ast),
        Command::Fmt { path, check } => cmd_fmt(path, check),
        Command::ExplainCode { code } => cmd_explain_code(&code),
    }
}

fn color() -> Color {
    Color::from_env(std::io::stderr().is_terminal())
}

fn cmd_check(path: PathBuf, json: bool, dump_ast: bool) -> ExitCode {
    let files = match collect_ply_files(&path) {
        Ok(f) => f,
        Err(e) => return fail(&format!("cannot read {}: {e}", path.display())),
    };
    if files.is_empty() {
        eprintln!("no .ply files under {}", path.display());
        return ExitCode::SUCCESS;
    }
    let analysis = match Analysis::of_paths(&files) {
        Ok(a) => a,
        Err(e) => return fail(&format!("cannot read a source file: {e}")),
    };

    if dump_ast {
        for (_, ast) in analysis.asts() {
            println!("{}", ply_syntax::dump::dump_file(ast));
        }
        return ExitCode::SUCCESS;
    }

    if json {
        println!("{}", analysis.json());
    } else {
        let text = analysis.human(color());
        if !text.is_empty() {
            eprint!("{text}");
        }
        match analysis.outcome() {
            CheckOutcome::Clean => {
                println!("checked {} file{}: no problems", files.len(), plural(files.len()))
            }
            CheckOutcome::Warnings(_) | CheckOutcome::Errors(_) => {}
        }
    }
    if analysis.has_errors() { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn cmd_fmt(path: PathBuf, check_only: bool) -> ExitCode {
    let files = match collect_ply_files(&path) {
        Ok(f) => f,
        Err(e) => return fail(&format!("cannot read {}: {e}", path.display())),
    };
    let mut changed = Vec::new();
    let mut broken = false;
    for file in &files {
        let src = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                broken = true;
                eprintln!("cannot read {}: {e}", file.display());
                continue;
            }
        };
        match ply_driver::format_source(&file.to_string_lossy(), &src) {
            FmtOutcome::Unchanged => {}
            FmtOutcome::Changed(text) => {
                changed.push(file.clone());
                if !check_only && let Err(e) = std::fs::write(file, text) {
                    broken = true;
                    eprintln!("cannot write {}: {e}", file.display());
                }
            }
            FmtOutcome::Failed(diags) => {
                broken = true;
                let mut sm = ply_diag::SourceMap::new();
                sm.add(file.to_string_lossy().to_string(), src);
                eprint!("{}", ply_diag::render_all(&sm, &diags, color()));
            }
        }
    }
    if check_only {
        for f in &changed {
            println!("{}", f.display());
        }
        if !changed.is_empty() {
            eprintln!(
                "{} file{} would be reformatted; run `ply fmt`",
                changed.len(),
                plural(changed.len())
            );
        }
        if changed.is_empty() && !broken {
            println!("{} file{} already formatted", files.len(), plural(files.len()));
        }
        return if changed.is_empty() && !broken { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    if !changed.is_empty() {
        println!("formatted {} file{}", changed.len(), plural(changed.len()));
    }
    if broken { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn cmd_explain_code(code: &str) -> ExitCode {
    let wanted = code.to_ascii_uppercase();
    match ply_diag::Code::ALL.iter().find(|c| c.as_str() == wanted) {
        Some(c) => {
            println!("{}  [{} / {}]", c.as_str(), c.severity(), c.phase());
            println!("{}", c.blurb());
            ExitCode::SUCCESS
        }
        None => fail(&format!("unknown diagnostic code `{code}`")),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn fail(msg: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "ply: {msg}");
    ExitCode::FAILURE
}
