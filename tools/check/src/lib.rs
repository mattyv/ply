//! Document-local `ply.yaml` validation (SPEC.md §5.1a, §5.1, §5.6) — the
//! subset of `cargo ply check` (§6) that needs no anchored Rust code:
//! unknown fields, schema shape, and required fields are already rejected
//! by `ply_model::parse_document`'s strict serde parse (nothing to
//! duplicate here). This crate only flags constructs that parse cleanly but
//! still violate SPEC.md. Anchor resolution, staleness, and the
//! architecture rules (§5.2, §5.3) need real code behind the anchors and
//! are out of scope.

use ply_model::{parse_check, parse_deny, parse_edge, Check, Component, Document};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

fn diag(code: &'static str, message: String) -> Diagnostic {
    Diagnostic { code, message }
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
fn check_syntax(checks: &[String], location: &str, out: &mut Vec<Diagnostic>) {
    for c in checks {
        if let Err(e) = parse_check(c) {
            out.push(diag("E0203", format!("{e} ({location})")));
        }
    }
}

/// §5.1: `mutate` without a `test` or `fuzz` entry in the *same* checks list
/// is `E0504`. Checked independently wherever a checks list appears
/// (a component's default checks, or a fn claim's own) — this rule does
/// not merge inherited and explicit checks, since that merge isn't spelled
/// out anywhere as part of the document-local grammar.
fn check_mutate_rule(checks: &[String], location: &str, out: &mut Vec<Diagnostic>) {
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
            format!("mutate without test/fuzz in checks list ({location})"),
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
    leaf_index.entry(leaf.to_string()).or_default().push(qualified.to_string());

    if !is_valid_path_form(&c.anchor) {
        out.push(diag(
            "E0304",
            format!("unsupported path form {:?} (component {qualified}, anchor)", c.anchor),
        ));
    }
    let location = format!("component {qualified}");
    check_syntax(&c.checks, &location, out);
    check_mutate_rule(&c.checks, &location, out);

    for (fn_name, fc) in &c.fns {
        if !is_valid_path_form(fn_name) {
            out.push(diag(
                "E0304",
                format!("unsupported path form {fn_name:?} (fn {fn_name})"),
            ));
        }
        let location = format!("fn {fn_name}");
        check_syntax(&fc.checks, &location, out);
        check_mutate_rule(&fc.checks, &location, out);

        for u in &fc.unresolved {
            unresolved_ids.push((u.id, location.clone()));
        }
    }

    for (child_name, nested) in &c.components {
        let nested_qualified = format!("{qualified}.{child_name}");
        walk_component(&nested_qualified, child_name, nested, out, unresolved_ids, leaf_index);
    }
}

/// §5.1a rule 6: a bare (unqualified) edge/deny endpoint resolves only if
/// its leaf name is unique across the merged component tree; otherwise
/// it's `E0206`, naming every qualified path it could mean. This
/// deliberately re-derives the leaf index from the component tree alone
/// (no layout, no coordinates) rather than depending on `ply-render` for
/// it — the ambiguity rule is a naming fact about the document, not a
/// drawing concern.
fn check_token_ambiguity(token: &str, leaf_index: &HashMap<String, Vec<String>>, out: &mut Vec<Diagnostic>) {
    if token == "*" || token.contains('.') {
        return;
    }
    if let Some(paths) = leaf_index.get(token) {
        if paths.len() > 1 {
            let mut candidates = paths.clone();
            candidates.sort();
            out.push(diag(
                "E0206",
                format!(
                    "ambiguous component reference {token:?}: matches {} — use the dotted qualified form (§5.1a rule 6)",
                    candidates.join(", ")
                ),
            ));
        }
    }
}

pub fn run_checks(doc: &Document) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut unresolved_ids: Vec<(u64, String)> = Vec::new();
    let mut leaf_index: HashMap<String, Vec<String>> = HashMap::new();

    for (name, c) in &doc.components {
        walk_component(name, name, c, &mut out, &mut unresolved_ids, &mut leaf_index);
    }

    for e in &doc.edges {
        match parse_edge(e) {
            Ok(edge) => {
                check_token_ambiguity(&edge.from, &leaf_index, &mut out);
                check_token_ambiguity(&edge.to, &leaf_index, &mut out);
            }
            Err(err) => out.push(diag("E0203", format!("{err} (edges)"))),
        }
    }
    for d in &doc.deny {
        match parse_deny(d) {
            Ok(deny) => {
                check_token_ambiguity(&deny.from, &leaf_index, &mut out);
                check_token_ambiguity(&deny.to, &leaf_index, &mut out);
            }
            Err(err) => out.push(diag("E0203", format!("{err} (deny)"))),
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
                format!("duplicate unresolved id {id} ({prev} and {location})"),
            )),
            None => {
                first_seen.insert(*id, location.clone());
            }
        }
    }

    out
}
