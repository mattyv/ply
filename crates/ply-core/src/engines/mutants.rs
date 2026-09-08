//! The cargo-mutants engine adapter for the `mutate` check (§5.4c, D12),
//! using the mechanism verified end to end in
//! `tests/spike/mutants/MUTANTS-FINDINGS.md` -- **not** the fabricated
//! "custom test command" the spec used to claim: there is no such flag.
//! The real mechanism is package targeting:
//!
//! ```text
//! cargo mutants -p <mutated-crate> --test-package <harness-crate> \
//!     --re <owned-fn-pattern> --exclude **/ply_generated*.rs \
//!     --copy-target true --no-times -t <secs> -- <test-name-filter>
//! ```
//!
//! (That is the command as it is actually spawned -- `mutants_argv` builds
//! it. `--gitignore false`, which earlier drafts of this doc showed here as
//! "the real mechanism", is falsified below and must never be passed. The
//! whole-invocation wall-clock cap mentioned below it is enforced separately,
//! in-process, by `engines::run_with_timeout` -- it is not part of this
//! argv at all.)
//!
//! §5.4c (pre-M4) said only `--gitignore false` was needed to make the
//! harness crate's `target/ply/fuzz/` placement copy-safe. **That claim is
//! falsified by this session's real runs, on two counts** (recorded in
//! docs/m4-findings.md):
//!
//! 1. `--gitignore`'s own observed *default* is already "off" (confirmed
//!    both in the earlier spike and in cargo-mutants 27.1.0's own test
//!    suite, `options.rs::gitignore_off_by_default`) -- passing
//!    `--gitignore false` explicitly is harmless but adds nothing.
//! 2. There is a **second, separate, previously-undiscovered skip** that
//!    `--gitignore` cannot reach at all. Reading cargo-mutants'
//!    `copy_tree.rs::copy_tree`'s own `filter_entry` closure:
//!    ```text
//!    let is_top_level_target = name == "target"
//!        && entry.path().parent().is_some_and(|p| p == from_path);
//!    ... && (copy_target || !is_top_level_target) ...
//!    ```
//!    A directory literally named `target` sitting *directly at the copy
//!    root* is pruned before the walk even descends into it -- unconditional
//!    on `.gitignore` entirely. Ply's harness crate lives at
//!    `<crate_dir>/target/ply/fuzz/<name>`, exactly one level inside the
//!    target crate's own top-level `target/`, so every `mutate` run hit
//!    this: `cargo build failed in an unmutated tree` / `No such file or
//!    directory` for the harness crate's own `Cargo.toml`, even with
//!    `--gitignore false` passed. The earlier mutants spike's own
//!    `harness-genloc` fixture (`tests/spike/mutants/scoped/lib/target/ply/fuzz/`)
//!    never actually exercised this path: its harness sat under
//!    `lib/target/...`, and `lib` is a *subdirectory* of that spike's own
//!    copy root (`scoped/`), so the `target` there is never the top-level
//!    one this special case matches -- an accident of that spike's fixture
//!    depth, not evidence this placement is copy-safe in the shape Ply
//!    actually generates it (one level under the *target crate's own*
//!    root, not nested inside another workspace member first).
//!
//! The real fix is `--copy-target true` -- and it cannot be combined with
//! `--gitignore` at all (clap rejects it: both flags share a mutually
//! exclusive `copy_opts` argument group in cargo-mutants' own CLI, verified
//! directly: `error: the argument '--gitignore <GITIGNORE>' cannot be used
//! with '--copy-target <COPY_TARGET>'`). Since `--gitignore`'s default
//! already matches what Ply wants (off), the adapter passes only
//! `--copy-target true`, never `--gitignore` at all. The honest cost: this
//! copies the target crate's **entire** `target/` directory into every
//! scratch tree cargo-mutants builds (its own build cache, not just the
//! harness crate) -- measured at ~13s for a 189MB `target/` plus 2 trivial
//! mutants in this session's `weakspec` fixture, and a real cost that scales
//! with the crate's build-cache size, not a free fix. See
//! docs/m4-findings.md for the follow-up this leaves for M5: moving the
//! harness crate outside `target/` entirely would remove the need for
//! `--copy-target true`, but requires its own git-ignore entry, since it
//! would no longer inherit `target/`'s.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

