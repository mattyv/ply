//! Scaffolding for the generated harness crate under `target/ply/fuzz/`
//! (§5.4c) that carries the `fuzz`/`test` checks' generated tests and,
//! per `tests/spike/mutants/MUTANTS-FINDINGS.md`'s verified mechanism, is
//! what `mutate` names via `--test-package`.
//!
//! **Two placements, chosen per target crate, never by user request.** The
//! original mechanism (below, still exactly what happens when the target
//! crate already declares `[workspace]` itself) idempotently registers the
//! harness as a member of *that same* workspace, because `mutate`'s
//! `--test-package` genuinely needs it there (part 1 below). But that
//! registration requires a `[workspace]` table to add a member *to*, and
//! adding one to a crate that doesn't have it either does nothing (an
//! ordinary `cargo new --lib` crate) or actively breaks the enclosing
//! project (a crate that is already a member of someone else's workspace --
//! Cargo refuses "multiple workspace roots" the moment a second
//! `[workspace]` table appears inside the first one's member tree). Both
//! were reproduced against a real `cargo-ply verify` run
//! (docs/review-caveats.md N1) and are why this module never edits a
//! target crate's `Cargo.toml` unless that file already opted in by
//! carrying `[workspace]` itself.
//!
//! When it doesn't (`crate_has_workspace_table` is false -- an ordinary
//! crate, or a member of a bigger workspace with no `[workspace]` table of
//! its own), the harness crate instead gets **its own** `[workspace]`
//! table (`write_harness_cargo_toml`'s `standalone` flag) -- the exact
//! shape every M3/M4 fixture crate already uses for itself: `[workspace]`
//! with no `members` key makes a crate its own workspace root, stopping
//! Cargo's upward search for an enclosing one dead at that file, so it
//! never joins -- and never risks colliding with -- whatever workspace (if
//! any) contains the target crate. The target crate is reached purely as
//! an ordinary path dependency (`../../../..`, unchanged), which needs no
//! shared workspace at all. `fuzz`/`test` (`engines::fuzz::run_harness_tests`
//! et al.) are invoked with *that* directory as their own working directory
//! in this case (`ply-cli/src/verify.rs`), never the target crate's, so
//! `cargo test -p <harness>` resolves the harness against its own
//! single-crate workspace instead of failing to find it in the target's.
//! `mutate` cannot make the same move -- see its own module
//! (`engines::mutants`) for why -- so a crate in this shape earns an honest
//! "not supported for this crate's layout yet" the moment `mutate` is
//! declared, rather than a cargo-mutants error nobody could act on.
//!
//! The registered-member mechanism (only reached once `crate_has_workspace_table`
//! is true) has three load-bearing parts, all confirmed in the spike and
//! reproduced here as real codegen rather than a hand-written fixture:
//!
//! 1. The harness must be a *proper workspace member* -- `cargo metadata`
//!    (which both `cargo test -p` and `cargo mutants -p`/`--test-package`
//!    resolve against) only sees packages that are members of the same
//!    workspace as the target crate. Since every M3/M4 fixture is its own
//!    single-package workspace (`[workspace]` with no `members` key), this
//!    module idempotently adds `members = [".", "target/ply/fuzz/<name>"]`
//!    to the target crate's root `Cargo.toml`.
//! 2. The harness crate's own `Cargo.toml` depends on the target crate by
//!    *path*, using its actual `[lib] name` (the Rust identifier `use`
//!    needs), not necessarily its package name (they can differ by
//!    dashes/underscores).
//! 3. This placement -- one level inside the target crate's own top-level
//!    `target/` -- is exactly the one cargo-mutants prunes from the tree it
//!    copies, unconditionally and independently of `.gitignore`. Making it
//!    work is the mutate adapter's job (`engines::mutants`), which passes
//!    `--copy-target true` for it. It must **never** pass `--gitignore` as
//!    well: cargo-mutants' own CLI rejects the two together (they share a
//!    mutually exclusive argument group), and `--gitignore`'s default
//!    already matches what Ply wants. See `engines::mutants`' module doc for
//!    the full falsification (M4, docs/m4-findings.md finding 1).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// The proptest version requirement Ply writes into every harness crate it
/// generates. Named because it is also *recorded*: it is the fuzz tier's
/// engine version in a result's fingerprint (§5.2a), and the two must be the
/// same string or the record would guard a version that was never used.
pub const PROPTEST_REQUIREMENT: &str = "1";

