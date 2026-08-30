mod audit;
mod check;
mod shared;
mod verify;
mod worklist;

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
// The absence vocabulary (§1: "an absence is a name, not a slot") lives in
// ply-core, because a second consumer now reads it -- the rule that decides
// whether a result may be recorded and reused at all (§5.2a records only
// results that earned evidence). Two copies of one vocabulary is how the
// next absence gets missed by one of them, which is the exact shape of the
// defect that put this rule here.
use ply_core::diag::is_absence;
use ply_core::model::parse_document;
use ply_core::visual::svg::{RenderOptions, render_svg_with_options};
use ply_core::visual::transcript::render_transcript;
use ply_core::visual::{
    DEFAULT_RETAINED_RUNS, VisualPublisher, build_visual_envelope_with_sources,
    completed_run_metadata, outcome_of,
};
use verify::VerifyOptions;

// The `about` string below is what `--help` prints, so it is written for
// someone who has never seen Ply rather than for this file's reader. It used
// to say the CLI "implements only `verify`", which stopped being true
// several milestones ago and read as a flat contradiction beside the four
// commands listed under it (found by a smoke test on a real project,
// 2026-08-28).
//
// `--version` reports two numbers because they answer different questions.
// The package version says which release this is. The build identity is the
// content hash of the source it was built from, and it is what decides
// whether a stored result may be carried forward -- so when a run says a
// claim was re-checked because "the build of Ply that checked it changed",
// this is the number that changed.
#[derive(Parser)]
#[command(
    name = "cargo-ply",
    bin_name = "cargo-ply",
    about = "cargo-ply -- declare what your code promises, and get evidence for it",
    version = concat!(
        env!("CARGO_PKG_VERSION"),
        " (build identity ",
        env!("PLY_BUILD_ID"),
        ")"
    )
)]
struct Cli {
    /// Emit the §8 JSON envelope instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Draw ply.yaml as SVG. This needs only the declaration file, not code or Cargo.
    Render {
        /// A ply.yaml file, or a directory containing one. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write the SVG here instead of printing it.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Fold components at this nesting level or deeper (top-level boxes are level 1).
        #[arg(long, value_parser = parse_render_depth)]
        depth: Option<usize>,
        /// Fully expand this component; dotted paths select nested components.
        #[arg(long)]
        focus: Option<String>,
        /// Fold this component; dotted paths select nested components. Repeat as needed.
        #[arg(long)]
        collapse: Vec<String>,
        /// Write the text form instead of the drawing: the same facts, every
        /// one of them, including the ones the drawing only shows on hover.
        /// For reading in a terminal, piping into another tool, or handing to
        /// a model -- none of which can hover.
        #[arg(long)]
        text: bool,
    },
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
        /// Publish this completed run for visual clients. This is explicit:
        /// editors never start verification themselves.
        #[arg(long)]
        publish_view: bool,
        /// Number of completed visual runs to retain for this Ply root.
        #[arg(long, default_value_t = DEFAULT_RETAINED_RUNS)]
        retain_views: usize,
    },
    /// Remove older published visual runs without deleting the current run.
    CleanViews {
        /// Path to the crate directory containing `ply.yaml`.
        path: PathBuf,
        /// Number of completed visual runs to keep.
        #[arg(long, default_value_t = DEFAULT_RETAINED_RUNS)]
        keep: usize,
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
        Commands::Render {
            path,
            output,
            depth,
            focus,
            collapse,
            text,
        } => {
            let options = RenderOptions {
                depth,
                focus,
                collapse,
            };
            let mut stdout = std::io::stdout().lock();
            render_command(&path, output.as_deref(), &options, text, &mut stdout)?;
        }
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
            publish_view,
            retain_views,
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
            let verification = verify::verify_crate_result(&path, &opts)?;
            let envelope = verification.envelope;
            if publish_view {
                let run = completed_run_metadata(&path, verify::PLY_VERSION, outcome_of(&envelope));
                let visual = build_visual_envelope_with_sources(
                    &verification.document,
                    &envelope,
                    run,
                    &verification.source_map,
                )?;
                let publication = VisualPublisher::new(&path).publish(&visual, retain_views)?;
                if let Some(warning) = publication.warning {
                    eprintln!("warning: {warning}");
                }
            }
            if cli.json {
                println!("{}", envelope.to_json_pretty());
            } else {
                print_human(&envelope);
            }
            std::process::exit(exit_code_for(&envelope, fail_on));
        }
        Commands::CleanViews { path, keep } => {
            let cleanup = VisualPublisher::new(path).cleanup(keep)?;
            println!(
                "Removed {} older visual run{} from the index; the current run was kept.",
                cleanup.removed,
                if cleanup.removed == 1 { "" } else { "s" }
            );
            if let Some(warning) = cleanup.warning {
                eprintln!("warning: {warning}");
            }
        }
    }

    Ok(())
}

