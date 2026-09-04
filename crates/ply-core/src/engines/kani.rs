//! The Kani engine adapter: runs `cargo kani` as a subprocess and parses its
//! output into a verdict that never conflates a genuine contract violation
//! with engine exhaustion (§5.4c's MUST). Never `--output-format=old`
//! (reports a timeout as success) and never `--quiet` (exits 0 on failure,
//! Kani issue #4745) -- both traps are structurally avoided by not passing
//! either flag anywhere in this module.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

/// One decoded `kani::any()` witness value, in call order.
#[derive(Debug, Clone, PartialEq)]
pub enum WitnessValue {
    UInt(u128),
    Int(i128),
    Bool(bool),
    /// A `Vec<u8>` witness, already truncated to its real (symbolic) length
    /// -- decoded from Kani's length-prefixed `any_vec` encoding (measured,
    /// see docs/m3-slice-findings.md).
    VecU8(Vec<u8>),
    /// A `Duration` witness: (whole seconds, nanoseconds -- always under one
    /// billion, the type's own invariant). Its own variant rather than two
    /// `UInt`s because `render_cex_test` needs both parts together to
    /// render one `Duration::new(secs, nanos)` call, and `decode_witness`
    /// consumes exactly two `kani::any()` entries to produce it -- see
    /// `RustType::Duration`'s own doc comment for why it is a pair, not a
    /// derived struct. A `NonZero{X}` witness needs no variant of its own:
    /// only its *construction* differs from a plain scalar (see
    /// `RustType::NonZero`'s doc comment), and its inner integer decodes
    /// (and renders) exactly like one, via `UInt`/`Int` below.
    Duration(u64, u32),
    /// A `String`/`&str` witness, already decoded back to its real content
    /// (never the wire-escaped form the marker printed). Added 2026-09-04:
    /// text was the one everyday parameter shape whose counterexample could
    /// not be replayed, and in Ply's own library 17 of 23 claims take it, so
    /// the "witness-only" path was the common case rather than the edge one.
    /// `render_cex_test` writes it back out with Rust's own `{:?}` for
    /// `str`, which is exactly a valid Rust string literal -- quotes,
    /// backslashes and control characters all escaped.
    Str(String),
}

/// The engine-honest outcome of one Kani run. `Timeout` and `Violation` are
/// structurally distinct variants precisely so an adapter cannot conflate
/// them (§5.4c MUST) -- there is no code path that can construct a
/// `Violation` from a timed-out run.
#[derive(Debug)]
pub enum KaniOutcome {
    /// `bounded(k)` earned cleanly.
    Verified,
    /// A genuine falsified claim, always carrying a witness (never
    /// constructed without one -- see `parse_output`).
    Violation {
        witness_bytes: Vec<Vec<u8>>,
        raw_output: String,
    },
    /// CBMC exhausted its budget -- distinguished from `Violation` by
    /// reading past Kani's shared "VERIFICATION:- FAILED" line to the
    /// "CBMC timed out" reason underneath (§5.4c).
    Timeout { raw_output: String },
    /// Kani's output did not match any recognized shape (parse failure, a
    /// failure with no witness, etc). Never silently reported as a
    /// violation or a clean pass -- surfaced as its own honest outcome.
    ToolError { raw_output: String, reason: String },
}

pub struct KaniRunConfig {
    pub crate_dir: std::path::PathBuf,
    pub harness_path: String,
    pub engine_timeout_secs: u32,
    /// Whether this harness carries `#[kani::stub(..)]` attributes (D5's
    /// second branch, §5.5). Kani rejects the attribute outright without
    /// `-Z stubbing` ("Using the stub attribute requires activating the
    /// unstable `stubbing` feature"), so the flag is passed exactly when a
    /// stub is present rather than always.
    pub enable_stubbing: bool,
}

