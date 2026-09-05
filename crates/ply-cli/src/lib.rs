//! The Ply command line, as a library.
//!
//! This crate was a binary and nothing else until 2026-09-04, which meant
//! Ply could not check a line of it: a claim resolves against a crate's
//! `src/lib.rs`, and a crate with only a `main.rs` is refused by name
//! ("there is no library for Ply to look in"). That exempted the 16,500
//! lines that produce every verdict from the tool that produces them --
//! the tool's own first rule (`skills/ply-checkable-code`, "separate
//! deciding from writing") applied one level up, with the shell being the
//! binary itself. `src/main.rs` is now the shell: it parses nothing and
//! decides nothing, it calls [`run`].

pub mod audit;
pub mod check;
pub mod explain;
pub mod shared;
pub mod verify;
pub mod worklist;

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
use ply_core::visual::svg::{RenderOptions, render_svg_with_state};
use ply_core::visual::{
    DEFAULT_RETAINED_RUNS, RunOutcome, VisualPublisher, build_declared_visual_envelope,
    build_visual_envelope_with_sources, completed_run_metadata, outcome_of,
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
    /// Explain a diagnostic code -- what it means, who reports it, and
    /// whether a run carrying it passed. With no code, lists every one this
    /// build can produce.
    Explain {
        /// A code as Ply prints it, like `K0502`. Case does not matter.
        code: Option<String>,
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

/// The whole command line: parse the arguments and run what they asked for.
pub fn run() -> anyhow::Result<()> {
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
            if cli.json {
                render_json_command(&path, output.as_deref(), &options, text, &mut stdout)?;
            } else {
                render_command(&path, output.as_deref(), &options, text, &mut stdout)?;
            }
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
        Commands::Explain { code } => {
            let mut stdout = std::io::stdout().lock();
            explain::explain_command(code.as_deref(), &mut stdout)?;
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
    render_command_with_format(requested_path, output, options, text, false, stdout)
}

fn render_json_command(
    requested_path: &Path,
    output: Option<&Path>,
    options: &RenderOptions,
    text: bool,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    render_command_with_format(requested_path, output, options, text, true, stdout)
}

fn render_command_with_format(
    requested_path: &Path,
    output: Option<&Path>,
    options: &RenderOptions,
    text: bool,
    json: bool,
    stdout: &mut impl Write,
) -> anyhow::Result<()> {
    if text && json {
        anyhow::bail!("--text and --json select different render outputs; use one or the other");
    }
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

    // Render draws documents `check` would refuse -- a picture is most useful
    // while the document is still wrong -- so this is deliberately not full
    // validation. The version is the one exception: every other invalid field
    // still renders faithfully, but this one selects the rules every other
    // line is read under, so drawing an unsupported version means applying
    // v1's inheritance and check semantics to a document that does not use
    // them, while looking exactly as authoritative (external review,
    // 2026-08-30).
    if document.ply != 1 {
        anyhow::bail!(
            "{} declares `ply: {}`, a version of the ply.yaml format this build of Ply does \
             not speak. Rendering it under version 1's rules could state every line below \
             wrong, so it is refused rather than guessed at. This build reads version 1; \
             upgrade Ply, or set `ply: 1`. `cargo ply check` reports this as E0201.",
            input.display(),
            document.ply
        );
    }
    // §7.1's rule for `state:`: the document names the type and the fields,
    // the *code* says what those fields are. So before drawing, read them --
    // from the crate each component's anchor names, which is what lets a
    // workspace document whose components live in six different crates draw
    // all six. Nothing resolvable (no code yet, or a document somewhere with
    // no crate under it) simply draws the type name alone. Resolved once,
    // ahead of every drawn form below (JSON envelope, transcript, plain SVG)
    // so none of them can disagree about what a component's state holds.
    let source_root = input.parent().unwrap_or(Path::new("."));
    let state_fields = ply_core::harness::resolve_state_fields(source_root, &document);

    if json {
        let visual = build_declared_visual_envelope(
            &document,
            completed_run_metadata(
                input.parent().unwrap_or_else(|| Path::new(".")),
                env!("CARGO_PKG_VERSION"),
                // Placeholder only: the builder replaces this with the outcome
                // it derives from the tree it constructs. Nothing has been
                // checked here, so what comes out says the evidence is missing.
                RunOutcome::MissingEvidence,
            ),
            options,
            Some(&state_fields),
        )?;
        let json = visual.to_json_pretty();
        return match output {
            Some(path) => std::fs::write(path, json)
                .map_err(|error| anyhow::anyhow!("could not write {}: {error}", path.display())),
            None => stdout.write_all(json.as_bytes()).map_err(|error| {
                anyhow::anyhow!("could not write the visual JSON to stdout: {error}")
            }),
        };
    }
    if text {
        // Same read the drawing does, for the same reason: the text form's
        // contract is that it states everything the drawing shows.
        let transcript = ply_core::visual::transcript::render_transcript_with_state(
            &document,
            Some(&state_fields),
        );
        return match output {
            Some(path) => std::fs::write(path, transcript)
                .map_err(|error| anyhow::anyhow!("could not write {}: {error}", path.display())),
            None => stdout.write_all(transcript.as_bytes()).map_err(|error| {
                anyhow::anyhow!("could not write the transcript to stdout: {error}")
            }),
        };
    }

    let svg = render_svg_with_state(&document, options, &state_fields)
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
        let plain = render_svg_with_state(&document, &RenderOptions::default(), &state_fields);
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
    // A random-sample check on a value built by parsing text (a version, an
    // identifier, a URL) cannot reach its evidence count by guessing text
    // uniformly -- almost none of it parses. Ply grows its inputs from a
    // pool of already-valid values instead (docs/reach-measurement-2.md),
    // and this mark is the disclosure that the count is real but the inputs
    // are not an unbiased sample of all possible text -- the diagnostic
    // beneath the tree names exactly where each one came from.
    if node.statuses.iter().any(|s| s == "seeded") {
        marks.push("seeded");
    }
    // A promise written as `a || b` is checked left to right, and every
    // case really did run -- nothing here is unbuildable, which is what
    // `partial-history`'s own mark ("narrower than it looks") means, so
    // this is deliberately a sibling mark rather than a reuse of it
    // (correction recorded 2026-09-01, TODO.md: reusing that one would make
    // its own printed legend false). This one is about what happens
    // *inside* the promise, after the call: one side of it decided almost
    // every case that held, so the other side is barely exercised even
    // though the count itself is real.
    if node.statuses.iter().any(|s| s == "promise-lopsided") {
        marks.push("lopsided");
    }
    // A value the document told Ply how to make, rather than one Ply's own
    // generator drew. Both facts already reached the JSON envelope as
    // statuses; until 2026-09-02 neither reached the terminal, so a run that
    // tested one value sixty-four times read as a clean `fuzzed(64)` to the
    // only reader who cannot query the envelope -- a person. Same rule as
    // the two marks above: the count is real, what it is a count *of* is
    // what needs saying.
    if node.statuses.iter().any(|s| s == "route-built") {
        marks.push("built to order");
    }
    if node.statuses.iter().any(|s| s == "route-collapsed") {
        marks.push("one value over and over");
    }
    // The same fact reached a different way, and the commoner way by far: a
    // constructor Ply found rather than one a document declared. A
    // no-argument constructor cannot vary, so this needs no run to notice --
    // and the mark is the same because what a reader has to know is the
    // same.
    if node.statuses.iter().any(|s| s == "one-value") {
        marks.push("one value over and over");
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
const MARK_GLOSS: [(&str, &str); 8] = [
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
    (
        "seeded",
        "this value is built by parsing text, and random text almost never parses, so Ply grew its \
         inputs from values already known to be valid instead of guessing blindly — the count below \
         is real, but it is evidence about text similar to what is already known to work, not about \
         arbitrary text",
    ),
    (
        "built to order",
        "this value is not one Ply drew itself — the document names a function that makes one, and \
         Ply called it, varying what it passed in. The count below is real, but it is evidence \
         about the values that function returns, not about everything this type could ever hold",
    ),
    (
        "one value over and over",
        "every case ran against the same value, so the count above is the number of \
         times one test ran rather than the number of different things tried. Either \
         the only way in this type has makes one value and nothing can change it \
         afterwards, or the function this document names for making one handed back \
         the same value every time — the lines below say which, and name it",
    ),
    (
        "lopsided",
        "this promise is written as \"either this, or that\" (`||`), and one side of it decided \
         almost every case where the promise held — real cases ran and the promise really held, but \
         the other side of it was barely exercised. The lines below say which side decided how \
         often, so you can judge whether the side you actually care about was tested at all",
    ),
];

/// `a`, `a and b`, `a, b and c` — a list a person reads, not a debug print.
pub fn join_plainly(items: &[String]) -> String {
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
    print!("{}", diagnostics_report(&envelope.diagnostics));
}

/// The terminal-readable half of `--json`'s `diagnostics` array. A
/// diagnostic's `title` alone used to be the whole report (2026-08-30, "a
/// counterexample is announced and then withheld"): a title can promise a
/// minimal failing input -- "proptest shrank a failing case to this minimal
/// example" -- and name a runnable test Ply just wrote into the user's own
/// `src/`, while the terminal, which is what people actually read, showed
/// neither. `--json` carried both the whole time under
/// `counterexample.inputs` and `counterexample.cargo_test`; this reuses that
/// same field rather than recomputing anything, so the terminal and
/// `--json` cannot disagree about what the failing input was.
fn diagnostics_report(diagnostics: &[ply_core::diag::Diagnostic]) -> String {
    let mut out = String::new();
    for d in diagnostics {
        out.push_str(&format!("[{}] {} — {}\n", d.code, d.node_id, d.title));
        if let Some(cex) = &d.counterexample {
            out.push_str(&counterexample_report(cex));
        }
    }
    out
}

/// The failing input itself, plainly -- whatever `inputs` holds, never
/// fabricated or reformatted into something tidier than what the engine
/// actually produced (the same rule `W0541` already keeps for a value that
/// could not be rendered as Rust source at all: shown as raw text, not
/// invented, and named as absent rather than guessed at -- `cargo_test` is
/// `None` in exactly that case, so no path is printed). Followed by the one
/// fact a person reading a terminal has no other way to learn: Ply just
/// wrote a file into their own `src/` directory, and where.
fn counterexample_report(cex: &ply_core::diag::Counterexample) -> String {
    let mut out = String::new();
    if !cex.inputs.is_empty() {
        let pairs: Vec<String> = cex
            .inputs
            .iter()
            // An empty value is a fact about the input, not a rendering
            // failure -- but printed bare they look identical, so it is
            // named. Everything else is passed through exactly as the
            // engine produced it (2026-09-04).
            .map(|(name, value)| {
                if value.is_empty() {
                    format!("{name} = (empty)")
                } else {
                    format!("{name} = {value}")
                }
            })
            .collect();
        out.push_str(&format!("    failing input: {}\n", pairs.join(", ")));
    }
    if let Some(path) = &cex.cargo_test {
        out.push_str(&format!(
            "    Ply wrote a test that reproduces this to {path} -- run `cargo test` from this \
             crate's root directory and it fails the same way this run just did.\n"
        ));
    }
    out
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

    /// Defect 3 (2026-08-30, "a test that reproduces this" is false):
    /// `cargo test` does not print the diagnostic's own title text back at
    /// you -- it prints the postcondition failure message the generated
    /// test panics with. "it fails with the same message above" claimed a
    /// verbatim match that never happens; "fails the same way" says the
    /// true, weaker thing. The reader also needs to know *where* to run
    /// the command, since the path Ply names is relative to the crate root,
    /// not the reader's current directory.
    /// An empty string as the failing input used to print as nothing at
    /// all after the `=`, which reads exactly like a value the renderer
    /// gave up on. Found 2026-09-04 by breaking `schema::dotted` on
    /// purpose: the shrunk input was `""`, and the report showed
    /// `pointer = ` with a blank line after it.
    #[test]
    fn an_empty_failing_value_says_so_rather_than_printing_nothing() {
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert("pointer".to_string(), String::new());
        let cex = ply_core::diag::Counterexample {
            inputs,
            kani_witness: None,
            cargo_test: None,
        };
        let report = counterexample_report(&cex);
        assert!(
            report.contains("pointer = (empty)"),
            "a reader cannot tell an empty value from a missing one unless the \
             report names it:\n{report}"
        );
    }

    #[test]
    fn counterexample_report_never_claims_cargo_test_prints_the_same_message() {
        let cex = ply_core::diag::Counterexample {
            inputs: std::collections::BTreeMap::new(),
            kani_witness: None,
            cargo_test: Some("src/ply_generated_cex.rs".to_string()),
        };
        let report = counterexample_report(&cex);
        assert_eq!(
            report,
            "    Ply wrote a test that reproduces this to src/ply_generated_cex.rs -- run \
             `cargo test` from this crate's root directory and it fails the same way this run \
             just did.\n"
        );
    }

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
        // Deliberately the *plain* renderer, not the state-aware one the
        // command now calls: with no `state:` to resolve and no crate under
        // the temp directory, the two must agree byte for byte. Every
        // committed drawing predating state rows depends on that being
        // exactly true.
        let canonical =
            ply_core::visual::svg::render_svg_with_options(&document, &RenderOptions::default())
                .unwrap();
        assert_eq!(String::from_utf8(stdout).unwrap(), canonical);
        assert!(canonical.starts_with("<svg"));
    }

    #[test]
    fn render_json_keeps_declaration_hierarchy_before_code_exists() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("finding-header.ply.yaml"),
            "ply: 1\ncomponents:\n  market_data:\n    anchor: app::market_data\n    components:\n      decoder:\n        anchor: app::decoder\n        fns:\n          decode:\n            requires: [input.len() > 0]\n            ensures: [result.len() > 0]\n",
        )
        .unwrap();

        let mut stdout = Vec::new();
        render_json_command(
            &root.path().join("finding-header.ply.yaml"),
            None,
            &RenderOptions::default(),
            false,
            &mut stdout,
        )
        .unwrap();

        let json = String::from_utf8(stdout).unwrap();
        let visual = ply_core::visual::VisualEnvelope::from_json(&json).unwrap();
        let function = visual
            .elements
            .values()
            .find(|element| element.kind == "fn")
            .unwrap();
        let component = visual
            .elements
            .get(function.parent_id.as_ref().unwrap())
            .unwrap();
        assert_eq!(component.label, "decoder");
        assert!(
            function
                .declaration
                .as_deref()
                .unwrap()
                .contains("Input (requires): input.len() > 0")
        );
        assert!(
            visual
                .svg
                .contains(&format!("data-element-id=\"{}\"", function.id))
        );
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
        assert_eq!(
            written,
            ply_core::visual::transcript::render_transcript(&document)
        );
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

    /// Render deliberately draws documents `check` would refuse — a picture
    /// is most useful while the document is still wrong. The `ply:` version
    /// is the one exception, because it is not a wrong field among right
    /// ones: it selects the rules every other line is read under, so drawing
    /// a version this build does not speak means applying the wrong
    /// semantics to all of it while looking exactly as authoritative.
    #[test]
    fn a_format_version_this_build_does_not_speak_is_refused_not_guessed_at() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ply.yaml"), "ply: 2\n").unwrap();

        for text in [true, false] {
            let mut stdout = Vec::new();
            let error = render_command(
                root.path(),
                None,
                &RenderOptions::default(),
                text,
                &mut stdout,
            )
            .expect_err("an unsupported version must be refused");
            assert!(
                error
                    .to_string()
                    .contains("a version of the ply.yaml format this build of Ply does not speak"),
                "got: {error}"
            );
            assert!(stdout.is_empty(), "a refused render must draw nothing");
        }
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

    /// The branch-decided measurement's own mark (CLAUDE.md, 2026-09-02),
    /// pinned the same way `a_result_resting_on_an_unchecked_promise_says_
    /// so_on_the_node_line` pins `assumed`/`evidence owed` above -- a
    /// person reading the tree, not just the JSON, must see this too.
    #[test]
    fn a_lopsided_or_promise_says_so_on_the_node_line() {
        let envelope = envelope_with_statuses(&["fuzzed(64)"], &["promise-lopsided"]);
        let report = tree_report(&envelope);
        assert!(
            report.contains("  f — fuzzed(64)  [lopsided]"),
            "the node line must carry the mark: {report}"
        );
        assert!(
            report.contains(
                "  [lopsided]       this promise is written as \"either this, or that\" (`||`), \
                 and one side of it decided almost every case where the promise held — real \
                 cases ran and the promise really held, but the other side of it was barely \
                 exercised. The lines below say which side decided how often, so you can judge \
                 whether the side you actually care about was tested at all"
            ),
            "a marker nobody can read is not a report: {report}"
        );
    }

    /// The same rule as the lopsided mark above, for a value built through a
    /// route the document names. Both facts already reach the JSON envelope
    /// as statuses; a person reading the terminal saw neither, so a run that
    /// tested one value sixty-four times read as a clean `fuzzed(64)`.
    #[test]
    fn a_route_built_value_and_a_collapsed_one_say_so_on_the_node_line() {
        let built = tree_report(&envelope_with_statuses(&["fuzzed(64)"], &["route-built"]));
        assert!(
            built.contains("  f — fuzzed(64)  [built to order]"),
            "a value made by a named route is not one Ply drew itself, and the line must say so: {built}"
        );

        let collapsed = tree_report(&envelope_with_statuses(
            &["fuzzed(64)"],
            &["route-built", "route-collapsed"],
        ));
        assert!(
            collapsed.contains("  f — fuzzed(64)  [built to order, one value over and over]"),
            "a route that returned the same value every time must say so where the count is read: {collapsed}"
        );
        assert!(
            collapsed.contains(
                "  [one value over and over]  every case ran against the same value, so \
                 the count above is the number of times one test ran rather than the \
                 number of different things tried. Either the only way in this type has \
                 makes one value and nothing can change it afterwards, or the function \
                 this document names for making one handed back the same value every \
                 time — the lines below say which, and name it"
            ),
            "a marker nobody can read is not a report: {collapsed}"
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

    fn cex_diagnostic(
        counterexample: Option<ply_core::diag::Counterexample>,
    ) -> ply_core::diag::Diagnostic {
        ply_core::diag::Diagnostic {
            code: "P0502".into(),
            severity: "error".into(),
            phase: "verify".into(),
            engine: "proptest".into(),
            check: "fuzz(64)".into(),
            node_id: "semver::Version::new".into(),
            title: "`Version::new` breaks its own postcondition \
                     `|result|result.pre.is_empty() && result.build.is_empty()` for at least one \
                     input -- proptest shrank a failing case to this minimal example. (P0502)"
                .into(),
            primary_span: None,
            pointer: None,
            counterexample,
            fixes: vec![],
            assumptions: vec![],
            open_item: None,
        }
    }

    /// Defect 1 (2026-08-30, "a counterexample is announced and then
    /// withheld"): the title promises a minimal failing example, and the
    /// terminal used to stop right there -- never printing the input, even
    /// though `--json` carried it all along. This is the terminal's own red
    /// test: the `[P0502] ... — <title>` line must be followed by the
    /// actual failing input, and by where the runnable test was written.
    #[test]
    fn a_diagnostic_with_a_counterexample_shows_the_failing_input_and_the_written_test_path() {
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert("major".to_string(), "7".to_string());
        inputs.insert("minor".to_string(), "7".to_string());
        inputs.insert("patch".to_string(), "0".to_string());
        let diag = cex_diagnostic(Some(ply_core::diag::Counterexample {
            inputs,
            kani_witness: Some(
                "captured from proptest shrinking, replayable with --seed abcd".into(),
            ),
            cargo_test: Some("src/ply_generated_cex.rs".into()),
        }));
        let report = diagnostics_report(&[diag]);
        assert!(
            report.contains("failing input: major = 7, minor = 7, patch = 0"),
            "the promised minimal example must actually appear: {report}"
        );
        assert!(
            report.contains("src/ply_generated_cex.rs"),
            "must name the path of the runnable test Ply just wrote into the user's own src/: \
             {report}"
        );
    }

    /// The W0541 case (docs, D7): when the inputs cannot be rendered as
    /// stable Rust source, there is no `cargo_test` path, and this must
    /// never invent one -- the report simply has no "wrote a test" line.
    #[test]
    fn a_diagnostic_whose_inputs_cannot_be_rendered_names_no_test_file() {
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert("params_raw".to_string(), "Foo { x: 4 }".to_string());
        let diag = cex_diagnostic(Some(ply_core::diag::Counterexample {
            inputs,
            kani_witness: None,
            cargo_test: None,
        }));
        let report = diagnostics_report(&[diag]);
        assert!(
            report.contains("failing input: params_raw = Foo { x: 4 }"),
            "the raw witness must still be shown, plainly, never fabricated: {report}"
        );
        assert!(
            !report.contains("wrote a test"),
            "no test file exists in this case, so none may be named: {report}"
        );
    }

    /// A diagnostic with no counterexample at all (a tool error) prints
    /// only its title line, exactly as before.
    #[test]
    fn a_diagnostic_with_no_counterexample_prints_only_its_title_line() {
        let diag = cex_diagnostic(None);
        let report = diagnostics_report(&[diag]);
        assert_eq!(
            report,
            "[P0502] semver::Version::new — `Version::new` breaks its own postcondition \
             `|result|result.pre.is_empty() && result.build.is_empty()` for at least one input \
             -- proptest shrank a failing case to this minimal example. (P0502)\n"
        );
    }
}
