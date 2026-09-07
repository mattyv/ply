//! Scaffolding for the generated harness crate under `target/ply/fuzz/`
//! (§5.4c) that carries the `fuzz`/`test` checks' generated tests and,
//! per `tests/spike/mutants/MUTANTS-FINDINGS.md`'s verified mechanism, is
//! what `mutate` names via `--test-package`.
//!
//! **Two placements, chosen from Cargo's real workspace, never by user
//! request.** `cargo_workspace_root` reads `cargo metadata`; this is
//! load-bearing because a member of a virtual workspace correctly carries
//! no `[workspace]` table in its own manifest. When Cargo reports either a
//! virtual root above the target or an explicit workspace in the target
//! package itself, Ply temporarily registers the harness in that real
//! root. `mutate`'s `--test-package` genuinely needs both packages in that
//! one graph.
//!
//! An ordinary package with no explicit `[workspace]` keeps the harness in
//! its own isolated workspace (`write_harness_cargo_toml`'s `standalone`
//! flag). Ply does not add a new workspace declaration to user code merely
//! to run mutation testing, because that may change Cargo's package
//! discovery. The target remains an ordinary path dependency and
//! `fuzz`/`test` run from the harness directory. `mutate` cannot make the
//! same move, so only this plain-package shape gets the named unsupported
//! result.
//!
//! The registered-member mechanism has three load-bearing parts, all confirmed in the spike and
//! reproduced here as real codegen rather than a hand-written fixture:
//!
//! 1. The harness must be a *proper workspace member* -- `cargo metadata`
//!    (which both `cargo test -p` and `cargo mutants -p`/`--test-package`
//!    resolve against) only sees packages that are members of the same
//!    workspace as the target crate. Since every M3/M4 fixture is its own
//!    single-package workspace (`[workspace]` with no `members` key), this
//!    module idempotently adds the harness path to the actual workspace
//!    root's `members` list. For a virtual workspace member, that path
//!    includes the member directory before `target/ply/fuzz/<name>`.
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

/// The workspace Cargo itself assigns to `crate_dir`.
///
/// Reading this from `cargo metadata` matters for member crates: their own
/// manifest correctly has no `[workspace]` table, while the virtual root
/// above them does. Mutation testing needs that real shared root so both
/// the target package and generated harness resolve in one package graph.
pub fn cargo_workspace_root(crate_dir: &Path) -> Result<PathBuf> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version=1", "--no-deps"])
        .current_dir(crate_dir)
        .output()
        .with_context(|| format!("spawning `cargo metadata` in {}", crate_dir.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`cargo metadata` could not resolve the workspace containing {} (status {}): {}",
            crate_dir.display(),
            output.status,
            stderr.trim()
        );
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("reading `cargo metadata` output while locating the workspace root")?;
    let root = metadata
        .get("workspace_root")
        .and_then(serde_json::Value::as_str)
        .context("`cargo metadata` output had no string `workspace_root`")?;
    PathBuf::from(root)
        .canonicalize()
        .with_context(|| format!("resolving Cargo workspace root {root}"))
}

/// Where the generated harness participates while verification runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessWorkspacePlan {
    pub workspace_root: PathBuf,
    pub workspace_manifest: PathBuf,
    pub harness_member_path: String,
    /// A plain package with no explicit workspace keeps the harness in its
    /// own isolated workspace. An explicit package workspace and a member
    /// of a virtual workspace both set this to false.
    pub standalone: bool,
}