/// The `-Z` set every `cargo kani` invocation here carries. These are
/// compilation inputs: they decide what the run even attempts, so a proof
/// earned under one set is not evidence about another.
///
/// One function rather than a literal at the call site, because a *second*
/// consumer reads it: the fingerprint a result is recorded under (§5.2a)
/// names the engine's flags, and if this list could change without that
/// string changing, results earned under the old flags would be reused
/// under the new ones. The list is pinned by an exact test below, so a
/// change to it is reviewed rather than incidental.
pub fn unstable_flags(enable_stubbing: bool) -> Vec<&'static str> {
    let mut flags = vec![
        "-Z",
        "function-contracts",
        "-Z",
        "unstable-options",
        "-Z",
        "concrete-playback",
    ];
    if enable_stubbing {
        flags.push("-Z");
        flags.push("stubbing");
    }
    flags
}

/// What this machine's Kani calls itself, for the record a result is stored
/// under (§5.2a): a proof earned under one engine version is not evidence
/// about another. `None` when `cargo kani` is not installed or will not
/// answer -- a `bounded` check then earns no evidence at all, so nothing is
/// recorded and nothing can be reused.
/// Probed in the crate being verified. Rustup shims `cargo` per toolchain,
/// so a probe run from the caller's directory can report a different engine
/// build than the one that will actually run -- the same defect found in the
/// compiler probe (external review, 2026-08-30). The engine version is a
/// fingerprint input, so getting it from the wrong place lets stale evidence
/// survive a real change.
pub fn version(crate_dir: &std::path::Path) -> Option<String> {
    let out = Command::new("cargo")
        .args(["kani", "--version"])
        .current_dir(crate_dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = super::strip_ansi(String::from_utf8_lossy(&out.stdout).trim());
    if text.is_empty() { None } else { Some(text) }
}

/// Runs `cargo kani` against one harness and classifies the result. Always
/// requests concrete playback (`-Z concrete-playback --concrete-playback
/// print`) -- cheap on success (nothing to print) and the only source of a
/// witness on failure.
pub fn run(cfg: &KaniRunConfig) -> Result<KaniOutcome> {
    Ok(parse_output(&invoke(cfg)?))
}

/// Runs one harness and returns its combined stdout+stderr. The flag set is
/// identical for every harness in a crate on purpose: Kani's `-Z` flags are
/// compilation inputs, so varying them between the proof and the
/// promise-content probes beside it would rebuild the crate for each.
fn invoke(cfg: &KaniRunConfig) -> Result<String> {
    let timeout_arg = format!("{}s", cfg.engine_timeout_secs);
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&cfg.crate_dir).arg("kani");
    cmd.args(unstable_flags(cfg.enable_stubbing));
    let output = cmd
        .args([
            "--harness-timeout",
            &timeout_arg,
            "--exact",
            "--harness",
            &cfg.harness_path,
            "--concrete-playback",
            "print",
        ])
        .output()
        .with_context(|| format!("spawning `cargo kani` in {}", cfg.crate_dir.display()))?;

    Ok(super::strip_ansi(&format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )))
}

/// Runs one **probe**: a harness asked a yes/no question about an assertion,
/// with no function body in it (§5.5's promise-content gate). Deliberately
/// not [`run`]: a probe wants no witness and demanding one would be the
/// wrong rule here. §5.4c's MUST is that a *violation verdict* never appears
/// without evidence; a probe's failing assertion is not a verdict about
/// anyone's code, it is the benign answer "a value exists".
pub fn run_probe(cfg: &KaniRunConfig) -> Result<ProbeOutcome> {
    Ok(classify_probe(&invoke(cfg)?))
}

/// What one promise-content probe said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The assertion held for every value in the probe's domain.
    Holds,
    /// A value breaking the assertion exists.
    Refuted,
    /// Neither -- a timeout, or output this adapter does not recognise.
    /// Never rounded to either answer.
    Undecided(String),
}

