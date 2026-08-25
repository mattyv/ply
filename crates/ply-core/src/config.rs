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
use crate::schema;

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

/// §5.1a rule 1 and the rest of the load-time schema tier, **read out of
/// `schema/ply.schema.json`** rather than restated here.
///
/// This used to be a second copy of the key vocabulary living next to the
/// serde model — three descriptions of one grammar, with nothing forcing
/// them to agree. The list now comes from the schema, so deleting a key
/// there changes what Ply accepts; `crates/ply-core/tests/schema.rs` holds
/// the model to the same document from the other side.
///
/// Reports the first violation, because that is what a caller loading a
/// document can act on. `cargo ply check` calls [`crate::schema::validate`]
/// directly and reports all of them.
pub fn validate_keys(yaml_text: &str) -> Result<()> {
    let doc: serde_yaml_ng::Value = match serde_yaml_ng::from_str(yaml_text) {
        Ok(v) => v,
        // A document that is not even YAML is the parser's error to report,
        // not this validator's.
        Err(_) => return Ok(()),
    };
    match schema::validate(&doc).into_iter().next() {
        Some(v) => bail!("{}: {}", v.code, v.message),
        None => Ok(()),
    }
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
