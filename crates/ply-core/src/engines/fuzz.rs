//! The proptest engine adapter for the M4 `fuzz`/`test` checks: runs
//! `cargo test` against the generated harness crate (`harness_crate.rs`)
//! and classifies the result the same way `engines::kani` classifies Kani's
//! output -- never conflating a timeout with a failure, and only ever
//! constructing a decoded witness from real captured output.
//!
//! `run_harness_tests` is shared by both `fuzz` and `test`: both checks'
//! generated tests live in the same per-fn harness module, and cargo test
//! reports failing test names directly, so one run classifies both.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::engines::kani::WitnessValue;
use crate::harness::{Param, RustType};

#[derive(Debug, Clone)]
pub struct HarnessTestRun {
    pub timed_out: bool,
    pub success: bool,
    pub combined_output: String,
    /// Fully-qualified failing test names (`<fn>_harness::ply_fuzz_<fn>`,
    /// etc.), extracted from cargo test's own `---- <name> stdout ----`
    /// failure-detail headers -- present regardless of `--nocapture`.
    pub failed_tests: Vec<String>,
}

/// Runs `cargo test -p <harness_package> --lib <filter> -- --nocapture` from
/// `workspace_root` (the target crate's root, which `harness_crate` has
/// registered the harness crate into as a workspace member). `--nocapture`
/// is load-bearing: the `PLY_FUZZ_HIGH_REJECT` marker is printed by a test
/// that otherwise *passes*, and libtest suppresses a passing test's output
/// without it.
pub fn run_harness_tests(
    workspace_root: &Path,
    harness_package: &str,
    filter: &str,
    timeout_secs: u32,
) -> Result<HarnessTestRun> {
    let timeout_arg = format!("{timeout_secs}s");
    let output = Command::new("timeout")
        .arg(&timeout_arg)
        .arg("cargo")
        .arg("test")
        .arg("-p")
        .arg(harness_package)
        .arg("--lib")
        .arg(filter)
        .arg("--")
        .arg("--nocapture")
        .current_dir(workspace_root)
        .output()
        .with_context(|| format!("spawning `cargo test -p {harness_package}` in {}", workspace_root.display()))?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // GNU `timeout` exits 124 when it had to kill the child.
    let timed_out = output.status.code() == Some(124);
    let failed_tests = parse_failed_test_names(&combined);
    Ok(HarnessTestRun {
        timed_out,
        success: output.status.success() && !timed_out,
        combined_output: combined,
        failed_tests,
    })
}