/// The same reading discipline as [`parse_output`], minus the witness: read
/// PAST the shared "VERIFICATION:- FAILED" line to the reason underneath,
/// because Kani renders a CBMC timeout and a genuine failing check
/// identically at that line (§5.4c).
pub fn classify_probe(combined: &str) -> ProbeOutcome {
    if combined.contains("VERIFICATION:- SUCCESSFUL") {
        return ProbeOutcome::Holds;
    }
    if combined.contains("CBMC timed out") {
        return ProbeOutcome::Undecided("the engine ran out of time on it".into());
    }
    if combined.contains("VERIFICATION:- FAILED") {
        return ProbeOutcome::Refuted;
    }
    ProbeOutcome::Undecided(
        "the engine printed neither a pass nor a failure for it, so Ply has no answer to \
         report"
            .into(),
    )
}

/// Parses Kani's combined stdout+stderr into a `KaniOutcome`. Pure function,
/// tested directly against captured real output (no subprocess needed) --
/// this is the module's invariant surface: never emit `Violation` without a
/// witness, never conflate timeout with violation.
pub fn parse_output(combined: &str) -> KaniOutcome {
    if combined.contains("VERIFICATION:- SUCCESSFUL") {
        return KaniOutcome::Verified;
    }
    if !combined.contains("VERIFICATION:- FAILED") {
        return KaniOutcome::ToolError {
            raw_output: combined.to_string(),
            reason: "neither VERIFICATION:- SUCCESSFUL nor VERIFICATION:- FAILED appeared in Kani's output".into(),
        };
    }
    // Read PAST the shared "VERIFICATION:- FAILED" line to the real reason,
    // per §5.4c's MUST: Kani renders a CBMC timeout and a genuine failed
    // check identically at that line.
    if combined.contains("CBMC timed out") {
        return KaniOutcome::Timeout {
            raw_output: combined.to_string(),
        };
    }
    match extract_witness_bytes(combined) {
        Some(witness_bytes) => KaniOutcome::Violation { witness_bytes, raw_output: combined.to_string() },
        None => KaniOutcome::ToolError {
            raw_output: combined.to_string(),
            reason: "a failing check was reported but no concrete-playback witness could be \
                     extracted -- never reporting this as `violation` without a witness (the MUST in §5.4c)"
                .into(),
        },
    }
}

