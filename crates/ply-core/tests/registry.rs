//! The registry's two invariant tests (`docs/rule-registry-design.md`,
//! `crates/ply-core/src/registry.rs`), in the style of
//! `every_painted_element_resolves_a_style_rule`: both walk the *real*
//! source under `crates/*/src`, not a second hand-maintained list, so a
//! defect one level up (two lists checked against each other, both wrong
//! the same way) cannot hide here the way it hid before this table existed.
//!
//! **How an emitting site is found.** A diagnostic code only ever reaches a
//! user by one of two syntactic routes in this codebase (verified by
//! reading every call site while building the table): as the first string
//! argument to a helper whose name ends in `diag`/`violation` -- the bare
//! `diag(...)`/`violation(...)` of `ply-core/src/check.rs` and
//! `ply-core/src/schema.rs`, and named variants such as
//! `state_diag_warning(...)` that take the code as a parameter -- or as the
//! value of a struct field literally named `code` (`Diagnostic { code:
//! "...", .. }`, `ArchFinding { code: "...", .. }`). The scanner below
//! looks for exactly those two shapes and nothing else.
//!
//! The helper-name half of that was widened on 2026-09-03, by this test
//! going red rather than by review: `state:`'s three codes are emitted
//! through helpers that take the code as an argument (so the literal sits
//! at the call site, not in a `code:` field), and the previous regex
//! required `diag(` to start a word. Test 2 caught all three as falsely
//! enforced. The prediction two bullets down -- that a third route should
//! extend the regexes here -- is what actually happened.
//!
//! **What could fool it, honestly stated:**
//! - A *new* emitting shape (a helper named nothing like `diag`, a field
//!   renamed away from `code`) would not be found by this scanner, so a
//!   genuinely new diagnostic introduced that way would silently escape
//!   test 1 rather than fail it. The two shapes above are the only ones in
//!   the tree today; a reviewer adding a third route should extend the
//!   regexes here, not treat their silent passing as license to skip it.
//! - Code inside a `#[cfg(test)]` module is stripped by brace-counting
//!   before the scan runs, so a code used only as a test fixture's
//!   placeholder value (this tree has two real examples: `"E0000"` and
//!   `"F0502"`, neither a real Ply code, both only ever passed to a test
//!   helper inside `#[cfg(test)]`) is correctly never counted as emitted.
//!   The brace counter does not understand string or char literals, so a
//!   literal `'{'` or `'}'` inside a `#[cfg(test)]` module could throw the
//!   count off; none of this tree's test modules contain one.
//! - A code quoted inside a comment or a prose string (this tree quotes
//!   rustc's own `E0308`/`E0382`/`E0252`/`E0424` inside messages that
//!   explain *why* generated code would fail to compile) is not matched,
//!   because neither regex fires on plain prose -- only on the two real
//!   construction shapes above.

use regex::Regex;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    // `crates/ply-core` -> `crates` -> the workspace root (same convention
    // `docs_consistency.rs` and `ply-cli/build.rs` already use).
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ply-core sits two directories below the workspace root")
        .to_path_buf()
}