pub struct MutantsRunConfig {
    /// The Cargo workspace root cargo-mutants should run from. This may be
    /// a virtual root above the target crate; the generated harness is
    /// temporarily registered there as a workspace member.
    pub workspace_root: std::path::PathBuf,
    pub mutated_package: String,
    pub harness_package: String,
    /// Function names converted by [`mutation_selector`] into regexes for
    /// cargo-mutants' descriptive mutant names (`src/lib.rs:8:5: replace
    /// vacuous -> u32 with 0`). A bare unanchored name can match a literal
    /// inside another function, so the adapter owns the exact conversion.
    /// Every function the check runs, claimed one first -- not just the
    /// claimed one. `spec-strong` says deliberate bugs were planted and the
    /// checks caught them all; planting only in the claimed function reports
    /// that over a thin wrapper whose helpers were never touched, which is
    /// the shape Ply's own writing guide teaches authors to write.
    pub fn_regexes: Vec<String>,
    /// The same cargo-mutants owner names paired with their source files.
    /// Out-of-line modules lose their module prefix in cargo-mutants' owner
    /// text, so the file is what distinguishes `a::helper` from
    /// `b::helper` after both become bare `helper`.
    pub fn_owner_files: Vec<(String, String)>,
    /// The cargo-test name filter appended after `--` -- narrows the
    /// harness package's test run to this fn's own generated tests, so a
    /// harness crate covering many functions never lets one fn's mutants
    /// hide behind another's passing tests.
    pub test_filter: String,
    /// cargo-mutants' own `-t`: the cap on *each mutant's* test phase.
    pub timeout_secs: u32,
    /// The cap on the whole invocation, enforced in-process by
    /// `engines::run_with_timeout` -- the same helper `engines::fuzz` and
    /// `engines::kani::run_playback` use, never an external `timeout`
    /// command. `-t` alone leaves the tree copy and the unmutated baseline
    /// build uncapped, so a hang there hung `verify` with no report at all
    /// -- §5.4c forbids exactly that ("never a silent hang").
    pub wall_clock_secs: u32,
}

#[derive(Debug, Clone, Default)]
pub struct MutantsOutcome {
    pub caught: u32,
    /// Plain-English descriptions of surviving mutants (one per line of
    /// cargo-mutants' own `mutants.out/missed.txt`) -- carried into the
    /// `W0502` diagnostic so a reader sees *what* survived, not just a
    /// count.
    pub missed: Vec<String>,
    pub unviable: u32,
    pub timeout: u32,
    pub raw_output: String,
}

impl MutantsOutcome {
    pub fn total(&self) -> u32 {
        self.caught + self.missed.len() as u32 + self.unviable + self.timeout
    }

    /// `mutate` succeeds -- earns `·spec-strong` -- only when every mutant
    /// that could run was caught. Unviable mutants (didn't compile) carry
    /// no information either way and are excluded, matching cargo-mutants'
    /// own convention of never counting them as findings.
    pub fn all_caught(&self) -> bool {
        self.missed.is_empty() && (self.caught > 0)
    }
}

/// The engine-honest outcome of one `cargo mutants` invocation -- mirrors
/// `engines::kani::KaniOutcome`'s discipline of a structurally distinct
/// `Timeout`/`ToolError` so an adapter cannot conflate an inconclusive run
/// with a completed one.
pub enum MutantsRunOutcome {
    Completed(MutantsOutcome),
    Timeout { raw_output: String },
    ToolError { raw_output: String, reason: String },
}

