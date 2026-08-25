//! `cargo ply check` (§6): "schema + anchors + staleness + architecture.
//! Fast, no engines."
//!
//! **Two of those four tiers do not exist yet**, and this command says so
//! in both its surfaces rather than letting a clean run read as full
//! coverage. Staleness needs `ply.lock` (D14), which Ply does not write;
//! the architecture tier needs the crate and call graphs (M2). What runs
//! here is:
//!
//! - **schema** — the document against `schema/ply.schema.json` (`E0201`,
//!   `E0204`), then every document-local rule that needs no code behind the
//!   anchors (`ply_core::check`: `E0202`, `E0203`, `E0205`-`E0209`,
//!   `E0304`, `E0504`, `W0409`, `W0410`).
//! - **anchors** — every fn claim, resolved the same way `verify` resolves
//!   it (`harness::discover_fn`), so the two commands agree about which
//!   claims point at real code. When that fails, the `use`-following
//!   resolver (`callgraph::Resolver`) is asked a second question — does
//!   this path resolve *anywhere* in the crate? — purely so `E0301` can
//!   say which of the two things went wrong: a name that exists nowhere,
//!   or a name that exists somewhere this slice cannot verify from.
//!
//! No engines start, so this command produces no verdicts. Every node in
//! its envelope reads `unclaimed`, and that is the command reporting no
//! evidence of its own — not a judgement about the code.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use ply_core::callgraph::{CallSite, CalleeStatus, Resolver};
use ply_core::check::Target;
use ply_core::diag::{Coverage, Diagnostic, Envelope, Node, Tier};
use ply_core::harness;
use ply_core::model::{Component, Document};
use ply_core::schema;

const PLY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `check` could not look at, in the words a user needs to know what
/// their green run did not cover. Exact strings: they are the whole point of
/// the tier, and they are tested as such.
const STALENESS_GAP: &str = "NOT CHECKED. Ply compares each claim against a fingerprint of the \
     code it was last verified against, and that fingerprint lives in `ply.lock` — a file this \
     version of Ply does not write yet. So a claim whose function has changed since it was \
     verified is not reported here, and this run says nothing about whether your evidence is \
     still current.";
const ARCHITECTURE_GAP: &str = "NOT CHECKED. The `edges:` and `deny:` lines are read and their \
     form is checked, and an edge that is redundant or names an external is reported — but \
     nothing compares them against what your code actually calls. A call that violates a `deny` \
     rule will not be reported here.";

/// The sentence that stops a clean `check` from reading like a verified
/// codebase.
const NO_VERDICTS: &str = "`check` runs no engines, so it produces no verdicts: every claim in \
     this run's `--json` envelope reads `unclaimed` because this command gathered no evidence \
     about it, not because the code is unverified. `cargo ply verify` is what produces verdicts.";

#[derive(Debug)]
pub struct CheckReport {
    pub envelope: Envelope,
    /// The `ply.yaml` this run read, for the human header.
    pub document: String,
}

impl CheckReport {
    /// §6's exit codes for `check`: 0 clean (or advisory findings only), 1
    /// violations, 2 tool error (which never gets this far — a tool error is
    /// an `Err` out of [`check_crate`]).
    ///
    /// Advisory findings do not fail the run on their own (§5.3): a `W0409`
    /// redundant edge is worth saying and is not a violation.
    pub fn exit_code(&self) -> i32 {
        if self
            .envelope
            .diagnostics
            .iter()
            .any(|d| d.severity == "error")
        {
            1
        } else {
            0
        }
    }
}

pub fn check_crate(crate_dir: &Path) -> Result<CheckReport> {
    let yaml_path = crate_dir.join("ply.yaml");
    let text = std::fs::read_to_string(&yaml_path).with_context(|| {
        format!(
            "Ply could not read a ply.yaml at {}. `cargo ply check` expects the path to a crate \
             directory that has one.",
            yaml_path.display()
        )
    })?;

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Tier 1a: the document against the schema. A document that is not even
    // YAML is the parser's error, not a finding: there is nothing to report
    // findings *about*.
    let violations = schema::validate_text(&text).map_err(|e| {
        anyhow::anyhow!(
            "{} is not valid YAML, so Ply could not read it as a ply.yaml at all: {e}",
            yaml_path.display()
        )
    })?;
    for v in &violations {
        diagnostics.push(schema_diag(v));
    }

    // A document with schema violations does not get a second opinion: the
    // model would refuse it anyway, and a pile of consequential errors on
    // top of the real one helps nobody.
    if !diagnostics.is_empty() {
        return Ok(CheckReport {
            envelope: envelope(empty_workspace(), diagnostics, coverage(None)),
            document: yaml_path.display().to_string(),
        });
    }

    let doc = ply_core::model::parse_document(&text).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Tier 1b: every document-local rule, in document order.
    for d in ply_core::check::run_checks(&doc) {
        diagnostics.push(document_diag(&d));
    }

    // Tier 2: anchors.
    let anchors = check_anchors(crate_dir, &doc, &mut diagnostics);

    let root = workspace_node(&doc);
    Ok(CheckReport {
        envelope: envelope(root, diagnostics, coverage(Some(anchors))),
        document: yaml_path.display().to_string(),
    })
}

