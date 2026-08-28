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
use crate::harness_crate::ModuleSpan;

#[derive(Debug, Clone)]
pub struct HarnessTestRun {
    pub timed_out: bool,
    pub success: bool,
    pub combined_output: String,
    /// Fully-qualified failing test names (`<fn>_harness::ply_fuzz_<fn>`,
    /// etc.), extracted from libtest's final `failures:` summary block --
    /// see `parse_failed_test_names`. It is deliberately *not* the per-test
    /// `---- <name> stdout ----` detail header, which libtest never emits
    /// under `--nocapture` (which this adapter always passes): relying on
    /// that header reported every fuzz-found violation as a clean pass, the
    /// real bug docs/m4-findings.md finding 3 records.
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
        .with_context(|| {
            format!(
                "spawning `cargo test -p {harness_package}` in {}",
                workspace_root.display()
            )
        })?;

    let combined = super::strip_ansi(&format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ));
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

/// Whether the harness crate's *test binary* built at all, with none of it
/// executed (`cargo test --no-run`) -- the misattribution fix's preflight
/// check. It exists because one broken function's generated module used to
/// take the whole crate's compile failure down with it onto every other
/// claimed function sharing the same generated file: each one's own,
/// separately-filtered `cargo test -p <pkg> --lib <fn>_harness::` recompiles
/// the *entire* crate first, so a build failure anywhere reproduced
/// identically -- same compiler error, same missing fn -- no matter which
/// fn's tests were asked for. Checking the build once, before any per-fn run,
/// is what lets a failure be isolated (`attribute_build_errors`) and the
/// crate rebuilt without the offending module(s) so every innocent claim
/// still gets to run for real. `--no-run` is deliberate: this never needs to
/// execute a single fuzz case, only to know whether the crate compiles.
pub struct HarnessBuildCheck {
    pub timed_out: bool,
    pub build_ok: bool,
    pub combined_output: String,
}

