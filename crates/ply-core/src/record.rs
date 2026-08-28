//! The committed record of results, and the fingerprint that guards it
//! (The-Ply-Spec.md §5.2a, D14).
//!
//! A result Ply earned is written to `ply.lock` beside a hash of what the
//! answer depended on -- `INPUT_GROUPS` below is that list, and §5.2a names
//! the same one. Before that result is ever used or shown, the hash is
//! recomputed from today's inputs: it matches, the result is reused; it
//! does not, the check runs again and the record is rewritten.
//!
//! **The input that was missing, and what it cost.** Until 2026-08-25 the
//! list held the checked function's own tokens and the promises declared
//! for the callees a proof replaces, and nothing else the check *runs*. A
//! function with a contract, calling a plain local helper, could have that
//! helper broken and still report `fuzzed(64)` `[reused]` in 0.03s over
//! code a cold run reports as a violation. `reach::code_scope` is what
//! supplies the missing input; `reach`'s own module comment states the one
//! limit it cannot pass, and §5.2a states it to users.
//!
//! **A second input was missing, found 2026-08-26.** D5's first branch
//! (§5.5) lets a caller's proof stand on a same-crate callee's own earned
//! `bounded(k)`, composing to the weaker of the two bounds. Composing
//! against a number is depending on that number, and nothing hashed it: a
//! caller's record stayed unchanged when the callee's *declared checks*
//! moved in `ply.yaml` (no source touched anywhere), so editing a callee
//! from `bounded(5)` down to `bounded(2)` re-earned the callee's own record
//! correctly while the caller kept reporting its stale `bounded(5)` --
//! `[reused]`, standing on a callee this same run had just proved only to
//! depth 2. `verified_bounds` is the fixed input.
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
pub const FORMAT: u32 = 2;

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
    /// The callee's signature as the stub is generated from it. A promise
    /// is only half of what replaces the body: the other half is the shape
    /// of the value the proof invents in its place, and a widened return
    /// type changes the proof while the caller's own tokens, absorbing it
    /// through inference, do not move at all.
    pub signature: String,
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
    /// D5's first branch (§5.5): the same-crate callees this claim stands
    /// on rather than assumes, each with the `bounded(k)` it earned *this
    /// run* -- `(canonical path, k)`. Added 2026-08-26 after a review found
    /// the gap it closes: composing a caller's bound against a callee's
    /// earned depth is new evidence this claim's result depends on, and
    /// before this field existed nothing hashed it. Editing only the
    /// callee's *declared checks* in `ply.yaml` (`bounded(5)` down to
    /// `bounded(2)`) with no source touched anywhere re-earns the callee's
    /// own record correctly, but left the caller's record fingerprint
    /// unchanged -- so the caller kept reporting its old, deeper composed
    /// bound over a callee this same run had just proved shallower. That is
    /// the exact overclaim §5.5's bound-composition rule exists to refuse,
    /// arriving through the record instead of through composition.
    pub verified_bounds: Vec<(String, u32)>,
    /// The worked examples a `test` check compiles into assertions. Editing
    /// one changes what the check asserts, so it changes the result.
    pub examples: Vec<String>,
    /// How the code this check runs was covered: `reached` when Ply
    /// followed every path out of the function and can name the whole set,
    /// `whole-crate` when it could not and hashed all of the first-party
    /// source instead (`reach::CodeScope`).
    pub code_scope: String,
    /// `(label, token text)` for every first-party body in that scope.
    /// **This is the input whose absence made a stored result lie**: before
    /// 2026-08-25 a check that ran a plain local helper hashed the caller
    /// and not the helper, so breaking the helper and re-running produced a
    /// carried-forward pass over code a cold run proves broken.
    pub code: Vec<(String, String)>,
    /// The resolved identity of everything outside this workspace that the
    /// check runs or descends into (`reach::dependency_identity`).
    pub deps: String,
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