/// Extracts failing test names from libtest's own final summary block:
///
/// ```text
/// failures:
///     some_mod::some_test
///
/// test result: FAILED. ...
/// ```
///
/// This is libtest's *only* reliable failure listing under `--nocapture`
/// (which `run_harness_tests` always passes, so the `PLY_FUZZ_HIGH_REJECT`
/// marker is visible even on a passing test): libtest's other,
/// per-test `---- <name> stdout ----` detail dump only appears when output
/// was actually captured, i.e. *without* `--nocapture` -- relying on it
/// silently reported every fuzz-found violation as a pass the first time
/// this was run for real (recorded in docs/m4-findings.md). There can be
/// two `failures:` blocks (the detail dump's own header, when present, and
/// this summary) -- the summary is always the *last* one.
fn parse_failed_test_names(combined: &str) -> Vec<String> {
    let lines: Vec<&str> = combined.lines().collect();
    let Some(start) = lines.iter().rposition(|l| l.trim() == "failures:") else {
        return Vec::new();
    };
    lines[start + 1..]
        .iter()
        .take_while(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect()
}

/// Parses the last `PLY_FUZZED_CEX|<fn>|k1=v1;k2=v2` marker line out of
/// captured output, returning the fn name and its fields. Fields with a
/// `[...]` value (a `Vec`/`BTreeSet`) keep the brackets for
/// `decode_marker_fields` to parse.
pub fn parse_fuzz_marker(combined: &str) -> Option<(String, BTreeMap<String, String>)> {
    let line = combined.lines().rev().find(|l| l.contains("PLY_FUZZED_CEX|"))?;
    let after = line.split_once("PLY_FUZZED_CEX|")?.1;
    let (fn_name, rest) = after.split_once('|')?;
    let mut fields = BTreeMap::new();
    for entry in split_top_level_semicolons(rest) {
        if let Some((k, v)) = entry.split_once('=') {
            fields.insert(k.to_string(), v.to_string());
        }
    }
    Some((fn_name.to_string(), fields))
}

/// Splits on `;` but never inside a `[...]` bracket (a collection field's
/// joined elements never contain `;`, but this keeps the parser correct if
/// that ever changes).
fn split_top_level_semicolons(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ';' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Parses the `PLY_FUZZ_HIGH_REJECT|<fn>|<detail>` marker (§5.4c: "a
/// warning when the rejection rate is high"), if present.
pub fn parse_high_reject_marker(combined: &str) -> Option<(String, String)> {
    let line = combined.lines().find(|l| l.contains("PLY_FUZZ_HIGH_REJECT|"))?;
    let after = line.split_once("PLY_FUZZ_HIGH_REJECT|")?.1;
    let (fn_name, detail) = after.split_once('|')?;
    Some((fn_name.trim().to_string(), detail.trim().to_string()))
}

fn parse_u8_list(raw: &str) -> Option<Vec<u8>> {
    let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() {
        return Some(vec![]);
    }
    inner.split(',').map(|s| s.trim().parse::<u8>().ok()).collect()
}

/// Decodes a fuzz marker's fields into the *same* `WitnessValue` type Kani
/// witnesses decode into (the D7 plan's "two consumers, one renderer"), in
/// `params` order. Returns `None` -- never a fabricated value -- for any
/// parameter whose type has no `WitnessValue` representation: a
/// `Vec`/`BTreeSet` of anything but `u8` cannot be rendered as a Rust
/// literal this renderer knows how to write, so that case is reported as a
/// witness-only violation (`W0541`) by the caller, not force-rendered.
pub fn decode_marker_fields(fields: &BTreeMap<String, String>, params: &[Param]) -> Option<Vec<WitnessValue>> {
    let mut out = Vec::with_capacity(params.len());
    for p in params {
        let raw = fields.get(&p.name)?;
        let value = match &p.ty {
            RustType::Bool => WitnessValue::Bool(raw.parse::<bool>().ok()?),
            RustType::U8 | RustType::U16 | RustType::U32 | RustType::U64 => {
                WitnessValue::UInt(raw.parse::<u128>().ok()?)
            }
            RustType::I8 | RustType::I16 | RustType::I32 | RustType::I64 => {
                WitnessValue::Int(raw.parse::<i128>().ok()?)
            }
            RustType::VecU8 => WitnessValue::VecU8(parse_u8_list(raw)?),
            RustType::Vec(inner) if inner.as_ref() == &RustType::U8 => WitnessValue::VecU8(parse_u8_list(raw)?),
            RustType::Vec(_) | RustType::BTreeSet(_) | RustType::Unsupported(_) => return None,
        };
        out.push(value);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_failing_test_names_from_libtest_headers() {
        let combined = "\nrunning 2 tests\ntest clamp_harness::ply_fuzz_clamp ... FAILED\n\nfailures:\n\n---- clamp_harness::ply_fuzz_clamp stdout ----\nthread panicked\n\nfailures:\n    clamp_harness::ply_fuzz_clamp\n\ntest result: FAILED. 0 passed; 1 failed;\n";
        let names = parse_failed_test_names(combined);
        assert_eq!(names, vec!["clamp_harness::ply_fuzz_clamp".to_string()]);
    }

    /// Regression test for a real bug found running this against an actual
    /// fixture (docs/m4-findings.md): `run_harness_tests` always passes
    /// `--nocapture` (load-bearing for `PLY_FUZZ_HIGH_REJECT` on a passing
    /// test), and under `--nocapture` libtest never emits the per-test
    /// `---- name stdout ----` detail header this parser originally looked
    /// for -- only the final `failures:\n    name\n` summary block, with no
    /// preceding detail section at all. The original implementation
    /// silently reported zero failing tests here, meaning a real fuzz-found
    /// violation was reported as a clean pass.
    #[test]
    fn parses_failing_test_names_under_nocapture_with_no_detail_headers() {
        let combined = "\nrunning 1 test\nPLY_FUZZED_CEX|seeded_bug|x=7\n\nthread panicked at src/lib.rs:1:1:\nproptest found a failing case\ntest seeded_bug_harness::ply_fuzz_seeded_bug ... FAILED\n\nfailures:\n\nfailures:\n    seeded_bug_harness::ply_fuzz_seeded_bug\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n\n";
        let names = parse_failed_test_names(combined);
        assert_eq!(names, vec!["seeded_bug_harness::ply_fuzz_seeded_bug".to_string()]);
    }

    #[test]
    fn parses_the_fuzz_marker_scalar_and_vec_fields() {
        let combined = "some noise\nPLY_FUZZED_CEX|vec_sum|v=[1,2,3]\nmore noise\n";
        let (fname, fields) = parse_fuzz_marker(combined).unwrap();
        assert_eq!(fname, "vec_sum");
        assert_eq!(fields.get("v").unwrap(), "[1,2,3]");
    }

    #[test]
    fn decodes_marker_fields_into_witness_values() {
        let params = vec![Param { name: "x".into(), ty: RustType::U32, by_ref: false }];
        let mut fields = BTreeMap::new();
        fields.insert("x".to_string(), "4294967295".to_string());
        let decoded = decode_marker_fields(&fields, &params).unwrap();
        assert_eq!(decoded, vec![WitnessValue::UInt(4294967295)]);
    }

    #[test]
    fn decodes_vec_u8_marker_field() {
        let params = vec![Param { name: "v".into(), ty: RustType::VecU8, by_ref: true }];
        let mut fields = BTreeMap::new();
        fields.insert("v".to_string(), "[255,0,3]".to_string());
        let decoded = decode_marker_fields(&fields, &params).unwrap();
        assert_eq!(decoded, vec![WitnessValue::VecU8(vec![255, 0, 3])]);
    }

    #[test]
    fn refuses_to_decode_a_vec_of_non_u8_never_fabricating_a_value() {
        let params = vec![Param { name: "xs".into(), ty: RustType::Vec(Box::new(RustType::I32)), by_ref: true }];
        let mut fields = BTreeMap::new();
        fields.insert("xs".to_string(), "[-1,2,3]".to_string());
        assert!(decode_marker_fields(&fields, &params).is_none());
    }

    #[test]
    fn parses_high_reject_marker() {
        let combined = "noise\nPLY_FUZZ_HIGH_REJECT|safe_increment|12/20\nmore\n";
        let (fname, detail) = parse_high_reject_marker(combined).unwrap();
        assert_eq!(fname, "safe_increment");
        assert_eq!(detail, "12/20");
    }
}
