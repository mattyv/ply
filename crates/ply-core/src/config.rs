//! Minimal `ply.yaml` reader for the M3 thin slice (§5 of The-Ply-Spec.md).
//!
//! TODO(M1): this is a hand-rolled ~4-struct subset, not the full model.
//! `tools/model` already has a complete model with checks-inheritance,
//! schema validation, and multi-file merge. Do NOT depend on it across the
//! workspace boundary (tools/ is a separate workspace) and do NOT reproduce
//! it here. M1 must reconcile the two: promote one, delete the other.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
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
        bail!(
            "E0203: unrecognized check string `{s}` (expected test | fuzz(N) | bounded(K) | prove | mutate)"
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FnClaim {
    #[serde(default)]
    pub checks: Vec<String>,
    /// §5.4's external-spec route: "`requires`/`ensures` entries in
    /// `ply.yaml` are ANDed in, for teams that prefer external specs."
    /// Read by the verify path since 2026-08-25 -- before that serde
    /// silently dropped both keys (vetting 004 finding 7), so a team
    /// declaring a contract for an unclaimed callee got no contract and no
    /// warning. They are also the mechanism D5's second branch needs to
    /// admit a legacy callee's assumption at all (§5.5).
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub ensures: Vec<String>,
    /// §5.4a: "examples entries are exempt -- they are arbitrary Rust `==`
    /// expressions, compiled as plain `#[test]`s and never translated for
    /// an engine." Raw strings; parsed at codegen time (`fuzz_gen`).
    #[serde(default)]
    pub examples: Vec<String>,
}

impl FnClaim {
    pub fn parsed_checks(&self) -> Result<Vec<Check>> {
        self.checks.iter().map(|s| Check::parse(s)).collect()
    }
}

/// D12's own MUST: `mutate` requires a `test` or `fuzz` entry in the *same*
/// list, else `E0504` -- `mutate` has no kill signal of its own. Returns
/// `Ok(())` when the list is fine (including when `mutate` is simply
/// absent), `Err` naming the defect otherwise.
pub fn validate_mutate_has_kill_signal(checks: &[Check]) -> Result<()> {
    let has_mutate = checks.iter().any(|c| matches!(c, Check::Mutate));
    if !has_mutate {
        return Ok(());
    }
    let has_kill_signal = checks
        .iter()
        .any(|c| matches!(c, Check::Test | Check::Fuzz(_)));
    if has_kill_signal {
        return Ok(());
    }
    bail!(
        "E0504: `mutate` has no `test` or `fuzz` entry in the same checks list to use as its \
         mutant-kill signal -- mutation testing works by re-running an existing test suite \
         against a deliberately broken copy of the function, so without `test` or `fuzz` \
         alongside it, `mutate` has nothing to run. Add `test` or `fuzz(n)` to this fn's checks."
    );
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

/// The §5 key vocabulary, level by level. This is deliberately the **full
/// grammar** (the same set `tools/model` deserializes for `ply-check` and
/// `ply-render`), not the subset `verify` acts on: one document is read by
/// three tools, so `verify` must accept every key §5 defines and quietly
/// ignore the ones it has no use for. What it must never do is accept a key
/// §5 does *not* define -- that is a typo, and §5.1a rule 1 says a typo must
/// be caught, never ignored.
const DOC_KEYS: &[&str] = &[
    "ply",
    "components",
    "externals",
    "edges",
    "deny",
    "profiles",
    "unresolved",
];
const COMPONENT_KEYS: &[&str] = &[
    "anchor",
    "pure",
    "strict",
    "uses",
    "owns",
    "profile",
    "checks",
    "components",
    "fns",
];
const FN_KEYS: &[&str] = &[
    "checks",
    "mode",
    "requires",
    "ensures",
    "examples",
    "check_with",
    "trusted",
    "unresolved",
    "entry",
];
const EXTERNAL_KEYS: &[&str] = &["note"];
const TRUSTED_KEYS: &[&str] = &["claim", "evidence"];
const UNRESOLVED_KEYS: &[&str] = &["id", "note"];

/// What a key belongs to, in words a reader can act on.
fn level_name(keys: &[&str]) -> &'static str {
    if std::ptr::eq(keys.as_ptr(), DOC_KEYS.as_ptr()) {
        "the top level of a ply.yaml document"
    } else if std::ptr::eq(keys.as_ptr(), COMPONENT_KEYS.as_ptr()) {
        "a component"
    } else if std::ptr::eq(keys.as_ptr(), FN_KEYS.as_ptr()) {
        "a fn claim"
    } else if std::ptr::eq(keys.as_ptr(), EXTERNAL_KEYS.as_ptr()) {
        "an external"
    } else if std::ptr::eq(keys.as_ptr(), TRUSTED_KEYS.as_ptr()) {
        "a `trusted` entry"
    } else {
        "an `unresolved` entry"
    }
}

/// Plain Levenshtein distance, used only to pick the closest known key for
/// an E0204 suggestion.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The closest known key, when one is close enough to be worth naming.
fn nearest_key(unknown: &str, known: &[&str]) -> Option<String> {
    known
        .iter()
        .map(|k| (edit_distance(unknown, k), *k))
        .filter(|(d, k)| *d <= 3 || k.starts_with(unknown) || unknown.starts_with(*k))
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k.to_string())
}