/// One group of hashed inputs, named the way a person would say it. The
/// fingerprint is taken over every group in this order; each group is also
/// hashed on its own, so a run that could not carry a result forward can
/// say **which** of them moved instead of silently re-paying engine cost
/// (§5.2a).
pub const INPUT_GROUPS: [&str; 12] = [
    "which claim this is",
    "the function's own source",
    "its contract",
    "the code it runs",
    "the worked examples it asserts",
    "the promises it assumes",
    "the callees it stands on",
    "the checks that ran",
    "the engines behind them",
    "the compiler and the build target",
    "the crate's features",
    "the versions of everything outside this workspace",
];

impl FingerprintInputs {
    /// The canonical bytes of one named group. Every value is written as
    /// `label`, its byte length, then its bytes, so no value can be
    /// mistaken for a field boundary: without the length prefix, a contract
    /// containing a newline could be arranged to hash the same as two
    /// different fields.
    fn group(&self, name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut put = |label: &str, value: &str| {
            out.extend_from_slice(label.as_bytes());
            out.push(0);
            out.extend_from_slice(value.len().to_string().as_bytes());
            out.push(0);
            out.extend_from_slice(value.as_bytes());
            out.push(0);
        };
        put("group", name);
        match name {
            "which claim this is" => {
                put("ply-fingerprint", "2");
                put("ply-version", &self.ply_version);
                put("node", &self.node_id);
                put("fn-path", &self.fn_path);
            }
            "the function's own source" => put("fn-source", &self.fn_source),
            "its contract" => {
                put("inline-requires", &self.inline_requires);
                put("inline-ensures", &self.inline_ensures);
                for r in &self.declared_requires {
                    put("declared-requires", r);
                }
                for e in &self.declared_ensures {
                    put("declared-ensures", e);
                }
            }
            "the code it runs" => {
                put("code-scope", &self.code_scope);
                for (label, tokens) in &self.code {
                    put("code-unit", label);
                    put("code-tokens", tokens);
                }
            }
            "the worked examples it asserts" => {
                for e in &self.examples {
                    put("example", e);
                }
            }
            "the promises it assumes" => {
                for a in &self.assumed {
                    put("assumed-callee", &a.callee);
                    for r in &a.requires {
                        put("assumed-requires", r);
                    }
                    for e in &a.ensures {
                        put("assumed-ensures", e);
                    }
                    put("assumed-signature", &a.signature);
                }
            }
            "the callees it stands on" => {
                for (path, k) in &self.verified_bounds {
                    put("verified-callee", path);
                    put("verified-bound", &k.to_string());
                }
            }
            "the checks that ran" => {
                for c in &self.checks {
                    put("check", c);
                }
                put("seed", &self.seed);
            }
            "the engines behind them" => {
                for e in &self.engines {
                    put("engine-name", &e.name);
                    put("engine-version", &e.version);
                    put("engine-flags", &e.flags);
                }
            }
            "the compiler and the build target" => {
                put("target", &self.target);
                put("rustc", &self.rustc);
            }
            "the crate's features" => put("features", &self.features),
            "the versions of everything outside this workspace" => put("deps", &self.deps),
            other => unreachable!("no such fingerprint input group: {other}"),
        }
        out
    }

    /// The canonical byte string this fingerprint is taken over: every
    /// group, in `INPUT_GROUPS` order.
    fn canonical(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for name in INPUT_GROUPS {
            out.extend_from_slice(&self.group(name));
        }
        out
    }

    /// A short digest per named group, stored beside the result. It is not
    /// what decides reuse -- the whole fingerprint is -- it exists so that
    /// a run which had to re-earn a result can name the input that moved.
    /// Sixteen hex characters: enough that an accidental collision would
    /// only mis-word an explanation, never change a verdict.
    pub fn per_group_digests(&self) -> BTreeMap<String, String> {
        INPUT_GROUPS
            .iter()
            .map(|name| {
                let hex = blake3::hash(&self.group(name)).to_hex().to_string();
                (name.to_string(), hex[..16].to_string())
            })
            .collect()
    }
}

