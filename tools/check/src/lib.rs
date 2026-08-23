//! Document-local `ply.yaml` validation (The-Ply-Spec.md §5.1a, §5.1, §5.6) — the
//! subset of `cargo ply check` (§6) that needs no anchored Rust code:
//! unknown fields, schema shape, and required fields are already rejected
//! by `ply_model::parse_document`'s strict serde parse (nothing to
//! duplicate here). This crate only flags constructs that parse cleanly but
//! still violate The-Ply-Spec.md. Anchor resolution, staleness, and the
//! architecture rules (§5.2, §5.3) need real code behind the anchors and
//! are out of scope.

use ply_model::{
    Check, Component, Document, Edge, InheritedChecks, component_default_checks, effective_checks,
    parse_check, parse_deny, parse_edge,
};
use std::collections::{HashMap, HashSet};

/// Where a diagnostic attaches for drawing (The-Ply-Spec.md §7.1 "finding" row).
/// `ply-render` consumes this to know what to mark red; `ply-check`'s own
/// stdout/exit-code contract never prints it (see `Diagnostic`'s `Display`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A fn claim, named by its component's fully qualified dotted path
    /// (top-level component name, or `parent.child` for nested ones) plus
    /// its own fn key.
    Fn {
        component_path: String,
        fn_name: String,
    },
    /// A component (its box), named by the same qualified path.
    Component(String),
    /// An entry in `doc.edges`, by position.
    EdgeIndex(usize),
    /// An entry in `doc.deny`, by position.
    DenyIndex(usize),
    /// An unresolved marker, by its declared id — may attach to more than
    /// one drawn pin if that id is a duplicate (that's the finding).
    UnresolvedId(u64),
    /// No single item in the document is the offender; the renderer falls
    /// back to a count next to the workspace title.
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub target: Target,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl Diagnostic {
    /// Severity, derived from the code prefix (The-Ply-Spec.md §5.3: item-tier
    /// rules are `W`-severity by default). A `W`-code is advisory — `ply
    /// check` reports it, but it must not by itself fail the run; every other
    /// code this crate emits is a document-local error.
    pub fn is_advisory(&self) -> bool {
        self.code.starts_with('W')
    }
}

fn diag(code: &'static str, message: String, target: Target) -> Diagnostic {
    Diagnostic {
        code,
        message,
        target,
    }
}

