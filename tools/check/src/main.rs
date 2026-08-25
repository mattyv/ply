use std::process::ExitCode;

/// `ply-check <path>`: parse a `ply.yaml` document and validate it against
/// The-Ply-Spec.md's statically-checkable document-local rules (§5.1a, §5.1, §5.6).
/// Never renders anything. Exit codes mirror the `cargo ply check` contract
/// in The-Ply-Spec.md §6: 0 clean, 1 violations, 2 tool error.
fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: ply-check <path-to-ply.yaml>");
        return ExitCode::from(2);
    };

    let yaml = match std::fs::read_to_string(&path) {
        Ok(y) => y,
        Err(e) => {
            eprintln!("error: could not read {path}: {e}");
            return ExitCode::from(2);
        }
    };

    let doc = match ply_core::model::parse_document(&yaml) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {path} did not parse as ply.yaml: {e}");
            return ExitCode::from(2);
        }
    };

    let diagnostics = ply_core::check::run_checks(&doc);
    for d in &diagnostics {
        println!("{d}");
    }

    // §5.3: `W`-severity findings are reported but don't fail the run on
    // their own — only an `E`-severity (or other non-advisory) diagnostic
    // does. An empty list is vacuously "all advisory".
    if diagnostics
        .iter()
        .all(ply_core::check::Diagnostic::is_advisory)
    {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
