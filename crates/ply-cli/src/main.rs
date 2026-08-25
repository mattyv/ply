mod audit;
mod check;
mod shared;
mod verify;
mod worklist;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
// The absence vocabulary (§1: "an absence is a name, not a slot") lives in
// ply-core, because a second consumer now reads it -- the rule that decides
// whether a result may be recorded and reused at all (§5.2a records only
// results that earned evidence). Two copies of one vocabulary is how the
// next absence gets missed by one of them, which is the exact shape of the
// defect that put this rule here.
use ply_core::diag::is_absence;
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
    /// Validate ply.yaml and the anchors it points at (§6). Fast, no engines.
    Check {
        /// Path to the crate directory containing `ply.yaml`.
        path: PathBuf,
    },
    /// List the trust surface: what this codebase's evidence rests on and
    /// Ply never checks (§6). Fast, no engines.
    Audit {
        /// Path to the crate directory containing `ply.yaml`.
        path: PathBuf,
    },
    /// List what is owed: unresolved markers and assumed contracts still
    /// waiting on evidence (§6). Fast, no engines.
    Worklist {
        /// Path to the crate directory containing `ply.yaml`.
        path: PathBuf,
    },
    /// Run checks via engines and write cex artifacts (§6).
    Verify {
        /// Path to the crate directory containing `ply.yaml`.
        path: PathBuf,
        /// Per-check engine time budget, in seconds. Omit to use the
        /// shape-aware default (a `Vec`-typed `bounded(k)` check gets more
        /// budget than a scalar one -- see
        /// `verify::default_engine_timeout_secs`); pass a value to override
        /// it for every check in this run.
        #[arg(long)]
        engine_timeout: Option<u32>,
        /// What makes this run fail (§6). The default, `evidence`, fails on
        /// any node whose verdict is an absence of evidence -- a timeout, an
        /// unsupported shape, a tool error, an unclaimed claim, a missing
        /// engine -- as well as on a violation. `error` is the looser,
        /// pre-2026-08-25 behaviour: only an error-severity diagnostic
        /// fails. `warn` is stricter still: any warning fails too.
        #[arg(long, value_enum, default_value_t = FailOn::Evidence)]
        fail_on: FailOn,
        /// Replay a recorded `fuzz(n)` run exactly: 64 hex characters, as
        /// printed in the §8 envelope's `evidence.seed`. Omit and each fn's
        /// seed is derived from its own name and contract text, which is
        /// still identical run to run for identical source (§5.4c).
        #[arg(long)]
        seed: Option<String>,
    },
}

/// §6's `--fail-on`. It relaxes the default; it never enables it -- §1's
/// absence-of-evidence principle is what the default implements, and
/// choosing `error` is a statement that this run's green means less.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum FailOn {
    /// Any diagnostic of warning severity or worse fails the run.
    Warn,
    /// Any absence of evidence, or any error-severity diagnostic (default).
    Evidence,
    /// Only an error-severity diagnostic.
    Error,
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
        Commands::Check { path } => {
            let report = check::check_crate(&path)?;
            if cli.json {
                println!("{}", report.envelope.to_json_pretty());
            } else {
                check::print_human(&report);
            }
            std::process::exit(report.exit_code());
        }
        Commands::Audit { path } => {
            let report = audit::audit_crate(&path)?;
            if cli.json {
                println!("{}", report.envelope.to_json_pretty());
            } else {
                audit::print_human(&report);
            }
            std::process::exit(report.exit_code());
        }
        Commands::Worklist { path } => {
            let report = worklist::worklist_crate(&path)?;
            if cli.json {
                println!("{}", report.envelope.to_json_pretty());
            } else {
                worklist::print_human(&report);
            }
            std::process::exit(report.exit_code());
        }
        Commands::Verify {
            path,
            engine_timeout,
            fail_on,
            seed,
        } => {
            let seed = match seed {
                Some(text) => match ply_core::fuzz_gen::seed_from_hex(&text) {
                    Some(bytes) => Some(bytes),
                    // Refused, never padded into a different run: a `--seed`
                    // that silently ran something else would be worse than
                    // no `--seed` at all.
                    None => anyhow::bail!(
                        "`--seed` takes the 64 hexadecimal characters printed as `evidence.seed` \
                         in a previous run's JSON envelope; `{text}` is not that. Run \
                         `cargo ply verify <path> --json` and copy the seed from the node whose \
                         run you want to replay."
                    ),
                },
                None => None,
            };
            let opts = VerifyOptions {
                engine_timeout_secs: engine_timeout,
                seed,
            };
            let envelope = verify::verify_crate(&path, &opts)?;
            if cli.json {
                println!("{}", envelope.to_json_pretty());
            } else {
                print_human(&envelope);
            }
            std::process::exit(exit_code_for(&envelope, fail_on));
        }
    }
}

