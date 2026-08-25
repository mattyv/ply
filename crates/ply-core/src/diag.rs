//! Diagnostic types and the §8 JSON envelope. This slice implements the
//! shape needed for `verify`'s output, not the full exhaustive code
//! registry (§8's "one exhaustive enum" is a later-milestone concern).
//!
//! D7 rename applied here, pre-M3 as §8's stability rule permits:
//! `counterexample.kani_playback` -> `counterexample.kani_witness`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    pub start: [u32; 2],
    pub end: [u32; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counterexample {
    /// Rendered-source values for each parameter, e.g. `{"x": "4294967295u32"}`.
    pub inputs: BTreeMap<String, String>,
    /// Input storage: the exact failing bytes, engine-version-bound. Never a
    /// reproduction (D7/ADR-0003 caveat 3) -- kept as text describing where
    /// it was captured from, since this slice does not persist a separate
    /// artifact file for it beyond the rendered `#[test]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kani_witness: Option<String>,
    /// Present only when the inputs rendered as stable Rust source (D7);
    /// else absent and a `W0541` diagnostic explains why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_test: Option<String>,
}

/// One suggested repair (§8): "Ply proposes, never rewrites" -- a `Fix` is
/// always a suggestion the caller may apply and a human may review, never
/// something Ply applies itself. `edits` is left empty when Ply can name
/// *what* would help (lower the bound, switch check kind, add a `requires`)
/// but does not have a concrete source edit to offer -- an empty `edits` is
/// still a real fix entry, not a placeholder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fix {
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<Edit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edit {
    pub span: String,
    pub insert: String,
}

/// One assumption a verdict rests on (§8's `assumptions` array). Today the
/// only kind is `assumed_contract`: D5's second branch (§5.5) stubbed a
/// callee out of a proof and trusted its declared contract instead of its
/// body. `verdict` is what the callee itself earned -- `unclaimed` for a
/// legacy callee nothing has checked, which is exactly the case that makes
/// the assumption *owed evidence* rather than settled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assumption {
    pub kind: String,
    #[serde(rename = "fn")]
    pub fn_path: String,
    pub verdict: String,
    /// The contract text being assumed, as declared.
    pub contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub phase: String,
    pub engine: String,
    pub check: String,
    pub node_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_span: Option<Span>,
    /// §5: a schema violation carries "the JSON-pointer path" into the
    /// document — `/components/pricing/fns/quote/ensure`. Present on
    /// `E0201`/`E0204`, absent on everything else, since a diagnostic about
    /// a *function* points at source, not at YAML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<Counterexample>,
    /// §8's non-result MUST: `timeout`/`unsupported`/`engine-missing` (and,
    /// per M4, `weak-spec`) SHOULD populate this with the concrete options
    /// a repair would need -- never left for the reader to guess from prose
    /// alone.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fixes: Vec<Fix>,
    /// §8's `assumptions` array -- present only on a verdict that rests on
    /// one (D5's second branch, §5.5). Empty for every other diagnostic.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub assumptions: Vec<Assumption>,
    pub open_item: Option<String>,
}

/// How a node's verdict was produced, concretely enough to reproduce it
/// (§1, 2026-08-25). A fuzz verdict carries its seed and case count the way
/// a violation carries its witness: without it, a `fuzzed(256)` names no
/// run anyone can repeat, and the run that missed a bug is indistinguishable
/// from the run that could not have found one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub engine: String,
    /// The proptest RNG seed, as 64 hex characters -- replay with
    /// `cargo ply verify <path> --seed <hex>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
    /// Cases the engine actually reached -- never the number the checks list
    /// asked for. Absent when the run happened but the count is not the
    /// declared one and not knowable either: a run cut short by its time
    /// budget, or stopped at the first failing case. The whole `Evidence`
    /// block is absent when no run happened at all (2026-08-25: it used to
    /// be attached whenever `fuzz(n)` was declared, so a harness that never
    /// compiled reported `cases: n` for a run of zero).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cases: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub verdict: String,
    pub statuses: Vec<String>,
    /// Whether this result was carried forward from an earlier run rather
    /// than earned in this one (§5.2a). Present only when true, never
    /// `false`: absent means "this ran just now".
    ///
    /// **Not a status** (D6). A status qualifies the evidence — how strong
    /// it is, what it rests on — and travels upward as a flag. Reuse
    /// qualifies nothing: it says *when* the run happened. It stays on the
    /// node that earned it, enters neither the evidence order nor any exit
    /// code, and can only be set after the node's fingerprint was
    /// recomputed from today's inputs and matched (`record::Record::matching`).
    #[serde(skip_serializing_if = "is_false")]
    pub reused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A name that reports **no evidence** (§1): the engine was exhausted, the