/// The exact argv one `mutate` run is spawned with, program name first.
/// Split out from `run` so the invocation itself is testable without a real
/// cargo-mutants run.
///
/// Never includes a `timeout` wrapper: the whole-invocation wall-clock cap
/// (2026-08-24 M4 review, D5 -- `-t` below is only cargo-mutants' own
/// per-mutant budget and leaves the tree copy and the unmutated baseline
/// build uncapped) is enforced separately and in-process, by
/// [`wall_clock_budget`] passed to `engines::run_with_timeout`.
pub fn mutants_argv(cfg: &MutantsRunConfig) -> Vec<String> {
    let mut out = vec![
        "cargo".to_string(),
        "mutants".to_string(),
        "-p".to_string(),
        cfg.mutated_package.clone(),
        "--test-package".to_string(),
        cfg.harness_package.clone(),
        "--copy-target".to_string(),
        "true".to_string(),
        "--no-times".to_string(),
        "--exclude".to_string(),
        "**/ply_generated*.rs".to_string(),
        "-t".to_string(),
        cfg.timeout_secs.to_string(),
        "--".to_string(),
        cfg.test_filter.clone(),
    ];
    // One `--re` per body the check runs; cargo-mutants ORs them. Inserted
    // before the `--` so they reach cargo-mutants rather than the test
    // binary.
    let at = out.len() - 2;
    for name in cfg.fn_regexes.iter().rev() {
        out.insert(at, mutation_selector(name));
        out.insert(at, "--re".to_string());
    }
    out
}

/// Matches the three forms where cargo-mutants names the function that owns
/// a mutant: after `replace` with its return type, after `replace` in the
/// implicit-unit whole-body form, or at the end after `in`. Text elsewhere
/// in the description (a string literal, type, or called function) is not
/// ownership and must not widen the planting scope.
fn mutation_selector(name: &str) -> String {
    format!(r"(?:replace {name} ->|replace {name} with \(\)$| in {name}$)")
}

/// The same ownership rule as [`mutation_selector`], applied to one result
/// line without shipping a regex engine solely to defend an adapter
/// boundary. Rust paths contain neither spaces nor arrows, so these three
/// literal forms are unambiguous.
fn mutation_description_is_owned_by(description: &str, name: &str) -> bool {
    description.contains(&format!("replace {name} ->"))
        || description.ends_with(&format!("replace {name} with ()"))
        || description.ends_with(&format!(" in {name}"))
}

fn mutation_description_is_in_file(description: &str, file: &str) -> bool {
    let Some((reported, _)) = description.split_once(':') else {
        return false;
    };
    let reported = reported.replace('\\', "/");
    let file = file.replace('\\', "/");
    reported == file || reported.ends_with(&format!("/{file}"))
}

/// The whole-invocation wall-clock budget `run` enforces via
/// `engines::run_with_timeout` -- distinct from `-t` in [`mutants_argv`],
/// which is cargo-mutants' own per-mutant budget. Split out, like
/// `mutants_argv`, so the cap is testable without a real cargo-mutants run.
pub fn wall_clock_budget(cfg: &MutantsRunConfig) -> Duration {
    Duration::from_secs(cfg.wall_clock_secs as u64)
}

/// Runs cargo-mutants and classifies the result by reading its own
/// structured `mutants.out/*.txt` files (one mutant description per line) --
/// far more robust than scraping the human-readable summary line, and
/// exactly what the spike's own inspection (`--leak-dirs`) confirmed those
/// files contain.
pub fn run(cfg: &MutantsRunConfig) -> Result<MutantsRunOutcome> {
    let mutants_out = cfg.workspace_root.join("mutants.out");
    let _ = std::fs::remove_dir_all(&mutants_out); // stale run from a prior verify, if any

    let argv = mutants_argv(cfg);
    let mut cmd = Command::new(&argv[0]);
    cmd.current_dir(&cfg.workspace_root).args(&argv[1..]);
    // The whole-invocation wall-clock cap is enforced in-process here, never
    // by shelling out to a `timeout` binary -- macOS ships neither `timeout`
    // nor `gtimeout`, so wrapping the real command in one made `mutate`
    // fail to spawn at all rather than ever run a single mutant.
    let output = super::run_with_timeout(&mut cmd, wall_clock_budget(cfg)).with_context(|| {
        format!(
            "running `cargo mutants` in {}",
            cfg.workspace_root.display()
        )
    })?;

    let combined = super::strip_ansi(&format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ));

    Ok(classify_run_for_owners(
        output.timed_out,
        combined,
        &mutants_out,
        &cfg.fn_regexes,
        &cfg.fn_owner_files,
    ))
}