pub fn check_harness_builds(
    workspace_root: &Path,
    harness_package: &str,
    timeout_secs: u32,
) -> Result<HarnessBuildCheck> {
    let timeout_arg = format!("{timeout_secs}s");
    let output = Command::new("timeout")
        .arg(&timeout_arg)
        .arg("cargo")
        .arg("test")
        .arg("-p")
        .arg(harness_package)
        .arg("--lib")
        .arg("--no-run")
        .current_dir(workspace_root)
        .output()
        .with_context(|| {
            format!(
                "spawning `cargo test -p {harness_package} --lib --no-run` in {}",
                workspace_root.display()
            )
        })?;
    let combined = super::strip_ansi(&format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ));
    let timed_out = output.status.code() == Some(124);
    Ok(HarnessBuildCheck {
        timed_out,
        build_ok: output.status.success() && !timed_out,
        combined_output: combined,
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

/// Counts the individual libtest per-test lines (`test <name> ... ok` /
/// `... FAILED`) reporting that one of *this function's own* generated tests
/// actually ran -- the positive evidence a passing `test`/`fuzz` check must
/// show before Ply trusts it (2026-08-27 review, "the eleventh false pass":
/// a receiver method with no worked examples and no direct-contract cases
/// generated no test module at all, `cargo test`'s own filter then matched
/// nothing, the run exited 0 with zero tests run, and "no failing test" was
/// read as "held").
///
/// `cargo test`'s own filter argument is a *plain substring* match, not an
/// anchor, so the invocation this counts can have executed more than this
/// function's own tests -- a top-level `parse` and a `util::parse` collide
/// this way, because `parse_harness::` is a substring of
/// `util_parse_harness::` (docs/review-strings-receivers.md finding 2).
/// Counting only lines whose test name actually starts with `module_prefix`
/// (`harness_module_name(cf) + "::"`) is what makes this a safe positive-
/// evidence check rather than a new way to launder someone else's tests
/// into "this function ran something": a function whose own module
/// contributed nothing still reads zero here even when cargo, underneath,
/// executed a same-shaped sibling's tests instead.
///
/// Libtest always prints this per-test line, independent of `--nocapture`
/// (`run_harness_tests` always passes that flag for the `PLY_FUZZ_HIGH_REJECT`
/// marker; `--nocapture` only suppresses a test's own captured stdout, never
/// this pass/fail line), so it is a reliable positive signal even when the
/// final `test result:` summary line is scoped to the same over-broad match.
pub fn count_tests_executed(combined: &str, module_prefix: &str) -> u32 {
    let needle = format!("test {module_prefix}");
    combined
        .lines()
        .filter(|l| {
            let t = l.trim();
            t.starts_with(&needle) && (t.ends_with("... ok") || t.ends_with("... FAILED"))
        })
        .count() as u32
}

/// The first *specific* compiler error in a failed harness build -- the one
/// line that names what actually went wrong (`error[E0308]: mismatched
/// types`), not cargo's own trailing summary (`error: could not compile
/// ...`, which names no cause at all). Returns `None` when the output
/// carries no error line, so a caller never invents one.
///
/// This exists because §8 forbids passing engine output through raw while
/// §5.4c requires carrying "the distinguishing engine output into the
/// diagnostic": a harness that fails to build has no test result to parse,
/// and this line is the only handle a reader (or an agent mid-repair) has.
pub fn first_build_error(combined: &str) -> Option<String> {
    let mut summary_only: Option<String> = None;
    for line in combined.lines() {
        let t = line.trim();
        if !t.starts_with("error") {
            continue;
        }
        if t.starts_with("error: could not compile") || t.starts_with("error: aborting") {
            if summary_only.is_none() {
                summary_only = Some(t.to_string());
            }
            continue;
        }
        return Some(t.to_string());
    }
    summary_only
}

/// One specific compiler error from a failed harness build, together with
/// the line its own `--> path:LINE:COL` names -- `None` when the error
/// carries no such span at all (a linker failure, an ICE), which
/// `attribute_build_errors` must treat as unattributable rather than guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    pub message: String,
    pub line: Option<usize>,
}

/// Every *specific* compiler error in a failed build (the same filter
/// `first_build_error` uses: never cargo's own summary line, which names no
/// cause), each paired with the line number its own `--> ` span names in a
/// path ending `path_suffix` -- the generated harness crate's own
/// `src/lib.rs`, so an error rustc actually reports against some other file
/// (the target crate's own source, a dependency) is never mistaken for one
/// of Ply's generated modules. rustc prints the `--> ` line immediately
/// after an error's own header line in every version this adapter has seen;
/// this looks a few lines ahead rather than assuming exactly one, so a
/// blank line rustc might insert first does not defeat it.
pub fn build_errors_with_lines(combined: &str, path_suffix: &str) -> Vec<BuildError> {
    let lines: Vec<&str> = combined.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if !t.starts_with("error") {
            continue;
        }
        if t.starts_with("error: could not compile") || t.starts_with("error: aborting") {
            continue;
        }
        let message = t.to_string();
        let mut found_line = None;
        for look in lines.iter().skip(i + 1).take(5) {
            let lt = look.trim();
            if lt.is_empty() {
                continue;
            }
            if let Some(rest) = lt.strip_prefix("--> ") {
                found_line = parse_span_line(rest, path_suffix);
            }
            break;
        }
        out.push(BuildError {
            message,
            line: found_line,
        });
    }
    out
}

/// Parses a `--> ` span's own text (`path:LINE:COL`) into `LINE`, only when
/// `path` ends with `path_suffix`.
fn parse_span_line(rest: &str, path_suffix: &str) -> Option<usize> {
    let mut parts = rest.rsplitn(3, ':');
    let _col = parts.next()?;
    let line_text = parts.next()?;
    let path = parts.next()?;
    if !path.ends_with(path_suffix) {
        return None;
    }
    line_text.parse::<usize>().ok()
}

/// Maps each `BuildError` with a known line onto the `ModuleSpan` whose
/// range contains it, returning the *first* attributed message per fn (the
/// misattribution fix's core: what would otherwise be reported against
/// every claim in the crate is instead pinned to the one whose own
/// generated code the compiler actually pointed at). An error with no line,
/// or whose line falls outside every known module, contributes nothing --
/// callers must treat any fn left unattributed as still unexplained, never
/// silently declare it innocent.
pub fn attribute_build_errors(
    errors: &[BuildError],
    spans: &[ModuleSpan],
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for err in errors {
        let Some(line) = err.line else { continue };
        let Some(span) = spans
            .iter()
            .find(|s| line >= s.start_line && line <= s.end_line)
        else {
            continue;
        };
        out.entry(span.fn_ident.clone())
            .or_insert_with(|| err.message.clone());
    }
    out
}

