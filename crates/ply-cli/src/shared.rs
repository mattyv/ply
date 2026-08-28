//! What every engine-free command needs, in one copy.
//!
//! `check`, `audit` and `worklist` all read the same `ply.yaml`, describe
//! the same declared shape as a §7 tree, and decide the same way which
//! components belong to the crate in front of them. Phase 1a's lesson was
//! that two readers of one document is the defect (§5.1a rule 1), and by
//! the time a third command wanted `local_anchor_names` there were already
//! two copies of it — one in `check`, one in `verify`. This module is the
//! one copy; the commands differ in what they *say*, never in what they
//! read.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use ply_core::callgraph::{DeclaredContract, Resolution};
use ply_core::diag::{Diagnostic, Node};
use ply_core::model::{
    Component, Document, InheritedChecks, component_default_checks, effective_checks,
};
use ply_core::schema;

/// The entries of an order-preserving map, in key order.
///
/// `verify`'s output order is sorted by name and its goldens pin that; the
/// promoted model preserves declaration order (the renderer lays boxes out
/// that way), so the sort is explicit wherever name order is what a caller
/// depends on.
pub(crate) fn sorted_by_key<V>(map: &indexmap::IndexMap<String, V>) -> Vec<(&String, &V)> {
    let mut entries: Vec<(&String, &V)> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
}

/// The crate names an `anchor:` may use to mean "this crate": its `[lib]
/// name` and its package name, both normalised to Rust identifier spelling.
///
/// Empty when there is no readable `Cargo.toml`, in which case every
/// component is treated as local — the pre-2026-08-25 behaviour, kept as
/// the fallback so a missing manifest degrades rather than mis-reports.
pub(crate) fn local_anchor_names(crate_dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(crate_dir.join("Cargo.toml")) else {
        return vec![];
    };
    match ply_core::harness_crate::read_crate_names(&text) {
        Ok(names) => vec![
            names.lib_ident.replace('-', "_"),
            names.package_name.replace('-', "_"),
        ],
        Err(_) => vec![],
    }
}

/// Whether a component's `anchor:` names the crate Ply is standing in
/// (§5.5): a component anchored elsewhere is a **boundary component**, whose
/// contracts Ply reads rather than whose code it checks.
pub(crate) fn is_local(local_anchors: &[String], anchor: &str) -> bool {
    local_anchors.is_empty() || local_anchors.contains(&anchor.replace('-', "_"))
}

/// §5.4's external-spec route: every contract a `ply.yaml` declares for a
/// function, keyed by the path a caller writes.
///
/// A contract on a local component is keyed by the fn's own path; one on a
/// boundary component is keyed by `<anchor>::<fn>`, which is how the call
/// reads at the caller's site. This is the map §5.5's second branch
/// consults to decide whether an unclaimed callee has a promise a caller
/// may assume.
pub(crate) fn declared_contracts(
    doc: &Document,
    local_anchors: &[String],
) -> BTreeMap<String, DeclaredContract> {
    let mut declared = BTreeMap::new();
    for (_, comp) in sorted_by_key(&doc.components) {
        for (fn_key, claim) in sorted_by_key(&comp.fns) {
            if claim.requires.is_empty() && claim.ensures.is_empty() {
                continue;
            }
            let path = if is_local(local_anchors, &comp.anchor) {
                fn_key.clone()
            } else {
                format!("{}::{}", comp.anchor, fn_key)
            };
            declared.insert(
                path.clone(),
                DeclaredContract {
                    path,
                    requires: claim.requires.clone(),
                    ensures: claim.ensures.clone(),
                },
            );
        }
    }
    declared
}

/// A `ply.yaml` read far enough to report on.
pub(crate) enum Loaded {
    /// The document parsed, and every schema rule held.
    Document(Box<Document>),
    /// The document did not pass the schema. It gets one honest answer, not
    /// a pile of consequences: the model would refuse it anyway.
    SchemaViolations(Vec<Diagnostic>),
}