/// §5.1a rule 3: anchors and fn keys are plain segment paths —
/// `IDENT(::IDENT)*`, where a segment may also be a type name in
/// `Type::method` position. No generics, no trait-qualified paths, no
/// lifetimes — anything else is `E0304 unsupported path form`.
fn is_valid_path_form(s: &str) -> bool {
    !s.is_empty()
        && s.split("::").all(|seg| {
            let mut chars = seg.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// §5.1 / §5.1a rule 4: every check string must parse under the real
/// `test | fuzz(N)? | bounded(K)? | prove | mutate` micro-syntax, numeric
/// bounds included. A string serde accepted as a plain `String` but that
/// fails this parser is `E0203`.
fn check_syntax(checks: &[String], location: &str, target: &Target, out: &mut Vec<Diagnostic>) {
    for c in checks {
        if let Err(e) = parse_check(c) {
            out.push(diag("E0203", format!("{e} ({location})"), target.clone()));
        }
    }
}

/// §5.1: `mutate` without a `test` or `fuzz` entry in the *same* checks list
/// is `E0504`. Checked wherever a checks list actually governs something:
/// a component's own declared default (checked in isolation, as its own
/// list — `walk_component` calls this once per component with `c.checks`),
/// and separately, for each fn, its *effective* list (`ply_model::
/// effective_checks` — the fn's own non-empty list if it has one, else the
/// nearest ancestor component's default). A fn's own list always wins
/// entirely over anything inherited — `mutate` riding on an *inherited*
/// `test`/`fuzz` does not satisfy this rule, since there is no merge, only
/// an override (§5.1's plain reading of "default").
fn check_mutate_rule(
    checks: &[String],
    location: &str,
    target: &Target,
    out: &mut Vec<Diagnostic>,
) {
    let mut has_mutate = false;
    let mut has_test_or_fuzz = false;
    for c in checks {
        match parse_check(c) {
            Ok(Check::Mutate) => has_mutate = true,
            Ok(Check::Test) | Ok(Check::Fuzz(_)) => has_test_or_fuzz = true,
            _ => {}
        }
    }
    if has_mutate && !has_test_or_fuzz {
        out.push(diag(
            "E0504",
            format!(
                "mutate has nothing to catch its planted bugs: add a test or fuzz check \
                 beside it — mutation testing works by deliberately breaking the code and \
                 checking those checks notice ({location})"
            ),
            target.clone(),
        ));
    }
}

/// Walks one component (and its nested components), running every
/// per-component and per-fn rule, and collecting `(unresolved id, location)`
/// pairs and the leaf-name index used by the §5.1a rule 6 ambiguity check.
///
/// `inherited` is the §5.1 checks default this component itself inherited
/// from further up the tree (`None` above the document root, or wherever no
/// ancestor ever declared one) — threaded in so a fn with no `checks` of its
/// own can be validated against its real effective list, not silently
/// skipped.
fn walk_component<'a>(
    qualified: &str,
    leaf: &'a str,
    c: &'a Component,
    out: &mut Vec<Diagnostic>,
    unresolved_ids: &mut Vec<(u64, String)>,
    leaf_index: &mut HashMap<String, Vec<String>>,
    inherited: Option<InheritedChecks<'a>>,
) {
    leaf_index
        .entry(leaf.to_string())
        .or_default()
        .push(qualified.to_string());

    let component_target = Target::Component(qualified.to_string());
    if !is_valid_path_form(&c.anchor) {
        out.push(diag(
            "E0304",
            format!(
                "{:?} cannot be used as an anchor path: generics, lifetimes, and \
                 trait-qualified paths are not accepted — use a plain module::item path \
                 (component {qualified}, anchor)",
                c.anchor
            ),
            component_target.clone(),
        ));
    }
    let location = format!("component {qualified}");
    check_syntax(&c.checks, &location, &component_target, out);
    check_mutate_rule(&c.checks, &location, &component_target, out);

    // §5.1: what this component's own fns (and any nested component that
    // declares no default of its own) inherit — this component's own
    // `checks` if non-empty, else whatever it itself inherited.
    let this_default = component_default_checks(leaf, c, inherited);

    for (fn_name, fc) in &c.fns {
        let fn_target = Target::Fn {
            component_path: qualified.to_string(),
            fn_name: fn_name.clone(),
        };
        if !is_valid_path_form(fn_name) {
            out.push(diag(
                "E0304",
                format!(
                    "{fn_name:?} cannot be used as a fn path: generics, lifetimes, and \
                     trait-qualified paths are not accepted — use a plain module::item path \
                     (fn {fn_name})"
                ),
                fn_target.clone(),
            ));
        }
        let location = format!("fn {fn_name}");
        // Syntax validation stays on the fn's own literal strings — an
        // inherited list was already syntax-checked where it was declared
        // (as that ancestor's own `component {..}` location above), so
        // re-validating it here would only duplicate that diagnostic.
        check_syntax(&fc.checks, &location, &fn_target, out);

        // §5.1 D12: `mutate` needs a `test`/`fuzz` in the *effective* list —
        // the fn's own non-empty list if it has one (which always wins
        // entirely, never merges with anything above it), else the nearest
        // ancestor default. A fn with no checks and no ancestor default has
        // an empty effective list, which trivially can't trip this rule.
        let effective = effective_checks(fc, this_default);
        let mutate_location = if fc.checks.is_empty() {
            match this_default {
                Some(d) => format!(
                    "fn {fn_name}, checks inherited from component {}",
                    d.from_component
                ),
                None => location.clone(),
            }
        } else {
            location.clone()
        };
        check_mutate_rule(effective, &mutate_location, &fn_target, out);

        for u in &fc.unresolved {
            unresolved_ids.push((u.id, location.clone()));
        }
    }

    for (child_name, nested) in &c.components {
        let nested_qualified = format!("{qualified}.{child_name}");
        walk_component(
            &nested_qualified,
            child_name,
            nested,
            out,
            unresolved_ids,
            leaf_index,
            this_default,
        );
    }
}

/// §5.1a rule 6: a bare (unqualified) edge/deny endpoint resolves only if
/// its leaf name is unique across the merged component tree; otherwise
/// it's `E0206`, naming every qualified path it could mean. This
/// deliberately re-derives the leaf index from the component tree alone
/// (no layout, no coordinates) rather than depending on `ply-render` for
/// it — the ambiguity rule is a naming fact about the document, not a
/// drawing concern.
/// Plain-language "A or B" / "A, B, or C" list, for naming every candidate
/// an ambiguous reference could mean without reading like a data dump.
fn join_or(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [a, b] => format!("{a} or {b}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

fn check_token_ambiguity(
    token: &str,
    leaf_index: &HashMap<String, Vec<String>>,
    target: &Target,
    out: &mut Vec<Diagnostic>,
) {
    if token == "*" || token.contains('.') {
        return;
    }
    if let Some(paths) = leaf_index.get(token)
        && paths.len() > 1
    {
        let mut candidates = paths.clone();
        candidates.sort();
        out.push(diag(
            "E0206",
            format!(
                "ambiguous component reference {token:?}: it could mean {} — write the dotted \
                 form (e.g. {}) to say which",
                join_or(&candidates),
                candidates[0]
            ),
            target.clone(),
        ));
    }
}

/// §5.3 "containment implies permission": resolves an edge endpoint token
/// to the one qualified component path it must mean, or `None` if it
/// doesn't resolve to exactly one (a wildcard, an ambiguous bare leaf
/// already flagged by `E0206`, or a dotted path that names no real
/// component). Uses the same resolution as §5.1a rule 6: a bare name
/// resolves only if its leaf is unique across the tree; a dotted token is
/// taken as the fully qualified path it already looks like.
fn resolve_component_ref(
    token: &str,
    leaf_index: &HashMap<String, Vec<String>>,
    all_qualified: &HashSet<String>,
) -> Option<String> {
    if token == "*" {
        return None;
    }
    if token.contains('.') {
        return all_qualified.contains(token).then(|| token.to_string());
    }
    match leaf_index.get(token) {
        Some(paths) if paths.len() == 1 => Some(paths[0].clone()),
        _ => None,
    }
}

/// True if `other` is `ancestor` itself plus at least one more dotted
/// segment — i.e. `ancestor` is a strict prefix of `other` ending on a `.`
/// boundary, matching how nested qualified paths are built in
/// `walk_component`.
fn is_strict_ancestor(ancestor: &str, other: &str) -> bool {
    other.len() > ancestor.len()
        && other.starts_with(ancestor)
        && other.as_bytes()[ancestor.len()] == b'.'
}

/// §5.3: an edge whose two endpoints lie on one nesting line — a component
/// and its own descendant, either direction, at any depth — is redundant:
/// containment already grants the permission the edge would declare.
/// Applies to both edge kinds (`->` and `~>`); the spec paragraph states the
/// rule in terms of "an explicit edge" and closes with "Edges are for
/// crossings between nesting lines", neither qualified to calls only.
fn check_containment_redundancy(
    edge_str: &str,
    edge: &Edge,
    leaf_index: &HashMap<String, Vec<String>>,
    all_qualified: &HashSet<String>,
    target: &Target,
    out: &mut Vec<Diagnostic>,
) {
    let Some(from) = resolve_component_ref(&edge.from, leaf_index, all_qualified) else {
        return;
    };
    let Some(to) = resolve_component_ref(&edge.to, leaf_index, all_qualified) else {
        return;
    };

    let (outer, inner) = if is_strict_ancestor(&from, &to) {
        (from, to)
    } else if is_strict_ancestor(&to, &from) {
        (to, from)
    } else {
        return;
    };

    out.push(diag(
        "W0409",
        format!(
            "\"edge {}\" is redundant: {inner} is inside {outer}, and a component may always \
             call within its own nesting line — no edge needed",
            edge_str.trim(),
        ),
        target.clone(),
    ));
}

pub fn run_checks(doc: &Document) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut unresolved_ids: Vec<(u64, String)> = Vec::new();
    let mut leaf_index: HashMap<String, Vec<String>> = HashMap::new();

    for (name, c) in &doc.components {
        walk_component(
            name,
            name,
            c,
            &mut out,
            &mut unresolved_ids,
            &mut leaf_index,
            None,
        );
    }

    // Every qualified path any component actually has, used to resolve a
    // dotted edge endpoint literally (§5.1a rule 6's dotted form) without
    // re-deriving it from layout.
    let all_qualified: HashSet<String> = leaf_index.values().flatten().cloned().collect();

    for (i, e) in doc.edges.iter().enumerate() {
        let target = Target::EdgeIndex(i);
        match parse_edge(e) {
            Ok(edge) => {
                check_token_ambiguity(&edge.from, &leaf_index, &target, &mut out);
                check_token_ambiguity(&edge.to, &leaf_index, &target, &mut out);
                check_containment_redundancy(
                    e,
                    &edge,
                    &leaf_index,
                    &all_qualified,
                    &target,
                    &mut out,
                );
            }
            Err(err) => out.push(diag("E0203", format!("{err} (edges)"), target)),
        }
    }
    for (i, d) in doc.deny.iter().enumerate() {
        let target = Target::DenyIndex(i);
        match parse_deny(d) {
            Ok(deny) => {
                check_token_ambiguity(&deny.from, &leaf_index, &target, &mut out);
                check_token_ambiguity(&deny.to, &leaf_index, &target, &mut out);
            }
            Err(err) => out.push(diag("E0203", format!("{err} (deny)"), target)),
        }
    }

    for entry in &doc.unresolved {
        unresolved_ids.push((entry.id, "registry".to_string()));
    }
    // §5.1a rule 5: unresolved ids are unique across the whole merged
    // workspace, registry and fn entries together. Report in encounter
    // order (component walk, then top-level registry) so output is
    // deterministic without needing a stable-sorted map.
    let mut first_seen: HashMap<u64, String> = HashMap::new();
    for (id, location) in &unresolved_ids {
        match first_seen.get(id) {
            Some(prev) => out.push(diag(
                "E0205",
                format!(
                    "unresolved id {id} is used twice ({prev} and {location}): each open \
                     decision needs its own number"
                ),
                Target::UnresolvedId(*id),
            )),
            None => {
                first_seen.insert(*id, location.clone());
            }
        }
    }

    out
}