/// Parses the last `PLY_FUZZED_CEX|<fn>|k1=v1;k2=v2` marker line out of
/// captured output, returning the fn name and its fields. Fields with a
/// `[...]` value (a `Vec`/`BTreeSet`) keep the brackets for
/// `decode_marker_fields` to parse.
pub fn parse_fuzz_marker(combined: &str) -> Option<(String, BTreeMap<String, String>)> {
    let line = combined
        .lines()
        .rev()
        .find(|l| l.contains("PLY_FUZZED_CEX|"))?;
    let after = line.split_once("PLY_FUZZED_CEX|")?.1;
    let (fn_name, rest) = after.split_once('|')?;
    let mut fields = BTreeMap::new();
    for entry in split_top_level_semicolons(rest) {
        if let Some((k, v)) = entry.split_once('=') {
            // Universal, not gated on the field's own type: a `String`
            // field is the only one `marker_display_expr` ever escapes
            // (see its own doc), so unescaping every field is a no-op for
            // every other type -- their rendered text never contains a
            // backslash -- and means `decode_marker_fields` (which only
            // ever sees the already-unescaped text) and the raw display
            // path (`verify.rs`'s `fields.get(&p.name)`, shown on a
            // witness-only violation) never have to know which fields were
            // escaped and which were not.
            fields.insert(k.to_string(), unescape_marker_value(v));
        }
    }
    Some((fn_name.to_string(), fields))
}

/// The exact reverse of `marker_display_expr`'s `RustType::String` arm:
/// `\\`, `\;`, `\=`, `\[`, `\]`, `\n`, `\r` each collapse back to the one
/// character they stand for. An unrecognised escape (a lone trailing `\`,
/// or `\` followed by something neither side ever emits) is kept literally
/// rather than dropped, so a decode this function cannot make sense of
/// loses no information -- the caller sees the odd text rather than a
/// silently mangled one.
fn unescape_marker_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some(';') => out.push(';'),
            Some('=') => out.push('='),
            Some('[') => out.push('['),
            Some(']') => out.push(']'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits on `;` but never inside a `[...]` bracket (a collection field's
/// joined elements never contain `;`, but this keeps the parser correct if
/// that ever changes), and never on an *escaped* `;`/`[`/`]`
/// (`marker_display_expr`'s `String` arm escapes exactly these -- among
/// others -- so a sampled string containing a raw `;` or bracket can never
/// be mistaken for this wire format's own field separator or collection
/// delimiter). A `\` outside an escape sequence is not itself special here
/// -- it only ever appears as the first half of one of the pairs
/// `unescape_marker_value` reverses, so simply not inspecting the character
/// right after it is enough to keep this splitter correct without having
/// to know which escape it is.
fn split_top_level_semicolons(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
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

/// Recovers the shrunk failing input from **proptest's own** failure report.
///
/// Ply's generated harness prints its `PLY_FUZZED_CEX` marker only from the
/// postcondition arm, so a body that *panics* never reaches it -- and until
/// 2026-08-25 the adapter then reported `X0901`/`tool_error`, which meant a
/// genuine crash bug could never be called a `violation` at any seed
/// (docs/review-post-004-strategy.md's correction to vetting 004's finding
/// 4). proptest itself catches the panic, shrinks, and prints the minimal
/// input in its own report -- `Test failed: <why>.\nminimal failing input:
/// <pretty Debug>` -- which the adapter was discarding. This reads it back.
///
/// Returns one text per value in call order (a single value, or the members
/// of proptest's tuple), or `None` when the report has no such line, so the
/// caller still reports `X0901` rather than inventing a witness.
pub fn parse_proptest_minimal_input(combined: &str) -> Option<Vec<String>> {
    const MARKER: &str = "minimal failing input: ";
    let start = combined.rfind(MARKER)? + MARKER.len();
    let mut text = String::new();
    for line in combined[start..].lines() {
        let t = line.trim();
        // proptest prints with `{:#?}`, so a tuple or collection spans
        // several lines; libtest's own trailer is where it stops.
        if t.starts_with("note:")
            || t.starts_with("stack backtrace")
            || t.starts_with("test ")
            || t == "failures:"
            || t.starts_with("error")
        {
            break;
        }
        text.push_str(t);
        if !text.is_empty() && is_balanced(&text) {
            break;
        }
    }
    let text = normalise_debug(&text);
    if text.is_empty() {
        return None;
    }
    Some(if text.starts_with('(') && text.ends_with(')') {
        let inner = &text[1..text.len() - 1];
        split_top_level(inner, ',')
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![text]
    })
}

fn is_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth == 0
}

/// `{:#?}` leaves a trailing comma before every closing bracket once the
/// newlines are gone (`(1,2,)`), which every downstream parser here would
/// otherwise read as an extra, empty value.
fn normalise_debug(s: &str) -> String {
    let mut out = s.trim().to_string();
    while out.contains(",)") || out.contains(",]") || out.contains(",}") {
        out = out.replace(",)", ")").replace(",]", "]").replace(",}", "}");
    }
    out
}

/// Splits on `sep` at bracket depth 0.
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
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
    let line = combined
        .lines()
        .find(|l| l.contains("PLY_FUZZ_HIGH_REJECT|"))?;
    let after = line.split_once("PLY_FUZZ_HIGH_REJECT|")?.1;
    let (fn_name, detail) = after.split_once('|')?;
    Some((fn_name.trim().to_string(), detail.trim().to_string()))
}

/// What the generated test reports when proptest *abandoned* the run: its
/// own global-reject limit fired, so it stopped drawing inputs long before
/// the requested case count was reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzAbort {
    pub fn_name: String,
    /// proptest's own reason text (e.g. "Too many global rejects").
    pub reason: String,
    /// Cases that passed the `requires` filter and were actually checked.
    pub accepted: u32,
    /// Inputs the `requires` filter threw away.
    pub rejected: u32,
}