/// Reads `<crate_dir>/ply.yaml` and validates it against
/// `schema/ply.schema.json` (§5's normative definition).
///
/// A missing or unreadable document is an `Err` — a tool error, not a
/// finding: there is no document to have findings about. `phase` is the
/// command's own name, which is what a schema diagnostic reports itself as
/// having come from.
pub(crate) fn load_document(yaml_path: &Path, phase: &str) -> Result<Loaded> {
    let text = std::fs::read_to_string(yaml_path).with_context(|| {
        format!(
            "Ply could not read a ply.yaml at {}. `cargo ply {phase}` expects the path to a crate \
             directory that has one.",
            yaml_path.display()
        )
    })?;
    let violations = schema::validate_text(&text).map_err(|e| {
        anyhow::anyhow!(
            "{} is not valid YAML, so Ply could not read it as a ply.yaml at all: {e}",
            yaml_path.display()
        )
    })?;
    if !violations.is_empty() {
        return Ok(Loaded::SchemaViolations(
            violations.iter().map(|v| schema_diag(v, phase)).collect(),
        ));
    }
    let doc = ply_core::model::parse_document(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Loaded::Document(Box::new(doc)))
}

pub(crate) fn schema_diag(v: &schema::SchemaViolation, phase: &str) -> Diagnostic {
    Diagnostic {
        code: v.code.into(),
        severity: "error".into(),
        phase: phase.into(),
        engine: "ply".into(),
        check: "schema".into(),
        node_id: "ply.yaml".into(),
        title: v.message.clone(),
        primary_span: None,
        pointer: Some(v.pointer.clone()),
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// The document's declared shape as a §7 tree. Order is the document's own,
/// not sorted: a person fixing a `ply.yaml` reads it top to bottom.
///
/// Every node reads `unclaimed`, in every engine-free command: that is the
/// command reporting no evidence of its own, not a judgement about the code.
pub(crate) fn workspace_node(doc: &Document) -> Node {
    fn component_node(name: &str, c: &Component) -> Node {
        let mut children: Vec<Node> = c
            .fns
            .keys()
            .map(|f| Node {
                id: format!("{name}::{f}"),
                kind: "fn".into(),
                verdict: "unclaimed".into(),
                statuses: vec![],
                reused: false,
                evidence: None,
                children: vec![],
            })
            .collect();
        children.extend(
            c.components
                .iter()
                .map(|(child, nested)| component_node(&format!("{name}.{child}"), nested)),
        );
        Node {
            id: name.to_string(),
            kind: "component".into(),
            verdict: "unclaimed".into(),
            statuses: vec![],
            reused: false,
            evidence: None,
            children,
        }
    }
    Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: "unclaimed".into(),
        statuses: vec![],
        reused: false,
        evidence: None,
        children: doc
            .components
            .iter()
            .map(|(n, c)| component_node(n, c))
            .collect(),
    }
}

/// The root a run that never got past the schema reports: the workspace
/// exists, and nothing below it was read well enough to describe.
pub(crate) fn empty_workspace() -> Node {
    Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: "unclaimed".into(),
        statuses: vec![],
        reused: false,
        evidence: None,
        children: vec![],
    }
}

/// Every fn claim in the document, depth first, each paired with the
/// qualified name of the component that declares it, that component's
/// `anchor:`, and the §5.1 `checks:` default in force where it sits.
///
/// Nested components are walked, unlike `verify`'s own loop, which reads
/// only top-level ones (recorded as a gap in TODO.md rather than papered
/// over here): a claim a user wrote inside a nested component is still a
/// claim they wrote, and a listing command that skipped it would be
/// reporting less than the document says.
///
/// The inherited default is carried down the same walk, from the same
/// shared resolution `check`, `verify` and the renderer use
/// (`ply_core::model::component_default_checks`), so no command can decide
/// on its own which list governs a fn. `audit` and `worklist` used to read
/// a fn's own `checks:` and nothing else, which made a fn that takes its
/// checks from its component look as though it declared none — a listing
/// that misreads which functions are checked misreports what the evidence
/// rests on.
pub(crate) fn walk_fn_claims<'a>(doc: &'a Document, mut visit: impl FnMut(FnClaimRef<'a>)) {
    fn walk<'a>(
        qualified: &str,
        leaf: &'a str,
        comp: &'a Component,
        inherited: Option<InheritedChecks<'a>>,
        visit: &mut impl FnMut(FnClaimRef<'a>),
    ) {
        let below = component_default_checks(leaf, comp, inherited);
        for (fn_name, claim) in sorted_by_key(&comp.fns) {
            visit(FnClaimRef {
                component: qualified.to_string(),
                anchor: &comp.anchor,
                fn_name,
                claim,
                inherited: below,
            });
        }
        for (child, nested) in sorted_by_key(&comp.components) {
            walk(&format!("{qualified}.{child}"), child, nested, below, visit);
        }
    }
    for (name, comp) in sorted_by_key(&doc.components) {
        walk(name, name, comp, None, &mut visit);
    }
}