/// Turns one finished invocation into an engine-honest outcome: a run the
/// wall-clock cap killed is a `Timeout`, a run whose own output or result
/// files cannot be read is a `ToolError`, and only a run that produced real
/// per-mutant result files is `Completed`. Pure, so the classification is
/// testable without a real cargo-mutants run.
///
/// `timed_out` comes straight from `engines::TimedOutput` -- it is never
/// inferred from an exit code (GNU `timeout`'s old 124 convention has no
/// counterpart here, because nothing spawns that program any more).
pub fn classify_run(timed_out: bool, combined: String, mutants_out: &Path) -> MutantsRunOutcome {
    classify_run_for_owners(timed_out, combined, mutants_out, &[], &[])
}

fn classify_run_for_owners(
    timed_out: bool,
    combined: String,
    mutants_out: &Path,
    fn_regexes: &[String],
    fn_owner_files: &[(String, String)],
) -> MutantsRunOutcome {
    if timed_out {
        return MutantsRunOutcome::Timeout {
            raw_output: combined,
        };
    }

    if combined.contains("cargo build failed in an unmutated tree") {
        return MutantsRunOutcome::ToolError {
            raw_output: combined,
            reason: "the unmutated baseline build failed -- this is a build problem in the copied \
                     tree (commonly a missing workspace member), not a spec-strength finding"
                .into(),
        };
    }

    let read_lines = |name: &str| -> Vec<String> {
        std::fs::read_to_string(mutants_out.join(name))
            .unwrap_or_default()
            .lines()
            .map(|l| l.to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };
    let raw_missed = read_lines("missed.txt");
    let raw_caught = read_lines("caught.txt");
    let raw_unviable = read_lines("unviable.txt");
    let raw_timeout = read_lines("timeout.txt");

    if raw_caught.is_empty()
        && raw_missed.is_empty()
        && raw_unviable.is_empty()
        && raw_timeout.is_empty()
    {
        return MutantsRunOutcome::ToolError {
            raw_output: combined,
            reason: "cargo-mutants produced no mutants.out/*.txt result files -- could not \
                     determine caught/missed counts"
                .into(),
        };
    }

    // cargo-mutants 27.1.0 applies `--re` to ordinary mutations but its
    // struct-literal field deletion path pushes candidates without calling
    // that filter. Treat the engine's result files as untrusted input and
    // enforce the same owner selectors here for every outcome category.
    // Filtering only `missed.txt` would still let an unrelated caught
    // mutation earn `spec-strong`.
    let in_scope = |line: &String| {
        if fn_regexes.is_empty() {
            return true;
        }
        if fn_owner_files.is_empty() {
            return fn_regexes
                .iter()
                .any(|name| mutation_description_is_owned_by(line, name));
        }
        fn_owner_files.iter().any(|(name, file)| {
            mutation_description_is_owned_by(line, name)
                && (file.is_empty() || mutation_description_is_in_file(line, file))
        })
    };
    let missed: Vec<String> = raw_missed.into_iter().filter(&in_scope).collect();
    let caught = raw_caught.into_iter().filter(&in_scope).count() as u32;
    let unviable = raw_unviable.into_iter().filter(&in_scope).count() as u32;
    let timeout = raw_timeout.into_iter().filter(&in_scope).count() as u32;

    MutantsRunOutcome::Completed(MutantsOutcome {
        caught,
        missed,
        unviable,
        timeout,
        raw_output: combined,
    })
}

/// What this machine's cargo-mutants calls itself, for the record a result
/// is stored under (§5.2a). `None` when it is not installed -- the `mutate`
/// check then reports a missing engine, which is an absence and is never
/// recorded.
/// Probed in the crate being verified. Rustup shims `cargo` per toolchain,
/// so a probe run from the caller's directory can report a different engine
/// build than the one that will actually run -- the same defect found in the
/// compiler probe (external review, 2026-08-30). The engine version is a
/// fingerprint input, so getting it from the wrong place lets stale evidence
/// survive a real change.
pub fn version(crate_dir: &std::path::Path) -> Option<String> {
    let out = Command::new("cargo")
        .args(["mutants", "--version"])
        .current_dir(crate_dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = super::strip_ansi(String::from_utf8_lossy(&out.stdout).trim());
    if text.is_empty() { None } else { Some(text) }
}

/// Lets a caller pre-flight-check whether `cargo mutants` is on `PATH` at
/// all, so a missing engine can be reported as `engine-missing`/`W0110`
/// (D9) rather than a confusing subprocess-spawn error.
pub fn is_available() -> bool {
    Command::new("cargo")
        .args(["mutants", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[allow(dead_code)]
fn unused_path_hint(_p: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MutantsRunConfig {
        MutantsRunConfig {
            workspace_root: std::path::PathBuf::from("/tmp/x"),
            mutated_package: "target-pkg".into(),
            harness_package: "harness-pkg".into(),
            fn_regexes: vec!["add_small".into()],
            fn_owner_files: vec![],
            test_filter: "add_small_harness::".into(),
            timeout_secs: 60,
            wall_clock_secs: 600,
        }
    }

    /// `spec-strong` says deliberate bugs were planted and the checks caught
    /// them all. Planting only in the claimed function reports that over a
    /// thin wrapper whose helpers were never touched -- and a thin wrapper
    /// over helpers is the shape Ply's own writing guide teaches. Every body
    /// the check runs has to reach cargo-mutants, and the flags have to land
    /// before the `--` or they go to the test binary instead.
    #[test]
    fn every_body_the_check_runs_is_offered_to_the_planter() {
        let mut c = cfg();
        c.fn_regexes = vec!["scaled".into(), "doubled_then_capped".into()];
        let argv = mutants_argv(&c);
        let dashdash = argv
            .iter()
            .position(|a| a == "--")
            .expect("a `--` separator");
        for name in ["scaled", "doubled_then_capped"] {
            let selector = mutation_selector(name);
            let at = argv
                .iter()
                .position(|a| a == &selector)
                .unwrap_or_else(|| panic!("`{name}` never reached cargo-mutants: {argv:?}"));
            assert_eq!(
                argv[at - 1],
                "--re",
                "`{name}` must arrive as a `--re` value: {argv:?}"
            );
            assert!(
                at < dashdash,
                "`{name}` landed after the `--`, so cargo-mutants never saw it -- it went to \
                 the test binary as a filter: {argv:?}"
            );
        }
    }

    /// Generated proof and replay modules are Ply's checking machinery,
    /// not application code whose specification is being measured. If
    /// cargo-mutants plants changes there, an intentionally empty proof
    /// harness survives and is misreported as weakness in the user's spec.
    #[test]
    fn generated_ply_modules_are_never_mutation_targets() {
        let argv = mutants_argv(&cfg());
        let dashdash = argv.iter().position(|a| a == "--").unwrap();
        let exclude = argv
            .iter()
            .position(|a| a == "**/ply_generated*.rs")
            .expect("the generated-module exclusion must reach cargo-mutants");
        assert_eq!(argv[exclude - 1], "--exclude");
        assert!(
            exclude < dashdash,
            "the exclusion is a cargo-mutants option: {argv:?}"
        );
    }

    /// A bare function name is ambiguous inside cargo-mutants' descriptive
    /// strings: `component_balance` identifies its own body, but can also
    /// occur as a string literal in a mutant belonging to `required_inputs`.
    /// Select the two positions where cargo-mutants names the containing
    /// function instead of accepting any substring hit.
    #[test]
    fn a_function_selector_cannot_match_its_name_inside_another_body() {
        let mut c = cfg();
        c.fn_regexes = vec!["component_balance".into()];
        let argv = mutants_argv(&c);
        let pattern = argv
            .iter()
            .skip_while(|a| *a != "--re")
            .nth(1)
            .expect("a selector after --re");
        let re = regex::Regex::new(pattern).unwrap();

        assert!(re.is_match(
            "src/lib.rs:75:5: replace component_balance -> Balance with Default::default()"
        ));
        assert!(re.is_match("src/lib.rs:76:21: replace + with - in component_balance"));
        assert!(!re.is_match(
            "src/lib.rs:106:9: delete match arm \"component_balance\" in required_inputs"
        ));
    }

    /// Inline modules and impl methods are qualified in cargo-mutants'
    /// owner text. The selector must retain that qualification in both
    /// description forms and must not accept the same leaf in another
    /// namespace.
    #[test]
    fn a_namespaced_function_selector_matches_only_that_owner() {
        for name in ["maths::helper", "Widget::adjust"] {
            let re = regex::Regex::new(&mutation_selector(name)).unwrap();
            assert!(re.is_match(&format!("src/lib.rs:20:5: replace {name} -> u32 with 0")));
            assert!(re.is_match(&format!("src/lib.rs:21:9: replace + with - in {name}")));
            let leaf = name.rsplit("::").next().unwrap();
            assert!(!re.is_match(&format!(
                "src/lib.rs:30:5: replace other::{leaf} -> u32 with 0"
            )));
        }
    }

    /// An implicit `()` return has no `-> TYPE` in Rust source, and
    /// cargo-mutants consequently describes its whole-body replacement as
    /// `replace reset with ()` rather than the usual `replace reset -> ...`.
    /// That is still a mutation owned by `reset`, not a third-party mention
    /// of the name elsewhere in the description.
    #[test]
    fn an_implicit_unit_body_replacement_matches_its_owner_selector() {
        let re = regex::Regex::new(&mutation_selector("reset")).unwrap();

        assert!(re.is_match("src/lib.rs:5:5: replace reset with ()"));
        assert!(!re.is_match("src/lib.rs:6:5: replace reset_all with ()"));
        assert!(!re.is_match("src/lib.rs:7:9: replace call with reset() in another"));
    }

    /// cargo-mutants 27.1.0 applies `--re` to ordinary mutations but not to
    /// struct-literal field deletions. Ply must apply the same owner filter
    /// to the result files before an unrelated deletion can be blamed on a
    /// function—or, worse, counted as a caught mutant that earns strength.
    #[test]
    fn result_files_are_filtered_against_the_requested_function_owners() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("caught.txt"),
            "src/lib.rs:5:5: replace wanted -> u32 with 0\n\
             src/fixtures.rs:10:9: delete field cursor from struct Pool expression in unrelated\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("missed.txt"),
            "src/lib.rs:6:9: replace + with - in wanted\n\
             src/fixtures.rs:11:9: delete field cursor from struct Pool expression in unrelated\n",
        )
        .unwrap();

        let outcome = classify_run_for_owners(
            false,
            String::new(),
            dir.path(),
            &["wanted".to_string()],
            &[],
        );
        let MutantsRunOutcome::Completed(outcome) = outcome else {
            panic!("result files with selected mutants must be a completed run");
        };
        assert_eq!(
            outcome.caught, 1,
            "unrelated caught mutants cannot earn strength"
        );
        assert_eq!(
            outcome.missed,
            vec!["src/lib.rs:6:9: replace + with - in wanted"],
            "only survivors owned by the requested function may be reported"
        );
    }

    /// The post-run filter is a second ownership boundary because some
    /// cargo-mutants candidates bypass `--re`. It must admit the same
    /// implicit-unit whole-body form as the command selector or a genuine
    /// survivor disappears after the engine reports it.
    #[test]
    fn an_implicit_unit_body_survivor_passes_the_result_owner_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("missed.txt"),
            "src/lib.rs:5:5: replace reset with ()\n\
             src/lib.rs:6:5: replace reset_all with ()\n",
        )
        .unwrap();

        let outcome = classify_run_for_owners(
            false,
            String::new(),
            dir.path(),
            &["reset".to_string()],
            &[],
        );
        let MutantsRunOutcome::Completed(outcome) = outcome else {
            panic!("selected result file must produce a completed run");
        };
        assert_eq!(
            outcome.missed,
            vec!["src/lib.rs:5:5: replace reset with ()"],
            "the requested unit-return body survivor must remain, without admitting a prefix owner"
        );
    }

    #[test]
    fn same_named_owners_are_filtered_by_source_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("caught.txt"),
            "src/a.rs:5:5: replace helper -> u32 with 0\n\
             src/b.rs:5:5: replace helper -> u32 with 0\n",
        )
        .unwrap();

        let outcome = classify_run_for_owners(
            false,
            String::new(),
            dir.path(),
            &["helper".to_string()],
            &[("helper".to_string(), "src/a.rs".to_string())],
        );
        let MutantsRunOutcome::Completed(outcome) = outcome else {
            panic!("selected result file must produce a completed run");
        };
        assert_eq!(outcome.caught, 1, "src/b.rs belongs to another function");
    }

    /// §5.4c MUST: "every engine invocation carries a hard cap ... Exceeding
    /// it yields `timeout`, never a silent hang." `-t` caps each *mutant's*
    /// test phase inside cargo-mutants; it does not cap the invocation, so a
    /// hung tree copy or baseline build hung `verify` with no cap at all
    /// (2026-08-24 M4 review, D5). The sibling adapters (`engines::fuzz`,
    /// `engines::kani::run_playback`) enforce the same whole-invocation cap
    /// the same way: in-process, via `engines::run_with_timeout`, never by
    /// wrapping the spawn in an external `timeout` binary (macOS ships
    /// neither `timeout` nor `gtimeout`, so that used to fail every run
    /// outright rather than ever cap one).
    #[test]
    fn the_whole_invocation_carries_a_wall_clock_cap_not_just_a_per_mutant_one() {
        let argv = mutants_argv(&cfg());
        assert_eq!(
            argv[0], "cargo",
            "the real program is spawned directly, never wrapped: {argv:?}"
        );
        assert!(
            !argv.iter().any(|a| a == "timeout"),
            "the invocation must never shell out through an external `timeout` wrapper: {argv:?}"
        );
        assert!(
            argv.contains(&"-t".to_string()) && argv.contains(&"60".to_string()),
            "{argv:?}"
        );
        assert_eq!(
            wall_clock_budget(&cfg()),
            Duration::from_secs(600),
            "the whole run is still capped by the config's wall-clock budget, just enforced \
             in-process rather than baked into argv"
        );
    }

    /// Before the fix, whether a run had been killed for exceeding its
    /// budget was inferred from GNU `timeout`'s exit code 124 -- indistinguishable
    /// from any other failed run whenever that inference broke, and it fell
    /// through to `ToolError`. `MutantsRunOutcome::Timeout` (and with it
    /// `M0601`) was declared, matched on, and never constructed by anything.
    /// Now the flag comes straight from `engines::TimedOutput`, with no exit
    /// code involved at all.
    #[test]
    fn a_killed_run_is_a_timeout_not_a_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = classify_run(true, String::new(), dir.path());
        assert!(
            matches!(outcome, MutantsRunOutcome::Timeout { .. }),
            "a run the wall-clock cap killed must be reported as `timeout`, never conflated with a \
             tool error or a completed run"
        );
    }

    #[test]
    fn all_caught_is_false_with_zero_mutants() {
        let outcome = MutantsOutcome::default();
        assert!(
            !outcome.all_caught(),
            "zero mutants run is not evidence of a strong spec"
        );
    }

    #[test]
    fn all_caught_true_only_when_nothing_survived_and_something_ran() {
        let outcome = MutantsOutcome {
            caught: 5,
            missed: vec![],
            unviable: 1,
            timeout: 0,
            raw_output: String::new(),
        };
        assert!(outcome.all_caught());
        let with_survivor = MutantsOutcome {
            missed: vec!["x".into()],
            ..outcome
        };
        assert!(!with_survivor.all_caught());
    }
}
