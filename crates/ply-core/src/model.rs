//! The `ply.yaml` serde model (The-Ply-Spec.md §5) plus the three embedded
//! micro-syntaxes: check strings, edge strings, and deny strings (§5, items 1-3).
//!
//! This is the product's model layer (§4's `model/` row). It was written
//! first in `tools/model`, for the spec-validation tooling, while
//! `ply-core` carried a hand-rolled four-struct subset of the same format —
//! two readers disagreeing about one document, which is exactly the defect
//! §5.1a rule 1 was amended to name. Phase 1a promoted this one and deleted
//! the subset; `tools/render` and `tools/check` now consume it from here.
//!
//! Order is preserved (`IndexMap`, not `BTreeMap`): the renderer lays
//! components out in the order the author declared them, so a document's
//! reading order and its picture agree.

use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub ply: u32,
    #[serde(default)]
    pub components: IndexMap<String, Component>,
    /// docs/plans/external-elements.md §3: named outside parties (systems,
    /// people) this codebase talks to but Ply never verifies. Top-level
    /// only — an external has no interior and cannot nest. Shares the
    /// component reference namespace for `~>` edge/`entry:` resolution
    /// (§5.1a rule 6 applies unchanged), but is never a node of the §7
    /// verdict tree.
    #[serde(default)]
    pub externals: IndexMap<String, External>,
    #[serde(default)]
    pub edges: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub profiles: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub unresolved: Vec<UnresolvedEntry>,
    /// §5.4b's generator hook, promised since the first spike and built
    /// here: names a public function -- free or associated, Ply's resolver
    /// does not care which -- that returns a value of the given type. This
    /// is the one line an author (or an agent reading the code) writes to
    /// lift a type with no other way in into the set Ply can build values
    /// of: Ply samples the *named function's own parameters*, the same way
    /// it already builds any other value, and calls it -- an author never
    /// lists values by hand. Top-level only, keyed by the bare type name
    /// (`Handle`, not a module-qualified path): the resolver that reads
    /// this already scans the whole crate for where a type is declared, the
    /// same way it does for a type's own constructor.
    #[serde(default)]
    pub routes: IndexMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub anchor: String,
    /// Why this component exists and why its rules are what they are --
    /// read by people and by agents, checked by nothing.
    ///
    /// Every other prose slot in this grammar sits exactly where checking
    /// is impossible: an outside party (`externals`, whose note is
    /// *required*, because "a bare name tells a newbie nothing"), an unmade
    /// decision (`unresolved`), a human's word (`trusted`). This is the
    /// fourth, and it was argued for rather than assumed. A component's
    /// rules -- why the macro crate must stand alone, why the core must
    /// never reach up into the command-line layer -- have no contract form
    /// and no engine will ever consume them, yet they are exactly what a
    /// reader or an agent needs and what the format currently discards:
    /// Ply's own document was forty lines of comment to twelve of
    /// configuration, every one dropped by the parser.
    ///
    /// Deliberately **not** offered on a function, and the reason is a
    /// failure seen in use rather than a principle: someone wrote a real
    /// invariant as an `examples` string -- a test case wearing a
    /// specification's clothes -- because no better slot was visible. The
    /// answer there is `ensures`, which a reader and an engine can both act
    /// on. A prose slot beside it makes that mistake comfortable: the next
    /// invariant lands in the note and never becomes a promise.
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub pure: bool,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub uses: Vec<String>,
    #[serde(default)]
    pub owns: Vec<String>,
    /// §5.1's `state:` -- the structure this component holds, and which of
    /// its fields are worth drawing.
    #[serde(default)]
    pub state: Option<StateClaim>,
    #[serde(default)]
    pub profile: Option<String>,
    /// §5.1's "optional default checks for all fns in scope". `None` is *no
    /// list written*; `Some([])` is an empty list written on purpose, which
    /// means "check nothing" and is not the same statement (§5.4c).
    #[serde(default)]
    pub checks: Option<Vec<String>>,
    #[serde(default)]
    pub components: IndexMap<String, Component>,
    #[serde(default)]
    pub fns: IndexMap<String, FnClaim>,
}

