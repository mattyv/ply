//! The committed record of results, and the fingerprint that guards it
//! (The-Ply-Spec.md §5.2a, D14).
//!
//! A result Ply earned is written to `ply.lock` beside a hash of everything
//! the answer depended on. Before that result is ever used or shown, the
//! hash is recomputed from today's inputs: it matches, the result is reused;
//! it does not, the check runs again and the record is rewritten.
//!
//! **The honesty rule is the whole design.** A stored verdict that reaches a
//! reader without being re-hashed is a remembered opinion, and it can drift
//! from the code silently for as long as nobody re-blesses it. A stored
//! verdict that is re-hashed at the moment of use cannot drift at all, which
//! is why there is no "may be out of date" state anywhere in Ply and nothing
//! for a human to confirm.
//!
//! **Ply's own version is one of the hashed inputs**, and it is the one that
//! makes the scheme sound rather than merely fast. On 2026-08-25 four
//! defects were fixed that each changed what a result *means*: a harness
//! that failed to compile earned a confident pass, an ordinary `use` import
//! let an unvouched-for body into a proof, an unsatisfiable declared promise
//! passed vacuously, and a claim inside a nested component was skipped in
//! silence. Every result recorded by the previous build carries one of those
//! risks, and every one of them would hash-match perfectly today, because
//! the user's source did not change -- Ply did.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::diag::{Diagnostic, Evidence};

/// The format version written into the file. A record written by a format
/// this build does not know is ignored rather than misread -- a wrong
/// reading of a stored result is exactly the failure this module exists to
/// prevent.
pub const FORMAT: u32 = 1;

/// One engine as it stood for one run: the name Ply calls it, the version it
/// reported, and the flags that shaped the obligation it discharged.
///
/// The per-check wall-clock budget is deliberately *not* here. A proof that
/// finished inside 300s is not made false by a later run that would have
/// allowed only 60s, and folding the budget in would re-pay every proof in a
/// CI job that sets a different one (§5.2a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineId {
    pub name: String,
    pub version: String,
    pub flags: String,
}

/// One promise a proof stood on instead of the callee's real body (§5.5's
/// second branch). The caller's result is *about* this text, so editing it
/// in `ply.yaml` must re-run every caller resting on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssumedPromise {
    pub callee: String,
    pub requires: Vec<String>,
    pub ensures: Vec<String>,
}

/// Everything one claim's result depended on. The field list is the spec's
/// list (§5.2a), in the spec's order, so a reader can check one against the
/// other.
#[derive(Debug, Clone, Default)]
pub struct FingerprintInputs {
    /// Which claim this is. Two claims on the same function with different
    /// checks are different results.
    pub node_id: String,
    /// The function's path from the crate root.
    pub fn_path: String,
    /// The function item's own token stream. Formatting and comments do not
    /// count as change; every token does.
    pub fn_source: String,
    /// The inline `#[ply::requires]` / `#[ply::ensures]` text.
    pub inline_requires: String,
    pub inline_ensures: String,
    /// Anything `ply.yaml` declares for this same function.
    pub declared_requires: Vec<String>,
    pub declared_ensures: Vec<String>,
    /// The promises assumed for the callees this claim crosses into.
    pub assumed: Vec<AssumedPromise>,
    /// The checks that ran, spelled as they are written in `ply.yaml`.
    pub checks: Vec<String>,
    /// The seed the sampling engine drew from (§5.4c). A `--seed` replaying
    /// a different run must not match a record written by the derived one.
    pub seed: String,
    /// One entry per engine the checks used.
    pub engines: Vec<EngineId>,
    /// The target triple, and the compiler that built for it.
    pub target: String,
    pub rustc: String,
    /// The crate's declared feature table. Ply passes no `--features`, so
    /// the active set is the default set this text defines.
    pub features: String,
    /// Ply's own version.
    pub ply_version: String,
}