fn check_mapping(value: &serde_yaml_ng::Value, known: &[&str], path: &str) -> Result<()> {
    let Some(map) = value.as_mapping() else {
        return Ok(());
    };
    for key in map.keys() {
        let Some(name) = key.as_str() else { continue };
        if known.contains(&name) {
            continue;
        }
        let where_at = if path.is_empty() {
            name.to_string()
        } else {
            format!("{path}.{name}")
        };
        let suggestion = match nearest_key(name, known) {
            Some(s) => format!(" Did you mean `{s}`?"),
            None => String::new(),
        };
        bail!(
            "E0204: `{name}:` is not a key Ply knows. The keys {level} accepts are: {list}.{suggestion} \
             A key Ply does not know is almost always a typo, and a typo has to be caught rather \
             than ignored (§5.1a rule 1) -- an ignored key is a contract you think you wrote and \
             Ply never read. Found at `{where_at}` in ply.yaml.",
            level = level_name(known),
            list = known
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    Ok(())
}

fn check_sequence_of_mappings(
    parent: &serde_yaml_ng::Value,
    field: &str,
    known: &[&str],
    path: &str,
) -> Result<()> {
    let Some(seq) = parent.get(field).and_then(|v| v.as_sequence()) else {
        return Ok(());
    };
    for (i, entry) in seq.iter().enumerate() {
        check_mapping(entry, known, &format!("{path}.{field}[{i}]"))?;
    }
    Ok(())
}

fn check_component(value: &serde_yaml_ng::Value, path: &str) -> Result<()> {
    check_mapping(value, COMPONENT_KEYS, path)?;
    if let Some(fns) = value.get("fns").and_then(|v| v.as_mapping()) {
        for (name, claim) in fns {
            let name = name.as_str().unwrap_or("?");
            let p = format!("{path}.fns.{name}");
            check_mapping(claim, FN_KEYS, &p)?;
            check_sequence_of_mappings(claim, "trusted", TRUSTED_KEYS, &p)?;
            check_sequence_of_mappings(claim, "unresolved", UNRESOLVED_KEYS, &p)?;
        }
    }
    if let Some(nested) = value.get("components").and_then(|v| v.as_mapping()) {
        for (name, comp) in nested {
            let name = name.as_str().unwrap_or("?");
            check_component(comp, &format!("{path}.components.{name}"))?;
        }
    }
    Ok(())
}

/// §5.1a rule 1 on the *verify* path (2026-08-25). Until now `ply-check`
/// enforced `additionalProperties: false` on a document while `cargo ply
/// verify` read the same file with plain serde and silently dropped every
/// key its own structs did not name -- so a team writing an external
/// `ensures:` got no contract and no warning (vetting 004 finding 7). Two
/// tools disagreeing about the same file is how that happened; this is the
/// half that was missing.
pub fn validate_keys(yaml_text: &str) -> Result<()> {
    let doc: serde_yaml_ng::Value = match serde_yaml_ng::from_str(yaml_text) {
        Ok(v) => v,
        // A document that is not even YAML is the parser's error to report,
        // not this validator's.
        Err(_) => return Ok(()),
    };
    check_mapping(&doc, DOC_KEYS, "")?;
    if let Some(components) = doc.get("components").and_then(|v| v.as_mapping()) {
        for (name, comp) in components {
            let name = name.as_str().unwrap_or("?");
            check_component(comp, &format!("components.{name}"))?;
        }
    }
    if let Some(externals) = doc.get("externals").and_then(|v| v.as_mapping()) {
        for (name, ext) in externals {
            let name = name.as_str().unwrap_or("?");
            check_mapping(ext, EXTERNAL_KEYS, &format!("externals.{name}"))?;
        }
    }
    check_sequence_of_mappings(&doc, "unresolved", UNRESOLVED_KEYS, "")?;
    Ok(())
}

/// Loads and parses `ply.yaml` at `path`. Multi-file merge (§5) is still out
/// of scope for this slice -- see the TODO at the top of this module -- but
/// key validation (§5.1a rule 1, `E0204`) is enforced here, in parity with
/// `ply-check` on the same document.
pub fn load(path: &Path) -> Result<PlyFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading ply.yaml at {}", path.display()))?;
    validate_keys(&text)?;
    let file: PlyFile = serde_yaml_ng::from_str(&text)
        .with_context(|| format!("parsing ply.yaml at {}", path.display()))?;
    if file.ply != 1 {
        bail!(
            "E0201: unsupported `ply:` schema version {} (expected 1)",
            file.ply
        );
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
    fn mutate_alone_is_e0504() {
        let err = validate_mutate_has_kill_signal(&[Check::Mutate]).unwrap_err();
        assert!(err.to_string().contains("E0504"), "{err}");
    }

    #[test]
    fn mutate_with_fuzz_is_fine() {
        assert!(validate_mutate_has_kill_signal(&[Check::Fuzz(256), Check::Mutate]).is_ok());
    }

    #[test]
    fn mutate_with_test_is_fine() {
        assert!(validate_mutate_has_kill_signal(&[Check::Test, Check::Mutate]).is_ok());
    }

    #[test]
    fn no_mutate_at_all_is_fine() {
        assert!(validate_mutate_has_kill_signal(&[Check::Bounded(2)]).is_ok());
    }

    /// vetting 004 finding 7, the half that made it silent: `ensures:` was
    /// eaten by serde on the verify path while `ply-check` enforced
    /// §5.1a rule 1 on the very same document.
    #[test]
    fn a_typo_in_a_fn_key_is_e0204_with_the_nearest_key_named() {
        let err = validate_keys(
            r#"
ply: 1
components:
  withdrawal:
    anchor: withdrawal
    fns:
      fee_cents:
        ensure:
          - "|result| *result <= amount_cents"
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("E0204"), "{err}");
        assert!(err.contains("`ensure:`"), "{err}");
        assert!(err.contains("Did you mean `ensures`?"), "{err}");
        assert!(
            err.contains("components.withdrawal.fns.fee_cents.ensure"),
            "the message must say where the key is: {err}"
        );
    }

    /// The whole §5 grammar must survive, not just the subset `verify`
    /// acts on -- one document, three tools (vetting 004's own ply.yaml has
    /// `pure:` on a component and `edges:` at the top level).
    #[test]
    fn the_full_section_5_grammar_is_accepted_even_where_verify_ignores_it() {
        validate_keys(
            r#"
ply: 1
components:
  ledger:
    anchor: ledger
  withdrawal:
    anchor: withdrawal
    pure: true
    strict: false
    profile: core
    checks: [bounded(2)]
    fns:
      fee_cents:
        checks: [bounded(2)]
        mode: check
        requires: ["x < 10"]
        ensures: ["|result| *result <= x"]
        examples: ["fee_cents(1, 1) == 0"]
        check_with: { T: u64 }
        entry: [stripe]
        trusted:
          - claim: "loom-checked"
            evidence: "tests/loom.rs"
        unresolved:
          - id: 147
            note: "employee discount undecided"
externals:
  stripe:
    note: "the payment processor"
edges:
  - withdrawal -> ledger
deny:
  - "* -> ledger"
profiles:
  core: []
unresolved:
  - id: 9
    note: "open"
"#,
        )
        .unwrap();
    }

    #[test]
    fn an_unknown_top_level_key_is_caught_too() {
        let err = validate_keys(
            "ply: 1
component:
  x: 1
",
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("E0204") && err.contains("`components`"),
            "{err}"
        );
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