/// The marks a node line carries beside its verdict.
///
/// §7.1 gives statuses their own visual channel — corner markers on the
/// node, never a change to the fill, because a status is a different kind
/// of fact from a verdict (D6). The terminal had no such channel: a result
/// resting on a promise nobody had checked printed as a bare pass, exactly
/// like one resting on checked code. The qualifier and the debt were in the
/// JSON envelope and in the diagnostic prose underneath, and missing from
/// the one line most people read.
///
/// Plain words, not codes (CLAUDE.md's newbie bar): `conditional` and
/// `owed-evidence` are Ply's names for these facts, and neither means
/// anything to a reader who has not read the spec.
fn node_marks(node: &ply_core::diag::Node) -> Vec<&'static str> {
    let mut marks = Vec::new();
    if node.statuses.iter().any(|s| s == "conditional") {
        marks.push("assumed");
    }
    if node.statuses.iter().any(|s| s == "owed-evidence") {
        marks.push("evidence owed");
    }
    // Last, and from its own field rather than from `statuses`: reuse is
    // not a qualifier on the evidence (D6), it is a fact about when the run
    // happened. A person reading `bounded(2)` should be able to tell
    // whether that happened just now or was carried forward from an earlier
    // run whose inputs still hash the same (§5.2a).
    if node.reused {
        marks.push("reused");
    }
    marks
}

/// What each mark means, printed once beneath the tree and only when the
/// tree actually carries it. A mark a reader cannot decode is decoration.
const MARK_GLOSS: [(&str, &str); 3] = [
    (
        "assumed",
        "this result rests on a promise Ply was handed and did not check — if the promise is \
         wrong, the result is wrong with it",
    ),
    (
        "evidence owed",
        "nothing has run the real code against that promise yet; the lines below name it and say \
         what would settle it",
    ),
    (
        "reused",
        "this result was not re-run: an earlier run recorded it, and everything it depended on — \
         the code, the promises it assumes, the checks, the engines, Ply's own version — hashes \
         the same today",
    ),
];

/// The §7 tree as a person reads it in a terminal, with the status marks
/// and — when any appear — what they mean.
fn tree_report(envelope: &ply_core::diag::Envelope) -> String {
    fn walk(node: &ply_core::diag::Node, depth: usize, out: &mut String, seen: &mut Vec<&str>) {
        let marks = node_marks(node);
        let suffix = if marks.is_empty() {
            String::new()
        } else {
            for m in &marks {
                if !seen.contains(m) {
                    seen.push(m);
                }
            }
            format!("  [{}]", marks.join(", "))
        };
        out.push_str(&format!(
            "{}{} — {}{suffix}\n",
            "  ".repeat(depth),
            node.id,
            node.verdict
        ));
        for child in &node.children {
            walk(child, depth + 1, out, seen);
        }
    }
    let mut out = String::new();
    let mut seen: Vec<&str> = Vec::new();
    walk(&envelope.root, 0, &mut out, &mut seen);
    if !seen.is_empty() {
        out.push('\n');
        for (mark, gloss) in MARK_GLOSS {
            if seen.contains(&mark) {
                let label = format!("[{mark}]");
                out.push_str(&format!("  {label:<17}{gloss}\n"));
            }
        }
        // The diagnostics come next, and they are paragraphs. Without this
        // the gloss and the first diagnostic run together into one block.
        out.push('\n');
    }
    out
}