/// Parses the `PLY_FUZZ_ABORT|<fn>|<reason>|accepted=<a>|rejected=<r>`
/// marker. Distinct from the high-rejection *warning* marker on purpose: a
/// high rejection rate still produces the requested cases (weaker spread,
/// honest count), while an abort produces essentially none, and the two must
/// not reach the same verdict (2026-08-24 M4 review, D4).
pub fn parse_abort_marker(combined: &str) -> Option<FuzzAbort> {
    let line = combined.lines().find(|l| l.contains("PLY_FUZZ_ABORT|"))?;
    let after = line.split_once("PLY_FUZZ_ABORT|")?.1;
    let mut parts = after.split('|');
    let fn_name = parts.next()?.trim().to_string();
    let reason = parts.next()?.trim().to_string();
    let accepted = parts
        .next()?
        .trim()
        .strip_prefix("accepted=")?
        .parse::<u32>()
        .ok()?;
    let rejected = parts
        .next()?
        .trim()
        .strip_prefix("rejected=")?
        .parse::<u32>()
        .ok()?;
    Some(FuzzAbort {
        fn_name,
        reason,
        accepted,
        rejected,
    })
}

fn parse_u8_list(raw: &str) -> Option<Vec<u8>> {
    let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() {
        return Some(vec![]);
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<u8>().ok())
        .collect()
}

