//! Scaffolding for the generated harness crate under `target/ply/fuzz/`
//! (§5.4c) that carries the `fuzz`/`test` checks' generated tests and,
//! per `tests/spike/mutants/MUTANTS-FINDINGS.md`'s verified mechanism, is
//! what `mutate` names via `--test-package`.
//!
//! The mechanism has three load-bearing parts, all confirmed in the spike
//! and reproduced here as real codegen rather than a hand-written fixture:
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
//! 3. `--gitignore false` is the mutate adapter's job (`engines::mutants`),
//!    not this module's -- but the placement here is exactly the one the
//!    spike found dangerous under `--gitignore true`, so `engines::mutants`
//!    must always pass it explicitly.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

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
    Ok(CrateNames { package_name, lib_ident })
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
        if in_section {
            if let Some(rest) = trimmed.strip_prefix(key) {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let v = rest.trim();
                    let v = v.trim_matches('"');
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// The harness crate's package name for a given target package name --
/// deterministic so re-running `verify` finds the same crate every time.
pub fn harness_package_name(target_package_name: &str) -> String {
    format!("{target_package_name}-ply-harness")
}

/// Where the harness crate lives, relative to the target crate's root
/// (§5.4c: `target/ply/fuzz/`).
pub fn harness_rel_path(target_package_name: &str) -> String {
    format!("target/ply/fuzz/{}", harness_package_name(target_package_name))
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
        bail!("{} has no `[workspace]` table to add the harness crate to", crate_cargo_toml.display());
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
/// §6). `target_names` is the crate under test; `depth_to_target_root` is
/// how many `../` steps get from the harness crate's directory back to the
/// target crate's root (4, for the fixed `target/ply/fuzz/<name>/` depth).
pub fn write_harness_cargo_toml(
    harness_dir: &Path,
    harness_package: &str,
    target_names: &CrateNames,
) -> Result<()> {
    std::fs::create_dir_all(harness_dir)
        .with_context(|| format!("creating {}", harness_dir.display()))?;
    let toml = format!(
        "# Generated by Ply -- do not edit. The M4 fuzz/test/mutate harness\n\
         # crate for `{target}` (The-Ply-Spec.md §5.4c). Ply regenerates this\n\
         # file on every `verify` run.\n\
         [package]\n\
         name = \"{harness_package}\"\n\
         version = \"0.0.0\"\n\
         edition = \"2021\"\n\
         publish = false\n\n\
         [dev-dependencies]\n\
         {target_pkg} = {{ path = \"../../../..\" }}\n\
         proptest = \"1\"\n",
        target = target_names.package_name,
        target_pkg = target_names.package_name,
    );
    std::fs::write(harness_dir.join("Cargo.toml"), toml)
        .with_context(|| format!("writing {}/Cargo.toml", harness_dir.display()))?;
    Ok(())
}

/// Writes the harness crate's `src/lib.rs`: an empty (non-test) public
/// surface plus every generated `#[cfg(test)] mod {fn}_harness { ... }`
/// block, concatenated. Regenerated wholesale on every run (Ply owns this
/// file entirely -- unlike the in-crate `ply_generated*.rs` files, there is
/// no user code anywhere in this crate to preserve).
pub fn write_harness_lib_rs(harness_dir: &Path, fn_modules: &[String]) -> Result<PathBuf> {
    let src_dir = harness_dir.join("src");
    std::fs::create_dir_all(&src_dir).with_context(|| format!("creating {}", src_dir.display()))?;
    let mut out = String::from(
        "//! Generated by Ply -- do not edit. Fuzz/test/mutate harness (The-Ply-Spec.md §5.4c).\n\n",
    );
    for m in fn_modules {
        out.push_str(m);
        out.push('\n');
    }
    let path = src_dir.join("lib.rs");
    std::fs::write(&path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
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
    fn ensure_workspace_member_inserts_into_empty_workspace_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(
            &path,
            "[workspace]\n# comment\n\n[package]\nname = \"x\"\n",
        )
        .unwrap();
        ensure_workspace_member(&path, "target/ply/fuzz/x-ply-harness").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("members = [\".\", \"target/ply/fuzz/x-ply-harness\"]"), "{text}");
        assert!(text.contains("[package]"), "must not disturb the rest of the file:\n{text}");
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
        assert_eq!(after_first, after_second, "must not duplicate the member entry on rerun");
    }

    #[test]
    fn ensure_workspace_member_appends_to_existing_members_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\nmembers = [\".\"]\n\n[package]\nname = \"x\"\n").unwrap();
        ensure_workspace_member(&path, "target/ply/fuzz/x-ply-harness").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("members = [\".\", \"target/ply/fuzz/x-ply-harness\"]"), "{text}");
    }
}