/// §5.1's `state:`: the structure a component holds.
///
/// `show:` carries field **names** only, never their types. The shapes come
/// from the real source, which is the entire point: a document that restated
/// them would be a second hand-maintained copy of what the compiler already
/// owns, wrong the first time somebody changed a field. Naming a field the
/// type does not declare is `A0415`, and naming a type the crate does not
/// declare is `A0414` -- neither is a picture with a gap in it, both are the
/// document claiming something about code that is not there.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StateClaim {
    /// The type this component's state lives in, resolved under the
    /// component's own anchor.
    pub of: String,
    /// The fields worth drawing. Empty -- the default -- draws the header
    /// line alone: a real state struct has twenty fields and two that
    /// matter, and choosing is the author's job, not Ply's.
    #[serde(default)]
    pub show: Vec<String>,
    /// What must be true of this value at all times, written as Rust
    /// expressions over it (2026-09-04).
    ///
    /// `show:` names fields so a reader can see them; this says something
    /// about them that a run can be wrong about. §5.4c admits that a type's
    /// own invariants are **assumed, never asserted**, so a proof can rest
    /// on "the bids are sorted" while the code quietly breaks it. A clause
    /// here is that assumption written down and checked: Ply builds a value
    /// through the type's own constructor, calls the type's own operations
    /// on it, and asserts every clause after each one.
    ///
    /// Each clause is either a closure naming the value (`|book|
    /// book.bids.len() <= book.cap`) or a bare expression that calls it
    /// `state` (`state.len() <= 7`) -- the same two forms `ensures:` accepts,
    /// with the same reason: a reader who has written one has written both.
    #[serde(default)]
    pub holds: Vec<String>,
}

/// docs/plans/external-elements.md §3: a named outside party — a system or
/// person this codebase talks to but Ply never verifies. `note:` is
/// required: an external is nothing but a name and a sentence, and a bare
/// name tells a newbie nothing.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct External {
    pub note: String,
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
    /// This claim's own `checks:`. **`None` and `Some([])` are different
    /// statements** and Ply keeps them apart (§5.4c): no list at all leaves
    /// the choice to the nearest component default, or failing that to the
    /// shape-aware default; an empty list is the author saying "check
    /// nothing here", and nothing runs. Reading them as one was a silent
    /// way of proving a function whose document said not to.
    #[serde(default)]
    pub checks: Option<Vec<String>>,
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
    /// docs/plans/external-elements.md §3: names the externals that can
    /// reach this fn — turns its `requires` clauses into environmental
    /// assumptions (audit-only; no verdict change). Each name must resolve
    /// to a declared external (checked in `ply-check`, not here).
    #[serde(default)]
    pub entry: Vec<String>,
}

/// §5.1 `checks: [bounded(2)] # optional default checks for all fns in
/// scope`: the default a component declares for any fn in its subtree that
/// has none of its own. Carries the component's own (unqualified) name
/// alongside the list purely so a caller can name the source in a
/// diagnostic or tooltip — [`effective_checks`] itself compares nothing but
/// the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InheritedChecks<'a> {
    pub from_component: &'a str,
    pub checks: &'a [String],
}