/// Plans harness membership from Cargo's real workspace, not from whether
/// the target package's own manifest happens to contain `[workspace]`.
pub fn harness_workspace_plan(
    crate_dir: &Path,
    harness_dir: &Path,
) -> Result<HarnessWorkspacePlan> {
    let crate_root = crate_dir
        .canonicalize()
        .with_context(|| format!("resolving target crate directory {}", crate_dir.display()))?;
    let workspace_root = cargo_workspace_root(&crate_root)?;
    let target_manifest = std::fs::read_to_string(crate_root.join("Cargo.toml"))
        .with_context(|| format!("reading {}/Cargo.toml", crate_root.display()))?;
    let standalone = workspace_root == crate_root && !crate_has_workspace_table(&target_manifest);

    let harness_abs = if let Ok(suffix) = harness_dir.strip_prefix(crate_dir) {
        crate_root.join(suffix)
    } else if harness_dir.is_absolute() {
        harness_dir.to_path_buf()
    } else {
        crate_root.join(harness_dir)
    };
    let member = harness_abs.strip_prefix(&workspace_root).with_context(|| {
        format!(
            "generated harness {} does not sit under Cargo workspace root {}",
            harness_abs.display(),
            workspace_root.display()
        )
    })?;
    let harness_member_path = member.to_string_lossy().replace('\\', "/");

    Ok(HarnessWorkspacePlan {
        workspace_manifest: workspace_root.join("Cargo.toml"),
        workspace_root,
        harness_member_path,
        standalone,
    })
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

/// The harness registration above, made temporary.
///
/// Registering the harness edits a file the user owns. Before this guard
/// existed, that edit stayed on disk after the run: someone who ran Ply
/// once on their own workspace was left with a `members` entry pointing
/// into `target/` that they never wrote and would have to notice in `git
/// status` to remove. Ply reports on code; it does not leave changes in it.
///
/// So the registration lives exactly as long as the run that needs it. The
/// guard captures the manifest as it was, adds the member, and on drop --
/// including on the `?` of a failed engine run -- writes the original back
/// with the harness entry gone.
///
/// Two things it deliberately does *not* do. It never restores over a file
/// that changed since it wrote: if the bytes on disk are not the ones the
/// guard left, someone else owns that file now and the guard steps away
/// rather than overwriting an edit it cannot see. And its restore target is
/// the original *minus* the harness entry, so a stale entry left by an
/// earlier crashed run gets cleaned up too -- nobody hand-writes a member
/// path under `target/ply/fuzz/`.
pub struct ManifestRegistration {
    path: PathBuf,
    /// What the guard wrote, so drop can tell "unchanged" from "someone
    /// else edited this".
    written: String,
    /// What drop puts back.
    restore_to: String,
    /// Where the harness crate lives, so drop can leave it standing on its
    /// own once it stops being a member (see [`Self::register`]).
    harness_dir: PathBuf,
    harness_package: String,
    target_names: CrateNames,
}

impl ManifestRegistration {
    /// Adds the harness as a workspace member for the lifetime of the
    /// returned guard.
    ///
    /// Dropping the guard has to do two things, not one. Taking the harness
    /// out of the `members` list without anything else would orphan it: a
    /// crate that is neither a workspace of its own nor a member of one
    /// cannot be built at all, and the failing test Ply just generated
    /// would be unrunnable. So drop also rewrites the harness's own
    /// manifest into the standalone shape, the same one a crate with no
    /// `[workspace]` of its own gets from the start. The result is that
    /// `cargo test` in `target/ply/fuzz/<name>/` works after the run either
    /// way, and the user's manifest is untouched either way.
    pub fn register(
        crate_cargo_toml: &Path,
        harness_rel: &str,
        harness_dir: &Path,
        harness_package: &str,
        target_names: &CrateNames,
    ) -> Result<Self> {
        let original = std::fs::read_to_string(crate_cargo_toml)
            .with_context(|| format!("reading {}", crate_cargo_toml.display()))?;
        ensure_workspace_member(crate_cargo_toml, harness_rel)?;
        let written = std::fs::read_to_string(crate_cargo_toml)
            .with_context(|| format!("reading {}", crate_cargo_toml.display()))?;
        Ok(Self {
            path: crate_cargo_toml.to_path_buf(),
            written,
            restore_to: remove_workspace_member(&original, harness_rel),
            harness_dir: harness_dir.to_path_buf(),
            harness_package: harness_package.to_string(),
            target_names: target_names.clone(),
        })
    }
}

impl Drop for ManifestRegistration {
    fn drop(&mut self) {
        // Unreadable, or changed under us: not ours to put back.
        let Ok(now) = std::fs::read_to_string(&self.path) else {
            return;
        };
        if now != self.written {
            return;
        }
        // Standing the harness up on its own happens only once the
        // membership is actually gone: a manifest Ply could not rewrite
        // means the harness is still a member, and a member may not declare
        // `[workspace]` of its own.
        if std::fs::write(&self.path, &self.restore_to).is_err() {
            return;
        }
        let _ = write_harness_cargo_toml(
            &self.harness_dir,
            &self.harness_package,
            &self.target_names,
            true,
        );
    }
}

/// The inverse of [`ensure_workspace_member`]: `text` with `harness_rel`
/// gone from the `[workspace]` `members` list. A `members` line that held
/// nothing but the harness and `"."` is left as `members = ["."]` rather
/// than deleted, because deleting it would change which packages the
/// workspace contains -- `members` absent means "discover them", which is
/// not what was there before.
pub fn remove_workspace_member(text: &str, harness_rel: &str) -> String {
    let quoted = format!("\"{harness_rel}\"");
    if !text.contains(&quoted) {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let Some(ws_line_idx) = lines.iter().position(|l| l.trim() == "[workspace]") else {
        return text.to_string();
    };
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());
    // Was the `members` line one Ply itself inserted (`members = [".",
    // "<harness>"]`, written by the `None` arm above)? Then the whole line
    // goes; anything else keeps the line and loses one item.
    let ply_inserted = format!("members = [\".\", {quoted}]");
    let mut in_workspace = false;
    let mut done = false;
    for (i, line) in lines.iter().enumerate() {
        if i == ws_line_idx {
            in_workspace = true;
            out_lines.push((*line).to_string());
            continue;
        }
        if in_workspace && line.trim().starts_with('[') {
            in_workspace = false;
        }
        if in_workspace && !done && line.trim().starts_with("members") && line.contains(&quoted) {
            done = true;
            if line.trim() == ply_inserted {
                continue; // the line existed only to hold the harness
            }
            out_lines.push(strip_item(line, &quoted));
            continue;
        }
        out_lines.push((*line).to_string());
    }
    let mut out = out_lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Removes `item` from a TOML inline array line, along with whichever
/// comma joined it to its neighbours.
fn strip_item(line: &str, item: &str) -> String {
    let Some(pos) = line.find(item) else {
        return line.to_string();
    };
    let before = &line[..pos];
    let after = &line[pos + item.len()..];
    // Prefer eating the comma that precedes us; fall back to the one after.
    if let Some(comma) = before.rfind(',') {
        format!("{}{}", &before[..comma], after)
    } else {
        let after = after.trim_start();
        let after = after.strip_prefix(',').unwrap_or(after);
        format!("{before}{}", after.trim_start())
    }
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
    let (out, spans) = harness_lib_source(fn_modules);
    let path = src_dir.join("lib.rs");
    std::fs::write(&path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok((path, spans))
}

/// The text of that file and where each module lands in it, with no
/// filesystem anywhere.
///
/// Split out of [`write_harness_lib_rs`] on 2026-09-04, and the reason is
/// the point rather than tidiness. Ply refuses to generate values for a
/// function that writes files -- it runs the real body, so checking one
/// would mean creating files at paths it invented. That refusal was reading
/// as a limitation of Ply. It is better read as a fact about the function:
/// deciding and writing were in the same place, so the deciding could not be
/// checked.
///
/// What is decided here is not trivial. These spans are what map a compiler
/// error in the generated crate back to the one claim whose module caused
/// it, so an arithmetic slip here misattributes a build failure to an
/// innocent function -- which is the exact defect the attribution mechanism
/// was built to end. Now it takes modules and returns text, and `ply.yaml`
/// claims it.
pub fn harness_lib_source(fn_modules: &[HarnessModule]) -> (String, Vec<ModuleSpan>) {
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
    (out, spans)
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
    fn a_member_crate_finds_its_virtual_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        let member = dir.path().join("crates/member");
        std::fs::create_dir_all(member.join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/member\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        std::fs::write(
            member.join("Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(member.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();

        let root = cargo_workspace_root(&member).unwrap();
        assert_eq!(root, dir.path().canonicalize().unwrap());

        let harness = member.join("target/ply/fuzz/member-ply-harness");
        let plan = harness_workspace_plan(&member, &harness).unwrap();
        assert!(!plan.standalone);
        assert_eq!(plan.workspace_root, dir.path().canonicalize().unwrap());
        assert_eq!(
            plan.harness_member_path,
            "crates/member/target/ply/fuzz/member-ply-harness"
        );

        let workspace_manifest = dir.path().join("Cargo.toml");
        let original = std::fs::read_to_string(&workspace_manifest).unwrap();
        {
            let _registration = ManifestRegistration::register(
                &plan.workspace_manifest,
                &plan.harness_member_path,
                &harness,
                "member-ply-harness",
                &CrateNames {
                    package_name: "member".into(),
                    lib_ident: "member".into(),
                },
            )
            .unwrap();
            let registered = std::fs::read_to_string(&workspace_manifest).unwrap();
            assert!(
                registered.contains("crates/member/target/ply/fuzz/member-ply-harness"),
                "the harness must be registered in the virtual root, not refused:\n{registered}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&workspace_manifest).unwrap(),
            original,
            "the virtual workspace manifest must be restored byte-for-byte"
        );
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
    fn registration_puts_the_user_manifest_back_when_the_run_ends() {
        // The guard exists so a user who runs Ply once on their own
        // workspace is not left with an edit in `git status` they never
        // made. The assertion is byte equality with what was there before,
        // not "the entry is gone" -- whitespace the user wrote counts too.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        let original = "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\nresolver = \"2\"\n\n[package]\nname = \"x\"\n";
        std::fs::write(&path, original).unwrap();

        {
            let _guard = ManifestRegistration::register(
                &path,
                "target/ply/fuzz/x-ply-harness",
                &dir.path().join("target/ply/fuzz/x-ply-harness"),
                "x-ply-harness",
                &CrateNames {
                    package_name: "x".into(),
                    lib_ident: "x".into(),
                },
            )
            .unwrap();
            let during = std::fs::read_to_string(&path).unwrap();
            assert!(
                during.contains("\"target/ply/fuzz/x-ply-harness\""),
                "the harness has to be a member while the run needs it, got:\n{during}"
            );
        }

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after, original,
            "after the run the user's Cargo.toml must be exactly what they wrote"
        );
    }

    #[test]
    fn registration_removes_a_members_line_it_created_itself() {
        // A `[workspace]` with no `members` key means "discover members".
        // Ply writes the whole line in that case, so the whole line has to
        // go again -- leaving `members = ["."]` behind would silently stop
        // that discovery.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        let original = "[workspace]\n\n[package]\nname = \"x\"\n";
        std::fs::write(&path, original).unwrap();
        {
            let _guard = ManifestRegistration::register(
                &path,
                "target/ply/fuzz/x-ply-harness",
                &dir.path().join("target/ply/fuzz/x-ply-harness"),
                "x-ply-harness",
                &CrateNames {
                    package_name: "x".into(),
                    lib_ident: "x".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn registration_leaves_a_manifest_someone_else_edited_alone() {
        // If the bytes on disk are not the ones the guard left, the guard
        // cannot know what it would be destroying, so it destroys nothing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\n\n[package]\nname = \"x\"\n").unwrap();
        let edited = "[workspace]\nmembers = [\".\", \"target/ply/fuzz/x-ply-harness\"]\n\n[package]\nname = \"x\"\nversion = \"9.9.9\"\n";
        {
            let _guard = ManifestRegistration::register(
                &path,
                "target/ply/fuzz/x-ply-harness",
                &dir.path().join("target/ply/fuzz/x-ply-harness"),
                "x-ply-harness",
                &CrateNames {
                    package_name: "x".into(),
                    lib_ident: "x".into(),
                },
            )
            .unwrap();
            std::fs::write(&path, edited).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            edited,
            "a manifest that changed under the guard is not the guard's to rewrite"
        );
    }

    #[test]
    fn registration_clears_a_stale_entry_left_by_an_earlier_crashed_run() {
        // Nobody hand-writes a member under `target/ply/fuzz/`, so an entry
        // already there when the guard starts is Ply's own litter and goes
        // out with the rest.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(
            &path,
            "[workspace]\nmembers = [\".\", \"target/ply/fuzz/x-ply-harness\"]\n\n[package]\nname = \"x\"\n",
        )
        .unwrap();
        {
            let _guard = ManifestRegistration::register(
                &path,
                "target/ply/fuzz/x-ply-harness",
                &dir.path().join("target/ply/fuzz/x-ply-harness"),
                "x-ply-harness",
                &CrateNames {
                    package_name: "x".into(),
                    lib_ident: "x".into(),
                },
            )
            .unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[workspace]\n\n[package]\nname = \"x\"\n"
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
