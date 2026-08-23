//! Document-local `ply.yaml` validation (Ply-Spec.md §5.1a, §5.1, §5.6) — the
//! subset of `cargo ply check` (§6) that needs no anchored Rust code:
//! unknown fields, schema shape, and required fields are already rejected
//! by `ply_model::parse_document`'s strict serde parse (nothing to
//! duplicate here). This crate only flags constructs that parse cleanly but
//! still violate Ply-Spec.md. Anchor resolution, staleness, and the
//! architecture rules (§5.2, §5.3) need real code behind the anchors and
//! are out of scope.

use ply_model::{Check, Component, Document, parse_check, parse_deny, parse_edge};
use std::collections::HashMap;

/// Where a diagnostic attaches for drawing (Ply-Spec.md §7.1 "finding" row).
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
/// is `E0504`. Checked independently wherever a checks list appears
/// (a component's default checks, or a fn claim's own) — this rule does
/// not merge inherited and explicit checks, since that merge isn't spelled
/// out anywhere as part of the document-local grammar.
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
fn walk_component(
    qualified: &str,
    leaf: &str,
    c: &Component,
    out: &mut Vec<Diagnostic>,
    unresolved_ids: &mut Vec<(u64, String)>,
    leaf_index: &mut HashMap<String, Vec<String>>,
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
        check_syntax(&fc.checks, &location, &fn_target, out);
        check_mutate_rule(&fc.checks, &location, &fn_target, out);

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
        );
    }

    for (i, e) in doc.edges.iter().enumerate() {
        let target = Target::EdgeIndex(i);
        match parse_edge(e) {
            Ok(edge) => {
                check_token_ambiguity(&edge.from, &leaf_index, &target, &mut out);
                check_token_ambiguity(&edge.to, &leaf_index, &target, &mut out);
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