/// Decodes a fuzz marker's fields into the *same* `WitnessValue` type Kani
/// witnesses decode into (the D7 plan's "two consumers, one renderer"), in
/// `params` order. Returns `None` -- never a fabricated value -- for any
/// parameter whose type has no `WitnessValue` representation: `WitnessValue`
/// can spell scalars and a `Vec<u8>`, so **any** `BTreeSet` (`BTreeSet<u8>`
/// included -- the M4 acceptance shape) and any `Vec` of a non-`u8` scalar
/// land here. The caller reports that case as a witness-only violation
/// (`W0541`), never force-rendered.
pub fn decode_marker_fields(
    fields: &BTreeMap<String, String>,
    params: &[Param],
) -> Option<Vec<WitnessValue>> {
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
            RustType::Usize => WitnessValue::UInt(raw.parse::<u128>().ok()?),
            RustType::Isize => WitnessValue::Int(raw.parse::<i128>().ok()?),
            RustType::VecU8 => WitnessValue::VecU8(parse_u8_list(raw)?),
            RustType::Vec(inner) if inner.as_ref() == &RustType::U8 => {
                WitnessValue::VecU8(parse_u8_list(raw)?)
            }
            // `marker_display_expr` prints a `NonZero{X}` with its own
            // (plain-number) `Display` impl, so decoding it is exactly
            // decoding its inner integer.
            RustType::NonZero(inner)
                if matches!(
                    inner.as_ref(),
                    RustType::U8 | RustType::U16 | RustType::U32 | RustType::U64 | RustType::Usize
                ) =>
            {
                WitnessValue::UInt(raw.parse::<u128>().ok()?)
            }
            RustType::NonZero(inner)
                if matches!(
                    inner.as_ref(),
                    RustType::I8 | RustType::I16 | RustType::I32 | RustType::I64 | RustType::Isize
                ) =>
            {
                WitnessValue::Int(raw.parse::<i128>().ok()?)
            }
            // `marker_display_expr` prints `Duration` as `secs.nanos`
            // (nanos always 9 digits) precisely so this split is exact --
            // never `Duration`'s own SI-unit `Display` ("1.5s"), which a
            // decoder could not invert.
            RustType::Duration => {
                let (secs_str, nanos_str) = raw.split_once('.')?;
                let secs = secs_str.parse::<u64>().ok()?;
                let nanos = nanos_str.parse::<u32>().ok()?;
                WitnessValue::Duration(secs, nanos)
            }
            // The 2026-08-25 fragment widening: `char`, `Option`, `Result`
            // and `[T; N]` reach the engines, but `WitnessValue` has no way
            // to spell them as a literal, so a failure on one is reported
            // witness-only (`W0541`) rather than with an invented input.
            RustType::Char
            | RustType::Option(_)
            | RustType::Result(..)
            | RustType::Array(..)
            | RustType::Vec(_)
            | RustType::BTreeSet(_)
            | RustType::NonZero(_)
            // No `WitnessValue` variant for a float either -- rendering one
            // as a runnable Rust literal is real, separate work this task
            // did not take on (see `RustType::F32`/`F64`'s own doc comment
            // and the mechanism/floats-only scope note in fuzz_gen.rs). The
            // raw text `marker_display_expr` printed is still captured in
            // `fields` before this function ever runs, so the caller still
            // shows the real failing value -- just witness-only (`W0541`),
            // never a fabricated one.
            | RustType::F32
            | RustType::F64
            // Same reason as the float arms just above -- `String` is not
            // `is_witness_renderable` either (see that method's own doc),
            // so a failure on one is reported witness-only (`W0541`),
            // never with an invented Rust literal. The raw text is still
            // shown to the reader (via `fields`, populated by
            // `parse_fuzz_marker` below) -- already unescaped back to the
            // real string content by the time it gets there, never the
            // wire-escaped form `marker_display_expr` printed.
            | RustType::String
            // Never reached: both are return-only shapes, never a
            // parameter's, so no witness ever needs to decode one.
            | RustType::SelfType
            | RustType::Unit
            // Same reasoning as the float/`String` arms above: not
            // `is_witness_renderable`, so a struct/enum parameter's failing
            // value is reported witness-only (`W0541`) -- the raw
            // `marker_display_expr` text (a description built from the
            // constructor's/fields' own already-decodable arguments, never
            // a `{:?}` of the struct itself) is still shown to the reader
            // via `fields`, just not turned into a `WitnessValue` literal.
            | RustType::UserTypeCtor(_)
            | RustType::UserTypeFields(_)
            | RustType::Unsupported(_) => return None,
        };
        out.push(value);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eleventh false pass, pinned directly against the parser: a
    /// harness module that contributed zero tests (no worked examples, no
    /// direct-contract cases -- exactly a receiver method with only a
    /// `fuzz`/`test` promise) produces no `test <module>::... ok/FAILED`
    /// line at all, and this must read as zero, never as "ran, and none
    /// failed".
    #[test]
    fn counts_zero_when_no_test_line_for_the_module_exists_at_all() {
        let combined = "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\n";
        assert_eq!(count_tests_executed(combined, "Calc_value_harness::"), 0);
    }

    #[test]
    fn counts_one_passing_and_one_failing_line_for_the_named_module() {
        let combined = "\nrunning 2 tests\ntest clamp_harness::ply_fuzz_clamp ... ok\ntest clamp_harness::ply_direct_clamp_00 ... FAILED\n\nfailures:\n    clamp_harness::ply_direct_clamp_00\n\ntest result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n\n";
        assert_eq!(count_tests_executed(combined, "clamp_harness::"), 2);
    }

    /// The misattribution guard: cargo's own filter is a plain substring, so
    /// an invocation aimed at `parse` can still execute `util::parse`'s own
    /// tests too (`parse_harness::` is a substring of
    /// `util_parse_harness::`). Counting must not credit `parse`'s run with
    /// tests that only ever belonged to `util::parse`'s module.
    #[test]
    fn does_not_count_a_same_shaped_sibling_modules_tests() {
        let combined = "\nrunning 2 tests\ntest util_parse_harness::ply_direct_util_parse_00 ... FAILED\ntest util_parse_harness::ply_direct_util_parse_01 ... FAILED\n\nfailures:\n    util_parse_harness::ply_direct_util_parse_00\n    util_parse_harness::ply_direct_util_parse_01\n\ntest result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n\n";
        assert_eq!(count_tests_executed(combined, "parse_harness::"), 0);
        assert_eq!(count_tests_executed(combined, "util_parse_harness::"), 2);
    }

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
        assert_eq!(
            names,
            vec!["seeded_bug_harness::ply_fuzz_seeded_bug".to_string()]
        );
    }

    /// Real captured output from the `badexample` fixture's harness build
    /// (an `examples` entry comparing a `u32` to a string literal). The
    /// summary line cargo prints last names no cause, so the extractor must
    /// reach past it to the specific error.
    #[test]
    fn first_build_error_names_the_cause_not_cargos_summary_line() {
        let combined = "   Compiling ply-fixture-badexample-ply-harness v0.0.0 (/tmp/bx)\n\
             error[E0308]: mismatched types\n\
             \x20 --> target/ply/fuzz/x/src/lib.rs:65:38\n\
             \x20   |\n\
             65 |         assert!(add_small (0 , 0) == \"zero\");\n\
             \n\
             For more information about this error, try `rustc --explain E0308`.\n\
             error: could not compile `ply-fixture-badexample-ply-harness` (lib test) due to 1 previous error\n";
        assert_eq!(
            first_build_error(combined).unwrap(),
            "error[E0308]: mismatched types"
        );
    }

    #[test]
    fn first_build_error_is_none_when_nothing_failed_to_build() {
        assert!(first_build_error("running 1 test\ntest x ... ok\n").is_none());
    }

    /// The misattribution fix's whole point: two functions' generated
    /// modules are broken in the same build (`good_module` ends at line 5,
    /// `bad_module` starts at line 6), and each one's own error must land on
    /// its own fn -- never on the other, and never on both.
    #[test]
    fn attributes_two_distinct_errors_to_their_own_modules_not_each_other() {
        let combined = "\
             error[E0382]: borrow of moved value: `v`\n \
             --> target/ply/fuzz/pkg/src/lib.rs:8:38\n   |\n8 |     f(v); v.len()\n\n\
             error[E0308]: mismatched types\n \
             --> target/ply/fuzz/pkg/src/lib.rs:20:10\n   |\n20 |    g() == \"x\"\n";
        let errors = build_errors_with_lines(combined, "pkg/src/lib.rs");
        assert_eq!(errors.len(), 2, "{errors:?}");
        let spans = vec![
            ModuleSpan {
                fn_ident: "vector".into(),
                start_line: 3,
                end_line: 10,
            },
            ModuleSpan {
                fn_ident: "greet".into(),
                start_line: 15,
                end_line: 25,
            },
        ];
        let attributed = attribute_build_errors(&errors, &spans);
        assert_eq!(attributed.len(), 2, "{attributed:?}");
        assert!(attributed["vector"].contains("E0382"), "{attributed:?}");
        assert!(attributed["greet"].contains("E0308"), "{attributed:?}");
    }

    /// A build failure whose only error carries no `--> ` span at all (a
    /// linker failure, an ICE) must attribute to nothing -- inventing an
    /// attribution here would blame a function the compiler never actually
    /// named, which is the same dishonesty the misattribution fix exists to
    /// remove, just relocated.
    #[test]
    fn an_error_with_no_span_attributes_to_nothing_rather_than_guessing() {
        let combined =
            "error: linking with `cc` failed: exit status: 1\n  = note: some linker noise\n";
        let errors = build_errors_with_lines(combined, "pkg/src/lib.rs");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].line, None, "{errors:?}");
        let spans = vec![ModuleSpan {
            fn_ident: "vector".into(),
            start_line: 1,
            end_line: 50,
        }];
        let attributed = attribute_build_errors(&errors, &spans);
        assert!(
            attributed.is_empty(),
            "an unspanned error must never be pinned to a function that might be innocent: {attributed:?}"
        );
    }

    /// An error whose span points at some *other* file (the target crate's
    /// own source, not the generated harness) must not be attributed to any
    /// harness module either -- the path suffix check is the guard.
    #[test]
    fn an_error_in_a_different_file_is_not_attributed_to_a_harness_module() {
        let combined = "error[E0425]: cannot find value `y` in this scope\n --> src/lib.rs:3:5\n";
        let errors = build_errors_with_lines(combined, "pkg/src/lib.rs");
        assert_eq!(errors[0].line, None, "{errors:?}");
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
        let params = vec![Param {
            name: "x".into(),
            ty: RustType::U32,
            by_ref: false,
        }];
        let mut fields = BTreeMap::new();
        fields.insert("x".to_string(), "4294967295".to_string());
        let decoded = decode_marker_fields(&fields, &params).unwrap();
        assert_eq!(decoded, vec![WitnessValue::UInt(4294967295)]);
    }

    #[test]
    fn decodes_vec_u8_marker_field() {
        let params = vec![Param {
            name: "v".into(),
            ty: RustType::VecU8,
            by_ref: true,
        }];
        let mut fields = BTreeMap::new();
        fields.insert("v".to_string(), "[255,0,3]".to_string());
        let decoded = decode_marker_fields(&fields, &params).unwrap();
        assert_eq!(decoded, vec![WitnessValue::VecU8(vec![255, 0, 3])]);
    }

    #[test]
    fn decodes_a_nonzero_u32_marker_field_as_its_plain_number() {
        // `marker_display_expr`'s default arm prints a NonZero with its own
        // Display impl -- just the number, no wrapper syntax.
        let params = vec![Param {
            name: "n".into(),
            ty: RustType::NonZero(Box::new(RustType::U32)),
            by_ref: false,
        }];
        let mut fields = BTreeMap::new();
        fields.insert("n".to_string(), "7".to_string());
        let decoded = decode_marker_fields(&fields, &params).unwrap();
        assert_eq!(decoded, vec![WitnessValue::UInt(7)]);
    }

    #[test]
    fn decodes_a_duration_marker_field_from_its_secs_dot_nanos_spelling() {
        // `marker_display_expr` prints `Duration` as `secs.nanos` (nanos
        // always 9 digits), never the type's own SI-unit Display -- this is
        // the exact string that spelling produces, and the decoder must
        // split it back apart exactly.
        let params = vec![Param {
            name: "d".into(),
            ty: RustType::Duration,
            by_ref: false,
        }];
        let mut fields = BTreeMap::new();
        fields.insert("d".to_string(), "7.500000000".to_string());
        let decoded = decode_marker_fields(&fields, &params).unwrap();
        assert_eq!(decoded, vec![WitnessValue::Duration(7, 500_000_000)]);
    }

    #[test]
    fn refuses_to_decode_a_vec_of_non_u8_never_fabricating_a_value() {
        let params = vec![Param {
            name: "xs".into(),
            ty: RustType::Vec(Box::new(RustType::I32)),
            by_ref: true,
        }];
        let mut fields = BTreeMap::new();
        fields.insert("xs".to_string(), "[-1,2,3]".to_string());
        assert!(decode_marker_fields(&fields, &params).is_none());
    }

    /// Real shape of proptest 1.11's own report for a body that panics --
    /// the case Ply used to discard, reporting `X0901`/`tool_error` for a
    /// genuine crash bug (docs/review-post-004-strategy.md's correction to
    /// vetting 004's finding 4).
    #[test]
    fn recovers_proptests_own_shrunk_input_for_a_single_scalar() {
        let combined = "\nrunning 1 test\nthread 'halves_harness::ply_fuzz_halves' panicked at target/ply/fuzz/x/src/lib.rs:20:13:\nproptest found a failing case for `halves`: Test failed: halves() only accepts even numbers.\nminimal failing input: 1\nnote: run with `RUST_BACKTRACE=1` for a backtrace\ntest halves_harness::ply_fuzz_halves ... FAILED\n";
        assert_eq!(
            parse_proptest_minimal_input(combined),
            Some(vec!["1".to_string()])
        );
    }

    #[test]
    fn recovers_a_multi_parameter_shrunk_input_from_proptests_pretty_debug() {
        let combined = "proptest found a failing case for `f`: Test failed: boom.\nminimal failing input: (\n    3589630,\n    9568,\n)\nnote: run with `RUST_BACKTRACE=1`\n";
        assert_eq!(
            parse_proptest_minimal_input(combined),
            Some(vec!["3589630".to_string(), "9568".to_string()])
        );
    }

    #[test]
    fn recovers_a_vec_shaped_shrunk_input() {
        let combined = "minimal failing input: [\n    1,\n    2,\n]\nnote: x\n";
        assert_eq!(
            parse_proptest_minimal_input(combined),
            Some(vec!["[1,2]".to_string()])
        );
    }

    #[test]
    fn output_with_no_proptest_report_recovers_nothing_rather_than_inventing_it() {
        assert_eq!(parse_proptest_minimal_input("nothing here\n"), None);
    }

    #[test]
    fn parses_the_abort_marker_with_its_case_counts() {
        let combined = "noise\nPLY_FUZZ_ABORT|narrow_window|Too many global rejects|accepted=0|rejected=1024\nmore\n";
        assert_eq!(
            parse_abort_marker(combined).unwrap(),
            FuzzAbort {
                fn_name: "narrow_window".into(),
                reason: "Too many global rejects".into(),
                accepted: 0,
                rejected: 1024,
            }
        );
        assert!(parse_abort_marker("nothing here\n").is_none());
    }

    #[test]
    fn parses_high_reject_marker() {
        let combined = "noise\nPLY_FUZZ_HIGH_REJECT|safe_increment|12/20\nmore\n";
        let (fname, detail) = parse_high_reject_marker(combined).unwrap();
        assert_eq!(fname, "safe_increment");
        assert_eq!(detail, "12/20");
    }

    // -- the `String` marker wire-safety proof (task, 2026-08-27): a
    // sampled string containing the marker format's own separator
    // characters must not corrupt the field it belongs to, or any other
    // field on the same line. Mirrors record.rs's own smuggling-proof tests
    // this same session added for a different encoding, but for this
    // hand-rolled wire format instead.

    #[test]
    fn unescape_marker_value_reverses_every_escape_marker_display_expr_emits() {
        // Round-trip each individually-escaped character back to itself.
        assert_eq!(unescape_marker_value("a\\\\b"), "a\\b");
        assert_eq!(unescape_marker_value("a\\;b"), "a;b");
        assert_eq!(unescape_marker_value("a\\=b"), "a=b");
        assert_eq!(unescape_marker_value("a\\[b"), "a[b");
        assert_eq!(unescape_marker_value("a\\]b"), "a]b");
        assert_eq!(unescape_marker_value("a\\nb"), "a\nb");
        assert_eq!(unescape_marker_value("a\\rb"), "a\rb");
        // An unrecognised escape and a trailing lone backslash are kept
        // literally, never dropped -- no information silently vanishes.
        assert_eq!(unescape_marker_value("a\\qb"), "a\\qb");
        assert_eq!(unescape_marker_value("a\\"), "a\\");
    }

    /// The decisive adversarial case: a sampled string crafted to contain
    /// exactly what the wire format would read as a *second field's* own
    /// `name=value` pair -- the same attack shape record.rs's own
    /// `smuggling_a_field_boundary_inside_a_value_does_not_forge_a_collision`
    /// proved safe for a different encoding this same session. Here it must
    /// not corrupt a real later field (`x`), because `marker_display_expr`
    /// escapes the value's own `;`/`=` before it is ever printed.
    #[test]
    fn a_string_value_crafted_to_look_like_a_second_field_does_not_corrupt_a_later_one() {
        // What the harness would actually print for `s = "x;y=z"`, `x = 42`
        // (escaping applied exactly as `marker_display_expr` specifies).
        let combined = "PLY_FUZZED_CEX|f|s=x\\;y\\=z;x=42\n";
        let (fn_name, fields) = parse_fuzz_marker(combined).unwrap();
        assert_eq!(fn_name, "f");
        assert_eq!(
            fields.get("s").map(String::as_str),
            Some("x;y=z"),
            "the full crafted value must survive intact, not be truncated at its own embedded \
             separator: {fields:?}"
        );
        assert_eq!(
            fields.get("x").map(String::as_str),
            Some("42"),
            "a sibling field after the crafted string must decode to its real value, never a \
             fragment smuggled out of the string's own content: {fields:?}"
        );
    }

    /// The same attack, with the crafted field appearing *before* a
    /// same-named real field earlier in param order (the direction that
    /// would actually overwrite a correct decode, since a `BTreeMap` insert
    /// keeps the *last* write) -- still must not corrupt `x`.
    #[test]
    fn a_string_value_crafted_to_smuggle_a_same_named_field_does_not_overwrite_it() {
        let combined = "PLY_FUZZED_CEX|f|x=42;s=a\\;x=999\\;b\n";
        let (_, fields) = parse_fuzz_marker(combined).unwrap();
        assert_eq!(
            fields.get("x").map(String::as_str),
            Some("42"),
            "the real `x` field must never be overwritten by content smuggled inside `s`'s own \
             (escaped) value: {fields:?}"
        );
        assert_eq!(fields.get("s").map(String::as_str), Some("a;x=999;b"));
    }

    /// A raw, *unescaped* bracket inside a string value must not desync the
    /// bracket-depth tracking `split_top_level_semicolons` uses for
    /// `Vec`/`BTreeSet` fields -- `marker_display_expr` escapes `[`/`]` for
    /// exactly this reason. Simulates the encoded wire text directly
    /// (rather than running the generated harness) the same way the two
    /// tests above do.
    #[test]
    fn an_escaped_bracket_in_a_string_value_does_not_desync_collection_bracket_tracking() {
        let combined = "PLY_FUZZED_CEX|f|s=a\\[b;v=[1,2,3]\n";
        let (_, fields) = parse_fuzz_marker(combined).unwrap();
        assert_eq!(fields.get("s").map(String::as_str), Some("a[b"));
        assert_eq!(
            fields.get("v").map(String::as_str),
            Some("[1,2,3]"),
            "the real Vec field after the crafted string must still decode whole: {fields:?}"
        );
    }

    #[test]
    fn a_string_value_containing_only_ordinary_text_round_trips_unchanged() {
        let combined = "PLY_FUZZED_CEX|f|s=hello world\n";
        let (_, fields) = parse_fuzz_marker(combined).unwrap();
        assert_eq!(fields.get("s").map(String::as_str), Some("hello world"));
    }
}
