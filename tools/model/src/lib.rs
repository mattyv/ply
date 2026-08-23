//! The `ply.yaml` serde model (SPEC.md §5) plus the three embedded
//! micro-syntaxes: check strings, edge strings, and deny strings (§5, items 1-3).
//!
//! Shared by `ply-render` (SVG drawing) and `ply-check` (document-local
//! validation). This is deliberately a read-only subset: only the
//! declarative constructs, not verify data — verdicts, statuses, and
//! fingerprints need anchored code and are out of scope for this crate.

use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub ply: u32,
    #[serde(default)]
    pub components: IndexMap<String, Component>,
    #[serde(default)]
    pub edges: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub profiles: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub unresolved: Vec<UnresolvedEntry>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub anchor: String,
    #[serde(default)]
    pub pure: bool,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub uses: Vec<String>,
    #[serde(default)]
    pub owns: Vec<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub components: IndexMap<String, Component>,
    #[serde(default)]
    pub fns: IndexMap<String, FnClaim>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Check,
    Synth,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FnClaim {
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub ensures: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub check_with: IndexMap<String, String>,
    #[serde(default)]
    pub trusted: Vec<TrustedClaim>,
    #[serde(default)]
    pub unresolved: Vec<UnresolvedEntry>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TrustedClaim {
    pub claim: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedEntry {
    pub id: u64,
    pub note: String,
}

/// A schema-valid document that failed to parse or validate. Carries the
/// underlying serde-YAML message (which already names the offending field
/// and, for `deny_unknown_fields`, suggests nothing further — SPEC.md §5.1a
/// leaves the nearest-known-key suggestion (E0204) to the full `ply check`
/// implementation; this renderer only needs to refuse cleanly).
#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

pub fn parse_document(yaml: &str) -> Result<Document, ParseError> {
    serde_yaml_ng::from_str(yaml).map_err(|e| ParseError(e.to_string()))
}

/// §5 item 1 / §5.1a rule 4: `test | fuzz(N)? | bounded(K)? | prove | mutate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Test,
    Fuzz(u32),
    Bounded(u32),
    Prove,
    Mutate,
}

pub fn parse_check(s: &str) -> Result<Check, String> {
    let s = s.trim();
    match s {
        "test" => return Ok(Check::Test),
        "prove" => return Ok(Check::Prove),
        "mutate" => return Ok(Check::Mutate),
        _ => {}
    }
    if let Some(inner) = s.strip_prefix("fuzz(").and_then(|r| r.strip_suffix(')')) {
        let n: u32 = inner
            .trim()
            .parse()
            .map_err(|_| format!("invalid fuzz(N) count in {s:?}"))?;
        if !(1..=1_000_000).contains(&n) {
            return Err(format!("fuzz(N) out of range 1..=1_000_000: {s:?}"));
        }
        return Ok(Check::Fuzz(n));
    }
    if let Some(inner) = s.strip_prefix("bounded(").and_then(|r| r.strip_suffix(')')) {
        let k: u32 = inner
            .trim()
            .parse()
            .map_err(|_| format!("invalid bounded(K) count in {s:?}"))?;
        if !(0..=64).contains(&k) { // FAULT 3: bounded(0) accepted
            return Err(format!("bounded(K) out of range 1..=64: {s:?}"));
        }
        return Ok(Check::Bounded(k));
    }
    Err(format!("unrecognized check string: {s:?}"))
}

/// §5 item 2: `A -> B` (call) or `A ~> B : path::Type` (data flow).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    Call,
    Flow(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

pub fn parse_edge(s: &str) -> Result<Edge, String> {
    let s = s.trim();
    if let Some(idx) = s.find("~>") {
        let (left, right) = (s[..idx].trim(), s[idx + 2..].trim());
        let mut parts = right.splitn(2, ':');
        let to = parts.next().unwrap_or("").trim();
        let ty = parts
            .next()
            .ok_or_else(|| format!("expected ': Type' after '~>' in {s:?}"))?
            .trim();
        if left.is_empty() || to.is_empty() || ty.is_empty() {
            return Err(format!("malformed data-flow edge: {s:?}"));
        }
        return Ok(Edge {
            from: left.to_string(),
            to: to.to_string(),
            kind: EdgeKind::Flow(ty.to_string()),
        });
    }
    if let Some(idx) = s.find("->") {
        let (left, right) = (s[..idx].trim(), s[idx + 2..].trim());
        if left.is_empty() || right.is_empty() {
            return Err(format!("malformed call edge: {s:?}"));
        }
        return Ok(Edge {
            from: left.to_string(),
            to: right.to_string(),
            kind: EdgeKind::Call,
        });
    }
    Err(format!("edge string has no '->' or '~>': {s:?}"))
}

/// §5 item 3: `PAT -> PAT [except C1, C2]` where `PAT := IDENT | *`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deny {
    pub from: String,
    pub to: String,
    pub except: Vec<String>,
}

pub fn parse_deny(s: &str) -> Result<Deny, String> {
    let s = s.trim();
    let (main, except_part) = match s.find("except") {
        Some(idx) => (s[..idx].trim(), s[idx + "except".len()..].trim()),
        None => (s, ""),
    };
    let idx = main
        .find("->")
        .ok_or_else(|| format!("deny string missing '->': {s:?}"))?;
    let from = main[..idx].trim();
    let to = main[idx + 2..].trim();
    if from.is_empty() || to.is_empty() {
        return Err(format!("malformed deny rule: {s:?}"));
    }
    let except: Vec<String> = if except_part.is_empty() {
        Vec::new()
    } else {
        except_part
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    };
    Ok(Deny {
        from: from.to_string(),
        to: to.to_string(),
        except,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_glyphs_parse() {
        assert_eq!(parse_check("test").unwrap(), Check::Test);
        assert_eq!(parse_check("prove").unwrap(), Check::Prove);
        assert_eq!(parse_check("mutate").unwrap(), Check::Mutate);
        assert_eq!(parse_check("fuzz(1024)").unwrap(), Check::Fuzz(1024));
        assert_eq!(parse_check("bounded(3)").unwrap(), Check::Bounded(3));
    }

    #[test]
    fn check_bounds_are_enforced() {
        assert!(parse_check("fuzz(0)").is_err());
        assert!(parse_check("fuzz(1000001)").is_err());
        assert!(parse_check("bounded(0)").is_err());
        assert!(parse_check("bounded(65)").is_err());
        assert!(parse_check("nonsense").is_err());
    }

    #[test]
    fn call_edge_parses() {
        let e = parse_edge("pricing -> parser").unwrap();
        assert_eq!(e.from, "pricing");
        assert_eq!(e.to, "parser");
        assert_eq!(e.kind, EdgeKind::Call);
    }

    #[test]
    fn flow_edge_parses_with_type_label() {
        let e = parse_edge("pricing ~> risk : pricing::Quote").unwrap();
        assert_eq!(e.from, "pricing");
        assert_eq!(e.to, "risk");
        assert_eq!(e.kind, EdgeKind::Flow("pricing::Quote".to_string()));
    }

    #[test]
    fn edge_without_arrow_is_rejected() {
        assert!(parse_edge("pricing parser").is_err());
    }

    #[test]
    fn deny_rule_with_wildcard_and_except_parses() {
        let d = parse_deny("* -> db_raw except migrations").unwrap();
        assert_eq!(d.from, "*");
        assert_eq!(d.to, "db_raw");
        assert_eq!(d.except, vec!["migrations".to_string()]);
    }

    #[test]
    fn deny_rule_without_except_parses() {
        let d = parse_deny("a -> b").unwrap();
        assert_eq!(d.from, "a");
        assert_eq!(d.to, "b");
        assert!(d.except.is_empty());
    }
}