/// Extracts the `concrete_vals: Vec<Vec<u8>> = vec![ ... ]` byte vectors from
/// Kani's printed "Concrete playback unit test" block, in `kani::any()` call
/// order. Each inner `vec![...]` is one witness value's raw bytes.
fn extract_witness_bytes(combined: &str) -> Option<Vec<Vec<u8>>> {
    let marker = "let concrete_vals: Vec<Vec<u8>> = vec![";
    let start = combined.find(marker)?;
    let after_marker = start + marker.len();
    // Find the matching closing "];" for the outer vec![...] by bracket
    // depth over '[' and ']' from after_marker.
    let bytes = combined.as_bytes();
    let mut depth = 1i32; // one '[' already consumed by the marker
    let mut i = after_marker;
    let end = loop {
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break i;
                }
            }
            _ => {}
        }
        i += 1;
    };
    let body = &combined[after_marker..end];

    // body is a sequence of `// <comment>\n        vec![b, b, ...],` entries.
    // Extract each inner `vec![ ... ]` in order.
    let mut result = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find("vec![") {
        let inner_start = pos + "vec![".len();
        let inner_bytes = rest.as_bytes();
        let mut d = 1i32;
        let mut j = inner_start;
        let inner_end = loop {
            if j >= inner_bytes.len() {
                return None;
            }
            match inner_bytes[j] {
                b'[' => d += 1,
                b']' => {
                    d -= 1;
                    if d == 0 {
                        break j;
                    }
                }
                _ => {}
            }
            j += 1;
        };
        let list = &rest[inner_start..inner_end];
        let values: Result<Vec<u8>, _> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<u8>())
            .collect();
        match values {
            Ok(v) => result.push(v),
            Err(_) => return None,
        }
        rest = &rest[inner_end + 1..];
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Decodes raw witness byte-vectors into typed values, given the parameter
/// shapes in declaration order (matching the exact `kani::any()` call order
/// codegen emitted -- `harness::generate_proof_module`'s `lets` sequence).
/// `vec_bound` is the declared bound used for any `Vec<u8>` parameter.
///
/// Scalar decoding: little-endian, `scalar_byte_width()` bytes (measured
/// against real Kani output, see docs/m3-slice-findings.md).
/// `Vec<u8>` decoding: Kani's `any_vec::<u8, N>` encodes one length entry
/// (8 little-endian bytes, a `usize`) followed by exactly `N` single-byte
/// entries; only the first `length` of those N are the real elements
/// (measured empirically -- see docs/m3-slice-findings.md).
pub fn decode_witness(
    witness_bytes: &[Vec<u8>],
    params: &[crate::harness::Param],
    vec_bound: u32,
) -> Result<Vec<WitnessValue>> {
    let mut cursor = 0usize;
    let mut out = Vec::new();
    for p in params {
        match &p.ty {
            crate::harness::RustType::VecU8 => {
                let len_bytes = witness_bytes
                    .get(cursor)
                    .with_context(|| format!("missing length witness entry for `{}`", p.name))?;
                let mut len_arr = [0u8; 8];
                for (i, b) in len_bytes.iter().take(8).enumerate() {
                    len_arr[i] = *b;
                }
                let length = u64::from_le_bytes(len_arr) as usize;
                cursor += 1;
                let n = vec_bound as usize;
                let mut elems = Vec::with_capacity(length.min(n));
                for k in 0..n {
                    let entry = witness_bytes.get(cursor + k).with_context(|| {
                        format!("missing element witness entry {k} for `{}`", p.name)
                    })?;
                    if k < length {
                        elems.push(*entry.first().unwrap_or(&0));
                    }
                }
                cursor += n;
                out.push(WitnessValue::VecU8(elems));
            }
            // The inner integer is the only actually-symbolic `kani::any()`
            // call this codegen emitted for a `NonZero` (see
            // `harness::render_kani_args`) -- `NonZero{X}::new(..).unwrap()`
            // is deterministic from it, so decoding is exactly decoding one
            // scalar of the inner type. One witness entry consumed, same as
            // any plain scalar.
            crate::harness::RustType::NonZero(inner) => {
                let entry = witness_bytes
                    .get(cursor)
                    .with_context(|| format!("missing witness entry for `{}`", p.name))?;
                cursor += 1;
                out.push(decode_scalar_entry(entry, inner, &p.name)?);
            }
            // Two independent `kani::any()` calls, in the exact order
            // `render_kani_args` emitted them: seconds, then nanoseconds.
            crate::harness::RustType::Duration => {
                let secs_entry = witness_bytes
                    .get(cursor)
                    .with_context(|| format!("missing seconds witness entry for `{}`", p.name))?;
                cursor += 1;
                let nanos_entry = witness_bytes.get(cursor).with_context(|| {
                    format!("missing nanoseconds witness entry for `{}`", p.name)
                })?;
                cursor += 1;
                let mut secs_buf = [0u8; 8];
                for (i, b) in secs_entry.iter().take(8).enumerate() {
                    secs_buf[i] = *b;
                }
                let mut nanos_buf = [0u8; 4];
                for (i, b) in nanos_entry.iter().take(4).enumerate() {
                    nanos_buf[i] = *b;
                }
                out.push(WitnessValue::Duration(
                    u64::from_le_bytes(secs_buf),
                    u32::from_le_bytes(nanos_buf),
                ));
            }
            other => {
                let entry = witness_bytes
                    .get(cursor)
                    .with_context(|| format!("missing witness entry for `{}`", p.name))?;
                cursor += 1;
                out.push(decode_scalar_entry(entry, other, &p.name)?);
            }
        }
    }
    Ok(out)
}

