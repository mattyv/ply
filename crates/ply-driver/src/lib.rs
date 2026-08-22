//! The driver: file discovery, phase orchestration and the two output surfaces (§14).
//!
//! Everything the `ply` binary does lives here as a library function so it can be tested
//! without spawning a process. `main.rs` is only argument parsing and exit codes.

use ply_diag::{Color, Diagnostics, FileId, SourceMap};
use ply_syntax::ast;
use std::io;
use std::path::{Path, PathBuf};

/// The result of running the front end over a set of files.
pub struct Analysis {
    sm: SourceMap,
    files: Vec<(FileId, ast::File)>,
    diags: Diagnostics,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    Clean,
    Warnings(usize),
    Errors(usize),
}

impl Analysis {
    /// Run the front end over in-memory sources, in the order given.
    pub fn of_sources(sources: Vec<(String, String)>) -> Analysis {
        let mut sm = SourceMap::new();
        let mut files = Vec::new();
        let mut diags = Diagnostics::new();
        for (name, text) in sources {
            let id = sm.add(name, text);
            let ast = ply_syntax::parse_file(id, &sm.source(id), &mut diags);
            ply_syntax::naming::check_file(&ast, &mut diags);
            files.push((id, ast));
        }
        Analysis { sm, files, diags: diags.sorted() }
    }

    pub fn of_paths(paths: &[PathBuf]) -> io::Result<Analysis> {
        let mut sources = Vec::new();
        for p in paths {
            sources.push((display_path(p), std::fs::read_to_string(p)?));
        }
        Ok(Analysis::of_sources(sources))
    }

    pub fn sources(&self) -> &SourceMap {
        &self.sm
    }

    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diags
    }

    pub fn asts(&self) -> &[(FileId, ast::File)] {
        &self.files
    }

    pub fn has_errors(&self) -> bool {
        self.diags.has_errors()
    }

    pub fn outcome(&self) -> CheckOutcome {
        let e = self.diags.error_count();
        if e > 0 {
            CheckOutcome::Errors(e)
        } else if self.diags.warning_count() > 0 {
            CheckOutcome::Warnings(self.diags.warning_count())
        } else {
            CheckOutcome::Clean
        }
    }

    pub fn human(&self, color: Color) -> String {
        ply_diag::render_all(&self.sm, &self.diags, color)
    }

    /// The stable JSON surface (§13), pretty-printed so goldens diff cleanly.
    pub fn json(&self) -> String {
        serde_json::to_string_pretty(&self.diags.to_json(&self.sm))
            .unwrap_or_else(|e| format!("[] /* {e} */"))
    }
}

// ---------------------------------------------------------------------------------------
// fmt
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum FmtOutcome {
    /// The file is already canonical.
    Unchanged,
    /// The canonical text, which differs from the input.
    Changed(String),
    /// The file does not parse; formatting is refused rather than guessed at.
    Failed(Diagnostics),
}

pub fn format_source(name: &str, src: &str) -> FmtOutcome {
    let mut sm = SourceMap::new();
    let id = sm.add(name, src);
    let mut diags = Diagnostics::new();
    let ast = ply_syntax::parse_file(id, &sm.source(id), &mut diags);
    if diags.has_errors() {
        return FmtOutcome::Failed(diags.sorted());
    }
    let out = ply_syntax::format_file(&ast);
    if out == src { FmtOutcome::Unchanged } else { FmtOutcome::Changed(out) }
}

// ---------------------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------------------

/// Every `.ply` file under `path`, sorted. A file resolves to itself; build and VCS
/// directories are skipped.
pub fn collect_ply_files(path: &Path) -> io::Result<Vec<PathBuf>> {
    let meta = std::fs::metadata(path)?;
    if meta.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut out = Vec::new();
    walk(path, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if name.starts_with('.') || name == "target" {
                continue;
            }
            walk(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "ply") {
            out.push(path);
        }
    }
    Ok(())
}

fn display_path(p: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    p.strip_prefix(&cwd).unwrap_or(p).to_string_lossy().to_string()
}