impl FingerprintInputs {
    /// The canonical byte string this fingerprint is taken over. Every field
    /// is written as `label`, its byte length, then its bytes, so no value
    /// can be mistaken for a field boundary: without the length prefix, a
    /// contract containing a newline could be arranged to hash the same as
    /// two different fields.
    fn canonical(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut put = |label: &str, value: &str| {
            out.extend_from_slice(label.as_bytes());
            out.push(0);
            out.extend_from_slice(value.len().to_string().as_bytes());
            out.push(0);
            out.extend_from_slice(value.as_bytes());
            out.push(0);
        };
        put("ply-fingerprint", "1");
        put("ply-version", &self.ply_version);
        put("node", &self.node_id);
        put("fn-path", &self.fn_path);
        put("fn-source", &self.fn_source);
        put("inline-requires", &self.inline_requires);
        put("inline-ensures", &self.inline_ensures);
        for r in &self.declared_requires {
            put("declared-requires", r);
        }
        for e in &self.declared_ensures {
            put("declared-ensures", e);
        }
        for a in &self.assumed {
            put("assumed-callee", &a.callee);
            for r in &a.requires {
                put("assumed-requires", r);
            }
            for e in &a.ensures {
                put("assumed-ensures", e);
            }
        }
        for c in &self.checks {
            put("check", c);
        }
        put("seed", &self.seed);
        for e in &self.engines {
            put("engine-name", &e.name);
            put("engine-version", &e.version);
            put("engine-flags", &e.flags);
        }
        put("target", &self.target);
        put("rustc", &self.rustc);
        put("features", &self.features);
        out
    }
}

/// The hash of everything the answer depended on, as 64 hex characters.
pub fn fingerprint(inputs: &FingerprintInputs) -> String {
    blake3::hash(&inputs.canonical()).to_hex().to_string()
}

/// One claim's recorded result: the fingerprint, and everything the run that
/// earned it reported about that node.
///
/// The diagnostics are stored with it on purpose. A reused result has to
/// reach the reader as the run that earned it left it -- a reused
/// `conditional` verdict that printed its marks without the paragraph naming
/// the promise it assumed would be a worse report than no reuse at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordEntry {
    pub fingerprint: String,
    pub verdict: String,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

/// The file itself. `written_by` is informational (it is already inside
/// every fingerprint); it is there so a reader of the diff can see which
/// Ply produced these results without decoding a hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub format: u32,
    pub written_by: String,
    pub results: BTreeMap<String, RecordEntry>,
}

impl Record {
    pub fn new(ply_version: &str) -> Self {
        Record {
            format: FORMAT,
            written_by: ply_version.to_string(),
            results: BTreeMap::new(),
        }
    }

    pub fn to_json_pretty(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("Record always serializes");
        s.push('\n');
        s
    }

    /// The recorded result for this claim, **only if** the fingerprint
    /// handed in matches the recorded one. There is no way to read an entry
    /// without presenting today's hash: the honesty rule is enforced by the
    /// shape of this function, not by remembering to call a checker first.
    pub fn matching(&self, node_id: &str, fingerprint: &str) -> Option<&RecordEntry> {
        self.results
            .get(node_id)
            .filter(|e| e.fingerprint == fingerprint)
    }

    pub fn record(&mut self, node_id: &str, entry: RecordEntry) {
        self.results.insert(node_id.to_string(), entry);
    }

    /// Drops every entry except the ones this run reused or earned.
    ///
    /// Three things go at once: a claim somebody deleted from the document,
    /// a claim whose function no longer resolves, and a claim this run
    /// checked and got no evidence for. None of them can ever be reused --
    /// their fingerprints cannot match -- so keeping them would only leave a
    /// committed file showing a verdict the last run did not produce, which
    /// is the "remembered opinion" this whole design refuses (§6's
    /// housekeeping rule, §5.2a's honesty rule).
    pub fn retain_claims(&mut self, kept: &std::collections::BTreeSet<String>) {
        self.results.retain(|id, _| kept.contains(id));
    }
}