/// Decodes one raw witness entry as a plain scalar of `ty` -- shared between
/// the generic scalar arm above and `NonZero`'s inner integer, so a `NonZero`
/// is never a second, hand-duplicated copy of this decoding.
fn decode_scalar_entry(
    entry: &[u8],
    ty: &crate::harness::RustType,
    param_name: &str,
) -> Result<WitnessValue> {
    let width = ty
        .scalar_byte_width()
        .with_context(|| format!("`{param_name}` has no known scalar byte width"))?;
    let is_signed = matches!(
        ty,
        crate::harness::RustType::I8
            | crate::harness::RustType::I16
            | crate::harness::RustType::I32
            | crate::harness::RustType::I64
            | crate::harness::RustType::Isize
    );
    if matches!(ty, crate::harness::RustType::Bool) {
        return Ok(WitnessValue::Bool(entry.first().copied().unwrap_or(0) != 0));
    }
    let mut buf = [0u8; 16];
    for (i, b) in entry.iter().take(width).enumerate() {
        buf[i] = *b;
    }
    if is_signed {
        // Sign-extend from `width` bytes into i128.
        let mut v: i128 = i128::from_le_bytes(buf);
        let bits = width * 8;
        if bits < 128 {
            let sign_bit = 1i128 << (bits - 1);
            if v & sign_bit != 0 {
                v -= 1i128 << bits;
            }
        }
        Ok(WitnessValue::Int(v))
    } else {
        Ok(WitnessValue::UInt(u128::from_le_bytes(buf)))
    }
}