/// §5.1: the checks list that actually governs one fn, or `None` when the
/// document declares no list for it anywhere.
///
/// A fn's own `checks` wins *entirely* — there is no merge with anything
/// declared above it, and **an empty list is a list**: `checks: []` says
/// "check nothing here" and overrides an ancestor default the same way a
/// full list does (§5.4c). In particular, D12's "`mutate` requires a
/// `test`/`fuzz` entry in the same list" is checked against this effective
/// list: an inherited `[test]` does not save a fn-level `[mutate]]` that
/// declares no `test`/`fuzz` of its own, because the fn's own list replaces
/// the inherited one rather than joining it.
///
/// A fn with no `checks` of its own inherits `inherited` — the nearest
/// ancestor component's own declared `checks` (its own component first,
/// then that component's parent, and so on up, see
/// [`component_default_checks`]). `None` means no ancestor ever declared
/// one either, which is the only case a caller may fill in with a default
/// of its own: an empty *declared* list is an answer, not an absence.
pub fn effective_checks<'a>(
    fc: &'a FnClaim,
    inherited: Option<InheritedChecks<'a>>,
) -> Option<&'a [String]> {
    match fc.checks.as_deref() {
        Some(own) => Some(own),
        None => inherited.map(|d| d.checks),
    }
}

/// The default that `comp`'s own fns — and any descendant component that
/// declares none of its own — inherit: `comp`'s own `checks`, tagged with
/// its `name`, if it declared any list at all (an empty one included);
/// otherwise whatever `comp` itself inherited from further up (which may
/// itself be `None`, all the way to the root).
pub fn component_default_checks<'a>(
    name: &'a str,
    comp: &'a Component,
    inherited: Option<InheritedChecks<'a>>,
) -> Option<InheritedChecks<'a>> {
    match comp.checks.as_deref() {
        Some(checks) => Some(InheritedChecks {
            from_component: name,
            checks,
        }),
        None => inherited,
    }
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
/// and, for `deny_unknown_fields`, suggests nothing further — The-Ply-Spec.md §5.1a
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