/// The two names a dependent crate is known by: the Cargo package name
/// (used as the `[dependencies]` key) and the Rust crate identifier its
/// `[lib] name` gives `use` statements (falls back to the package name with
/// `-` replaced by `_`, matching Cargo's own default when `[lib] name` is
/// unset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateNames {
    pub package_name: String,
    pub lib_ident: String,
}

/// Reads a crate's package name and lib identifier out of its `Cargo.toml`
/// text via plain line scanning (not a full TOML parser -- deliberately
/// narrow, same convention as `harness::tidy_contract_text`: good enough for
/// the exact fixture shape this tool generates and edits, not a general
/// Cargo.toml reader).
pub fn read_crate_names(cargo_toml_text: &str) -> Result<CrateNames> {
    let package_name = find_key_after_section(cargo_toml_text, "[package]", "name")
        .context("Cargo.toml has no `[package]` name = \"...\" line")?;
    let lib_ident = find_key_after_section(cargo_toml_text, "[lib]", "name")
        .unwrap_or_else(|| package_name.replace('-', "_"));
    Ok(CrateNames {
        package_name,
        lib_ident,
    })
}

/// Finds `key = "value"` on a line after the given `[section]` header (and
/// before the next `[` header line), returning `value`.
fn find_key_after_section(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == section {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with('[') {
            break;
        }
        if in_section && let Some(rest) = trimmed.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let v = rest.trim();
                let v = v.trim_matches('"');
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Whether the target crate's own `Cargo.toml` already declares a
/// `[workspace]` table -- the fact this whole module now branches on
/// (module doc above). `find_key_after_section`'s line-scan convention:
/// deliberately not a full TOML parser, matching every other reader in this
/// file, and matching exactly what `ensure_workspace_member` itself already
/// looks for.
pub fn crate_has_workspace_table(cargo_toml_text: &str) -> bool {
    cargo_toml_text.lines().any(|l| l.trim() == "[workspace]")
}

/// The harness crate's package name for a given target package name --
/// deterministic so re-running `verify` finds the same crate every time.
pub fn harness_package_name(target_package_name: &str) -> String {
    format!("{target_package_name}-ply-harness")
}

/// Where the harness crate lives, relative to the target crate's root
/// (§5.4c: `target/ply/fuzz/`).
pub fn harness_rel_path(target_package_name: &str) -> String {
    format!(
        "target/ply/fuzz/{}",
        harness_package_name(target_package_name)
    )
}

/// Idempotently ensures `crate_dir`'s root `Cargo.toml` lists the harness
/// crate as a workspace member -- the load-bearing fact the mutants spike
/// found (`MUTANTS-FINDINGS.md` item 3): `cargo mutants -p X --test-package
/// Y` only resolves `Y` if it is a member of the same workspace `cargo
/// metadata` sees at the invocation directory.
pub fn ensure_workspace_member(crate_cargo_toml: &Path, harness_rel: &str) -> Result<()> {
    let text = std::fs::read_to_string(crate_cargo_toml)
        .with_context(|| format!("reading {}", crate_cargo_toml.display()))?;

    let Some(ws_line_idx) = text.lines().position(|l| l.trim() == "[workspace]") else {
        bail!(
            "{} has no `[workspace]` table to add the harness crate to",
            crate_cargo_toml.display()
        );
    };

    // Does a `members = [...]` line already exist in the [workspace]
    // section (before the next `[section]` header)?
    let lines: Vec<&str> = text.lines().collect();
    let mut members_line_idx = None;
    for (i, line) in lines.iter().enumerate().skip(ws_line_idx + 1) {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            break;
        }
        if trimmed.starts_with("members") {
            members_line_idx = Some(i);
            break;
        }
    }

    let quoted = format!("\"{harness_rel}\"");
    let new_text = match members_line_idx {
        Some(idx) => {
            if lines[idx].contains(&quoted) {
                return Ok(()); // already registered
            }
            let mut updated_lines = lines.clone();
            let with_new_member = insert_before_closing_bracket(lines[idx], &quoted);
            updated_lines[idx] = &with_new_member;
            // updated_lines borrows `with_new_member`'s lifetime, so join now.
            let mut out = updated_lines.join("\n");
            if text.ends_with('\n') {
                out.push('\n');
            }
            out
        }
        None => {
            let mut out = String::new();
            for (i, line) in lines.iter().enumerate() {
                out.push_str(line);
                out.push('\n');
                if i == ws_line_idx {
                    out.push_str(&format!("members = [\".\", {quoted}]\n"));
                }
            }
            out
        }
    };

    std::fs::write(crate_cargo_toml, new_text)
        .with_context(|| format!("writing {}", crate_cargo_toml.display()))?;
    Ok(())
}

fn insert_before_closing_bracket(line: &str, new_item: &str) -> String {
    match line.rfind(']') {
        Some(pos) => {
            let (before, after) = line.split_at(pos);
            if before.trim_end().ends_with('[') {
                format!("{before}{new_item}{after}")
            } else {
                format!("{before}, {new_item}{after}")
            }
        }
        None => line.to_string(),
    }
}

/// Writes the harness crate's own `Cargo.toml` (idempotent -- always
/// regenerated, since it is entirely Ply-owned, `target/ply/` housekeeping,
/// §6). `target_names` is the crate under test; the `../` steps back to the
/// target crate's root are fixed by the generated layout
/// (`target/ply/fuzz/<name>/`, four levels down) and written inline below,
/// not passed in.
///
/// `standalone` (module doc above): when true, the harness crate carries
/// its **own** `[workspace]` table, exactly the shape every M3/M4 fixture
/// already uses for itself -- a crate that is its own workspace root needs
/// no membership in, and never touches, whatever workspace (if any)
/// contains the target crate it depends on by path. When false (the target
/// crate already declared its own `[workspace]`), the harness carries no
/// `[workspace]` table of its own, unchanged from before this module grew
/// the standalone path: `ensure_workspace_member` registers it as a member
/// of the target's existing workspace instead.
pub fn write_harness_cargo_toml(
    harness_dir: &Path,
    harness_package: &str,
    target_names: &CrateNames,
    standalone: bool,
) -> Result<()> {
    std::fs::create_dir_all(harness_dir)
        .with_context(|| format!("creating {}", harness_dir.display()))?;
    let workspace_table = if standalone {
        "[workspace]\n# Empty table: this generated harness is its own workspace root, so it\n\
         # never needs to join -- or risk colliding with -- whatever workspace (if\n\
         # any) contains the target crate it depends on by path below.\n\n"
    } else {
        ""
    };
    let toml = format!(
        "# Generated by Ply -- do not edit. The M4 fuzz/test/mutate harness\n\
         # crate for `{target}` (The-Ply-Spec.md §5.4c). Ply regenerates this\n\
         # file on every `verify` run.\n\
         {workspace_table}\
         [package]\n\
         name = \"{harness_package}\"\n\
         version = \"0.0.0\"\n\
         edition = \"2021\"\n\
         publish = false\n\n\
         [dev-dependencies]\n\
         {target_pkg} = {{ path = \"../../../..\" }}\n\
         proptest = \"{proptest}\"\n",
        target = target_names.package_name,
        target_pkg = target_names.package_name,
        proptest = PROPTEST_REQUIREMENT,
    );
    std::fs::write(harness_dir.join("Cargo.toml"), toml)
        .with_context(|| format!("writing {}/Cargo.toml", harness_dir.display()))?;
    Ok(())
}

/// One generated per-function harness module, tagged with the identity a
/// build failure's line span must be attributed back to (the misattribution
/// fix: one broken function's compile error no longer blames its
/// crate-mates, all of whom share this one generated file). `fn_ident` must
/// be `ContractFn::ident()` -- the same identifier `fuzz_gen::wrap_fn_harness_module`
/// names its `{ident}_harness` module after and `engines::fuzz`'s per-fn
/// test filter matches against.
pub struct HarnessModule {
    pub fn_ident: String,
    pub source: String,
}

/// Where one function's generated module landed in the harness crate's
/// `src/lib.rs`, in 1-indexed line numbers -- the same numbering rustc's own
/// `--> path:LINE:COL` spans use, so a compiler error can be mapped straight
/// back to the function whose generated code it appeared inside
/// (`engines::fuzz::attribute_build_errors`).
#[derive(Debug, Clone)]
pub struct ModuleSpan {
    pub fn_ident: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Writes the harness crate's `src/lib.rs`: an empty (non-test) public
/// surface plus every generated `#[cfg(test)] mod {fn}_harness { ... }`
/// block, concatenated. Regenerated wholesale on every run (Ply owns this
/// file entirely -- unlike the in-crate `ply_generated*.rs` files, there is
/// no user code anywhere in this crate to preserve).
///
/// Returns each module's own line range alongside the file path: the
/// misattribution fix needs to know exactly which lines belong to which
/// function before it can read anything into a compiler error's span.
pub fn write_harness_lib_rs(
    harness_dir: &Path,
    fn_modules: &[HarnessModule],
) -> Result<(PathBuf, Vec<ModuleSpan>)> {
    let src_dir = harness_dir.join("src");
    std::fs::create_dir_all(&src_dir).with_context(|| format!("creating {}", src_dir.display()))?;
    let header = "//! Generated by Ply -- do not edit. Fuzz/test/mutate harness (The-Ply-Spec.md §5.4c).\n\n";
    let mut out = String::from(header);
    // Every module source ends in its own trailing `\n` (`fuzz_gen::wrap_fn_harness_module`),
    // so `matches('\n').count()` is exactly its line count -- no off-by-one
    // from a missing final newline to guard against.
    let mut line = header.matches('\n').count() + 1;
    let mut spans = Vec::with_capacity(fn_modules.len());
    for m in fn_modules {
        let start_line = line;
        let src_lines = m.source.matches('\n').count();
        let end_line = start_line + src_lines.saturating_sub(1);
        out.push_str(&m.source);
        out.push('\n'); // blank separator line between modules
        line = end_line + 2;
        spans.push(ModuleSpan {
            fn_ident: m.fn_ident.clone(),
            start_line,
            end_line,
        });
    }
    let path = src_dir.join("lib.rs");
    std::fs::write(&path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok((path, spans))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_package_and_lib_names() {
        let toml = r#"
[workspace]

[package]
name = "ply-fixture-clamp"
version = "0.0.0"

[lib]
name = "ply_fixture_clamp"
path = "src/lib.rs"
"#;
        let names = read_crate_names(toml).unwrap();
        assert_eq!(names.package_name, "ply-fixture-clamp");
        assert_eq!(names.lib_ident, "ply_fixture_clamp");
    }

    #[test]
    fn falls_back_to_dashes_to_underscores_when_no_lib_section() {
        let toml = "[package]\nname = \"my-crate\"\n";
        let names = read_crate_names(toml).unwrap();
        assert_eq!(names.lib_ident, "my_crate");
    }

    #[test]
    fn crate_has_workspace_table_detects_presence_and_absence() {
        assert!(crate_has_workspace_table(
            "[workspace]\n\n[package]\nname = \"x\"\n"
        ));
        assert!(!crate_has_workspace_table(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n"
        ));
        // A member crate of someone else's workspace carries no
        // `[workspace]` table of its own -- only the root does.
        assert!(!crate_has_workspace_table(
            "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\n"
        ));
    }

    #[test]
    fn write_harness_cargo_toml_standalone_gets_its_own_workspace_table() {
        let dir = tempfile::tempdir().unwrap();
        let harness_dir = dir.path().join("harness");
        let target_names = CrateNames {
            package_name: "plain".to_string(),
            lib_ident: "plain".to_string(),
        };
        write_harness_cargo_toml(&harness_dir, "plain-ply-harness", &target_names, true).unwrap();
        let text = std::fs::read_to_string(harness_dir.join("Cargo.toml")).unwrap();
        assert!(
            text.lines().any(|l| l.trim() == "[workspace]"),
            "standalone harness must be its own workspace root:\n{text}"
        );
        assert!(text.contains("path = \"../../../..\""));
    }

    #[test]
    fn write_harness_cargo_toml_non_standalone_has_no_workspace_table_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let harness_dir = dir.path().join("harness");
        let target_names = CrateNames {
            package_name: "plain".to_string(),
            lib_ident: "plain".to_string(),
        };
        write_harness_cargo_toml(&harness_dir, "plain-ply-harness", &target_names, false).unwrap();
        let text = std::fs::read_to_string(harness_dir.join("Cargo.toml")).unwrap();
        assert!(
            !text.lines().any(|l| l.trim() == "[workspace]"),
            "non-standalone harness must rely on being registered into the \
             target's own workspace instead:\n{text}"
        );
    }

    #[test]
    fn ensure_workspace_member_inserts_into_empty_workspace_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\n# comment\n\n[package]\nname = \"x\"\n").unwrap();
        ensure_workspace_member(&path, "target/ply/fuzz/x-ply-harness").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("members = [\".\", \"target/ply/fuzz/x-ply-harness\"]"),
            "{text}"
        );
        assert!(
            text.contains("[package]"),
            "must not disturb the rest of the file:\n{text}"
        );
    }

    #[test]
    fn ensure_workspace_member_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\n\n[package]\nname = \"x\"\n").unwrap();
        ensure_workspace_member(&path, "target/ply/fuzz/x-ply-harness").unwrap();
        let after_first = std::fs::read_to_string(&path).unwrap();
        ensure_workspace_member(&path, "target/ply/fuzz/x-ply-harness").unwrap();
        let after_second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after_first, after_second,
            "must not duplicate the member entry on rerun"
        );
    }

    #[test]
    fn ensure_workspace_member_appends_to_existing_members_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(
            &path,
            "[workspace]\nmembers = [\".\"]\n\n[package]\nname = \"x\"\n",
        )
        .unwrap();
        ensure_workspace_member(&path, "target/ply/fuzz/x-ply-harness").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("members = [\".\", \"target/ply/fuzz/x-ply-harness\"]"),
            "{text}"
        );
    }

    /// The misattribution fix's whole foundation: a `ModuleSpan`'s line
    /// range must point at *exactly* the lines the file actually holds for
    /// that module -- one off, and a compiler error on the line right
    /// before or after a break gets attributed to the wrong function. Two
    /// modules of different lengths, checked against the real written file
    /// by 1-indexed line number (rustc's own numbering).
    #[test]
    fn module_spans_point_at_the_exact_lines_each_module_occupies() {
        let dir = tempfile::tempdir().unwrap();
        let harness_dir = dir.path();
        let modules = vec![
            HarnessModule {
                fn_ident: "short_fn".to_string(),
                source: "#[cfg(test)]\nmod short_fn_harness {\n    // one body line\n}\n"
                    .to_string(),
            },
            HarnessModule {
                fn_ident: "long_fn".to_string(),
                source: "#[cfg(test)]\nmod long_fn_harness {\n    // line a\n    // line b\n    // line c\n}\n"
                    .to_string(),
            },
        ];
        let (path, spans) = write_harness_lib_rs(harness_dir, &modules).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(spans.len(), 2, "{spans:?}");
        let short = spans.iter().find(|s| s.fn_ident == "short_fn").unwrap();
        let long = spans.iter().find(|s| s.fn_ident == "long_fn").unwrap();

        // 1-indexed, matching rustc: line[start_line - 1] is the module's
        // own opening line, line[end_line - 1] is its own closing brace.
        assert_eq!(
            lines[short.start_line - 1],
            "#[cfg(test)]",
            "wrong start line for short_fn:\n{text}"
        );
        assert_eq!(
            lines[short.end_line - 1],
            "}",
            "wrong end line for short_fn:\n{text}"
        );
        assert_eq!(
            lines[long.start_line - 1],
            "#[cfg(test)]",
            "wrong start line for long_fn:\n{text}"
        );
        assert_eq!(
            lines[long.end_line - 1],
            "}",
            "wrong end line for long_fn:\n{text}"
        );
        assert!(
            long.start_line > short.end_line,
            "long_fn must start after short_fn ends, with no overlap: {spans:?}"
        );
    }
}