/// What the anchor tier managed to look at — the numbers the human summary
/// reports, so a reader can tell "all fine" from "nothing was looked at".
pub struct AnchorTally {
    pub resolved: usize,
    pub unresolved: usize,
    /// Fn claims on a component anchored to another crate. Not a defect:
    /// `verify` is single-crate, so their anchors simply cannot be resolved
    /// from here, and reporting them as errors would be wrong.
    pub elsewhere: usize,
}

fn check_anchors(
    crate_dir: &Path,
    doc: &Document,
    diagnostics: &mut Vec<Diagnostic>,
) -> AnchorTally {
    let mut tally = AnchorTally {
        resolved: 0,
        unresolved: 0,
        elsewhere: 0,
    };
    let lib_path = crate_dir.join("src/lib.rs");
    let Ok(lib_src) = std::fs::read_to_string(&lib_path) else {
        // No local crate to resolve against. Every claim falls into
        // "elsewhere", and the summary says so rather than inventing errors.
        tally.elsewhere = count_fn_claims(doc);
        return tally;
    };
    let local_anchors = local_anchor_names(crate_dir);
    let known_fns = harness::top_level_fn_names(&lib_path).unwrap_or_default();
    let mut resolver = Resolver::new(&lib_src, crate_dir, BTreeMap::new()).ok();

    for (name, comp) in &doc.components {
        walk_anchors(
            name,
            comp,
            &lib_path,
            &local_anchors,
            &known_fns,
            resolver.as_mut(),
            diagnostics,
            &mut tally,
        );
    }
    tally
}

#[allow(clippy::too_many_arguments)]
fn walk_anchors(
    qualified: &str,
    comp: &Component,
    lib_path: &Path,
    local_anchors: &[String],
    known_fns: &[String],
    mut resolver: Option<&mut Resolver>,
    diagnostics: &mut Vec<Diagnostic>,
    tally: &mut AnchorTally,
) {
    // The same locality test `verify` applies (§5.5): a component anchored
    // to another crate is a boundary component, and this slice reads its
    // declared contracts rather than its code.
    let local = local_anchors.is_empty() || local_anchors.contains(&comp.anchor.replace('-', "_"));
    if local {
        for fn_name in comp.fns.keys() {
            let node_id = format!("{qualified}::{fn_name}");
            if harness::discover_fn(lib_path, fn_name).is_ok() {
                tally.resolved += 1;
                continue;
            }
            tally.unresolved += 1;
            let elsewhere = resolver
                .as_deref_mut()
                .map(|r| {
                    r.classify(&CallSite {
                        path: fn_name.clone(),
                        line: 0,
                        col: 0,
                    })
                    .status
                })
                .is_some_and(|s| !matches!(s, CalleeStatus::Unresolved | CalleeStatus::Opaque(_)));
            diagnostics.push(unresolved_anchor_diag(
                &node_id, fn_name, known_fns, elsewhere, lib_path,
            ));
        }
    } else {
        tally.elsewhere += comp.fns.len();
    }
    for (child, nested) in &comp.components {
        walk_anchors(
            &format!("{qualified}.{child}"),
            nested,
            lib_path,
            local_anchors,
            known_fns,
            resolver.as_deref_mut(),
            diagnostics,
            tally,
        );
    }
}