fn parse_render_depth(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(0) => Err(
            "--depth 0 does not select anything: nesting levels start at 1 for top-level boxes"
                .to_string(),
        ),
        Ok(depth) => Ok(depth),
        Err(_) => Err(format!(
            "--depth needs a whole number of nesting levels; {value:?} is not one"
        )),
    }
}

fn render_command(
    requested_path: &Path,
    output: Option<&Path>,
    options: &RenderOptions,
    text: bool,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    // The folding flags narrow a *drawing* to fit a screen. The text form has
    // no screen to fit and always states the whole document, so a run asking
    // for both is asking for something that does not exist. Refusing beats
    // ignoring: a reader handed a quietly-unfolded transcript would believe
    // they had been given the narrowed view they asked for.
    if text {
        let named = if options.depth.is_some() {
            Some("--depth")
        } else if options.focus.is_some() {
            Some("--focus")
        } else if !options.collapse.is_empty() {
            Some("--collapse")
        } else {
            None
        };
        if let Some(named) = named {
            anyhow::bail!(
                "--text writes out the whole document, so it cannot be combined with --depth, \
                 --focus or --collapse. Those fold parts of the drawing away to fit a screen; \
                 the text form has no screen to fit. Drop {named} to get the text, or drop \
                 --text to get a folded drawing."
            );
        }
    }

    let input = if requested_path.is_dir() {
        requested_path.join("ply.yaml")
    } else {
        requested_path.to_path_buf()
    };
    // Refuse before reading anything. The document is the only artifact here
    // that cannot be regenerated -- the drawing and the text form are both
    // outputs -- and `-o` pointed at it used to overwrite it with prose, exit
    // 0, and say nothing (external review, 2026-08-30).
    //
    // Compared after canonicalizing, so a symlink, a `./` prefix, or the
    // directory form of the same path is still recognised as the same file.
    // A path that cannot be canonicalized (the output does not exist yet, the
    // usual case) falls back to comparing what we have.
    if let Some(out) = output {
        let same = match (input.canonicalize(), out.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => input == out,
        };
        if same {
            anyhow::bail!(
                "{} would overwrite the document it is rendering, and that document is the \
                 only thing here that cannot be regenerated -- the drawing and the text form \
                 are both outputs of it. Write to a different path, or drop -o to send the \
                 result to stdout.",
                out.display()
            );
        }
    }

    let yaml = std::fs::read_to_string(&input)
        .map_err(|error| anyhow::anyhow!("could not read {}: {error}", input.display()))?;
    let document = parse_document(&yaml).map_err(|error| {
        anyhow::anyhow!("{} did not parse as ply.yaml: {error}", input.display())
    })?;
    if text {
        let transcript = render_transcript(&document);
        return match output {
            Some(path) => std::fs::write(path, transcript)
                .map_err(|error| anyhow::anyhow!("could not write {}: {error}", path.display())),
            None => stdout.write_all(transcript.as_bytes()).map_err(|error| {
                anyhow::anyhow!("could not write the transcript to stdout: {error}")
            }),
        };
    }

    let svg = render_svg_with_options(&document, options)
        .map_err(|error| anyhow::anyhow!("could not render {}: {error}", input.display()))?;

    // A selection that selects nothing is worth saying out loud. On a flat
    // document `--depth 1` and `--focus x` produce exactly the default
    // drawing, and silence there reads as "the flag did nothing visible, so
    // something is broken" -- a smoke test on a real project recorded it as
    // a bug before deciding it was correct behaviour. The check is the
    // honest one: render the default too, and compare. It costs one extra
    // layout pass and cannot disagree with what was actually drawn. The
    // note goes to stderr, so it can never contaminate an SVG on stdout.
    if options.depth.is_some() || options.focus.is_some() || !options.collapse.is_empty() {
        let plain = render_svg_with_options(&document, &RenderOptions::default());
        if plain.as_deref().ok() == Some(svg.as_str()) {
            eprintln!(
                "note: this drawing is identical to the one with no --depth/--focus/--collapse \
                 at all. Nothing in {} nests deeply enough for that selection to fold anything \
                 away, so the flag had nothing to do -- not an error, and not a sign the flag \
                 was ignored.",
                input.display()
            );
        }
    }

    if let Some(path) = output {
        std::fs::write(path, svg)
            .map_err(|error| anyhow::anyhow!("could not write {}: {error}", path.display()))?;
    } else {
        stdout
            .write_all(svg.as_bytes())
            .map_err(|error| anyhow::anyhow!("could not write the SVG to stdout: {error}"))?;
    }
    Ok(())
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
    // A run that left one of a type's own operations or constructors out
    // of the histories it explores -- an unbuildable argument, a trait
    // method, a spelling it could not confirm names the same type, or a
    // constructor it never started from -- can still read as a clean pass
    // on a promise it never had the chance to break. The verdict carries
    // that fact, and until now it carried it only into `--json` -- the
    // tree, which is the line most people actually read, showed a bare
    // `fuzzed(n)` (found by hand, 2026-08-28). Making the narrowing visible
    // everywhere except where it is read is the same failure this status
    // exists to end. The gloss below was written for the first of these
    // causes only (the unbuildable argument) and quietly stopped being
    // true the moment the other three were added to the same status
    // (coordinator review, 2026-08-28) -- it now names all four in the one
    // sentence a reader who has never seen this status gets.
    if node.statuses.iter().any(|s| s == "partial-history") {
        marks.push("narrower than it looks");
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
const MARK_GLOSS: [(&str, &str); 4] = [
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
        "narrower than it looks",
        "real cases ran and this result is real, but this run could not call every one of this \
         type's own operations or build every one of its constructors — one had an argument it \
         could not build, belonged to a trait or a piece of code it could not confirm is this \
         same type, or was simply never the one this run started from — so it says nothing about \
         what the ones it skipped would have done. The lines below name which, and why",
    ),
    (
        "reused",
        "this result was not re-run: an earlier run recorded it, and every input Ply hashes still \
         hashes the same — the function's own source, the code it calls, the promises it assumes, \
         the examples it checks, the checks themselves, the engines, the compiler and target, the \
         crate's features, the resolved versions of its dependencies, and Ply's own version",
    ),
];

/// `a`, `a and b`, `a, b and c` — a list a person reads, not a debug print.
fn join_plainly(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

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
                // Pad to the same column for the short marks (unchanged
                // output), but never let a longer one run straight into its
                // own gloss.
                let pad = if label.len() >= 17 {
                    "  ".to_string()
                } else {
                    " ".repeat(17 - label.len())
                };
                out.push_str(&format!("  {label}{pad}{gloss}\n"));
            }
        }
        // The diagnostics come next, and they are paragraphs. Without this
        // the gloss and the first diagnostic run together into one block.
        out.push('\n');
    }
    // A claim that *had* a recorded result and could not use it. Saying
    // which input moved is the difference between "it re-proved everything
    // and I do not know why" and one line naming the compiler that updated
    // under you (§5.2a).
    if !envelope.not_carried_forward.is_empty() {
        out.push_str(
            "  Checked again rather than carried forward from an earlier run, because what \
             each one depended on has changed:\n",
        );
        for item in &envelope.not_carried_forward {
            out.push_str(&format!(
                "    {} — {} changed since that result was recorded\n",
                item.node_id,
                join_plainly(&item.because)
            ));
        }
        // Under the coarse mode "the code it runs" above means the whole
        // crate, so those lines can fire for an edit in a function the claim
        // never calls. Left unexplained that reads as Ply re-running for no
        // reason. The reason is a property of the crate rather than of any
        // one claim, so it is said once however many claims it displaced --
        // the same paragraph twenty times is noise, not explanation.
        let mut said: Vec<&str> = Vec::new();
        for item in &envelope.not_carried_forward {
            let Some(why) = item.widened_because.as_deref() else {
                continue;
            };
            if said.contains(&why) {
                continue;
            }
            said.push(why);
            let (claims, plural) = claims_sharing(&envelope.not_carried_forward, why);
            let (calls, they, them) = if plural {
                ("call", "they", "them")
            } else {
                ("calls", "it", "it")
            };
            out.push_str(&format!(
                "\n  For {claims}, \"the code it runs\" means every line of the crate, not \
                 only the functions {they} {calls}, because {why}. So any edit in that crate \
                 re-runs {them}, even an edit in code {they} never {calls}.\n"
            ));
        }
        out.push('\n');
    }
    out
}