/// Reads the record beside `ply.yaml`. A missing file is an empty record --
/// the ordinary first run.
///
/// A file that will not parse is an **error**, never an empty record: the
/// most likely cause is a merge conflict in a committed file, and silently
/// continuing would re-pay every proof while telling nobody why. The message
/// says what to do about it.
pub fn load(path: &Path, ply_version: &str) -> Result<Record> {
    if !path.exists() {
        return Ok(Record::new(ply_version));
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the recorded results at {}", path.display()))?;
    let mut record: Record = serde_json::from_str(&text).with_context(|| {
        format!(
            "`{}` holds the results of previous runs, and this one could not be read. If it has \
             merge-conflict markers in it, or was hand-edited, delete it and run `cargo ply \
             verify` again: nothing is lost except the engine time to earn those results back",
            path.display()
        )
    })?;
    // `written_by` names the version that last *wrote* the file, not the one
    // that first did: every entry that survives a run was either matched by
    // a fingerprint carrying this version or written by this run, so
    // stamping it here is a fact rather than a guess.
    record.written_by = ply_version.to_string();
    if record.format != FORMAT {
        // Ignored, not misread: a record written in a format this build does
        // not know says nothing this build can trust.
        return Ok(Record::new(ply_version));
    }
    Ok(record)
}

/// Writes the record, and only when its content actually changed -- a run
/// that reused everything must not dirty a working tree or a git status.
pub fn save(path: &Path, record: &Record) -> Result<()> {
    let text = record.to_json_pretty();
    if record.results.is_empty() && !path.exists() {
        return Ok(());
    }
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == text
    {
        return Ok(());
    }
    std::fs::write(path, text)
        .with_context(|| format!("writing the recorded results to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> FingerprintInputs {
        FingerprintInputs {
            node_id: "billing::tiered_fee".into(),
            fn_path: "tiered_fee".into(),
            fn_source: "pub fn tiered_fee (a : u32) -> u32 { a }".into(),
            inline_requires: "a <= 100".into(),
            inline_ensures: "| result | * result <= 100".into(),
            declared_requires: vec![],
            declared_ensures: vec![],
            assumed: vec![AssumedPromise {
                callee: "legacy_rate".into(),
                requires: vec![],
                ensures: vec!["|result| *result <= 10_000".into()],
            }],
            checks: vec!["bounded(2)".into()],
            seed: "00".repeat(32),
            engines: vec![EngineId {
                name: "kani".into(),
                version: "0.67.0".into(),
                flags: "-Z function-contracts".into(),
            }],
            target: "x86_64-unknown-linux-gnu".into(),
            rustc: "rustc 1.94.1".into(),
            features: "(none)".into(),
            ply_version: "0.1.0".into(),
        }
    }

    /// One test per hashed input, walked as one loop: a field added later
    /// that nothing feeds into the hash is exactly the defect this catches,
    /// and a per-field spot-check would not have caught the field nobody
    /// remembered to add.
    /// "change this one input" -- named, so the failure says which input
    /// stopped counting rather than printing two equal hashes.
    type Mutation = (&'static str, Box<dyn Fn(&mut FingerprintInputs)>);

    #[test]
    fn every_input_the_spec_lists_changes_the_fingerprint() {
        let base = fingerprint(&inputs());
        let mutations: Vec<Mutation> = vec![
            (
                "the function's own source",
                Box::new(|i: &mut FingerprintInputs| {
                    i.fn_source = "pub fn tiered_fee (a : u32) -> u32 { a + 0 }".into()
                }),
            ),
            (
                "its inline requires",
                Box::new(|i: &mut FingerprintInputs| i.inline_requires = "a <= 99".into()),
            ),
            (
                "its inline ensures",
                Box::new(|i: &mut FingerprintInputs| {
                    i.inline_ensures = "| result | * result <= 99".into()
                }),
            ),
            (
                "a contract ply.yaml declares for it",
                Box::new(|i: &mut FingerprintInputs| {
                    i.declared_ensures = vec!["|result| true".into()]
                }),
            ),
            (
                "a promise it assumes for a callee",
                Box::new(|i: &mut FingerprintInputs| {
                    i.assumed[0].ensures = vec!["|result| *result <= 9_000".into()]
                }),
            ),
            (
                "which callee that promise is for",
                Box::new(|i: &mut FingerprintInputs| i.assumed[0].callee = "other_rate".into()),
            ),
            (
                "the checks that ran",
                Box::new(|i: &mut FingerprintInputs| i.checks = vec!["bounded(3)".into()]),
            ),
            (
                "the seed the cases were drawn from",
                Box::new(|i: &mut FingerprintInputs| i.seed = "11".repeat(32)),
            ),
            (
                "the engine's version",
                Box::new(|i: &mut FingerprintInputs| i.engines[0].version = "0.68.0".into()),
            ),
            (
                "the engine's flags",
                Box::new(|i: &mut FingerprintInputs| {
                    i.engines[0].flags = "-Z function-contracts -Z stubbing".into()
                }),
            ),
            (
                "the build target",
                Box::new(|i: &mut FingerprintInputs| i.target = "aarch64-apple-darwin".into()),
            ),
            (
                "the compiler",
                Box::new(|i: &mut FingerprintInputs| i.rustc = "rustc 1.95.0".into()),
            ),
            (
                "the crate's features",
                Box::new(|i: &mut FingerprintInputs| i.features = "[features]\nbig = []".into()),
            ),
            (
                "Ply's own version",
                Box::new(|i: &mut FingerprintInputs| i.ply_version = "0.1.1".into()),
            ),
        ];
        for (what, mutate) in mutations {
            let mut changed = inputs();
            mutate(&mut changed);
            assert_ne!(
                base,
                fingerprint(&changed),
                "changing {what} must change the fingerprint, or a result earned before the \
                 change is reused after it"
            );
        }
    }

    /// The input the whole scheme rests on, stated on its own because it is
    /// the one nobody would think to look for: a fix to Ply changes what a
    /// result means, and the user's source is identical either side of it.
    #[test]
    fn a_new_ply_version_invalidates_a_result_whose_source_did_not_change() {
        let yesterday = inputs();
        let mut today = inputs();
        today.ply_version = "0.1.1".into();
        assert_eq!(
            yesterday.fn_source, today.fn_source,
            "the premise: not one character of the user's code changed"
        );
        assert_ne!(
            fingerprint(&yesterday),
            fingerprint(&today),
            "a result recorded by yesterday's build must not be reused by today's -- the four \
             defects fixed on 2026-08-25 would each have matched perfectly"
        );
    }

    /// Identical inputs must hash identically, or nothing is ever reused and
    /// the record is dead weight.
    #[test]
    fn identical_inputs_hash_identically() {
        assert_eq!(fingerprint(&inputs()), fingerprint(&inputs()));
    }

    /// Field boundaries are real: moving text from one field to the next
    /// must not produce the same hash.
    #[test]
    fn text_moved_across_a_field_boundary_does_not_hash_the_same() {
        let mut a = inputs();
        a.inline_requires = "x".into();
        a.inline_ensures = "y".into();
        let mut b = inputs();
        b.inline_requires = "xy".into();
        b.inline_ensures = String::new();
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    /// The honesty rule, at the one place it can be enforced structurally:
    /// there is no way to read a recorded result without presenting today's
    /// hash for it.
    #[test]
    fn a_recorded_result_is_unreachable_without_a_matching_fingerprint() {
        let mut record = Record::new("0.1.0");
        record.record(
            "billing::tiered_fee",
            RecordEntry {
                fingerprint: "aaaa".into(),
                verdict: "bounded(2)".into(),
                statuses: vec![],
                evidence: None,
                diagnostics: vec![],
            },
        );
        assert!(record.matching("billing::tiered_fee", "aaaa").is_some());
        assert!(
            record.matching("billing::tiered_fee", "bbbb").is_none(),
            "a stored result must never be handed back against inputs that no longer match it"
        );
    }

    /// Everything this run did not reuse and did not earn goes, whether it
    /// left the document, stopped resolving, or simply produced no evidence
    /// this time.
    #[test]
    fn a_claim_the_document_no_longer_contains_is_dropped() {
        let mut record = Record::new("0.1.0");
        for id in ["a::f", "a::g"] {
            record.record(
                id,
                RecordEntry {
                    fingerprint: "h".into(),
                    verdict: "tested".into(),
                    statuses: vec![],
                    evidence: None,
                    diagnostics: vec![],
                },
            );
        }
        let live = std::collections::BTreeSet::from(["a::f".to_string()]);
        record.retain_claims(&live);
        assert_eq!(record.results.len(), 1);
        assert!(record.results.contains_key("a::f"));
    }

    #[test]
    fn a_record_survives_a_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ply.lock");
        let mut record = Record::new("0.1.0");
        record.record(
            "a::f",
            RecordEntry {
                fingerprint: "h".into(),
                verdict: "fuzzed(256)".into(),
                statuses: vec!["conditional".into()],
                evidence: Some(Evidence {
                    engine: "proptest".into(),
                    seed: Some("ab".repeat(32)),
                    cases: Some(256),
                }),
                diagnostics: vec![],
            },
        );
        save(&path, &record).unwrap();
        let back = load(&path, "0.1.0").unwrap();
        let entry = back.matching("a::f", "h").expect("round trip");
        assert_eq!(entry.verdict, "fuzzed(256)");
        assert_eq!(entry.statuses, vec!["conditional".to_string()]);
        assert_eq!(entry.evidence.as_ref().unwrap().cases, Some(256));
    }

    /// A committed file gets merge conflicts. Reporting that plainly is the
    /// difference between "re-paid four minutes of engine time for no stated
    /// reason" and one sentence naming the file and the fix.
    #[test]
    fn an_unreadable_record_says_so_rather_than_pretending_there_was_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ply.lock");
        std::fs::write(&path, "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> other\n").unwrap();
        let err = load(&path, "0.1.0").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("delete it and run `cargo ply verify` again"),
            "{msg}"
        );
    }

    /// A record from a format this build does not know is ignored, not
    /// misread: reusing a result whose meaning is unknown is exactly the
    /// failure the fingerprint exists to prevent.
    /// The record says which Ply wrote it, so a reader of the diff can see
    /// that without decoding a hash. It names the version that last wrote
    /// the file, which after any run is the version that produced every
    /// entry in it -- a fingerprint carrying a different Ply version can
    /// never match.
    #[test]
    fn the_record_names_the_ply_that_last_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ply.lock");
        let mut record = Record::new("0.1.0");
        record.record(
            "a::f",
            RecordEntry {
                fingerprint: "h".into(),
                verdict: "tested".into(),
                statuses: vec![],
                evidence: None,
                diagnostics: vec![],
            },
        );
        save(&path, &record).unwrap();
        let reloaded = load(&path, "0.1.1").unwrap();
        assert_eq!(reloaded.written_by, "0.1.1");
    }

    #[test]
    fn a_record_in_an_unknown_format_is_ignored_rather_than_misread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ply.lock");
        std::fs::write(
            &path,
            r#"{"format":99,"written_by":"9.9.9","results":{"a::f":{"fingerprint":"h","verdict":"proved"}}}"#,
        )
        .unwrap();
        let record = load(&path, "0.1.0").unwrap();
        assert!(record.matching("a::f", "h").is_none());
    }

    /// A run that reused everything must leave the working tree exactly as
    /// it found it -- a lock file rewritten on every run turns `git status`
    /// into noise and every CI job into a diff.
    #[test]
    fn saving_an_unchanged_record_does_not_rewrite_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ply.lock");
        let mut record = Record::new("0.1.0");
        record.record(
            "a::f",
            RecordEntry {
                fingerprint: "h".into(),
                verdict: "tested".into(),
                statuses: vec![],
                evidence: None,
                diagnostics: vec![],
            },
        );
        save(&path, &record).unwrap();
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        save(&path, &record).unwrap();
        let second = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(first, second, "an unchanged record must not be rewritten");
    }
}