/// The hash of every input in `INPUT_GROUPS`, as 64 hex characters. Not of
/// everything the answer depended on: `reach` states what a syntactic walk
/// cannot see, and §5.2a states what is deliberately left out.
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
    /// A short digest per named input group, for explanation only. It never
    /// decides whether a result may be reused -- the fingerprint does -- it
    /// is what lets a run that had to re-earn a result say which input
    /// moved, instead of re-paying engine cost for no stated reason.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
}

/// What a lookup in the record found for one claim.
#[derive(Debug)]
pub enum Match<'a> {
    /// Nothing stored, or something stored whose fingerprint no longer
    /// matches today's inputs. Either way the check runs.
    Miss,
    /// A stored result whose fingerprint matches and whose verdict is one
    /// the recorded checks could actually have earned.
    Hit(&'a RecordEntry),
    /// Stored, matching, and **impossible**: the verdict is not one those
    /// checks can produce, so it did not come from a run of Ply. Refused,
    /// and the claim is checked again. The payload is the sentence to show.
    ///
    /// A hash cannot defend a file against a text editor, and nothing short
    /// of signing could. This catches the honest version -- a merge that
    /// went wrong, a copied entry, a hand-edit meant as a note -- at the
    /// cost of one table lookup, rather than believing "proved" forever
    /// because a fingerprint nobody changed still matches.
    Impossible(String),
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
    /// handed in matches the recorded one *and* the stored verdict is one
    /// the checks handed in could actually have earned. There is no way to
    /// read an entry without presenting both: the honesty rule is enforced
    /// by the shape of this function, not by remembering to call a checker
    /// first.
    pub fn matching(
        &self,
        node_id: &str,
        fingerprint: &str,
        checks: &[String],
        verified_bounds: &[(String, u32)],
    ) -> Match<'_> {
        let Some(entry) = self
            .results
            .get(node_id)
            .filter(|e| e.fingerprint == fingerprint)
        else {
            return Match::Miss;
        };
        if !verdict_is_earnable(&entry.verdict, checks, verified_bounds) {
            return Match::Impossible(format!(
                "The recorded result for `{node_id}` says `{}`, and the checks recorded beside it \
                 ({}) cannot produce that answer. A result file Ply wrote never contains this, so \
                 something else edited it -- a merge that went wrong, or a hand edit. Ply ignored \
                 the stored result and ran the checks again; what you see below was earned just \
                 now.",
                entry.verdict,
                if checks.is_empty() {
                    "none".to_string()
                } else {
                    checks.join(", ")
                }
            ));
        }
        Match::Hit(entry)
    }

    /// Which named inputs moved since a stored result was recorded, when
    /// one was stored and could not be carried forward.
    ///
    /// This is the only other thing readable out of an entry, and it is
    /// deliberately not the result: it hands back the names of inputs and
    /// nothing a verdict could be assembled from, so the honesty rule --
    /// no stored verdict reaches a reader without being re-hashed -- still
    /// has exactly one way through.
    pub fn displaced_by(&self, node_id: &str, inputs: &FingerprintInputs) -> Option<Vec<String>> {
        let entry = self.results.get(node_id)?;
        let today = fingerprint(inputs);
        if entry.fingerprint == today || entry.inputs.is_empty() {
            return None;
        }
        let now = inputs.per_group_digests();
        let moved: Vec<String> = INPUT_GROUPS
            .iter()
            .filter(|name| entry.inputs.get(**name) != now.get(**name))
            .map(|name| name.to_string())
            .collect();
        (!moved.is_empty()).then_some(moved)
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

/// The verdicts a claim's checks can actually earn (§5.4c's check-to-verdict
/// table). A `fuzz` check yields `fuzzed(n)`, never `proved`; `mutate` adds
/// a suffix to a verdict another check earned and produces none of its own;
/// `prove` has no engine behind it yet and so earns nothing at all.
fn earnable_verdicts(checks: &[String]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for check in checks {
        if let Some(rest) = check.strip_prefix("fuzz(") {
            out.insert(format!("fuzzed({rest}"));
        } else if check == "test" {
            out.insert("tested".to_string());
        }
        // `bounded(k)` is handled separately in `verdict_is_earnable`, not
        // here: it is the one shape a declared check can earn a *weaker*
        // verdict than its own number under (D5's first branch composing
        // against a callee's shallower proof, §5.5), so exact-string
        // membership is the wrong test for it.
    }
    out
}

/// The suffix `mutate` adds to a verdict when the tests killed every
/// planted bug. It is the only decoration a stored verdict may carry.
const SPEC_STRONG: &str = "\u{00b7}spec-strong";

/// The `k` out of a `bounded(k)` string, or `None` for anything else.
fn bounded_k(s: &str) -> Option<u32> {
    s.strip_prefix("bounded(")?.strip_suffix(')')?.parse().ok()
}

/// `verified_bounds` is the *lookup-time* fingerprint's own D5 dependency
/// list (`FingerprintInputs::verified_bounds`) -- by the time this is
/// called the fingerprint has already matched (`matching`, above), which
/// pins those bounds to exactly the ones the stored verdict was earned
/// under. That makes the honestly-earnable value for a `bounded(k)` check
/// *computable*, not merely bounded above: `min(k, min of verified_bounds)`,
/// and nothing else.
pub fn verdict_is_earnable(
    verdict: &str,
    checks: &[String],
    verified_bounds: &[(String, u32)],
) -> bool {
    let (base, decorated) = match verdict.strip_suffix(SPEC_STRONG) {
        Some(base) => (base, true),
        None => (verdict, false),
    };
    if decorated && !checks.iter().any(|c| c == "mutate") {
        return false;
    }
    // A `bounded(k)` check is a *declared upper bound*, not a fixed exact
    // one -- D5's first branch (§5.5) can compose a claim's own proof down
    // to a shallower bound than `k`, when the callee it stands on was only
    // proved that far this run. Found by adversarial review 2026-08-26,
    // twice: first, that before this exception existed a claim declaring
    // `bounded(5)` that composed to a genuinely-earned `bounded(2)` was
    // refused as `W0516` "impossible" on every single subsequent run --
    // this integrity check, written before D5's first branch existed,
    // still assumed a `bounded(k)` check could only ever produce `bounded(k)`
    // verbatim, so a real composed result looked identical to a hand-edited
    // one and was silently re-earned from scratch forever. Second, that
    // the first fix for that (accepting *any* `bounded(j)` with `j <= k`)
    // was a needless superset: the honestly-earnable `j` is not a range,
    // it is one computed number, `min(k, min of verified_bounds)`, and the
    // review demonstrated a hand-edited `bounded(4)` sailing through the
    // wider check on a claim that had actually composed to `bounded(2)`.
    // Equality against the computed value is what an "impossible" check
    // means; a range was the easy version, not the true one.
    if let Some(j) = bounded_k(base) {
        let declared = checks.iter().filter_map(|c| bounded_k(c)).max();
        let Some(declared) = declared else {
            return false;
        };
        let expected = verified_bounds
            .iter()
            .map(|(_, k)| *k)
            .fold(declared, u32::min);
        return j == expected;
    }
    earnable_verdicts(checks).contains(base)
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
                signature: "(tier: u8) -> u32".into(),
            }],
            verified_bounds: vec![("g".into(), 5)],
            examples: vec!["tiered_fee(1) == 1".into()],
            code_scope: "reached".into(),
            code: vec![(
                "helper".into(),
                "pub fn helper (x : u32) -> u32 { x * 2 }".into(),
            )],
            deps: "proptest 1.8.0".into(),
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
                "the signature the stub replacing that callee is built from",
                Box::new(|i: &mut FingerprintInputs| {
                    i.assumed[0].signature = "(tier: u8) -> u64".into()
                }),
            ),
            (
                "the bound a stub_verified callee earned",
                Box::new(|i: &mut FingerprintInputs| i.verified_bounds[0].1 = 2),
            ),
            (
                "which callee a stub_verified dependency is for",
                Box::new(|i: &mut FingerprintInputs| i.verified_bounds[0].0 = "h".into()),
            ),
            (
                "the worked examples a `test` check asserts",
                Box::new(|i: &mut FingerprintInputs| {
                    i.examples = vec!["tiered_fee(1) == 2".into()]
                }),
            ),
            (
                "the body of a helper the check runs or descends into",
                Box::new(|i: &mut FingerprintInputs| {
                    i.code[0].1 = "pub fn helper (x : u32) -> u32 { x / 2 }".into()
                }),
            ),
            (
                "which bodies were in reach at all",
                Box::new(|i: &mut FingerprintInputs| {
                    i.code.push(("extra".into(), "fn e () {}".into()))
                }),
            ),
            (
                "how far Ply could bound what the check reaches",
                Box::new(|i: &mut FingerprintInputs| i.code_scope = "whole-crate".into()),
            ),
            (
                "the resolved versions of everything outside this workspace",
                Box::new(|i: &mut FingerprintInputs| i.deps = "proptest 1.9.0".into()),
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

    /// A build whose own identity is unknown, empty, or a placeholder must
    /// never coincide with a real build's fingerprint -- "unavailable
    /// invalidates, never matches" (the task this module's `ply_version`
    /// field exists for). `build.rs` never actually produces such a value
    /// (a build that cannot compute its own identity fails outright rather
    /// than falling back to one, by design -- see `crates/ply-cli/build.rs`),
    /// so this is the honest version of that guarantee at the layer a unit
    /// test can reach: nothing here special-cases an "unknown" string into
    /// matching anything, empty or otherwise -- it is just one more value
    /// the hash treats as ordinary content, and every mutation test above
    /// already establishes that *any* different `ply_version` moves the
    /// hash. Stated on its own because it is the one value a naive
    /// implementation might be tempted to special-case (treat "no version
    /// known" as a wildcard, or skip hashing it when empty) -- exactly the
    /// silent fallback the task warned would reintroduce the bug.
    #[test]
    fn an_unknown_or_empty_build_identity_never_matches_a_real_one() {
        let real = inputs();
        let mut unknown = inputs();
        unknown.ply_version = "unknown".into();
        let mut empty = inputs();
        empty.ply_version = String::new();

        let real_fp = fingerprint(&real);
        assert_ne!(
            real_fp,
            fingerprint(&unknown),
            "a placeholder \"unknown\" identity must not match a real build's fingerprint"
        );
        assert_ne!(
            real_fp,
            fingerprint(&empty),
            "an empty identity must not match a real build's fingerprint either"
        );
        assert_ne!(
            fingerprint(&unknown),
            fingerprint(&empty),
            "two different kinds of \"no real identity\" must not even match each other -- there \
             is no wildcard value that silently matches everything"
        );
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

    /// The bug report's own premise, proved rather than argued -- and proved
    /// against the *real* encoder, not a hand-guessed stand-in for it, so
    /// this test cannot pass by accident if a later edit to `group()`
    /// reintroduces the ambiguity.
    ///
    /// The attack: take the exact bytes a genuine second field
    /// (`declared-ensures` holding `"y"`) contributes to `group()`'s
    /// output -- pulled straight out of the real encoder by diffing two
    /// runs of it, not retyped by hand -- and splice them, separator
    /// included, into the *value* of a single `declared-requires` entry
    /// instead. If a value's content could ever be mistaken for a field
    /// boundary, this input would read back identically to genuinely
    /// having two fields.
    ///
    /// A minimal `label + 0 + value + 0` stand-in (no length prefix) is
    /// checked first, to confirm the same construction really does
    /// collide when nothing defends against it -- otherwise this would not
    /// be testing the thing the previous caution was worried about.
    #[test]
    fn smuggling_a_field_boundary_inside_a_value_does_not_forge_a_collision() {
        // ---- Half 1: the previous caution's premise, proved real. ----
        fn naive(label: &str, value: &str) -> Vec<u8> {
            let mut out = label.as_bytes().to_vec();
            out.push(0);
            out.extend_from_slice(value.as_bytes());
            out.push(0);
            out
        }
        let naive_two_fields = [
            naive("declared-requires", "x"),
            naive("declared-ensures", "y"),
        ]
        .concat();
        let naive_smuggled = "x\u{0}declared-ensures\u{0}y".to_string();
        let naive_one_field = naive("declared-requires", &naive_smuggled);
        assert_eq!(
            naive_two_fields, naive_one_field,
            "the premise: a naive label+value+separator scheme really is ambiguous here -- \
             this is the exact collision the previous caution was right to distrust"
        );

        // ---- Half 2: group()'s real, current encoding, attacked with its
        // own real output rather than a guess at what that output is. ----
        let mut base = inputs();
        base.declared_requires = vec!["x".into()];
        base.declared_ensures = vec![];

        let mut a = base.clone();
        a.declared_ensures = vec!["y".into()];

        // Everything `group("its contract")` emits for the "y" field and
        // nothing else: the extra suffix bytes `a` has beyond `base`.
        let base_group = base.group("its contract");
        let a_group = a.group("its contract");
        assert!(
            a_group.starts_with(&base_group),
            "adding one declared_ensures entry must only append bytes, never rewrite earlier ones"
        );
        // Trimmed by one byte: that final byte is the closing separator
        // `put` itself will append to *this* atom's own value once it is
        // wrapped up below, so keeping it too would double it up and add
        // an extra byte no genuine attack would have.
        let second_atom = &a_group[base_group.len()..a_group.len() - 1];

        // Splice those exact bytes into declared_requires's *only* value --
        // a separator byte stands in for the closing byte a genuinely
        // separate "x" atom would have had -- and drop the real second
        // field entirely.
        let mut smuggled_bytes = b"x".to_vec();
        smuggled_bytes.push(0);
        smuggled_bytes.extend_from_slice(second_atom);
        let smuggled = String::from_utf8(smuggled_bytes)
            .expect("every byte group() writes is either an ASCII label, an ASCII digit, or a \
                     value already given to it as a String, so re-assembling them stays valid UTF-8");

        let mut b = base.clone();
        b.declared_requires = vec![smuggled];
        // b.declared_ensures is still [] -- the "second field" only exists
        // smuggled inside the first one's value.

        assert_ne!(
            a.canonical(),
            b.canonical(),
            "group()'s real output for two separate fields must not equal its output for one \
             field whose value contains what the second field would have written"
        );
        assert_ne!(
            fingerprint(&a),
            fingerprint(&b),
            "and the fingerprints built from those bytes must therefore differ too"
        );
    }

    /// The same experiment against a different field shape: `code` loops
    /// two *different* labels per entry (`code-unit`, then `code-tokens`)
    /// rather than one label repeated. The attack bytes are again pulled
    /// out of the real encoder, not guessed.
    #[test]
    fn smuggling_a_second_code_unit_inside_one_value_does_not_forge_a_collision() {
        let mut base = inputs();
        base.code = vec![("helper".into(), "fn helper() {}".into())];

        let mut a = base.clone();
        a.code.push(("other".into(), "fn other() {}".into()));

        let base_group = base.group("the code it runs");
        let a_group = a.group("the code it runs");
        assert!(
            a_group.starts_with(&base_group),
            "adding one code entry must only append bytes, never rewrite earlier ones"
        );
        // Trimmed by one byte for the same reason as the test above: the
        // wrapping `put` call supplies its own closing byte for this atom.
        let second_pair = &a_group[base_group.len()..a_group.len() - 1];

        let mut smuggled_bytes = b"fn helper() {}".to_vec();
        smuggled_bytes.push(0);
        smuggled_bytes.extend_from_slice(second_pair);
        let smuggled = String::from_utf8(smuggled_bytes)
            .expect("group() only ever writes ASCII labels, ASCII digits, and the given values");

        let mut b = base.clone();
        b.code = vec![("helper".into(), smuggled)];

        assert_ne!(a.canonical(), b.canonical());
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    /// A value legitimately containing the record's own separator byte
    /// must still hash like any other value: stable on repeat, and every
    /// byte after the embedded separator still counted rather than
    /// silently dropped or truncated at it.
    #[test]
    fn a_value_containing_the_records_own_separator_byte_round_trips() {
        let sep = '\u{0}';
        let mut a = inputs();
        a.examples = vec![format!("has_a{sep}nul(\"quoted, text; too\") == 1")];

        assert_eq!(
            fingerprint(&a),
            fingerprint(&a),
            "hashing the same input twice must give the same answer"
        );

        let mut b = a.clone();
        b.examples[0].push('!');
        assert_ne!(
            fingerprint(&a),
            fingerprint(&b),
            "content after the embedded separator byte must still count -- it must not be \
             silently dropped or truncated at the first NUL"
        );
    }

    /// A pin against `inputs()`'s current fingerprint. The investigation
    /// above found `group()`'s length-prefixed encoding already safe, so no
    /// change to it was needed or made -- this value must therefore never
    /// move without a deliberate, reviewed change to the encoding. If it
    /// does move unreviewed, every result recorded in every committed
    /// `ply.lock` goes stale in the same run, silently.
    #[test]
    fn the_fingerprint_of_the_reference_fixture_is_pinned() {
        assert_eq!(
            fingerprint(&inputs()),
            "b871446bca4bb77c08f1365dd373a43129b56822042f9da5215d146666b95a36",
            "the encoding did not change, so this must not move; if it moved on purpose, every \
             ply.lock committed against the old encoding is now stale"
        );
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
                inputs: BTreeMap::new(),
            },
        );
        let checks = vec!["bounded(2)".to_string()];
        assert!(matches!(
            record.matching("billing::tiered_fee", "aaaa", &checks, &[]),
            Match::Hit(_)
        ));
        assert!(
            matches!(
                record.matching("billing::tiered_fee", "bbbb", &checks, &[]),
                Match::Miss
            ),
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
                    inputs: BTreeMap::new(),
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
                inputs: BTreeMap::new(),
            },
        );
        save(&path, &record).unwrap();
        let back = load(&path, "0.1.0").unwrap();
        let Match::Hit(entry) = back.matching("a::f", "h", &["fuzz(256)".to_string()], &[]) else {
            panic!("round trip")
        };
        assert_eq!(entry.verdict, "fuzzed(256)");
        assert_eq!(entry.statuses, vec!["conditional".to_string()]);
        assert_eq!(entry.evidence.as_ref().unwrap().cases, Some(256));
    }

    /// The cheap half of the hand-editing gap (§5.2a). A hash cannot stop a
    /// text editor, and nothing short of signing could -- but a verdict no
    /// check in the file could ever earn did not come from a run of Ply,
    /// and saying so costs a table lookup.
    #[test]
    fn a_verdict_the_recorded_checks_could_never_earn_is_not_earnable() {
        let fuzz = vec!["fuzz(64)".to_string()];
        assert!(verdict_is_earnable("fuzzed(64)", &fuzz, &[]));
        assert!(
            !verdict_is_earnable("proved", &fuzz, &[]),
            "sampling cannot mint the strongest verdict Ply has"
        );
        assert!(
            !verdict_is_earnable("bounded(2)", &fuzz, &[]),
            "nor a proof"
        );
        assert!(
            !verdict_is_earnable("fuzzed(256)", &fuzz, &[]),
            "nor more cases than the check asked for"
        );
        assert!(verdict_is_earnable(
            "bounded(2)",
            &["bounded(2)".to_string()],
            &[]
        ));
        assert!(verdict_is_earnable("tested", &["test".to_string()], &[]));
        assert!(
            !verdict_is_earnable("proved", &["prove".to_string()], &[]),
            "`prove` has no engine behind it, so it earns nothing at all"
        );
        assert!(
            verdict_is_earnable(
                "fuzzed(64)\u{00b7}spec-strong",
                &["fuzz(64)".to_string(), "mutate".to_string()],
                &[]
            ),
            "the one decoration a stored verdict may carry"
        );
        assert!(
            !verdict_is_earnable("fuzzed(64)\u{00b7}spec-strong", &fuzz, &[]),
            "and only when `mutate` actually ran"
        );
    }

    /// The one deliberate exception to "a verdict must match a check
    /// exactly": D5's first branch (§5.5) can compose a `bounded(k)`
    /// claim's proof down to a shallower bound than `k`, when the callee it
    /// stands on was only proved that far this run -- so a claim declaring
    /// `bounded(5)` earning `bounded(2)` is not tampering, it is exactly
    /// what standing on a `g` proved only to depth 2 means. Before this was
    /// recognised, `W0516` refused that genuine result as "impossible" on
    /// every single subsequent run (adversarial review, 2026-08-26) --
    /// caught by `stubverifiedstalebound_fixture`, not by anything already
    /// in this file.
    ///
    /// **The earnable value is exactly one number, never a range** (a
    /// second adversarial pass, same day): the first fix here accepted any
    /// `bounded(j)` with `j <= k`, which let a hand-edited `bounded(4)`
    /// through on a claim that had actually composed to `bounded(2)` --
    /// self-contradicting the very envelope it shipped in (`W0517` on the
    /// same node still said "capped ... at bounded(2)"). Equality against
    /// `min(declared, min(verified_bounds))` is the honest value; passing
    /// `verified_bounds` in is what makes it computable at lookup time
    /// rather than merely bounded above.
    #[test]
    fn a_bounded_check_earns_exactly_the_computed_value_never_a_range() {
        let bounded5 = vec!["bounded(5)".to_string()];
        assert!(
            verdict_is_earnable("bounded(5)", &bounded5, &[]),
            "the ordinary case: a claim's own declared bound, uncomposed"
        );
        assert!(
            verdict_is_earnable("bounded(2)", &bounded5, &[("g".to_string(), 2)]),
            "D5's first branch composing against a callee proved only to depth 2"
        );
        assert!(
            !verdict_is_earnable("bounded(4)", &bounded5, &[("g".to_string(), 2)]),
            "the exact tamper the review demonstrated: a stored bound deeper than what \
             the recorded dependency actually composed to, even though 4 <= 5"
        );
        assert!(
            !verdict_is_earnable("bounded(1)", &bounded5, &[("g".to_string(), 2)]),
            "shallower than the computed value is just as much a lie in the other \
             direction -- not merely a conservative one"
        );
        assert!(
            !verdict_is_earnable("bounded(6)", &bounded5, &[]),
            "never a bound deeper than what was declared -- that really would be a \
             hand-edited or otherwise impossible record"
        );
        assert!(
            verdict_is_earnable(
                "bounded(2)",
                &bounded5,
                &[("g".to_string(), 4), ("h".to_string(), 2)]
            ),
            "the minimum over every stood-on callee, not just the first one"
        );
    }

    /// A full-price re-run that says nothing about what changed is the
    /// experience the record exists to end. When one happens, the run can
    /// name the input that caused it.
    #[test]
    fn a_displaced_result_names_the_input_that_moved() {
        let mut record = Record::new("0.1.0");
        let before = inputs();
        record.record(
            &before.node_id.clone(),
            RecordEntry {
                fingerprint: fingerprint(&before),
                verdict: "bounded(2)".into(),
                statuses: vec![],
                evidence: None,
                diagnostics: vec![],
                inputs: before.per_group_digests(),
            },
        );
        let mut after = inputs();
        after.code[0].1 = "pub fn helper (x : u32) -> u32 { x / 2 }".into();
        assert_eq!(
            record.displaced_by(&after.node_id, &after),
            Some(vec!["the code it runs".to_string()]),
            "the helper moved, and nothing else did"
        );
        assert_eq!(
            record.displaced_by(&before.node_id, &before),
            None,
            "a result that still matches was not displaced by anything"
        );
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
                inputs: BTreeMap::new(),
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
        assert!(matches!(
            record.matching("a::f", "h", &[], &[]),
            Match::Miss
        ));
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
                inputs: BTreeMap::new(),
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