/// §5.2: "an unresolvable anchor → `E0301` with nearest-name suggestions
/// (edit distance over the item index)". **A renamed function must break
/// CI, not silently orphan its claims.**
fn unresolved_anchor_diag(
    node_id: &str,
    fn_name: &str,
    known_fns: &[String],
    resolves_elsewhere: bool,
    lib_path: &Path,
) -> Diagnostic {
    let title = if resolves_elsewhere {
        format!(
            "`{fn_name}` exists in this crate, but not where Ply can verify it from. This slice \
             reads functions declared at the top level of {}; `{fn_name}` is inside a module or \
             behind a `use`. Move the claim's component to an `anchor:` on that module, or move \
             the function up, until Ply learns to descend.",
            lib_path.display()
        )
    } else {
        let suggestion = match schema::nearest_key(fn_name, known_fns) {
            Some(near) => format!(
                " The closest name Ply can see is `{near}` — if the function was renamed, the \
                 claim needs renaming with it."
            ),
            None if known_fns.is_empty() => {
                format!(" Ply found no functions at all in {}.", lib_path.display())
            }
            None => String::new(),
        };
        format!(
            "Ply could not find a function called `{fn_name}` in {}, so this claim describes \
             nothing.{suggestion}",
            lib_path.display()
        )
    };
    Diagnostic {
        code: "E0301".into(),
        severity: "error".into(),
        phase: "check".into(),
        engine: "ply".into(),
        check: "anchor".into(),
        node_id: node_id.into(),
        title,
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("unresolvable_anchor".into()),
    }
}

