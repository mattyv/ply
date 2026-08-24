//! Minimal `ply.yaml` reader for the M3 thin slice (§5 of The-Ply-Spec.md).
//!
//! TODO(M1): this is a hand-rolled ~4-struct subset, not the full model.
//! `tools/model` already has a complete model with checks-inheritance,
//! schema validation, and multi-file merge. Do NOT depend on it across the
//! workspace boundary (tools/ is a separate workspace) and do NOT reproduce
//! it here. M1 must reconcile the two: promote one, delete the other.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// One `checks:` list entry, parsed from its micro-syntax (§5, item 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Test,
    Fuzz(u32),
    Bounded(u32),
    Prove,
    Mutate,
}

impl Check {
    /// Parses one check string (`test`, `fuzz(256)`, `bounded(8)`, `prove`,
    /// `mutate`). Full range validation (§5.1a: `1 <= N <= 1_000_000`,
    /// `1 <= K <= 64`) is enforced; anything else is `E0203`.
    pub fn parse(s: &str) -> Result<Check> {
        let s = s.trim();
        if s == "test" {
            return Ok(Check::Test);
        }
        if s == "prove" {
            return Ok(Check::Prove);
        }
        if s == "mutate" {
            return Ok(Check::Mutate);
        }
        if let Some(inner) = s.strip_prefix("fuzz(").and_then(|r| r.strip_suffix(')')) {
            let n: u32 = inner
                .trim()
                .parse()
                .with_context(|| format!("E0203: `fuzz(N)` needs an integer N, got `{s}`"))?;
            if !(1..=1_000_000).contains(&n) {
                bail!("E0203: `fuzz(N)` needs 1 <= N <= 1_000_000, got fuzz({n})");
            }
            return Ok(Check::Fuzz(n));
        }
        if let Some(inner) = s.strip_prefix("bounded(").and_then(|r| r.strip_suffix(')')) {
            let k: u32 = inner
                .trim()
                .parse()
                .with_context(|| format!("E0203: `bounded(K)` needs an integer K, got `{s}`"))?;
            if !(1..=64).contains(&k) {
                bail!("E0203: `bounded(K)` needs 1 <= K <= 64, got bounded({k})");
            }
            return Ok(Check::Bounded(k));
        }
        bail!("E0203: unrecognized check string `{s}` (expected test | fuzz(N) | bounded(K) | prove | mutate)")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FnClaim {
    #[serde(default)]
    pub checks: Vec<String>,
}

impl FnClaim {
    pub fn parsed_checks(&self) -> Result<Vec<Check>> {
        self.checks.iter().map(|s| Check::parse(s)).collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    /// Crate name, or crate::module::path (§5.1). For this slice: the fixture
    /// crate's package name (e.g. `ply_fixture_clamp`).
    pub anchor: String,
    #[serde(default)]
    pub fns: BTreeMap<String, FnClaim>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlyFile {
    /// Schema version -- must be 1 for this slice.
    pub ply: u32,
    #[serde(default)]
    pub components: BTreeMap<String, Component>,
}

/// Loads and parses `ply.yaml` at `path`. Full schema validation (E02xx) and
/// multi-file merge (§5) are out of scope for this slice -- see the TODO at
/// the top of this module.
pub fn load(path: &Path) -> Result<PlyFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading ply.yaml at {}", path.display()))?;
    let file: PlyFile = serde_yaml_ng::from_str(&text)
        .with_context(|| format!("parsing ply.yaml at {}", path.display()))?;
    if file.ply != 1 {
        bail!("E0201: unsupported `ply:` schema version {} (expected 1)", file.ply);
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bounded_check() {
        assert_eq!(Check::parse("bounded(8)").unwrap(), Check::Bounded(8));
    }

    #[test]
    fn rejects_bounded_out_of_range() {
        assert!(Check::parse("bounded(65)").is_err());
        assert!(Check::parse("bounded(0)").is_err());
    }

    #[test]
    fn loads_minimal_ply_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ply.yaml");
        std::fs::write(
            &path,
            r#"
ply: 1
components:
  clamp:
    anchor: ply_fixture_clamp
    fns:
      clamp:
        checks: [bounded(2)]
"#,
        )
        .unwrap();
        let file = load(&path).unwrap();
        assert_eq!(file.ply, 1);
        let comp = file.components.get("clamp").unwrap();
        assert_eq!(comp.anchor, "ply_fixture_clamp");
        let fn_claim = comp.fns.get("clamp").unwrap();
        assert_eq!(fn_claim.parsed_checks().unwrap(), vec![Check::Bounded(2)]);
    }
}
