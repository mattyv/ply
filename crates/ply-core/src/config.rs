//! Loading a `ply.yaml` off disk (The-Ply-Spec.md §5): read, validate its
//! keys against the §5 vocabulary (`E0204`), parse it into
//! [`crate::model::Document`], and refuse a schema version this build does
//! not speak (`E0201`).
//!
//! Until Phase 1a this module *also* carried a hand-rolled four-struct
//! subset of the format, in parallel with the full model in `tools/model`.
//! Two readers of one document is the defect §5.1a rule 1 was amended to
//! name (vetting 004 finding 7), so the subset is gone: there is now one
//! model, [`crate::model`], and every command reads the document through
//! this function.
//!
//! Still out of scope for this slice: multi-file discovery and merge (§5's
//! "files named `ply.yaml` (or `*.ply.yaml`) ... merge into one model") —
//! `load` reads exactly the path it is given.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::model::{Check, Document, FnClaim, parse_check, parse_document};

/// One claim's own `checks:` strings, parsed, with `E0203` attached to
/// whichever entry failed (§5.1a rule 4). The plain-language reason comes
/// first and the code follows it — a newbie reads the sentence, a script
/// greps the code.
pub fn parsed_checks(claim: &FnClaim) -> Result<Vec<Check>> {
    claim
        .parsed_checks()
        .map_err(|reason| anyhow::anyhow!("{reason} (E0203)"))
}

/// One check string, parsed, with the same `E0203` attachment.
pub fn parse_check_string(s: &str) -> Result<Check> {
    parse_check(s).map_err(|reason| anyhow::anyhow!("{reason} (E0203)"))
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

/// Loads and parses the `ply.yaml` at `path`.
///
/// Order matters and is deliberate: `E0204` key validation runs *first*, so
/// a typo'd key gets the sentence that names the nearest key Ply knows
/// rather than serde's `unknown field` line, which suggests nothing.
pub fn load(path: &Path) -> Result<Document> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading ply.yaml at {}", path.display()))?;
    load_str(&text).with_context(|| format!("reading ply.yaml at {}", path.display()))
}

/// [`load`] over already-read text, for callers that have the document in
/// hand (tests, and `check`, which reads the file once and reports on it).
pub fn load_str(text: &str) -> Result<Document> {
    validate_keys(text)?;
    let doc = parse_document(text).map_err(|e| anyhow::anyhow!("{e}"))?;
    if doc.ply != 1 {
        bail!(
            "E0201: unsupported `ply:` schema version {} (expected 1)",
            doc.ply
        );
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::check::mutate_lacks_kill_signal;

    #[test]
    fn parses_bounded_check() {
        assert_eq!(parse_check_string("bounded(8)").unwrap(), Check::Bounded(8));
    }

    #[test]
    fn rejects_bounded_out_of_range() {
        assert!(parse_check_string("bounded(65)").is_err());
        assert!(parse_check_string("bounded(0)").is_err());
    }

    /// The `E0203` code follows the plain sentence rather than replacing it
    /// (CLAUDE.md's newbie bar): before Phase 1a the verify path had its own
    /// terser wording for the same defect.
    #[test]
    fn an_out_of_range_bound_says_why_before_it_says_the_code() {
        let err = parse_check_string("bounded(0)").unwrap_err().to_string();
        assert_eq!(
            err,
            "\"bounded(0)\" is not a valid check: the number is how many times loops are \
             unrolled during the proof, and it must be between 1 and 64 — a bound of 0 would \
             prove nothing (E0203)"
        );
    }

    #[test]
    fn mutate_alone_is_e0504() {
        assert!(mutate_lacks_kill_signal(&[Check::Mutate]));
    }

    #[test]
    fn mutate_with_fuzz_is_fine() {
        assert!(!mutate_lacks_kill_signal(&[
            Check::Fuzz(256),
            Check::Mutate
        ]));
    }

    #[test]
    fn mutate_with_test_is_fine() {
        assert!(!mutate_lacks_kill_signal(&[Check::Test, Check::Mutate]));
    }

    #[test]
    fn no_mutate_at_all_is_fine() {
        assert!(!mutate_lacks_kill_signal(&[Check::Bounded(2)]));
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
        assert_eq!(parsed_checks(fn_claim).unwrap(), vec![Check::Bounded(2)]);
    }
}
