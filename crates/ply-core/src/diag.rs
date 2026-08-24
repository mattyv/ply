//! Diagnostic types and the §8 JSON envelope. This slice implements the
//! shape needed for `verify`'s output, not the full exhaustive code
//! registry (§8's "one exhaustive enum" is a later-milestone concern).
//!
//! D7 rename applied here, pre-M3 as §8's stability rule permits:
//! `counterexample.kani_playback` -> `counterexample.kani_witness`.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Span {
    pub file: String,
    pub start: [u32; 2],
    pub end: [u32; 2],
}

#[derive(Debug, Clone, Serialize)]
pub struct Counterexample {
    /// Rendered-source values for each parameter, e.g. `{"x": "4294967295u32"}`.
    pub inputs: BTreeMap<String, String>,
    /// Input storage: the exact failing bytes, engine-version-bound. Never a
    /// reproduction (D7/ADR-0003 caveat 3) -- kept as text describing where
    /// it was captured from, since this slice does not persist a separate
    /// artifact file for it beyond the rendered `#[test]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kani_witness: Option<String>,
    /// Present only when the inputs rendered as stable Rust source (D7);
    /// else absent and a `W0541` diagnostic explains why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo_test: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub phase: String,
    pub engine: String,
    pub check: String,
    pub node_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_span: Option<Span>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<Counterexample>,
    pub open_item: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub verdict: String,
    pub statuses: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub command: String,
    pub ply_version: String,
    pub root: Node,
    pub diagnostics: Vec<Diagnostic>,
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
        assert!(!json.contains("kani_playback"), "D7's rename must not regress");
    }
}