/// One fn claim, with enough of its surroundings to name it, to know
/// whether it belongs to this crate, and to say which checks govern it.
pub(crate) struct FnClaimRef<'a> {
    /// The qualified component name — `pricing`, or `pricing.curves`.
    pub component: String,
    pub anchor: &'a str,
    pub fn_name: &'a String,
    pub claim: &'a ply_core::model::FnClaim,
    /// The default declared by the nearest ancestor component that declared
    /// one, `None` when no ancestor did. Private: every reader goes through
    /// [`FnClaimRef::governing_checks`] rather than resolving it again.
    inherited: Option<InheritedChecks<'a>>,
}

/// The checks list that actually governs one fn, and where it was written.
pub(crate) struct Governing<'a> {
    /// The governing list. **Empty is an answer**: `checks: []` says "check
    /// nothing here" (§5.4c), and is not the same fact as no list anywhere.
    pub checks: &'a [String],
    /// The component the list came from, when the fn wrote none of its own;
    /// `None` when the list is the fn's own. A sentence that points at a
    /// `checks:` line the reader will not find on the fn is a sentence that
    /// sends them looking for it.
    pub from_component: Option<&'a str>,
}

impl<'a> FnClaimRef<'a> {
    /// The §7 node id this claim would carry.
    pub fn node_id(&self) -> String {
        format!("{}::{}", self.component, self.fn_name)
    }

    /// §5.1, through the one shared resolution: the fn's own `checks:` if
    /// it wrote one (an empty one included), else the nearest ancestor
    /// component's default, else `None` — nothing written anywhere, which
    /// is the only case a caller may fill in with a default of its own.
    pub fn governing_checks(&self) -> Option<Governing<'a>> {
        let checks = effective_checks(self.claim, self.inherited)?;
        let from_component = match self.claim.checks {
            Some(_) => None,
            None => self.inherited.map(|i| i.from_component),
        };
        Some(Governing {
            checks,
            from_component,
        })
    }
}

/// One assumed boundary contract (§5.5's second branch): a `bounded`
/// claim whose proof stands on a promise `ply.yaml` makes for a callee Ply
/// never reads.
///
/// `audit` reports it as trust surface and `worklist` reports the evidence
/// owed on it. They read it from here, so the two commands cannot disagree
/// about what this codebase is assuming.
pub(crate) struct AssumedContract {
    pub caller_node_id: String,
    pub caller_fn: String,
    pub callee: String,
    /// The promise, as a reader would say it out loud: `requires x, ensures y`.
    pub contract: String,
    /// What the callee's `ply.yaml` entry asks for, if anything — the
    /// difference between "add a check" and "run the one you declared".
    /// Resolved the way every other reader resolves it: the entry's own
    /// list, else the default its component declares (§5.1).
    pub callee_checks: Vec<String>,
    /// The component that default came from, when the callee's entry wrote
    /// no list of its own. Advice that says "its entry already asks for
    /// `fuzz(256)`" about a line the reader cannot find on that entry is
    /// advice they cannot follow.
    pub callee_checks_from: Option<String>,
    /// The crate the callee's `ply.yaml` entry is anchored to, when that is
    /// **not** the crate this command is standing in.
    ///
    /// It is the difference between advice a reader can act on and advice
    /// that sends them in a circle: `cargo ply verify` checks one crate at
    /// a time, so a `checks:` entry written for a function in another crate
    /// is read for its promise and declined for its checks (`W0303`).
    /// Telling somebody to add that check without saying where to run it is
    /// telling them to do something this tool will refuse.
    pub callee_anchor: Option<String>,
    pub where_text: String,
}