/// Every `.rs` file under `crates/*/src`, recursively. Deliberately not a
/// hand-picked list of "the files that currently emit diagnostics" -- a new
/// source file under any crate's `src/` is picked up automatically, so a
/// diagnostic added in a file nobody thought to add to a list is still
/// found.
fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates_dir = root.join("crates");
    for crate_entry in std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", crates_dir.display()))
    {
        let crate_entry = crate_entry.expect("dir entry");
        let src = crate_entry.path().join("src");
        if src.is_dir() {
            walk_rs(&src, &mut out);
        }
    }
    out
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
    {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Strips every `#[cfg(test)] ... { ... }` module out of `src`, by brace
/// counting from the `{` that follows the attribute to its matching `}`.
/// Approximate (it does not understand string/char literals, so a literal
/// brace inside one would throw the count off -- see the module doc's
/// honesty note), exact enough for this tree today.
fn strip_cfg_test_modules(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after_attr = &rest[at..];
        let Some(brace_offset) = after_attr.find('{') else {
            // No `{` at all after the attribute in the rest of the file --
            // nothing left to strip; keep the remainder as-is.
            out.push_str(after_attr);
            rest = "";
            break;
        };
        let body_start = brace_offset + 1;
        let mut depth: i32 = 1;
        let mut end = None;
        for (i, ch) in after_attr[body_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(body_start + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.unwrap_or(after_attr.len());
        rest = &after_attr[end..];
    }
    out.push_str(rest);
    out
}

/// Drops `//`-led line comments (including `///` doc comments), which is
/// where the rustc-code false positives this task warns about
/// (`E0308`/`E0382`/`E0252`/`E0424`) and every prose mention of a real Ply
/// code live. Does not attempt `/* */` block comments -- this tree has
/// none containing a code-shaped string, verified by the resulting counts
/// matching a manual read of every emitting file.
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|line| match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every code with a real construction site under `crates/*/src`, found by
/// walking the actual source rather than checked against a second
/// hand-maintained list. See the module doc for the two shapes matched and
/// what could fool this.
fn emitted_codes() -> BTreeSet<String> {
    let field_re = Regex::new(r#"\bcode:\s*"([A-Z][0-9]{4})""#).unwrap();
    // No leading `\b`: the helper may be a named variant (`state_diag`,
    // `state_diag_warning`) whose code arrives as an argument rather than
    // in a `code:` field, and `_diag(` has no word boundary before `diag`.
    let call_re = Regex::new(r#"(?:diag|violation)\w*\(\s*"([A-Z][0-9]{4})""#).unwrap();

    let root = workspace_root();
    let mut out = BTreeSet::new();
    for path in source_files(&root) {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let text = strip_cfg_test_modules(&text);
        let text = strip_line_comments(&text);
        for re in [&field_re, &call_re] {
            for cap in re.captures_iter(&text) {
                out.insert(cap[1].to_string());
            }
        }
    }
    out
}

/// Test 1: every code the source actually emits has a row in the registry.
/// A new diagnostic introduced without a matching `Code` variant (and
/// `entry()` arm) fails here, naming the code, before it can ship with no
/// gloss anywhere.
#[test]
fn every_emitted_code_has_a_registry_row() {
    let registry_codes: BTreeSet<String> = ply_core::registry::Code::ALL
        .iter()
        .map(|c| format!("{c:?}"))
        .collect();

    let missing: Vec<String> = emitted_codes()
        .into_iter()
        .filter(|c| !registry_codes.contains(c))
        .collect();

    assert!(
        missing.is_empty(),
        "the source emits {missing:?} but crates/ply-core/src/registry.rs has no row for \
         {}; add a Code variant and an entry() arm before this can ship",
        if missing.len() == 1 { "it" } else { "them" }
    );
}

/// Test 2: every row the registry marks `Enforced` has a real emitting site
/// today. A row that stops being backed by source -- the site was deleted,
/// renamed, or never existed -- fails here, naming the code, rather than
/// silently keeping a status that used to be true.
#[test]
fn every_enforced_row_has_a_real_emitting_site() {
    use ply_core::registry::Status;

    let emitted = emitted_codes();

    let falsely_enforced: Vec<String> = ply_core::registry::Code::ALL
        .iter()
        .map(|c| c.entry())
        .filter(|e| e.status == Status::Enforced)
        .map(|e| format!("{:?}", e.code))
        .filter(|code| !emitted.contains(code))
        .collect();

    assert!(
        falsely_enforced.is_empty(),
        "{falsely_enforced:?} {} marked Status::Enforced in the registry, but no site under \
         crates/*/src emits {}; change the row to Status::DeclaredOnly, or restore the site \
         that used to emit it",
        if falsely_enforced.len() == 1 {
            "is"
        } else {
            "are"
        },
        if falsely_enforced.len() == 1 {
            "it"
        } else {
            "them"
        }
    );
}

/// A sanity check on the scanner itself, so a change to the regexes or the
/// comment/test stripping that quietly stopped matching *anything* would
/// be caught here rather than manifesting as both real tests trivially
/// passing on an empty set.
#[test]
fn the_scanner_actually_finds_a_known_real_code() {
    let emitted = emitted_codes();
    assert!(
        emitted.contains("E0204"),
        "the scanner found {} codes but missed E0204, which crates/ply-core/src/schema.rs \
         emits via violation(\"E0204\", ..) -- the scanner itself is broken",
        emitted.len()
    );
    assert!(
        !emitted.contains("E0308"),
        "the scanner matched E0308, a rustc compiler code Ply quotes inside its own \
         messages, never a Ply diagnostic code of its own -- the comment/prose filtering \
         has a hole"
    );
}