fn schema_diag(v: &schema::SchemaViolation) -> Diagnostic {
    Diagnostic {
        code: v.code.into(),
        severity: "error".into(),
        phase: "check".into(),
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

fn document_diag(d: &ply_core::check::Diagnostic) -> Diagnostic {
    Diagnostic {
        code: d.code.into(),
        severity: if d.is_advisory() { "warning" } else { "error" }.into(),
        phase: "check".into(),
        engine: "ply".into(),
        check: "schema".into(),
        node_id: node_id_for(&d.target),
        title: d.message.clone(),
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// The §7 node a document-local finding attaches to. `ply_core::check`'s
/// `Target` already knows *what to draw red* (§7.1); this is the same fact
/// spelled as a node id.
fn node_id_for(target: &Target) -> String {
    match target {
        Target::Fn {
            component_path,
            fn_name,
        } => format!("{component_path}::{fn_name}"),
        Target::Component(c) => c.clone(),
        Target::External(e) => format!("externals::{e}"),
        Target::EdgeIndex(i) => format!("edges[{i}]"),
        Target::DenyIndex(i) => format!("deny[{i}]"),
        Target::UnresolvedId(id) => format!("unresolved#{id}"),
        Target::Document => "ply.yaml".into(),
    }
}

fn count_fn_claims(doc: &Document) -> usize {
    fn walk(c: &Component) -> usize {
        c.fns.len() + c.components.values().map(walk).sum::<usize>()
    }
    doc.components.values().map(walk).sum()
}

/// The document's declared shape as a §7 tree. Order is the document's own,
/// not sorted: a person fixing a `ply.yaml` reads it top to bottom.
fn workspace_node(doc: &Document) -> Node {
    fn component_node(name: &str, c: &Component) -> Node {
        let mut children: Vec<Node> = c
            .fns
            .keys()
            .map(|f| Node {
                id: format!("{name}::{f}"),
                kind: "fn".into(),
                verdict: "unclaimed".into(),
                statuses: vec![],
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
            evidence: None,
            children,
        }
    }
    Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: "unclaimed".into(),
        statuses: vec![],
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
fn empty_workspace() -> Node {
    Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: "unclaimed".into(),
        statuses: vec![],
        evidence: None,
        children: vec![],
    }
}

fn coverage(anchors: Option<AnchorTally>) -> Coverage {
    let mut checked = vec![Tier {
        tier: "schema".into(),
        detail: "The document against schema/ply.schema.json, then every rule that can be \
                 settled from the document alone."
            .into(),
    }];
    match anchors {
        Some(t) => checked.push(Tier {
            tier: "anchors".into(),
            detail: anchor_detail(&t),
        }),
        None => checked.push(Tier {
            tier: "anchors".into(),
            detail: "NOT REACHED. The document did not pass the schema, so there was nothing \
                     well-formed to resolve anchors for."
                .into(),
        }),
    }
    Coverage {
        checked,
        not_checked: vec![
            Tier {
                tier: "staleness".into(),
                detail: STALENESS_GAP.into(),
            },
            Tier {
                tier: "architecture".into(),
                detail: ARCHITECTURE_GAP.into(),
            },
        ],
    }
}

fn anchor_detail(t: &AnchorTally) -> String {
    let total = t.resolved + t.unresolved;
    let mut s = format!(
        "{} of {} fn claims in this crate point at a function Ply can find.",
        t.resolved, total
    );
    if t.elsewhere > 0 {
        s.push_str(&format!(
            " {} more belong to a component anchored to another crate, which `verify` reads \
             contracts from rather than checking — their anchors are not resolved from here.",
            t.elsewhere
        ));
    }
    s
}

fn envelope(root: Node, diagnostics: Vec<Diagnostic>, coverage: Coverage) -> Envelope {
    Envelope {
        command: "check".into(),
        ply_version: PLY_VERSION.into(),
        root,
        diagnostics,
        coverage: Some(coverage),
    }
}

/// The crate names an `anchor:` may use to mean "this crate" — the same two
/// `verify` accepts, so the two commands never disagree about which
/// component is local.
fn local_anchor_names(crate_dir: &Path) -> Vec<String> {
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

/// The human surface. Ordered so the first thing a reader sees is what ran,
/// the second is what it found, and the last is what was NOT looked at —
/// which is the part a green run most needs to carry.
pub fn print_human(report: &CheckReport) {
    println!("cargo ply check — {}", report.document);
    println!();
    let cov = report
        .envelope
        .coverage
        .as_ref()
        .expect("check sets coverage");
    for tier in &cov.checked {
        println!("  {:<14}{}", tier.tier, wrap(&tier.detail, 16));
    }
    println!();

    if report.envelope.diagnostics.is_empty() {
        println!("  No problems found in the document.");
    } else {
        // No node id appended: every one of these sentences already names
        // where it is -- `(fn clamp)`, ``Found at `components.x.fns.y` `` --
        // because that is how they were written and exact-string tested.
        // Repeating it as `[clamp::clamp]` says the same thing twice in a
        // worse vocabulary. The id is in `--json`, where a machine wants it.
        for d in &report.envelope.diagnostics {
            println!("  {} {}", d.code, wrap(&d.title, 4));
        }
    }
    println!();
    println!("What this command did NOT check:");
    for tier in &cov.not_checked {
        println!("  {:<14}{}", tier.tier, wrap(&tier.detail, 16));
    }
    println!();
    println!("{}", wrap(NO_VERDICTS, 0));
}

/// Reflow a sentence to ~92 columns, indenting continuations to `indent`.
fn wrap(text: &str, indent: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A crate directory with a `src/lib.rs` and a `ply.yaml`, and nothing
    /// else — `check` reads no more than that.
    fn crate_with(lib: &str, yaml: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), lib).unwrap();
        std::fs::write(dir.path().join("ply.yaml"), yaml).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        dir
    }

    const CLEAN_YAML: &str = "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      clamp:\n        checks: [bounded(2)]\n";

    #[test]
    fn a_clean_crate_reports_no_problems_and_exits_zero() {
        let dir = crate_with("pub fn clamp(x: u32) -> u32 { x.min(100) }\n", CLEAN_YAML);
        let report = check_crate(dir.path()).unwrap();
        assert!(
            report.envelope.diagnostics.is_empty(),
            "{:#?}",
            report.envelope.diagnostics
        );
        assert_eq!(report.exit_code(), 0);
        let cov = report.envelope.coverage.as_ref().unwrap();
        assert_eq!(
            cov.checked[1].detail,
            "1 of 1 fn claims in this crate point at a function Ply can find."
        );
    }

    /// §5.2's own MUST: "a renamed function must break CI, not silently
    /// orphan its claims" — and §5.2 asks the diagnostic to suggest the
    /// nearest name, which is what makes the break actionable rather than
    /// merely loud.
    #[test]
    fn a_renamed_function_is_e0301_and_names_the_nearest_name() {
        let dir = crate_with("pub fn clamped(x: u32) -> u32 { x.min(100) }\n", CLEAN_YAML);
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "E0301")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert!(
            d.title.contains("could not find a function called `clamp`"),
            "{}",
            d.title
        );
        assert!(
            d.title
                .contains("The closest name Ply can see is `clamped`"),
            "{}",
            d.title
        );
        assert_eq!(report.exit_code(), 1);
    }

    /// The other shape of the same failure, and a different fix: the
    /// function is right there, just not somewhere this slice can verify
    /// from. Saying "could not find" would be false and would send the user
    /// hunting for a typo that is not there.
    #[test]
    fn a_function_ply_can_see_but_not_verify_from_says_which_of_the_two_it_is() {
        let dir = crate_with(
            "pub mod util { pub fn helper(x: u32) -> u32 { x } }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      util::helper:\n        checks: [bounded(2)]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "E0301")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert!(
            d.title
                .contains("exists in this crate, but not where Ply can verify it from"),
            "{}",
            d.title
        );
    }

    /// §5.3: an advisory finding is worth reporting and is not a violation.
    #[test]
    fn an_advisory_finding_is_reported_but_does_not_fail_the_run() {
        let dir = crate_with(
            "pub fn clamp(x: u32) -> u32 { x }\n",
            "ply: 1\ncomponents:\n  outer:\n    anchor: demo\n    components:\n      inner:\n        anchor: demo\nedges:\n  - \"outer -> outer.inner\"\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let d = &report.envelope.diagnostics[0];
        assert_eq!(d.code, "W0409");
        assert_eq!(d.severity, "warning");
        assert_eq!(report.exit_code(), 0, "a W-code must not fail the run");
    }

    /// A document that does not pass the schema gets one honest answer, not
    /// a pile of consequences — and the coverage block says the anchor tier
    /// never ran, rather than leaving a reader to assume it passed.
    #[test]
    fn a_schema_violation_stops_before_anchors_and_says_so() {
        let dir = crate_with(
            "pub fn clamp(x: u32) -> u32 { x }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      clamp:\n        ensure: [\"x > 0\"]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let d = &report.envelope.diagnostics[0];
        assert_eq!(d.code, "E0204");
        assert_eq!(
            d.pointer.as_deref(),
            Some("/components/demo/fns/clamp/ensure"),
            "§5: a schema violation carries its JSON-pointer path"
        );
        assert!(d.title.contains("Did you mean `ensures`?"), "{}", d.title);
        let cov = report.envelope.coverage.as_ref().unwrap();
        assert!(
            cov.checked[1].detail.starts_with("NOT REACHED."),
            "{}",
            cov.checked[1].detail
        );
    }

    /// The tiers §6 promises that this command does not deliver. Exact
    /// strings: a clean run's honesty is entirely carried by these two
    /// sentences, so they are reviewed like the diagnostics are.
    #[test]
    fn the_report_names_both_tiers_it_does_not_cover() {
        let dir = crate_with("pub fn clamp(x: u32) -> u32 { x }\n", CLEAN_YAML);
        let report = check_crate(dir.path()).unwrap();
        let cov = report.envelope.coverage.as_ref().unwrap();
        let names: Vec<&str> = cov.not_checked.iter().map(|t| t.tier.as_str()).collect();
        assert_eq!(names, ["staleness", "architecture"]);
        assert_eq!(cov.not_checked[0].detail, STALENESS_GAP);
        assert_eq!(cov.not_checked[1].detail, ARCHITECTURE_GAP);
        assert!(STALENESS_GAP.contains("ply.lock"));
        assert!(ARCHITECTURE_GAP.contains("`deny`"));
    }

    /// §8's envelope, with `check` in the command field and a node tree that
    /// claims no evidence anywhere — because none was gathered.
    #[test]
    fn the_envelope_is_the_section_8_shape_and_claims_no_evidence() {
        let dir = crate_with("pub fn clamp(x: u32) -> u32 { x }\n", CLEAN_YAML);
        let report = check_crate(dir.path()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&report.envelope.to_json_pretty()).unwrap();
        assert_eq!(json["command"], "check");
        assert_eq!(json["root"]["kind"], "workspace");
        assert_eq!(json["root"]["verdict"], "unclaimed");
        assert_eq!(json["root"]["children"][0]["id"], "demo");
        assert_eq!(
            json["root"]["children"][0]["children"][0]["id"],
            "demo::clamp"
        );
        assert_eq!(
            json["root"]["children"][0]["children"][0]["verdict"],
            "unclaimed"
        );
        assert!(
            json["root"]["children"][0]["children"][0]["evidence"].is_null(),
            "no engine ran, so nothing may carry evidence"
        );
        assert!(NO_VERDICTS.contains("runs no engines"));
    }

    /// A crate directory with no `ply.yaml` is a tool error, not a finding:
    /// there is no document to have findings about.
    #[test]
    fn a_missing_document_is_a_tool_error_naming_the_path_it_looked_at() {
        let dir = tempfile::tempdir().unwrap();
        let err = check_crate(dir.path()).unwrap_err().to_string();
        assert!(err.contains("could not read a ply.yaml at"), "{err}");
    }
}