fn print_human(envelope: &ply_core::diag::Envelope) {
    print!("{}", tree_report(envelope));
    for d in &envelope.diagnostics {
        println!("[{}] {} — {}", d.code, d.node_id, d.title);
    }
}

/// Every absence a node carries, in either field, over the whole tree.
fn walk_absences(node: &ply_core::diag::Node, f: &mut impl FnMut(&str)) {
    f(&node.verdict);
    for s in &node.statuses {
        f(s);
    }
    for c in &node.children {
        walk_absences(c, f);
    }
}

/// §6's exit-code table, with the row that was missing until 2026-08-25:
/// **a run that checked nothing is not a clean run**. Vetting 004's finding
/// 1 is the case this closes -- root verdict `timeout`, two of five claims
/// with no evidence at all, 7m13s, exit 0, CI green.
///
/// 3 (missing engine) and 2 (tool error) outrank 1 because they say the run
/// itself did not happen properly, which is a different fact from "your code
/// is wrong" -- §6's own ordering, implemented here for the first time.
fn exit_code_for(envelope: &ply_core::diag::Envelope, fail_on: FailOn) -> i32 {
    let has_error = envelope.diagnostics.iter().any(|d| d.severity == "error");
    let has_warning = envelope
        .diagnostics
        .iter()
        .any(|d| d.severity == "warning" || d.severity == "error");

    let mut absences: Vec<String> = Vec::new();
    walk_absences(&envelope.root, &mut |v| {
        if is_absence(v) {
            absences.push(v.to_string());
        }
    });

    let fails = match fail_on {
        FailOn::Error => has_error,
        FailOn::Evidence => has_error || !absences.is_empty(),
        FailOn::Warn => has_warning || !absences.is_empty(),
    };
    if !fails {
        return 0;
    }
    if absences.iter().any(|v| v == "engine-missing") {
        return 3;
    }
    if absences.iter().any(|v| v.starts_with("tool_error")) {
        return 2;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use ply_core::diag::{Envelope, Node};

    fn envelope(verdicts: &[&str]) -> Envelope {
        envelope_with_statuses(verdicts, &[])
    }

    fn envelope_with_statuses(verdicts: &[&str], statuses: &[&str]) -> Envelope {
        let children: Vec<Node> = verdicts
            .iter()
            .map(|v| Node {
                id: "f".into(),
                kind: "fn".into(),
                verdict: (*v).into(),
                statuses: statuses.iter().map(|s| (*s).to_string()).collect(),
                reused: false,
                evidence: None,
                children: vec![],
            })
            .collect();
        Envelope {
            command: "verify".into(),
            ply_version: "test".into(),
            root: Node {
                id: "workspace".into(),
                kind: "workspace".into(),
                verdict: verdicts.first().copied().unwrap_or("unclaimed").into(),
                statuses: vec![],
                reused: false,
                evidence: None,
                children,
            },
            diagnostics: vec![],
            coverage: None,
            trust_surface: None,
            open_items: None,
        }
    }

    /// The whole point of §1's principle: a run in which nothing was checked
    /// is not a passing run, however few error-severity diagnostics it
    /// carries. Vetting 004's s2 exited 0 on exactly this shape.
    #[test]
    fn a_run_whose_checks_all_timed_out_fails_by_default() {
        assert_eq!(exit_code_for(&envelope(&["timeout"]), FailOn::Evidence), 1);
    }

    #[test]
    fn real_evidence_still_exits_zero() {
        assert_eq!(
            exit_code_for(&envelope(&["bounded(2)", "fuzzed(256)"]), FailOn::Evidence),
            0
        );
    }

    /// `--fail-on=error` is the documented opt-out, and it must reproduce
    /// the old behaviour exactly -- otherwise it is not an opt-out, it is a
    /// third thing nobody asked for.
    #[test]
    fn fail_on_error_is_the_opt_out_and_lets_an_absence_through() {
        assert_eq!(exit_code_for(&envelope(&["timeout"]), FailOn::Error), 0);
    }

    /// The rule is over *names*, not over the field a name sits in (§1, D2 of
    /// the 2026-08-25 adversarial review). A `mutate` check whose engine is
    /// missing leaves the fn's verdict alone -- the fuzz check that ran is
    /// still real evidence -- and records the absence as a status. Reading
    /// only the verdict made that run exit 0, which said a declared check
    /// had been performed when nothing had performed it.
    #[test]
    fn an_absence_recorded_as_a_status_fails_the_run_like_one_recorded_as_a_verdict() {
        assert_eq!(
            exit_code_for(
                &envelope_with_statuses(&["fuzzed(64)"], &["engine-missing"]),
                FailOn::Evidence
            ),
            3,
            "a declared check with no engine behind it is §6's exit 3, wherever the envelope \
             records it"
        );
        assert_eq!(
            exit_code_for(
                &envelope_with_statuses(&["fuzzed(64)"], &["inconclusive"]),
                FailOn::Evidence
            ),
            1,
            "a check that ran and established nothing earned no evidence either"
        );
        assert_eq!(
            exit_code_for(
                &envelope_with_statuses(&["fuzzed(64)"], &["tool_error"]),
                FailOn::Evidence
            ),
            2
        );
    }

    /// §7.1 gives statuses their own visual channel: on the diagram they
    /// are corner markers beside the fill, not a weaker fill. The terminal
    /// is the surface most people actually read, and it had no channel for
    /// them at all -- a result standing on a promise nobody checked printed
    /// as a bare pass, identical to one standing on checked code. The
    /// qualifier and the debt were in the JSON and in the diagnostic prose,
    /// and absent from the one line a person scans.
    #[test]
    fn a_result_resting_on_an_unchecked_promise_says_so_on_the_node_line() {
        let envelope = envelope_with_statuses(&["bounded(2)"], &["conditional", "owed-evidence"]);
        let report = tree_report(&envelope);
        assert!(
            report.contains("  f — bounded(2)  [assumed, evidence owed]"),
            "the node line must carry both marks: {report}"
        );
        assert!(
            report.contains(
                "  [assumed]        this result rests on a promise Ply was handed and did not \
                 check — if the promise is wrong, the result is wrong with it"
            ),
            "a marker nobody can read is not a report: {report}"
        );
        assert!(
            report.contains(
                "  [evidence owed]  nothing has run the real code against that promise yet"
            ),
            "{report}"
        );
    }

    /// A run with nothing to qualify prints exactly what it printed before:
    /// no markers, and no explanation of markers that are not there.
    #[test]
    fn a_plain_result_prints_no_marker_and_no_legend() {
        let report = tree_report(&envelope(&["bounded(2)"]));
        assert_eq!(report, "workspace — bounded(2)\n  f — bounded(2)\n");
    }

    /// The statuses that are *not* absences must keep exiting 0, or the rule
    /// has turned into "any status fails", which would fail every legacy
    /// codebase §5.5 exists to serve on its very first `conditional` run.
    #[test]
    fn a_status_that_is_not_an_absence_still_exits_zero() {
        assert_eq!(
            exit_code_for(
                &envelope_with_statuses(
                    &["bounded(2)"],
                    &["conditional", "owed-evidence", "weak-spec", "stale"]
                ),
                FailOn::Evidence
            ),
            0,
            "`conditional` is the normal state of legacy-extension code (§5.5), and `weak-spec` \
             is a real finding beside real evidence -- neither is an absence"
        );
    }

    #[test]
    fn a_missing_engine_outranks_a_tool_error_which_outranks_everything_else() {
        assert_eq!(
            exit_code_for(
                &envelope(&["engine-missing", "tool_error"]),
                FailOn::Evidence
            ),
            3
        );
        assert_eq!(
            exit_code_for(&envelope(&["tool_error", "timeout"]), FailOn::Evidence),
            2
        );
        assert_eq!(
            exit_code_for(&envelope(&["unsupported"]), FailOn::Evidence),
            1
        );
    }
}
