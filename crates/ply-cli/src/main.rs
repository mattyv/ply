mod verify;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
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

/// A name that reports no evidence (§1): the engine was exhausted, the shape
/// was out of reach, the tool broke, nothing was claimed, no engine existed,
/// or a check ran and settled nothing. None of them is a claim about the
/// code; all of them used to exit 0.
///
/// **An absence is a name, not a slot** (adversarial review of the post-004
/// fixes, D2). The same names appear in two places in a §8 node -- as its
/// `verdict`, and as a `status` beside it (D6) -- and they mean the same
/// thing in both. The first version of this rule enumerated verdict strings
/// only, which was complete over the verdicts the tool can emit and blind to
/// every absence encoded as a status: a `mutate` check whose engine was
/// missing reported `inconclusive` beside an untouched `fuzzed(64)` verdict
/// and exited 0, against §1's own principle and §6's exit-3 row. Adding one
/// more verdict string would have left the next status-shaped absence open,
/// so the rule reads both fields against one vocabulary instead.
fn is_absence(name: &str) -> bool {
    name == "timeout"
        || name == "unclaimed"
        || name == "engine-missing"
        || name == "inconclusive"
        || name.starts_with("unsupported")
        || name.starts_with("tool_error")
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
                evidence: None,
                children,
            },
            diagnostics: vec![],
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