/// shape was out of reach, the tool broke, nothing was claimed, no engine
/// existed, or a check ran and settled nothing.
///
/// **An absence is a name, not a slot.** The same names appear in two places
/// in a §8 node — as its `verdict`, and as a `status` beside it (D6) — and
/// they mean the same thing in both. This vocabulary lives here, in one
/// place, because two consumers now read it: the exit-code rule in the CLI,
/// and the rule that decides whether a result may be recorded at all
/// (§5.2a records only results that earned evidence). Two copies of a
/// vocabulary is how the next absence gets missed by one of them.
pub fn is_absence(name: &str) -> bool {
    name == "timeout"
        || name == "unclaimed"
        || name == "engine-missing"
        || name == "inconclusive"
        || name.starts_with("unsupported")
        || name.starts_with("tool_error")
}

/// One tier of what a command checks, and what that tier found or why it
/// found nothing.
#[derive(Debug, Clone, Serialize)]
pub struct Tier {
    pub tier: String,
    pub detail: String,
}

/// What this run actually covered, and what it did not.
///
/// §6 lists three tiers for `check` — schema, anchors, architecture —
/// and one of them does not exist yet. A command that only
/// reports findings lets a clean run read as full coverage, which is the
/// same failure as an absence of evidence reported as a pass (§1). So the
/// envelope carries the gaps as data, and the human surface prints them.
///
/// Absent on `verify`, whose envelope is unchanged.
#[derive(Debug, Clone, Serialize)]
pub struct Coverage {
    pub checked: Vec<Tier>,
    pub not_checked: Vec<Tier>,
}

/// One entry on `cargo ply audit`'s trust surface (§6): something a
/// verdict in this codebase rests on that Ply itself never checked.
///
/// **Not a diagnostic.** Every entry is a decision somebody made on
/// purpose — an assumed boundary contract (§5.5), an attested claim
/// (§5.4d), a helper a contract calls (§5.4a), an escape from a ban
/// (§5.3), a derived body (§5.7), an assumption about the world outside
/// (§5.1's `entry:`). They are listed so the decision stays visible, never
/// so a user is pushed into deleting one: an honest declaration that gets
/// reported as a failure is a declaration people learn to stop writing.
#[derive(Debug, Clone, Serialize)]
pub struct TrustItem {
    /// Which tier this belongs to (`assumed_contract`, `trusted_claim`, …).
    pub kind: String,
    /// What is being trusted: the callee, the claim, the helper, the ban.
    pub subject: String,
    /// The §7 node whose evidence rests on it, or where it is declared.
    pub node_id: String,
    /// D6 statuses this item carries — `owed-evidence` on an assumption
    /// nothing has exercised, `staleness-unknown` on an attestation Ply
    /// cannot date.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub statuses: Vec<String>,
    /// `file:line`, where the item came from source rather than ply.yaml.
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    pub where_: Option<String>,
    /// The plain sentence a reader needs: what is trusted, what rests on
    /// it, and what would settle it.
    pub detail: String,
}

/// One entry on `cargo ply worklist` (§6): something that is owed and
/// expected to close — an unresolved marker (§5.6), a weak spec (`W0502`).
///
/// The distinction from [`TrustItem`] is the whole design: trust surface is
/// permanent and listing it must never read as a demand, while an open item
/// is a thing somebody intends to finish.
#[derive(Debug, Clone, Serialize)]
pub struct OpenItem {
    pub kind: String,
    /// The unresolved id (§5.6), where the item has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// The §7 node it sits in — the fn or component, or `ply.yaml` for a
    /// registry entry with no code behind it.
    pub node_id: String,
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    pub where_: Option<String>,
    /// What this item blocks right now, in one line (§5.6's "blocking
    /// status").
    pub blocking: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub command: String,
    pub ply_version: String,
    pub root: Node,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
    /// §6's `audit`. Absent on every other command — an empty array means
    /// "this crate rests on nothing Ply can see", which is a different fact
    /// from "this command does not report a trust surface".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_surface: Option<Vec<TrustItem>>,
    /// §6's `worklist`, with the same absent/empty distinction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_items: Option<Vec<OpenItem>>,
    /// Claims that had a recorded result and could not use it, each with
    /// the names of the inputs that moved (§5.2a). Empty on every command
    /// but `verify`, and on a `verify` that carried everything forward or
    /// had nothing recorded to carry.
    ///
    /// A full re-run with no explanation is the experience the record
    /// exists to end, arriving without a reason the moment a compiler or an
    /// engine updates. This is the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_carried_forward: Vec<NotCarriedForward>,
}

/// One claim whose recorded result could not be reused, and why.
#[derive(Debug, Clone, Serialize)]
pub struct NotCarriedForward {
    pub node_id: String,
    /// The named inputs whose content changed since the result was
    /// recorded, in plain words ("the code it runs", "the compiler and the
    /// build target").
    pub because: Vec<String>,
}

impl Envelope {
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("Envelope always serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counterexample_field_is_kani_witness_not_kani_playback() {
        let cex = Counterexample {
            inputs: BTreeMap::new(),
            kani_witness: Some("captured".into()),
            cargo_test: None,
        };
        let json = serde_json::to_string(&cex).unwrap();
        assert!(json.contains("kani_witness"));
        assert!(
            !json.contains("kani_playback"),
            "D7's rename must not regress"
        );
    }
}