/// Every assumed boundary contract in one crate, decided exactly the way
/// `verify` decides it — from the call graph, before any engine would
/// start.
///
/// Only a `bounded` check makes a callee's promise load-bearing: Kani
/// descends into a callee's body, while the fuzz tier crosses a legacy
/// boundary by simply running the code (§5.5).
pub(crate) fn assumed_contracts(
    crate_dir: &Path,
    doc: &Document,
    local_anchors: &[String],
) -> Vec<AssumedContract> {
    let declared = declared_contracts(doc, local_anchors);
    let lib_path = crate_dir.join("src/lib.rs");
    let lib_src = std::fs::read_to_string(&lib_path).unwrap_or_default();
    let Ok(mut resolver) = ply_core::callgraph::Resolver::new(&lib_src, crate_dir, declared) else {
        return vec![];
    };

    let mut found = Vec::new();
    walk_fn_claims(doc, |c| {
        if !is_local(local_anchors, c.anchor) {
            return;
        }
        let Ok(cf) = ply_core::harness::discover_fn(&lib_path, c.fn_name) else {
            return;
        };
        // §5.1, through the one shared resolution: a fn that wrote no
        // `checks:` of its own runs whatever its component declares for
        // everything inside it, and only a fn no list anywhere governs
        // falls through to the shape-aware default. Reading the fn's own
        // line alone made a component that asked for `fuzz(64)` look like a
        // proof, so this listed an assumption `verify` never makes.
        let checks = match c.governing_checks() {
            Some(g) => match g
                .checks
                .iter()
                .map(|s| ply_core::model::parse_check(s))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(parsed) => parsed,
                // An unparseable checks list is `E0203`, which `check`
                // reports; a listing command reads what it can.
                Err(_) => return,
            },
            None => crate::verify::default_checks_for(&cf),
        };
        if !checks
            .iter()
            .any(|k| matches!(k, ply_core::model::Check::Bounded(_)))
        {
            return;
        }
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for site in &cf.calls {
            // A callee that returns nothing is skipped, and that is not a
            // detail: Ply stands in for a callee by producing a value for
            // it, so with no return value the assumption cannot be encoded
            // at all. `verify` refuses it (`W0512`) and the caller earns no
            // evidence -- nobody is trusting anything, so there is nothing
            // for a trust surface to list and nothing owed on it.
            match resolver.classify(site).status {
                ply_core::callgraph::CalleeStatus::Assumed {
                    contract,
                    canonical_path,
                    signature,
                } if signature.return_type.is_some() && seen.insert(canonical_path.clone()) => {
                    let (checks, checks_from, anchor) =
                        callee_entry(doc, &canonical_path, local_anchors);
                    found.push(AssumedContract {
                        caller_node_id: c.node_id(),
                        caller_fn: c.fn_name.clone(),
                        callee: canonical_path.clone(),
                        contract: contract_text(&contract.requires, &contract.ensures),
                        callee_checks: checks,
                        callee_checks_from: checks_from,
                        callee_anchor: anchor,
                        where_text: site.where_text(),
                    });
                }
                // A same-crate callee carrying its own inline contract
                // (D5's first two branches, §5.5) is invisible here before
                // this arm existed: both trust-listing commands read only
                // the `ply.yaml`-declared route, so a caller conditional on
                // an inline-contracted callee reported `owed-evidence` in
                // `verify` while `audit`'s trust surface and `worklist`'s
                // count both stayed empty (adversarial review, 2026-08-26,
                // fixture `privmod`) -- §5.5's own honesty condition 3
                // ("trust that is never checked is green paint ... `cargo
                // ply audit` lists it") silently did not hold for this
                // class. Listed here whenever the callee is not itself
                // claimed with a `bounded` check anywhere in the document:
                // that is exactly the condition under which `verify` can
                // never treat it as D5's first branch (which requires the
                // callee to be an independently bounded-checked claim), so
                // it is unconditionally an assumption here, never a
                // narrower guess. **Known gap, not solved here**: a
                // same-crate callee that *is* claimed with `bounded`
                // elsewhere but still lands on branch two at `verify` time
                // (a cycle, or an unclean run) needs the same ordering
                // computation `verify` does to tell clean from assumed --
                // this listing does not attempt that and under-reports
                // exactly that case.
                ply_core::callgraph::CalleeStatus::Contracted if seen.insert(site.path.clone()) => {
                    if let Resolution::Found(found_fn) = resolver.lookup_fn(&site.path)
                        && found_fn.local
                        && found_fn.unnameable.is_none()
                        && let Ok(callee_cf) = ply_core::harness::build_contract_fn(
                            &found_fn.item,
                            &ply_core::harness::alias_map(&found_fn.file),
                            &found_fn.canonical,
                            found_fn.is_method,
                        )
                    {
                        let canonical = found_fn.canonical.clone();
                        let (checks, checks_from, anchor) =
                            callee_entry(doc, &canonical, local_anchors);
                        let claimed_bounded = checks.iter().any(|c| c.starts_with("bounded("));
                        if !claimed_bounded {
                            let requires = callee_cf
                                .requires
                                .as_ref()
                                .map(|(_, t)| vec![t.clone()])
                                .unwrap_or_default();
                            let ensures = callee_cf
                                .ensures
                                .as_ref()
                                .map(|(_, t)| vec![t.clone()])
                                .unwrap_or_default();
                            found.push(AssumedContract {
                                caller_node_id: c.node_id(),
                                caller_fn: c.fn_name.clone(),
                                callee: canonical,
                                contract: contract_text(&requires, &ensures),
                                callee_checks: checks,
                                callee_checks_from: checks_from,
                                callee_anchor: anchor,
                                where_text: site.where_text(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    });
    found
}

/// One contract, as a reader would say it out loud.
pub(crate) fn contract_text(requires: &[String], ensures: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for r in requires {
        parts.push(format!("requires {r}"));
    }
    for e in ensures {
        parts.push(format!("ensures {e}"));
    }
    if parts.is_empty() {
        "a contract declared with no clauses".to_string()
    } else {
        parts.join(", ")
    }
}

/// The callee's `ply.yaml` entry, as far as advice needs it: the checks
/// that govern it, the component those checks were written on when they
/// were not written on the entry itself, and the crate it is anchored to
/// when that is not this one. The path is the one a caller writes, so a
/// boundary component's fn is matched by `<anchor>::<fn>` (§5.5).
fn callee_entry(
    doc: &Document,
    path: &str,
    local_anchors: &[String],
) -> (Vec<String>, Option<String>, Option<String>) {
    let mut found = (Vec::new(), None, None);
    walk_fn_claims(doc, |c| {
        let local = is_local(local_anchors, c.anchor);
        let key = if local {
            c.fn_name.clone()
        } else {
            format!("{}::{}", c.anchor, c.fn_name)
        };
        if key == path {
            let governing = c.governing_checks();
            found = (
                governing
                    .as_ref()
                    .map(|g| g.checks.to_vec())
                    .unwrap_or_default(),
                governing
                    .as_ref()
                    .and_then(|g| g.from_component.map(str::to_string)),
                if local {
                    None
                } else {
                    Some(c.anchor.to_string())
                },
            );
        }
    });
    found
}

/// Reflow a sentence to ~92 columns, indenting continuations to `indent`.
pub(crate) fn wrap(text: &str, indent: usize) -> String {
    let mut out = String::new();
    let mut col = indent;
    for word in text.split_whitespace() {
        if col + word.len() + 1 > 92 && col > indent {
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            col = indent;
        } else if !out.is_empty() {
            out.push(' ');
            col += 1;
        }
        out.push_str(word);
        col += word.len();
    }
    out
}