/// The claims one widening reason displaced, named rather than counted:
/// "for `billing::total`" beats "for 1 claim", and a person scanning the
/// list above wants to match them up.
fn claims_sharing(items: &[ply_core::diag::NotCarriedForward], why: &str) -> (String, bool) {
    let names: Vec<String> = items
        .iter()
        .filter(|i| i.widened_because.as_deref() == Some(why))
        .map(|i| format!("`{}`", i.node_id))
        .collect();
    (join_plainly(&names), names.len() > 1)
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

    #[test]
    fn render_is_available_before_there_is_a_cargo_project() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ply.yaml"), "ply: 1\n").unwrap();

        let parsed = Cli::try_parse_from(["cargo-ply", "render", root.path().to_str().unwrap()]);
        if let Err(error) = parsed {
            panic!("a directory containing only ply.yaml must be renderable: {error}");
        }
    }

    #[test]
    fn render_defaults_to_the_current_directory_and_keeps_all_renderer_controls() {
        let cli = Cli::try_parse_from([
            "cargo-ply",
            "render",
            "--depth",
            "2",
            "--focus",
            "outer.inner",
            "--collapse",
            "left",
            "--collapse",
            "right",
            "-o",
            "drawing.svg",
        ])
        .unwrap();

        match cli.command {
            Commands::Render {
                path,
                output,
                depth,
                focus,
                collapse,
                text,
            } => {
                assert_eq!(path, PathBuf::from("."));
                assert_eq!(output, Some(PathBuf::from("drawing.svg")));
                assert_eq!(depth, Some(2));
                assert_eq!(focus.as_deref(), Some("outer.inner"));
                assert_eq!(collapse, ["left", "right"]);
                assert!(
                    !text,
                    "the drawing is still the default; --text opts out of it"
                );
            }
            _ => panic!("render should parse as render"),
        }
    }

    #[test]
    fn a_directory_with_only_ply_yaml_renders_the_canonical_svg_to_stdout() {
        let root = tempfile::tempdir().unwrap();
        let yaml = "ply: 1\n";
        std::fs::write(root.path().join("ply.yaml"), yaml).unwrap();
        assert!(!root.path().join("Cargo.toml").exists());

        let mut stdout = Vec::new();
        render_command(
            root.path(),
            None,
            &RenderOptions::default(),
            false,
            &mut stdout,
        )
        .unwrap();

        let document = parse_document(yaml).unwrap();
        let canonical = render_svg_with_options(&document, &RenderOptions::default()).unwrap();
        assert_eq!(String::from_utf8(stdout).unwrap(), canonical);
        assert!(canonical.starts_with("<svg"));
    }

    /// The text form has to be on the installed command, not only on the
    /// development binary in `tools/`. Shipping a reader aid from one entry
    /// point and not the other is the split this project called out once
    /// already, over the fold-nothing-away notice.
    #[test]
    fn render_text_writes_the_transcript_where_the_drawing_would_have_gone() {
        let root = tempfile::tempdir().unwrap();
        let yaml = "ply: 1\ncomponents:\n  pricing:\n    anchor: app::pricing\n";
        std::fs::write(root.path().join("ply.yaml"), yaml).unwrap();

        let mut stdout = Vec::new();
        render_command(
            root.path(),
            None,
            &RenderOptions::default(),
            true,
            &mut stdout,
        )
        .unwrap();
        let written = String::from_utf8(stdout).unwrap();

        assert!(
            !written.contains("<svg"),
            "--text asked for the text form and got a drawing: {written:?}"
        );
        let document = parse_document(yaml).unwrap();
        assert_eq!(written, render_transcript(&document));
    }

    #[test]
    fn render_text_honours_the_output_path() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("diagram.ply.yaml");
        let output = root.path().join("diagram.txt");
        std::fs::write(&input, "ply: 1\n").unwrap();
        let mut stdout = Vec::new();

        render_command(
            &input,
            Some(&output),
            &RenderOptions::default(),
            true,
            &mut stdout,
        )
        .unwrap();

        assert!(stdout.is_empty());
        assert!(
            std::fs::read_to_string(output)
                .unwrap()
                .starts_with("This is a Ply transcript:")
        );
    }

    /// The folding flags narrow a drawing to fit a screen; the text form has
    /// no screen to fit and always states the whole document. Silently
    /// ignoring the flag would leave the reader believing they had been
    /// handed a narrowed view, so the run is refused and says why.
    #[test]
    fn render_text_refuses_the_folding_flags_rather_than_ignoring_them() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ply.yaml"), "ply: 1\n").unwrap();
        let options = RenderOptions {
            depth: Some(1),
            ..RenderOptions::default()
        };
        let mut stdout = Vec::new();

        let error =
            render_command(root.path(), None, &options, true, &mut stdout).expect_err("refused");

        assert_eq!(
            error.to_string(),
            "--text writes out the whole document, so it cannot be combined with --depth, \
             --focus or --collapse. Those fold parts of the drawing away to fit a screen; the \
             text form has no screen to fit. Drop --depth to get the text, or drop --text to \
             get a folded drawing."
        );
        assert!(stdout.is_empty(), "a refused run must write nothing");
    }

    /// `cargo ply render ply.yaml --text -o ply.yaml` used to read the
    /// document, render it, and write the prose straight over the source --
    /// exit 0, no warning, specification gone. One plausible typo (`-o` where
    /// you meant nothing at all) destroys the file the whole tool exists to
    /// serve, and it is unrecoverable outside version control.
    #[test]
    fn rendering_over_the_document_being_rendered_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("ply.yaml");
        let yaml = "ply: 1\ncomponents:\n  a:\n    anchor: app::a\n";
        std::fs::write(&input, yaml).unwrap();

        for text in [true, false] {
            let mut stdout = Vec::new();
            let error = render_command(
                &input,
                Some(&input),
                &RenderOptions::default(),
                text,
                &mut stdout,
            )
            .expect_err("writing over the input must be refused");

            assert!(
                error
                    .to_string()
                    .contains("would overwrite the document it is rendering"),
                "got: {error}"
            );
            assert_eq!(
                std::fs::read_to_string(&input).unwrap(),
                yaml,
                "the source must be untouched after a refused render"
            );
        }
    }

    /// The same file reached through a directory argument, a symlink, or a
    /// non-normalised path is still the same file.
    #[test]
    fn the_overwrite_check_sees_through_a_directory_argument() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ply.yaml"), "ply: 1\n").unwrap();
        let mut stdout = Vec::new();

        let error = render_command(
            root.path(),
            Some(&root.path().join("ply.yaml")),
            &RenderOptions::default(),
            true,
            &mut stdout,
        )
        .expect_err("a directory input resolves to ply.yaml inside it");
        assert!(
            error.to_string().contains("would overwrite the document"),
            "got: {error}"
        );
    }

    #[test]
    fn render_writes_the_requested_file_instead_of_stdout() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("diagram.ply.yaml");
        let output = root.path().join("diagram.svg");
        std::fs::write(&input, "ply: 1\n").unwrap();
        let mut stdout = Vec::new();

        render_command(
            &input,
            Some(&output),
            &RenderOptions::default(),
            false,
            &mut stdout,
        )
        .unwrap();

        assert!(stdout.is_empty());
        assert!(std::fs::read_to_string(output).unwrap().starts_with("<svg"));
    }

    #[test]
    fn render_errors_name_the_actual_input_and_problem() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("ply.yaml");
        let mut stdout = Vec::new();
        let error = render_command(
            root.path(),
            None,
            &RenderOptions::default(),
            false,
            &mut stdout,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(&missing.display().to_string()), "{error}");
        assert!(error.contains("could not read"), "{error}");
        assert!(
            error.contains("No such file") || error.contains("not found"),
            "{error}"
        );

        std::fs::write(&missing, "this is: [not valid yaml").unwrap();
        let error = render_command(
            root.path(),
            None,
            &RenderOptions::default(),
            false,
            &mut stdout,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(&missing.display().to_string()), "{error}");
        assert!(error.contains("did not parse as ply.yaml"), "{error}");
    }

    #[test]
    fn publishing_a_view_is_explicit_and_retains_twenty_by_default() {
        let cli = Cli::try_parse_from(["cargo-ply", "verify", ".", "--publish-view"]).unwrap();
        match cli.command {
            Commands::Verify {
                publish_view,
                retain_views,
                ..
            } => {
                assert!(publish_view);
                assert_eq!(retain_views, DEFAULT_RETAINED_RUNS);
            }
            _ => panic!("verify should parse as verify"),
        }

        let cli = Cli::try_parse_from(["cargo-ply", "verify", "."]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Verify {
                publish_view: false,
                ..
            }
        ));
    }

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
                ..Default::default()
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
                ..Default::default()
            },
            diagnostics: vec![],
            coverage: None,
            trust_surface: None,
            open_items: None,
            not_carried_forward: vec![],
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
                    &["conditional", "owed-evidence", "weak-spec"]
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
