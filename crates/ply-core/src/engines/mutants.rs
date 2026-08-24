//! The cargo-mutants engine adapter for the `mutate` check (§5.4c, D12),
//! using the mechanism verified end to end in
//! `tests/spike/mutants/MUTANTS-FINDINGS.md` -- **not** the fabricated
//! "custom test command" the spec used to claim: there is no such flag.
//! The real mechanism is package targeting:
//!
//! ```text
//! timeout <wall-clock>s \
//! cargo mutants -p <mutated-crate> --test-package <harness-crate> \
//!     --re <fn> --copy-target true --no-times -t <secs> -- <test-name-filter>
//! ```
//!
//! (That is the command as it is actually spawned -- `mutants_argv` builds
//! it. `--gitignore false`, which earlier drafts of this doc showed here as
//! "the real mechanism", is falsified below and must never be passed.)
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

use anyhow::{Context, Result};

pub struct MutantsRunConfig {
    /// The workspace root cargo-mutants should run from -- the target
    /// crate's root, which now has the harness crate registered as a
    /// workspace member (`harness_crate::ensure_workspace_member`).
    pub workspace_root: std::path::PathBuf,
    pub mutated_package: String,
    pub harness_package: String,
    /// A regex matched against cargo-mutants' own *descriptive* mutant
    /// names (`src/lib.rs:8:5: replace vacuous -> u32 with 0`), not against
    /// the bare function name -- so Ply passes the fn name **unanchored**.
    /// `^fn$` matched zero mutants in a real run (docs/m4-findings.md
    /// finding 4). Known limitation recorded there: an unanchored name can
    /// over-match a fn whose name is a substring of another's.
    pub fn_regex: String,
    /// The cargo-test name filter appended after `--` -- narrows the
    /// harness package's test run to this fn's own generated tests, so a
    /// harness crate covering many functions never lets one fn's mutants
    /// hide behind another's passing tests.
    pub test_filter: String,
    /// cargo-mutants' own `-t`: the cap on *each mutant's* test phase.
    pub timeout_secs: u32,
    /// The cap on the whole invocation, enforced by Ply with the `timeout`
    /// command exactly as `engines::fuzz` and `engines::kani::run_playback`
    /// do. `-t` alone leaves the tree copy and the unmutated baseline build
    /// uncapped, so a hang there hung `verify` with no report at all --
    /// §5.4c forbids exactly that ("never a silent hang").
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
pub fn mutants_argv(cfg: &MutantsRunConfig) -> Vec<String> {
    vec![
        // The whole invocation is capped, not just each mutant's test phase
        // (2026-08-24 M4 review, D5): `-t` below is cargo-mutants' own
        // per-mutant budget and leaves the tree copy and the unmutated
        // baseline build uncapped. Same `timeout` wrapper the fuzz and Kani
        // adapters use, so exit code 124 means "killed by the cap".
        "timeout".to_string(),
        format!("{}s", cfg.wall_clock_secs),
        "cargo".to_string(),
        "mutants".to_string(),
        "-p".to_string(),
        cfg.mutated_package.clone(),
        "--test-package".to_string(),
        cfg.harness_package.clone(),
        "--re".to_string(),
        cfg.fn_regex.clone(),
        "--copy-target".to_string(),
        "true".to_string(),
        "--no-times".to_string(),
        "-t".to_string(),
        cfg.timeout_secs.to_string(),
        "--".to_string(),
        cfg.test_filter.clone(),
    ]
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
    let output = Command::new(&argv[0])
        .current_dir(&cfg.workspace_root)
        .args(&argv[1..])
        .output()
        .with_context(|| format!("spawning `cargo mutants` in {}", cfg.workspace_root.display()))?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(classify_run(output.status.code(), combined, &mutants_out))
}

/// Turns one finished invocation into an engine-honest outcome: a run the
/// wall-clock cap killed is a `Timeout`, a run whose own output or result
/// files cannot be read is a `ToolError`, and only a run that produced real
/// per-mutant result files is `Completed`. Pure, so the classification is
/// testable without a real cargo-mutants run.
pub fn classify_run(exit_code: Option<i32>, combined: String, mutants_out: &Path) -> MutantsRunOutcome {
    // GNU `timeout` exits 124 when it had to kill the child.
    if exit_code == Some(124) {
        return MutantsRunOutcome::Timeout { raw_output: combined };
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
    let missed = read_lines("missed.txt");
    let caught = read_lines("caught.txt").len() as u32;
    let unviable = read_lines("unviable.txt").len() as u32;
    let timeout = read_lines("timeout.txt").len() as u32;

    if caught == 0 && missed.is_empty() && unviable == 0 && timeout == 0 {
        return MutantsRunOutcome::ToolError {
            raw_output: combined,
            reason: "cargo-mutants produced no mutants.out/*.txt result files -- could not \
                     determine caught/missed counts"
                .into(),
        };
    }

    MutantsRunOutcome::Completed(MutantsOutcome { caught, missed, unviable, timeout, raw_output: combined })
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
            fn_regex: "add_small".into(),
            test_filter: "add_small_harness::".into(),
            timeout_secs: 60,
            wall_clock_secs: 600,
        }
    }

    /// §5.4c MUST: "every engine invocation carries a hard cap ... Exceeding
    /// it yields `timeout`, never a silent hang." `-t` caps each *mutant's*
    /// test phase inside cargo-mutants; it does not cap the invocation, so a
    /// hung tree copy or baseline build hung `verify` with no cap at all
    /// (2026-08-24 M4 review, D5). The sibling adapters (`engines::fuzz`,
    /// `engines::kani::run_playback`) already wrap their spawn in `timeout`.
    #[test]
    fn the_whole_invocation_carries_a_wall_clock_cap_not_just_a_per_mutant_one() {
        let argv = mutants_argv(&cfg());
        assert_eq!(argv[0], "timeout", "the run itself must be capped, not only each mutant: {argv:?}");
        assert_eq!(argv[1], "600s", "the cap is the config's whole-run budget: {argv:?}");
        assert_eq!(argv[2], "cargo");
        assert!(argv.contains(&"-t".to_string()) && argv.contains(&"60".to_string()), "{argv:?}");
    }

    /// GNU `timeout` exits 124 when it had to kill the child. Before the fix
    /// this was indistinguishable from any other failed run and fell through
    /// to `ToolError` -- `MutantsRunOutcome::Timeout` (and with it `M0601`)
    /// was declared, matched on, and never constructed by anything.
    #[test]
    fn a_killed_run_is_a_timeout_not_a_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = classify_run(Some(124), String::new(), dir.path());
        assert!(
            matches!(outcome, MutantsRunOutcome::Timeout { .. }),
            "a run the wall-clock cap killed must be reported as `timeout`, never conflated with a \
             tool error or a completed run"
        );
    }

    #[test]
    fn all_caught_is_false_with_zero_mutants() {
        let outcome = MutantsOutcome::default();
        assert!(!outcome.all_caught(), "zero mutants run is not evidence of a strong spec");
    }

    #[test]
    fn all_caught_true_only_when_nothing_survived_and_something_ran() {
        let outcome = MutantsOutcome { caught: 5, missed: vec![], unviable: 1, timeout: 0, raw_output: String::new() };
        assert!(outcome.all_caught());
        let with_survivor = MutantsOutcome { missed: vec!["x".into()], ..outcome };
        assert!(!with_survivor.all_caught());
    }
}
