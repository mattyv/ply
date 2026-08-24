//! The cargo-mutants engine adapter for the `mutate` check (§5.4c, D12),
//! using the mechanism verified end to end in
//! `tests/spike/mutants/MUTANTS-FINDINGS.md` -- **not** the fabricated
//! "custom test command" the spec used to claim: there is no such flag.
//! The real mechanism is package targeting:
//!
//! ```text
//! cargo mutants -p <mutated-crate> --test-package <harness-crate> \
//!     --re <fn> --gitignore false -- <test-name-filter>
//! ```
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
    /// A regex matched against `--list`'s mutant names (cargo-mutants'
    /// `--re`) -- Ply always anchors this to one function name.
    pub fn_regex: String,
    /// The cargo-test name filter appended after `--` -- narrows the
    /// harness package's test run to this fn's own generated tests, so a
    /// harness crate covering many functions never lets one fn's mutants
    /// hide behind another's passing tests.
    pub test_filter: String,
    pub timeout_secs: u32,
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

/// Runs cargo-mutants and classifies the result by reading its own
/// structured `mutants.out/*.txt` files (one mutant description per line) --
/// far more robust than scraping the human-readable summary line, and
/// exactly what the spike's own inspection (`--leak-dirs`) confirmed those
/// files contain.
pub fn run(cfg: &MutantsRunConfig) -> Result<MutantsRunOutcome> {
    let mutants_out = cfg.workspace_root.join("mutants.out");
    let _ = std::fs::remove_dir_all(&mutants_out); // stale run from a prior verify, if any

    let timeout_arg = cfg.timeout_secs.to_string();
    let output = Command::new("cargo")
        .current_dir(&cfg.workspace_root)
        .args([
            "mutants",
            "-p",
            &cfg.mutated_package,
            "--test-package",
            &cfg.harness_package,
            "--re",
            &cfg.fn_regex,
            "--copy-target",
            "true",
            "--no-times",
            "-t",
            &timeout_arg,
            "--",
            &cfg.test_filter,
        ])
        .output()
        .with_context(|| format!("spawning `cargo mutants` in {}", cfg.workspace_root.display()))?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    if combined.contains("cargo build failed in an unmutated tree") {
        return Ok(MutantsRunOutcome::ToolError {
            raw_output: combined,
            reason: "the unmutated baseline build failed -- this is a build problem in the copied \
                     tree (commonly a missing workspace member), not a spec-strength finding"
                .into(),
        });
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
        return Ok(MutantsRunOutcome::ToolError {
            raw_output: combined,
            reason: "cargo-mutants produced no mutants.out/*.txt result files -- could not \
                     determine caught/missed counts"
                .into(),
        });
    }

    Ok(MutantsRunOutcome::Completed(MutantsOutcome { caught, missed, unviable, timeout, raw_output: combined }))
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