impl FnClaim {
    /// This claim's own literal `checks:` strings, parsed. An entry that
    /// fails the micro-syntax is `E0203` (§5.1a rule 4); the returned string
    /// is the plain-language reason, which the caller prefixes with the
    /// code. Inheritance is NOT applied here — call [`effective_checks`]
    /// first when the governing list is what you want.
    pub fn parsed_checks(&self) -> Result<Vec<Check>, String> {
        self.checks
            .iter()
            .flatten()
            .map(|s| parse_check(s))
            .collect()
    }
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

/// The count inside `fuzz(...)`/`bounded(...)`, in exactly the form
/// `schema/ply.schema.json` declares: digits only, no sign, no surrounding
/// spaces, no leading zeros. `Some(0)` is returned for a literal `0` so the
/// caller can say "must be between 1 and N" rather than "not a number" —
/// a zero is a range mistake, not a typing mistake.
///
/// Rust's own `u32::from_str` is looser than this (it takes `+5` and
/// `0256`), which is how the parser and the schema disagreed until
/// 2026-08-25. The schema is normative, so the parser narrowed to match it.
fn canonical_count(inner: &str) -> Option<u32> {
    if inner.is_empty() || !inner.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if inner.len() > 1 && inner.starts_with('0') {
        return None;
    }
    inner.parse().ok()
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
        let n = canonical_count(inner).ok_or_else(|| {
            format!(
                "{s:?} is not a valid check: the part in brackets is how many random inputs \
                 get tried, and it has to be a plain whole number written in digits, like \
                 fuzz(256)"
            )
        })?;
        if !(1..=1_000_000).contains(&n) {
            return Err(format!(
                "{s:?} is not a valid check: the number is how many random inputs get tried, \
                 and it must be between 1 and 1,000,000"
            ));
        }
        return Ok(Check::Fuzz(n));
    }
    if let Some(inner) = s.strip_prefix("bounded(").and_then(|r| r.strip_suffix(')')) {
        let k = canonical_count(inner).ok_or_else(|| {
            format!(
                "{s:?} is not a valid check: the part in brackets is how many times loops are \
                 unrolled during the proof, and it has to be a plain whole number written in \
                 digits, like bounded(3)"
            )
        })?;
        if !(1..=64).contains(&k) {
            return Err(format!(
                "{s:?} is not a valid check: the number is how many times loops are unrolled \
                 during the proof, and it must be between 1 and 64 — a bound of 0 would prove \
                 nothing"
            ));
        }
        return Ok(Check::Bounded(k));
    }
    Err(format!(
        "{s:?} is not a check Ply knows: expected test, fuzz(N), bounded(K), prove, or mutate"
    ))
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
            .ok_or_else(|| {
                format!(
                    "{s:?} is missing its payload type: a data-flow edge is written \
                     \"a ~> b : Type\""
                )
            })?
            .trim();
        if left.is_empty() || to.is_empty() {
            return Err(format!(
                "{s:?} is missing the component name on one side of the arrow"
            ));
        }
        if ty.is_empty() {
            return Err(format!(
                "{s:?} is missing its payload type: a data-flow edge is written \"a ~> b : Type\""
            ));
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
            return Err(format!(
                "{s:?} is missing the component name on one side of the arrow"
            ));
        }
        return Ok(Edge {
            from: left.to_string(),
            to: right.to_string(),
            kind: EdgeKind::Call,
        });
    }
    Err(format!(
        "{s:?} is not an edge: expected \"a -> b\" (a may call b) or \"a ~> b : Type\" \
         (data flows from a to b)"
    ))
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
    let idx = main.find("->").ok_or_else(|| {
        format!("{s:?} is not a deny rule: expected \"a -> b\" or \"a -> b except c, d\"")
    })?;
    let from = main[..idx].trim();
    let to = main[idx + 2..].trim();
    if from.is_empty() || to.is_empty() {
        return Err(format!(
            "{s:?} is missing the component name on one side of the arrow"
        ));
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

    /// Every diagnostic a user can meet must read as plain language: say
    /// what is wrong and why it matters, no bare spec ranges. These pin the
    /// exact wording (not just "it errored") so a future edit can't quietly
    /// regress back to jargon.
    #[test]
    fn fuzz_out_of_range_message_is_plain() {
        assert_eq!(
            parse_check("fuzz(0)").unwrap_err(),
            "\"fuzz(0)\" is not a valid check: the number is how many random inputs get tried, \
             and it must be between 1 and 1,000,000"
        );
    }

    #[test]
    fn bounded_out_of_range_message_is_plain() {
        assert_eq!(
            parse_check("bounded(0)").unwrap_err(),
            "\"bounded(0)\" is not a valid check: the number is how many times loops are \
             unrolled during the proof, and it must be between 1 and 64 — a bound of 0 would \
             prove nothing"
        );
    }

    #[test]
    fn unknown_check_message_is_plain() {
        assert_eq!(
            parse_check("nonsense").unwrap_err(),
            "\"nonsense\" is not a check Ply knows: expected test, fuzz(N), bounded(K), prove, \
             or mutate"
        );
    }

    #[test]
    fn edge_without_arrow_message_is_plain() {
        assert_eq!(
            parse_edge("pricing parser").unwrap_err(),
            "\"pricing parser\" is not an edge: expected \"a -> b\" (a may call b) or \
             \"a ~> b : Type\" (data flows from a to b)"
        );
    }

    #[test]
    fn flow_edge_missing_payload_type_message_is_plain() {
        assert_eq!(
            parse_edge("a ~> b").unwrap_err(),
            "\"a ~> b\" is missing its payload type: a data-flow edge is written \"a ~> b : Type\""
        );
    }

    #[test]
    fn flow_edge_blank_payload_type_message_is_plain() {
        assert_eq!(
            parse_edge("a ~> b :").unwrap_err(),
            "\"a ~> b :\" is missing its payload type: a data-flow edge is written \"a ~> b : Type\""
        );
    }

    #[test]
    fn malformed_call_edge_message_is_plain() {
        assert_eq!(
            parse_edge("-> b").unwrap_err(),
            "\"-> b\" is missing the component name on one side of the arrow"
        );
    }

    #[test]
    fn malformed_flow_edge_message_is_plain() {
        assert_eq!(
            parse_edge("~> b : Type").unwrap_err(),
            "\"~> b : Type\" is missing the component name on one side of the arrow"
        );
    }

    #[test]
    fn deny_without_arrow_message_is_plain() {
        assert_eq!(
            parse_deny("a b").unwrap_err(),
            "\"a b\" is not a deny rule: expected \"a -> b\" or \"a -> b except c, d\""
        );
    }

    #[test]
    fn malformed_deny_message_is_plain() {
        assert_eq!(
            parse_deny("-> b").unwrap_err(),
            "\"-> b\" is missing the component name on one side of the arrow"
        );
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

    /// §5.1 checks inheritance: a fn that writes no `checks:` key at all
    /// inherits the nearest ancestor component's default.
    #[test]
    fn fn_with_no_checks_inherits_the_component_default() {
        let doc = parse_document(
            r#"
ply: 1
components:
  pricing:
    anchor: app::pricing
    checks: [bounded(2)]
    fns:
      quote: {}
"#,
        )
        .unwrap();
        let pricing = &doc.components["pricing"];
        let default = component_default_checks("pricing", pricing, None);
        let fc = &pricing.fns["quote"];
        assert_eq!(
            effective_checks(fc, default),
            Some(["bounded(2)".to_string()].as_slice())
        );
    }

    /// §5.4c: `checks: []` is a *written* list. It says "check nothing
    /// here", and it overrides an ancestor default exactly the way a full
    /// list does — which is the whole difference between the two spellings.
    /// Reading them as one is how a function whose document said not to
    /// check it got proved anyway.
    #[test]
    fn an_empty_checks_list_overrides_the_component_default_and_asks_for_nothing() {
        let doc = parse_document(
            r#"
ply: 1
components:
  pricing:
    anchor: app::pricing
    checks: [bounded(2)]
    fns:
      quote:
        checks: []
"#,
        )
        .unwrap();
        let pricing = &doc.components["pricing"];
        let default = component_default_checks("pricing", pricing, None);
        let fc = &pricing.fns["quote"];
        assert_eq!(
            effective_checks(fc, default),
            Some([].as_slice()),
            "an empty list is an answer -- `Some([])` -- and never the absence a caller may \
             fill in with a default of its own"
        );
    }

    /// A fn's own non-empty `checks` wins entirely — no merge with the
    /// component default.
    #[test]
    fn fn_with_own_checks_overrides_the_component_default_entirely() {
        let doc = parse_document(
            r#"
ply: 1
components:
  pricing:
    anchor: app::pricing
    checks: [bounded(2)]
    fns:
      quote:
        checks: [test]
"#,
        )
        .unwrap();
        let pricing = &doc.components["pricing"];
        let default = component_default_checks("pricing", pricing, None);
        let fc = &pricing.fns["quote"];
        assert_eq!(
            effective_checks(fc, default),
            Some(["test".to_string()].as_slice())
        );
    }

    /// A fn that writes no checks and has no ancestor default anywhere in
    /// the chain has no declared list at all — `None`, the one case a
    /// caller may answer with a default of its own (§5.4c's shape-aware
    /// routing).
    #[test]
    fn fn_with_no_checks_and_no_ancestor_default_has_no_declared_list() {
        let doc = parse_document(
            r#"
ply: 1
components:
  pricing:
    anchor: app::pricing
    fns:
      quote: {}
"#,
        )
        .unwrap();
        let pricing = &doc.components["pricing"];
        let default = component_default_checks("pricing", pricing, None);
        let fc = &pricing.fns["quote"];
        assert_eq!(effective_checks(fc, default), None);
    }

    /// Nesting: a grandchild fn with no checks and no default of its own
    /// component skips over it to the grandparent's default (nearest
    /// *non-empty* ancestor, not merely the immediate parent).
    #[test]
    fn nested_component_without_its_own_default_inherits_from_grandparent() {
        let doc = parse_document(
            r#"
ply: 1
components:
  pricing:
    anchor: app::pricing
    checks: [bounded(2)]
    components:
      curves:
        anchor: app::pricing::curves
        fns:
          discount: {}
"#,
        )
        .unwrap();
        let pricing = &doc.components["pricing"];
        let pricing_default = component_default_checks("pricing", pricing, None);
        let curves = &pricing.components["curves"];
        let curves_default = component_default_checks("curves", curves, pricing_default);
        let fc = &curves.fns["discount"];
        assert_eq!(
            effective_checks(fc, curves_default),
            Some(["bounded(2)".to_string()].as_slice())
        );
    }

    /// docs/plans/external-elements.md §3: a top-level `externals:` map,
    /// each entry a name plus a required `note:`. Parses alongside
    /// `components:`, independent of it.
    #[test]
    fn externals_block_parses_with_required_note() {
        let doc = parse_document(
            r#"
ply: 1
externals:
  venue:
    note: "the exchange: accepts orders, returns fills; market data source"
"#,
        )
        .unwrap();
        assert_eq!(doc.externals.len(), 1);
        assert_eq!(
            doc.externals["venue"].note,
            "the exchange: accepts orders, returns fills; market data source"
        );
    }

    /// `note:` is required — an external with none must fail to parse
    /// (docs/plans/external-elements.md §3: "a bare name tells a newbie
    /// nothing, and the tooltip must carry its own gloss").
    #[test]
    fn external_without_note_fails_to_parse() {
        let err = parse_document(
            r#"
ply: 1
externals:
  venue: {}
"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("note"),
            "expected the parse error to name the missing `note` field, got: {err}"
        );
    }

    /// A document with no `externals:` block at all parses with an empty map
    /// — the field is optional, matching every other top-level collection.
    #[test]
    fn externals_block_is_optional() {
        let doc = parse_document("ply: 1\n").unwrap();
        assert!(doc.externals.is_empty());
    }

    /// docs/plans/external-elements.md §3: a per-fn `entry: [name, ...]`
    /// field, naming the externals that can reach this fn.
    #[test]
    fn fn_claim_parses_entry_list() {
        let doc = parse_document(
            r#"
ply: 1
components:
  oms:
    anchor: oms
    fns:
      Oms::submit:
        entry: [venue]
"#,
        )
        .unwrap();
        assert_eq!(
            doc.components["oms"].fns["Oms::submit"].entry,
            vec!["venue".to_string()]
        );
    }

    /// A fn claim with no `entry:` at all parses with an empty list — the
    /// overwhelming common case (most fns are not externally reachable).
    #[test]
    fn fn_claim_without_entry_defaults_to_empty() {
        let doc = parse_document(
            r#"
ply: 1
components:
  oms:
    anchor: oms
    fns:
      Oms::submit: {}
"#,
        )
        .unwrap();
        assert!(doc.components["oms"].fns["Oms::submit"].entry.is_empty());
    }

    /// Nesting: a nested component that declares its own non-empty default
    /// shadows the grandparent's — nearest ancestor wins, not the topmost.
    #[test]
    fn nested_component_with_its_own_default_shadows_the_grandparent() {
        let doc = parse_document(
            r#"
ply: 1
components:
  pricing:
    anchor: app::pricing
    checks: [bounded(2)]
    components:
      curves:
        anchor: app::pricing::curves
        checks: [fuzz(64)]
        fns:
          discount: {}
"#,
        )
        .unwrap();
        let pricing = &doc.components["pricing"];
        let pricing_default = component_default_checks("pricing", pricing, None);
        let curves = &pricing.components["curves"];
        let curves_default = component_default_checks("curves", curves, pricing_default);
        let fc = &curves.fns["discount"];
        assert_eq!(
            effective_checks(fc, curves_default),
            Some(["fuzz(64)".to_string()].as_slice())
        );
    }
}