/// Runs `cargo kani playback` (`--lib`, per FINDINGS.md's "playback needs
/// --lib" cost) so the concrete-playback artifact stored as `kani_witness`
/// (D7's rename) can be shown to actually replay under the pinned toolchain
/// -- the §9 oracle's other half. Never asserts the replay *fails*: D7's
/// caveat 3 established that an `ensures`-violation witness replays green
/// (contract closures are never re-evaluated during playback).
///
/// The wall-clock budget is enforced in-process by
/// `engines::run_with_timeout`, never by shelling out to a `timeout`/
/// `gtimeout` binary -- macOS ships neither, so wrapping the real command in
/// one made this fail to spawn at all rather than ever run playback.
pub fn run_playback(
    crate_dir: &Path,
    exact_test_name: &str,
    timeout: Duration,
) -> Result<super::TimedOutput> {
    let mut cmd = Command::new("cargo");
    cmd.arg("kani")
        .arg("playback")
        .args([
            "-Z",
            "concrete-playback",
            "-Z",
            "function-contracts",
            "-Z",
            "unstable-options",
        ])
        .arg("--lib")
        .current_dir(crate_dir)
        .arg("--")
        .arg("--exact")
        .arg(exact_test_name);
    super::run_with_timeout(&mut cmd, timeout)
        .with_context(|| format!("running `cargo kani playback` in {}", crate_dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{Param, RustType};

    /// Exact, because these flags are recorded as part of what a stored
    /// result stood on (§5.2a). Changing them changes what a `bounded`
    /// verdict means, so the change has to be typed here too -- and every
    /// result earned under the old set stops matching, which is the point.
    #[test]
    fn the_unstable_flag_set_is_pinned_because_a_result_is_recorded_under_it() {
        assert_eq!(
            unstable_flags(false),
            [
                "-Z",
                "function-contracts",
                "-Z",
                "unstable-options",
                "-Z",
                "concrete-playback"
            ]
        );
        assert_eq!(
            unstable_flags(true).last(),
            Some(&"stubbing"),
            "a stubbed proof replaces a callee with its promise, which is a different \
             obligation from the same proof without it"
        );
    }

    #[test]
    fn recognizes_clean_success() {
        let out = "blah blah\nVERIFICATION:- SUCCESSFUL\nComplete\n";
        assert!(matches!(parse_output(out), KaniOutcome::Verified));
    }

    #[test]
    fn recognizes_timeout_never_as_violation() {
        let out = "Unwinding loop ...\nCBMC failed\nVERIFICATION:- FAILED\nCBMC timed out. You may want to rerun your proof with a larger timeout.\n";
        match parse_output(out) {
            KaniOutcome::Timeout { .. } => {}
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[test]
    fn recognizes_violation_with_witness() {
        let out = r#"
SUMMARY:
 ** 1 of 61 failed
Failed Checks: | result | * result == x

VERIFICATION:- FAILED

Concrete playback unit test for `ply_generated::ply_proof_clamp`:
```
#[test]
fn kani_concrete_playback_ply_proof_clamp_123() {
    let concrete_vals: Vec<Vec<u8>> = vec![
        // 4294967295
        vec![255, 255, 255, 255],
    ];
    kani::concrete_playback_run(concrete_vals, ply_proof_clamp);
}
```
"#;
        match parse_output(out) {
            KaniOutcome::Violation { witness_bytes, .. } => {
                assert_eq!(witness_bytes, vec![vec![255, 255, 255, 255]]);
            }
            other => panic!("expected Violation, got {other:?}"),
        }
    }

    #[test]
    fn failure_without_witness_is_tool_error_never_violation() {
        let out = "VERIFICATION:- FAILED\nsome other reason with no playback block\n";
        match parse_output(out) {
            KaniOutcome::ToolError { .. } => {}
            other => panic!("a failure with no witness must never become Violation, got {other:?}"),
        }
    }

    #[test]
    fn decodes_u32_witness_matching_real_kani_output() {
        // Bytes observed for real from `cargo kani` on the clamp fixture:
        // x = u32::MAX, little-endian [255,255,255,255].
        let params = vec![Param {
            name: "x".into(),
            ty: RustType::U32,
            by_ref: false,
        }];
        let decoded = decode_witness(&[vec![255, 255, 255, 255]], &params, 0).unwrap();
        assert_eq!(decoded, vec![WitnessValue::UInt(u32::MAX as u128)]);
    }

    #[test]
    fn decodes_vec_u8_witness_length_prefixed() {
        // Bytes observed for real from a Vec<u8> violation: one 8-byte
        // little-endian length entry, then N single-byte entries.
        let params = vec![Param {
            name: "v".into(),
            ty: RustType::VecU8,
            by_ref: true,
        }];
        let witness = vec![
            vec![1, 0, 0, 0, 0, 0, 0, 0], // length = 1
            vec![255],
            vec![255],
            vec![255],
        ];
        let decoded = decode_witness(&witness, &params, 3).unwrap();
        assert_eq!(decoded, vec![WitnessValue::VecU8(vec![255])]);
    }

    // -- 2026-08-27: NonZero and Duration witnesses ------------------------
    //
    // `render_kani_args` emits exactly one `kani::any()` call for a
    // `NonZero`'s inner integer (the wrapper itself is never symbolic) and
    // exactly two for a `Duration` (seconds, then nanoseconds) -- these pin
    // that decoding consumes the same number of witness entries codegen
    // produced, in the same order.

    #[test]
    fn decodes_a_nonzero_u32_witness_as_its_inner_integer() {
        let params = vec![Param {
            name: "n".into(),
            ty: RustType::NonZero(Box::new(RustType::U32)),
            by_ref: false,
        }];
        // The only kani::any() call this shape emits is for the inner u32.
        let decoded = decode_witness(&[vec![7, 0, 0, 0]], &params, 0).unwrap();
        assert_eq!(decoded, vec![WitnessValue::UInt(7)]);
    }

    #[test]
    fn decodes_a_duration_witness_as_two_entries_in_construction_order() {
        let params = vec![Param {
            name: "d".into(),
            ty: RustType::Duration,
            by_ref: false,
        }];
        let witness = vec![
            // secs = 7 (u64, 8 bytes)
            vec![7, 0, 0, 0, 0, 0, 0, 0],
            // nanos = 500_000_000 (u32, 4 bytes) = 0x1DCD6500
            vec![0x00, 0x65, 0xCD, 0x1D],
        ];
        let decoded = decode_witness(&witness, &params, 0).unwrap();
        assert_eq!(decoded, vec![WitnessValue::Duration(7, 500_000_000)]);
    }
}
