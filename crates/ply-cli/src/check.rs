//! `cargo ply check` (§6): "schema + anchors + architecture.
//! Fast, no engines."
//!
//! Architecture's **crate tier** (§5.3, first paragraph) now runs: the real
//! crate dependency graph, from `cargo metadata`, checked against declared
//! components and `edges:`/`deny:`. Its **item tier** (calls, capabilities,
//! ownership, profile bans — approximate, from syn) is a separate, later
//! milestone and does not run here; the coverage report says so rather than
//! letting a clean run read as full coverage. (There is no staleness tier
//! any more: `verify` re-hashes every recorded result at the moment of use,
//! D14/§5.2a, so there is no recorded-but-possibly-stale state for this
//! command to report on.) What runs here is:
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
//! - **architecture** — `ply_core::arch`: the real crate
//!   dependency graph from `cargo metadata`, checked against declared
//!   components (`A0401`) and `deny:` patterns (`A0405`).
//!
//! No engines start, so this command produces no verdicts. Every node in
//! its envelope reads `unclaimed`, and that is the command reporting no
//! evidence of its own — not a judgement about the code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ply_core::arch::{self, ArchFinding, ArchTally};
use ply_core::callgraph::Resolver;
use ply_core::check::Target;
use ply_core::diag::{Coverage, Diagnostic, Envelope, Node, Tier};
use ply_core::harness::{self, AnchorError};
use ply_core::model::{Component, Document};
use ply_core::schema;

use crate::shared::{
    Loaded, empty_workspace, is_local, load_document, local_anchor_names, workspace_node, wrap,
};

const PLY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `check` could not look at, in the words a user needs to know what
/// their green run did not cover. Exact strings: they are the whole point of
/// the tier, and they are tested as such.
///
/// The crate-level half of architecture is now checked (see
/// [`ArchOutcome::Ran`]'s own detail sentence); this is what remains --
/// the item tier, which needs the syn-backed extractor this milestone does
/// not build.
const ITEM_TIER_GAP: &str = "NOT CHECKED. Ply now checks whether one crate depends on another \
     crate across a boundary no `edges:` line allows. It does not yet look inside your \
     functions: a call from one function to another, use of a capability like the filesystem or \
     the network, or a change to a type another component owns can still cross that same \
     boundary with nothing here noticing.";

/// The crate-tier architecture check did not run at all -- the document
/// failed the schema before `check` ever got this far, and there was
/// nothing well-formed to read dependencies for.
const ARCH_NOT_REACHED: &str = "NOT REACHED. The document did not pass the schema, so there was \
     nothing well-formed to check crate dependencies against.";

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

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Tier 1a: the document against the schema. A document that is not even
    // YAML is the parser's error, not a finding: there is nothing to report
    // findings *about*. A document that fails the schema does not get a
    // second opinion either -- the model would refuse it anyway, and a pile
    // of consequential errors on top of the real one helps nobody.
    let doc = match load_document(&yaml_path, "check")? {
        Loaded::Document(doc) => *doc,
        Loaded::SchemaViolations(violations) => {
            return Ok(CheckReport {
                envelope: envelope(
                    empty_workspace(),
                    violations,
                    coverage(None, ArchOutcome::NotReached),
                ),
                document: yaml_path.display().to_string(),
            });
        }
    };

    // Tier 1b: every document-local rule, in document order.
    for d in ply_core::check::run_checks(&doc) {
        diagnostics.push(document_diag(&d));
    }

    // Tier 2: anchors.
    let anchors = check_anchors(crate_dir, &doc, &mut diagnostics);

    // Tier 3: architecture's crate tier (§5.3, first paragraph) -- the real
    // crate dependency graph from `cargo metadata`, checked against
    // declared components and `edges:`/`deny:`.
    let arch_outcome = run_architecture_tier(crate_dir, &doc, &mut diagnostics);

    // §7.1's derive-links brief: a component links to another document
    // when that document's own top-level anchor sits under this one's,
    // discovered from real crate directories rather than a declared key.
    // No engine and no `cargo metadata` call, so this runs whether or not
    // the architecture tier above could.
    for f in &ply_core::config::derive_links(&doc, crate_dir).findings {
        diagnostics.push(link_diag(f));
    }

    let root = workspace_node(&doc);
    Ok(CheckReport {
        envelope: envelope(root, diagnostics, coverage(Some(anchors), arch_outcome)),
        document: yaml_path.display().to_string(),
    })
}

/// What the crate-tier architecture check managed to look at, for the
/// coverage report -- the same honest-gap discipline [`AnchorTally`]
/// already follows: a green run must say what ran, not just what it found.
pub enum ArchOutcome {
    /// The tier ran: `cargo metadata` produced a real graph, and every
    /// cross-component dependency in it was classified.
    Ran(ArchTally),
    /// `cargo metadata` could not be run, or its output could not be read
    /// -- the tier did not run, and the reason is reported rather than the
    /// run being assumed clean.
    Unavailable(String),
    /// The document never reached this tier at all (it failed the schema
    /// first).
    NotReached,
}

/// Runs `cargo metadata` in `crate_dir`, classifies every real
/// cross-component crate dependency against `doc`'s declared components,
/// edges and deny rules (`ply_core::arch::check_architecture`), and pushes
/// every finding (`A0401`, `A0405`, and the rest of the `A04xx` family) onto
/// `diagnostics`.
///
/// docs/review-architecture-tier.md, finding 1: when `cargo metadata` itself
/// fails -- a broken manifest, `cargo` missing from `PATH`, a package
/// dependency cycle -- this tier did not merely skip a component of the
/// report, it produced **no error at all**: no diagnostic, no status on any
/// node, exit 0, with the failure buried as a sentence inside the coverage
/// report. That is an absence of evidence reported as a pass (§1), so it is
/// now `A0409`, an error-severity diagnostic -- which is what makes
/// `CheckReport::exit_code` return 1 for it, the same way any other
/// error-severity finding does, with no separate exit-code special case
/// needed.
fn run_architecture_tier(
    crate_dir: &Path,
    doc: &Document,
    diagnostics: &mut Vec<Diagnostic>,
) -> ArchOutcome {
    match arch::crate_dependency_graph(crate_dir) {
        Ok(graph) => {
            let (findings, tally) = arch::check_architecture(doc, &graph);
            for f in &findings {
                diagnostics.push(arch_diag(f));
            }
            ArchOutcome::Ran(tally)
        }
        Err(e) => {
            diagnostics.push(arch_unavailable_diag(&e.to_string()));
            ArchOutcome::Unavailable(e.to_string())
        }
    }
}

/// `A0409`: the architecture check could not run at all, because Ply could
/// not get this crate's real dependency graph. Reproduced
/// (docs/review-architecture-tier.md, finding 1) three ways: a broken
/// `Cargo.toml` anywhere in the workspace, `cargo` missing from `PATH`, and
/// a package dependency cycle -- the last one matters most, because a cycle
/// is exactly the shape a `ply.yaml` boundary rule is usually written to
/// prevent, so a run that cannot see the graph at all is the one case where
/// silence would be most dangerous.
fn arch_unavailable_diag(reason: &str) -> Diagnostic {
    Diagnostic {
        code: "A0409".into(),
        severity: "error".into(),
        phase: "check".into(),
        engine: "ply".into(),
        check: "architecture".into(),
        node_id: "ply.yaml".into(),
        title: format!(
            "Ply could not check whether one crate in this workspace depends on another across \
             a boundary this document does not allow, because it could not get the real \
             dependency graph: {reason} — that is not a clean result, it means this run did not \
             look. Fix the problem named above, then check again."
        ),
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("architecture_not_checked".into()),
    }
}

/// §5.3: the crate tier is exact and sound, so every finding it produces is
/// an error -- there is no advisory form the way item-tier findings have
/// one under `strict: false`.
/// The same shape as [`state_diag`], at warning severity: the document is not
/// wrong, Ply just could not look. Kept a separate function so the two can
/// never be confused at a call site -- "this is false" and "I could not
/// check this" are different sentences and must not share one.
/// Checks one component's `state:` against real source, wherever that
/// source lives.
///
/// Shared by the two walks below -- the one that has a library under the
/// document and the one that does not -- because they were never really
/// two different checks. A workspace-root document has no library of its
/// own, but its components are anchored at crates that do, and reading
/// those is the same read. What is genuinely different is the case where
/// no crate can be found at all, and that is the one warning here.
fn verify_state(
    qualified: &str,
    comp: &Component,
    crates: &std::collections::BTreeMap<String, PathBuf>,
    local: Option<&Path>,
    diagnostics: &mut Vec<Diagnostic>,
    tally: &mut AnchorTally,
) {
    let Some(state) = &comp.state else {
        return;
    };
    // The promises first, and without needing any source at all: whether a
    // `holds:` line can be read as an expression is a fact about the line,
    // not about the code. `check` reported "No problems found" on a
    // document `verify` then refused outright, which is the two-commands-
    // two-answers failure this file exists to prevent -- and worse here
    // than for a fn key, because the schema table tells a reader `check`'s
    // closing output is the authority.
    for clause in &state.holds {
        if let Err(bad) = ply_core::harness::parse_holds_clause(clause) {
            diagnostics.push(state_diag(
                "E0506",
                qualified,
                format!(
                    "one of the things `{qualified}` promises about `{of}` could not be read, so \
                     none of them will be checked: {reason}. The line as written is `{clause}`. \
                     Every promise about this structure is held back together rather than \
                     checking the ones that do parse: a partly-checked promise reported as a \
                     checked one is the failure this refuses. (E0506)",
                    of = state.of,
                    reason = bad.reason.trim_end_matches('.'),
                    clause = bad.clause,
                ),
            ));
        }
    }
    // The crate the anchor names, when the document spans a workspace;
    // otherwise the crate the document sits in. Either way the rest of the
    // anchor is a module path *inside* that crate, and the type has to be
    // declared at or under it -- "resolved under its own anchor, never
    // guessed at" is what this file tells the user two messages down, and a
    // crate-wide scan would make that untrue.
    let mut segments: Vec<String> = comp.anchor.split("::").map(str::to_string).collect();
    let head = segments.first().cloned().unwrap_or_default();
    let dir = match local {
        Some(dir) => {
            // The anchor may name this crate itself rather than a module of
            // it (`crates/ply-core/ply.yaml` anchors at `ply_core`), and
            // reading that as a module would look for `ply_core` inside
            // `ply_core`. Caught by running `check` on that document, not
            // by review.
            if ply_core::harness::crate_own_name(dir).as_deref() == Some(head.as_str()) {
                segments.remove(0);
            }
            Some(dir.to_path_buf())
        }
        None => {
            let found = crates.get(&head).cloned();
            if found.is_some() {
                segments.remove(0);
            }
            found
        }
    };
    // A claim Ply could not look at must not read like a claim it looked at
    // and was satisfied by (§1). Measured before this existed: a completely
    // invented type in a workspace-root document passed with exit 0, which
    // is the false clean this whole grammar was added to prevent.
    let Some(dir) = dir else {
        diagnostics.push(state_diag_warning(
            "W0413",
            qualified,
            format!(
                "`{qualified}` says its state lives in `{of}`, and Ply could not check that. \
                 Resolving a state claim means reading the crate this component is anchored \
                 at, and Ply found no crate named `{head}` with source of its own under \
                 this document. The claim is not wrong; it is unverified, and this line \
                 says so rather than letting it pass in silence. (W0413)",
                of = state.of,
            ),
        ));
        return;
    };
    let Some(fields) = ply_core::harness::scan_type_fields_under(&dir, &segments, &state.of) else {
        // Three different facts, three different sentences. A name declared
        // twice used to read as a name not declared at all, which sends the
        // reader looking for a type that is sitting right there (2026-09-04).
        let sites = ply_core::harness::type_declaration_sites(&dir, &state.of);
        let where_at = |m: &String| {
            if m.is_empty() {
                "the crate root".to_string()
            } else {
                format!("`{m}`")
            }
        };
        let message = match sites.len() {
            0 => format!(
                "`{qualified}` says its state lives in `{of}`, but this crate declares no \
                 type called that anywhere. Check the spelling. (A0414)",
                of = state.of,
            ),
            1 => format!(
                "`{qualified}` says its state lives in `{of}`, but `{anchor}` does not \
                 declare it -- {site} does. A component's state is resolved under its own \
                 anchor, never guessed at, so a type of that name declared elsewhere in \
                 the crate is not this component's and was not accepted. Move the claim to \
                 the component that type belongs to, or re-anchor this one. (A0414)",
                of = state.of,
                anchor = comp.anchor,
                site = where_at(&sites[0]),
            ),
            _ => format!(
                "`{qualified}` says its state lives in `{of}`, and this crate declares more \
                 than one type by that name: {list}. Ply will not guess which one a \
                 component holds, so the claim was not accepted -- and this is not a \
                 missing type, it is an ambiguous one. Either rename one of them, or \
                 anchor this component at the module that owns the one you mean. (A0414)",
                of = state.of,
                list = sites.iter().map(where_at).collect::<Vec<_>>().join(" and "),
            ),
        };
        diagnostics.push(state_diag("A0414", qualified, message));
        return;
    };
    let declared: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    for wanted in &state.show {
        let Some(field) = fields.iter().find(|f| f.name == wanted.name) else {
            diagnostics.push(state_diag(
                "A0415",
                qualified,
                format!(
                    "`{qualified}` asks to show `{of}`'s `{name}` field, but `{of}` has no \
                     field called that. The fields it does have are: {have}. Field names come \
                     from the code, so this line cannot be drawn as written. (A0415)",
                    of = state.of,
                    name = wanted.name,
                    have = if declared.is_empty() {
                        "none it can name -- it is a tuple struct, whose fields have no names \
                         to show"
                            .to_string()
                    } else {
                        declared
                            .iter()
                            .map(|d| format!("`{d}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    },
                ),
            ));
            // A field the type does not have is `A0415`'s failure alone --
            // there is nothing here to compare a declared shape against, so
            // this entry does not also get a second report under `A0416`.
            continue;
        };
        // §7.1 (2026-09-04): the field resolved, so a declared shape --
        // written as one of the seven tokens, never a type -- is now
        // something to check rather than to draw. `classify` is the same
        // call the renderer makes on this exact field, never a second
        // reading of `RustType` here: the two must agree about
        // `Option<Vec<T>>`, a type alias, or `Arc<RwLock<..>>`, or the
        // drawing and this diagnostic would disagree about the same field.
        let Some(declared_shape) = wanted.declared else {
            continue;
        };
        let real_shape = ply_core::visual::state_shapes::classify(&field.ty, &field.rendered);
        if real_shape == declared_shape.to_field_shape() {
            tally.declared_shapes_checked += 1;
        } else {
            diagnostics.push(state_diag(
                "A0416",
                qualified,
                format!(
                    "`{qualified}` declares `{of}`'s `{name}` field as {declared_noun}, but the \
                     code says it is {real_noun} (written `{written}`). A declared shape is \
                     checked against the code the moment there is code to check it against, so \
                     a declaration and the source may not disagree. Two ways to close this: \
                     change the declaration to match what the code really is, or -- if the \
                     document is right and the code drifted -- treat this as the regression it \
                     is and fix the code. A declaration is also never required once code \
                     exists; dropping back to the plain name (`{name}` with no shape) is always \
                     a legitimate way to retire it. (A0416)",
                    of = state.of,
                    name = wanted.name,
                    declared_noun = declared_shape.to_field_shape().noun(),
                    real_noun = real_shape.noun(),
                    written = field.rendered,
                ),
            ));
        }
    }
}

fn state_diag_warning(code: &str, node_id: &str, title: String) -> Diagnostic {
    Diagnostic {
        severity: "warning".into(),
        ..state_diag(code, node_id, title)
    }
}

/// A `state:` finding: the document claimed something about the code that
/// the code does not say. An error, not a warning -- a picture drawn from an
/// invented field is worse than no picture, and the fix is always one edit.
fn state_diag(code: &str, node_id: &str, title: String) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: "error".into(),
        phase: "check".into(),
        engine: "ply".into(),
        check: "anchors".into(),
        node_id: node_id.into(),
        title,
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

fn arch_diag(f: &ArchFinding) -> Diagnostic {
    Diagnostic {
        code: f.code.into(),
        severity: "error".into(),
        phase: "check".into(),
        engine: "ply".into(),
        check: "architecture".into(),
        node_id: f.node_id.clone(),
        title: f.message.clone(),
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// A derived cross-document link's finding (§7.1's derive-links brief:
/// `A0417` target missing/unparseable, `W0532` anchor drift, `W0533`
/// duplicate claim, `W0534` a chain that leads back into itself),
/// attached to the *including* component -- `ply_core::config::LinkFinding`
/// already carries its own severity, since three of the four are advisory
/// (the link simply does not form) and one is a real defect in the target
/// document.
fn link_diag(f: &ply_core::config::LinkFinding) -> Diagnostic {
    Diagnostic {
        code: f.code.into(),
        severity: f.severity.into(),
        phase: "check".into(),
        engine: "ply".into(),
        check: "architecture".into(),
        node_id: f.component_path.clone(),
        title: f.message.clone(),
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    }
}

/// What the anchor tier managed to look at — the numbers the human summary
/// reports, so a reader can tell "all fine" from "nothing was looked at".
///
/// **`resolved` now also counts a `Type::method` claim Ply found but
/// refuses to check for a stated, scope reason** -- a receiver method, a
/// trait method, a generic `impl` block, or two candidates its syntactic
/// reader cannot pick between -- exactly as `verify` counts them among the
/// fns it resolved (adversarial review, 2026-08-27: before this, `check`
/// was the one command that had no pre-check for these, so it folded all
/// four into `unresolved` under `E0301`, the code that means "this claim
/// describes nothing" — which made `check` and `verify` report two
/// different numbers for the same document: 6 of 39 here, 39 of 39 there,
/// for a crate where every single claim named a real method). A private fn
/// Ply cannot verify from, or a signature/contract shape it cannot read,
/// still count as `unresolved` here -- deliberately unchanged, because
/// `verify` folds those two into its own `unclaimed` bucket the same way
/// today; narrowing `unresolved` further than that would just move the
/// disagreement rather than close it.
pub struct AnchorTally {
    pub resolved: usize,
    pub unresolved: usize,
    /// Fn claims on a component anchored to another crate. Not a defect:
    /// `verify` is single-crate, so their anchors simply cannot be resolved
    /// from here, and reporting them as errors would be wrong.
    pub elsewhere: usize,
    /// There was no `src/lib.rs` to resolve any claim against, so the
    /// counts above describe a search that never happened. The summary
    /// sentence has to say that outright rather than report a tidy zero.
    pub no_library: bool,
    /// How many local fn claims write a `requires:`/`ensures:` contract
    /// directly in ply.yaml *and* also declare a non-empty `checks:` list of
    /// their own -- they are asking to be verified (2026-08-30, "a
    /// documented way of writing contracts is accepted, then silently
    /// ignored"): `verify` never adds the ply.yaml contract to a claim's own
    /// checks -- only inline `#[ply::requires]`/`#[ply::ensures]`
    /// attributes do -- and `check` used to give no hint of that at all, so
    /// the very first command a user runs looked clean right up until
    /// `verify` quietly checked nothing against it.
    ///
    /// Kept separate from `yaml_contract_boundary_fns` (2026-08-31 review,
    /// "`check` tells users of the boundary-contract feature to destroy
    /// it"): a fn with no `checks:` of its own that declares a ply.yaml
    /// contract is not an oversight, it is §5.5's boundary-contract
    /// declaration working exactly as designed, and telling its author to
    /// "move the contract onto it as an attribute" is advice to delete the
    /// feature the fixture exists to demonstrate. The old single counter
    /// fired for both shapes alike and gave that advice to both.
    pub yaml_contract_checked_fns: usize,
    /// The one such fn's own name, kept only when there is exactly one
    /// (`yaml_contract_checked_fns == 1`) -- naming it beats "1 of them"
    /// when there is only one "them" to mean.
    pub yaml_contract_checked_fn_name: Option<String>,
    /// How many local fn claims declare a ply.yaml `requires:`/`ensures:`
    /// contract but ask for **no** `checks:` of their own -- §5.5's
    /// boundary-contract declaration: legacy code nobody edits, given a
    /// contract from outside so a caller's proof can assume it, with no
    /// intent that anything here ever verify the callee itself.
    pub yaml_contract_boundary_fns: usize,
    /// The one such fn's own name, kept only when there is exactly one
    /// (`yaml_contract_boundary_fns == 1`).
    pub yaml_contract_boundary_fn_name: Option<String>,
    /// How many `show:` entries declared a shape that the real field agrees
    /// with (2026-09-04) -- a kept promise, reported as confirmed rather
    /// than passing in silence, because this project counts evidence out
    /// loud. Only ever incremented where a field actually resolved: with no
    /// code there is nothing to agree or disagree with, which is `A0416`'s
    /// own gate too.
    pub declared_shapes_checked: usize,
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
        no_library: false,
        yaml_contract_checked_fns: 0,
        yaml_contract_checked_fn_name: None,
        yaml_contract_boundary_fns: 0,
        yaml_contract_boundary_fn_name: None,
        declared_shapes_checked: 0,
    };
    let lib_path = crate_dir.join("src/lib.rs");
    let Ok(lib_src) = std::fs::read_to_string(&lib_path) else {
        // There is no library here to resolve anything against. This used
        // to be counted as "anchored to another crate", which reads as a
        // deliberate boundary and let the command exit 0 having resolved
        // nothing -- a binary-only crate (`src/main.rs`, no `src/lib.rs`)
        // got a clean `check` for claims `verify` then refused by name
        // (`E0301`, exit 1). A run that could not look must not read like a
        // run that looked and was satisfied (§1), and the two commands have
        // to agree about the same fact, so every claim is unresolved and
        // says why.
        // Nested components carry claims too, so this walks the same tree
        // `count_fn_claims` does rather than only the top level -- a claim
        // Ply cannot look for must be reported wherever it was written.
        fn walk_no_library(
            qualified: &str,
            comp: &Component,
            lib_path: &Path,
            crates: &std::collections::BTreeMap<String, PathBuf>,
            diagnostics: &mut Vec<Diagnostic>,
            tally: &mut AnchorTally,
        ) {
            // A fn claim here really is unresolvable -- there is no library
            // to read. A `state:` claim is not: this document's components
            // are anchored at crates that have libraries of their own, and
            // reading one of those is the same read `walk_anchors` does. So
            // the workspace-root document, which used to get a warning
            // saying nothing could be checked, is now checked.
            verify_state(qualified, comp, crates, None, diagnostics, tally);
            for fn_name in comp.fns.keys() {
                tally.unresolved += 1;
                diagnostics.push(no_library_diag(
                    &format!("{qualified}::{fn_name}"),
                    fn_name,
                    lib_path,
                ));
            }
            for (child, sub) in &comp.components {
                walk_no_library(
                    &format!("{qualified}::{child}"),
                    sub,
                    lib_path,
                    crates,
                    diagnostics,
                    tally,
                );
            }
        }
        // The crates this document's components are anchored at. Found from
        // the directory the document sits in, which for a workspace-root
        // document is the workspace.
        let crates = ply_core::harness::workspace_library_crates(crate_dir);
        for (name, comp) in &doc.components {
            walk_no_library(name, comp, &lib_path, &crates, diagnostics, &mut tally);
        }
        debug_assert_eq!(tally.unresolved, count_fn_claims(doc));
        tally.no_library = true;
        return tally;
    };
    let local_anchors = local_anchor_names(crate_dir);
    // Where each component's state type may live. A single-crate document
    // resolves everything locally and never consults this; a document that
    // names components in other crates resolves each against the crate its
    // anchor points at.
    let state_crates = ply_core::harness::workspace_library_crates(crate_dir);
    let known_fns = harness::crate_fn_paths(&lib_path).unwrap_or_default();
    let mut resolver = Resolver::new(&lib_src, crate_dir, BTreeMap::new()).ok();

    for (name, comp) in &doc.components {
        walk_anchors(
            name,
            comp,
            &lib_path,
            &state_crates,
            &local_anchors,
            &known_fns,
            resolver.as_mut(),
            diagnostics,
            &mut tally,
            &doc.routes,
        );
    }
    tally
}

#[allow(clippy::too_many_arguments)]
fn walk_anchors(
    qualified: &str,
    comp: &Component,
    lib_path: &Path,
    state_crates: &std::collections::BTreeMap<String, PathBuf>,
    local_anchors: &[String],
    known_fns: &[String],
    mut resolver: Option<&mut Resolver>,
    diagnostics: &mut Vec<Diagnostic>,
    tally: &mut AnchorTally,
    routes: &indexmap::IndexMap<String, String>,
) {
    // The same locality test `verify` applies (§5.5): a component anchored
    // to another crate is a boundary component, and this slice reads its
    // declared contracts rather than its code.
    // `state:` is checked against the real source before anything else in
    // this component, because it is the one claim in the grammar whose whole
    // value is that it cannot be invented. A document names a type and some
    // of its fields; nothing stops whoever wrote it -- a person in a hurry,
    // or a model that guessed -- from naming a field nobody declared. Drawn
    // unchecked, that is a confident picture of code that does not exist.
    // A component anchored inside this crate resolves against this crate;
    // one anchored at another crate resolves against that one. Both are the
    // same read, so both go through `verify_state`, which returns at once
    // for a component that declares no state at all.
    let local = lib_path
        .parent()
        .and_then(|src| src.parent())
        .filter(|_| is_local(local_anchors, &comp.anchor) && lib_path.exists());
    verify_state(qualified, comp, state_crates, local, diagnostics, tally);
    if let Some(module_path) = crate::shared::local_module_path(local_anchors, &comp.anchor) {
        for (fn_name, claim) in &comp.fns {
            let node_id = format!("{qualified}::{fn_name}");
            // §5.2, the same resolution `verify` does: a key is written
            // relative to the component's own anchor, and both the resolver
            // and the generated harness spell a function from the crate
            // root. The two commands must agree about which claims point at
            // real code, so this is the one place either of them adjusts it.
            let fn_path = crate::shared::crate_root_fn_key(&module_path, fn_name);
            let fn_path = fn_path.as_str();
            if !claim.requires.is_empty() || !claim.ensures.is_empty() {
                // §5.5: a claim with no `checks:` of its own is a boundary
                // contract declaration, not a request to be verified --
                // "asks to be checked" and "asks for nothing" get different
                // advice below (2026-08-31 review, "`check` tells users of
                // the boundary-contract feature to destroy it").
                let has_own_checks = claim.checks.as_ref().is_some_and(|c| !c.is_empty());
                if has_own_checks {
                    tally.yaml_contract_checked_fns += 1;
                    tally.yaml_contract_checked_fn_name = if tally.yaml_contract_checked_fns == 1 {
                        Some(fn_name.clone())
                    } else {
                        None
                    };
                } else {
                    tally.yaml_contract_boundary_fns += 1;
                    tally.yaml_contract_boundary_fn_name = if tally.yaml_contract_boundary_fns == 1
                    {
                        Some(fn_name.clone())
                    } else {
                        None
                    };
                }
            }
            // The same pre-check `verify` runs, in the same order, for the
            // same reason (`verify.rs`'s own comment on this): a
            // `Type::method` claim naming something real but out of this
            // slice's scope (a receiver, a generic `impl` block, a trait
            // method) or that the syntactic reader cannot pick between must
            // never be reported as "could not find the function" -- that is
            // false, Ply found it and is refusing it for a stated reason.
            // Before this, only `verify` ran this pre-check, so this
            // command folded all four of those into `unresolved` under the
            // "describes nothing" code, and the two commands' headline
            // counts disagreed (adversarial review, 2026-08-27).
            if let Some(r) = resolver.as_deref_mut() {
                match r.lookup_fn(fn_path) {
                    ply_core::callgraph::Resolution::Refused(reason) => {
                        tally.resolved += 1;
                        // The resolver refuses every method that takes a
                        // receiver, because it predates receiver
                        // construction. `verify` no longer stops there: it
                        // tries to build one from the type's own constructor
                        // plus a bounded sequence of the type's own
                        // operations, and where that succeeds it checks the
                        // method and finds real violations. Reporting the
                        // resolver's blanket refusal here told users a
                        // feature that works does not exist -- and `check`
                        // is the command people run first, so they would
                        // never reach `verify` to find out otherwise
                        // (adversarial review, 2026-08-27). Ask the same
                        // question `verify` asks, and only report the
                        // refusal when the answer is genuinely no.
                        let buildable = lib_path.parent().and_then(|src| src.parent()).is_some_and(
                            |crate_dir| {
                                ply_core::harness::discover_method_with_receiver(
                                    crate_dir, fn_path, routes,
                                )
                                .is_ok()
                            },
                        );
                        if !buildable {
                            diagnostics.push(crate::verify::refused_anchor_diag(&node_id, &reason));
                        }
                        continue;
                    }
                    ply_core::callgraph::Resolution::Ambiguous(reason) => {
                        tally.resolved += 1;
                        diagnostics.push(crate::verify::ambiguous_anchor_diag(
                            &node_id, fn_name, &reason,
                        ));
                        continue;
                    }
                    ply_core::callgraph::Resolution::Found(_)
                    | ply_core::callgraph::Resolution::Opaque(_)
                    | ply_core::callgraph::Resolution::NotFound => {}
                }
            }
            // The same resolver `verify` anchors with, so the two commands
            // cannot disagree about which claims point at real code -- and,
            // since 2026-08-25, the same one call classification uses, so
            // Ply can no longer name a callee as unvouched-for and then
            // refuse the claim that would vouch for it.
            let outcome = match resolver.as_deref_mut() {
                Some(r) => harness::resolve_anchor(r, fn_path, lib_path).err(),
                None => Some(harness::AnchorError::Unreadable(format!(
                    "Ply could not parse {} at all, so no claim in this document could be \
                     resolved against it",
                    lib_path.display()
                ))),
            };
            match outcome {
                None => tally.resolved += 1,
                Some(err) => {
                    tally.unresolved += 1;
                    diagnostics.push(unresolved_anchor_diag(
                        &node_id,
                        fn_name,
                        fn_path,
                        &module_path,
                        known_fns,
                        &err,
                        lib_path,
                    ));
                }
            }
        }
    } else {
        tally.elsewhere += comp.fns.len();
    }
    for (child, nested) in &comp.components {
        walk_anchors(
            &format!("{qualified}.{child}"),
            nested,
            lib_path,
            state_crates,
            local_anchors,
            known_fns,
            resolver.as_deref_mut(),
            diagnostics,
            tally,
            routes,
        );
    }
}

/// §5.2: "an unresolvable anchor → `E0301` with nearest-name suggestions
/// (edit distance over the item index)". **A renamed function must break
/// CI, not silently orphan its claims.**
fn unresolved_anchor_diag(
    node_id: &str,
    fn_name: &str,
    fn_path: &str,
    module_path: &str,
    known_fns: &[String],
    err: &AnchorError,
    lib_path: &Path,
) -> Diagnostic {
    // Four different facts, four different sentences. Saying "could not
    // find" about a function that is right there sends a reader hunting for
    // a typo that is not there — which is the whole reason this branches.
    let title = match err {
        AnchorError::NotFound => {
            // Matched on the *crate-root* key, because that is the shape
            // `known_fns` holds: comparing the key as written under a module
            // anchor (`dottted`) against `schema::dotted` puts every real
            // name far outside edit distance, and the suggestion goes quiet
            // exactly where module anchors are used (2026-09-04). Shown
            // back relative to the same anchor, so it can be pasted straight
            // over the typo.
            // Edit distance cannot see a claim that sits under the wrong
            // module: `visual::examples_prose` is five characters from
            // `visual::svg::examples_prose`, so the reader was told the
            // function does not exist when it plainly does, just one box
            // over. Matching on the final segment finds it, and is only
            // consulted when the distance match found nothing.
            let same_leaf = || {
                let leaf = fn_path.rsplit("::").next().unwrap_or(fn_path);
                let mut hits = known_fns
                    .iter()
                    .filter(|k| k.rsplit("::").next() == Some(leaf));
                // One unambiguous answer only: two functions with the same
                // name in different modules is a question, not a suggestion.
                let first = hits.next()?;
                hits.next().is_none().then(|| first.clone())
            };
            // Two different facts, two different sentences: a near-miss on
            // spelling is a rename, a same-name-elsewhere is a claim under
            // the wrong box. Telling someone their function "was renamed"
            // when it is sitting one module over sends them looking for a
            // change nobody made.
            let by_distance = schema::nearest_key(fn_path, known_fns);
            let moved = by_distance.is_none();
            let suggestion = match by_distance.or_else(same_leaf) {
                Some(near) => {
                    let shown = near
                        .strip_prefix(module_path)
                        .and_then(|rest| rest.strip_prefix("::"))
                        .filter(|_| !module_path.is_empty())
                        .unwrap_or(near.as_str());
                    if moved {
                        format!(
                            " There is a function of that name at `{shown}` — the claim is \
                             under the wrong component, or its anchor points at the wrong \
                             module."
                        )
                    } else {
                        format!(
                            " The closest name Ply can see is `{shown}` — if the function was \
                             renamed, the claim needs renaming with it."
                        )
                    }
                }
                None if known_fns.is_empty() => {
                    format!(" Ply found no functions at all in {}.", lib_path.display())
                }
                None => String::new(),
            };
            format!(
                "Ply could not find a function called `{fn_name}` in {}, or in any module it \
                 declares, so this claim describes nothing.{suggestion}",
                lib_path.display()
            )
        }
        AnchorError::Private(reason) => format!(
            "Ply found `{fn_name}` but cannot verify from it: {reason}. Make it (and every \
             module between it and the crate root) `pub` or `pub(crate)`, or move the claim to \
             a function that is reachable."
        ),
        AnchorError::Unreadable(reason) => format!(
            "Ply could not read the source `{fn_name}` would be in: {reason}. Not being able to \
             look is not the same as there being nothing there, so this claim is reported as \
             unresolved rather than assumed fine."
        ),
        AnchorError::Shape(e) => format!(
            "Ply found `{fn_name}` and cannot read its shape: {e}. The claim points at real \
             code; what this slice cannot handle is the function's signature or its contract."
        ),
    };
    // `Private`/`Shape` are left exactly as they were -- `E0301`, error,
    // "unresolvable_anchor" -- and deliberately so: `verify` itself reports
    // both of these the same way today (`discover_fn_with` wraps every
    // `AnchorError` variant as one `anyhow::Error` and `verify`'s own
    // `unresolved_anchor_diag` gives all of them `E0301`/error, folding the
    // resulting fn into `unclaimed`). Only `Refused`/`Ambiguous` earned a
    // sharper, warning-level diagnostic in `verify` (checked *before*
    // `discover_fn_with` even runs), and this tier's pre-check above now
    // mirrors exactly that split -- narrowing this function to the two
    // facts `verify` still folds together here would make `check` disagree
    // with `verify` in the opposite direction, which is not the fix.
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

fn coverage(anchors: Option<AnchorTally>, arch: ArchOutcome) -> Coverage {
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

    let mut not_checked = Vec::new();
    match arch {
        ArchOutcome::Ran(tally) => {
            checked.push(Tier {
                tier: "architecture".into(),
                detail: arch_detail(&tally),
            });
            not_checked.push(Tier {
                tier: "item-level".into(),
                detail: ITEM_TIER_GAP.into(),
            });
        }
        ArchOutcome::Unavailable(reason) => {
            not_checked.push(Tier {
                tier: "architecture".into(),
                detail: format!(
                    "NOT CHECKED. Ply could not get this crate's real dependency graph, so \
                     neither the crate-level nor the item-level part of the architecture check \
                     ran: {reason}"
                ),
            });
        }
        ArchOutcome::NotReached => {
            not_checked.push(Tier {
                tier: "architecture".into(),
                detail: ARCH_NOT_REACHED.into(),
            });
        }
    }

    Coverage {
        checked,
        not_checked,
    }
}

/// The claim could not be looked for at all, because there is no library
/// here to look in. Written to be actionable by someone who has never seen
/// Ply: name the missing thing, say what it does *not* mean, then give the
/// two things that are actually wrong in practice.
fn no_library_diag(node_id: &str, fn_name: &str, lib_path: &Path) -> Diagnostic {
    Diagnostic {
        code: "E0301".into(),
        severity: "error".into(),
        phase: "check".into(),
        engine: "ply".into(),
        check: "anchor".into(),
        node_id: node_id.into(),
        title: format!(
            "there is no library for Ply to look in, so `{fn_name}` was never searched for. \
             Ply reads a crate's library code, which lives in {}, and that file does not \
             exist here. This does not mean `{fn_name}` is missing -- nothing was looked at, \
             which is a different and worse answer. If this crate is a binary (a `src/main.rs` \
             and no `src/lib.rs`), move the code you want checked into a `src/lib.rs` and have \
             `main.rs` call it. If you meant a different crate, point Ply at that crate's own \
             directory rather than this one.",
            lib_path.display()
        ),
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: Some("unresolvable_anchor".into()),
    }
}

fn anchor_detail(t: &AnchorTally) -> String {
    if t.no_library && t.unresolved == 0 {
        // A document with no fn claims at all has nothing to resolve, so
        // the missing library cost it nothing. Saying "NOT RESOLVED" here
        // would report a failure that did not happen -- the mirror of the
        // bug this branch is otherwise about. Ply's own root `ply.yaml`,
        // which declares only architecture, is exactly this case.
        return "This document declares no fn claims, so there was nothing for this tier to \
                resolve."
            .to_string();
    }
    if t.no_library {
        let claims = if t.unresolved == 1 {
            "the one fn claim in this document was never looked for".to_string()
        } else {
            format!(
                "none of the {} fn claims in this document were ever looked for",
                t.unresolved
            )
        };
        return format!(
            "NOT RESOLVED. Ply resolved nothing: {claims}, because there is no `src/lib.rs` here for Ply \
             to look in. This is not a count of zero problems -- it is a count of zero \
             searches."
        );
    }
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
    // §5.5: a ply.yaml `requires:`/`ensures:` contract is used exactly one
    // way -- a caller crossing into that fn may assume it -- and is never
    // added to the fn's own checks. That fact reads differently depending
    // on whether the fn also asked to be checked (2026-08-31 review,
    // "`check` tells users of the boundary-contract feature to destroy
    // it"): telling a fn with no `checks:` of its own to "move the contract
    // onto it as an attribute" is advice to delete a feature that is
    // working exactly as intended for legacy code nobody wants to edit.
    if t.yaml_contract_checked_fns == 1 {
        let fn_name = t
            .yaml_contract_checked_fn_name
            .as_deref()
            .unwrap_or("the function");
        s.push_str(&format!(
            " 1 of them also writes a `requires:`/`ensures:` contract directly in ply.yaml. \
             That contract is used both ways: `verify` checks `{fn_name}` against it, \
             alongside any `#[ply::requires]`/`#[ply::ensures]` attribute written on \
             `{fn_name}` itself, and a caller of `{fn_name}` may assume it at a boundary."
        ));
    } else if t.yaml_contract_checked_fns > 1 {
        s.push_str(&format!(
            " {} of them also write a `requires:`/`ensures:` contract directly in ply.yaml. \
             Those contracts are used both ways: `verify` checks each function against its \
             own, alongside any `#[ply::requires]`/`#[ply::ensures]` attribute written on it, \
             and callers may assume them at a boundary.",
            t.yaml_contract_checked_fns
        ));
    }
    if t.yaml_contract_boundary_fns == 1 {
        let fn_name = t
            .yaml_contract_boundary_fn_name
            .as_deref()
            .unwrap_or("the function");
        s.push_str(&format!(
            " 1 of them, `{fn_name}`, declares a `requires:`/`ensures:` contract in ply.yaml but \
             asks for no checks of its own -- that is deliberate: it lets any function calling \
             `{fn_name}` assume the promise is true, without Ply ever verifying that \
             `{fn_name}` itself keeps it. This is the normal way to describe legacy code you do \
             not want to edit; any caller's result will say it rests on an unchecked promise."
        ));
    } else if t.yaml_contract_boundary_fns > 1 {
        s.push_str(&format!(
            " {} of them declare a `requires:`/`ensures:` contract in ply.yaml but ask for no \
             checks of their own -- that is deliberate: it lets any function calling them \
             assume the promise is true, without Ply ever verifying that they keep it. This is \
             the normal way to describe legacy code you do not want to edit; any caller's \
             result will say it rests on these unchecked promises.",
            t.yaml_contract_boundary_fns
        ));
    }
    // §7.1 (2026-09-04): a `show:` entry may declare its field's shape, and
    // once code exists that declaration is checked rather than drawn. A
    // disagreement is its own error (`A0416`, reported below); an agreement
    // is not silence -- it is a kept promise, and this project counts
    // evidence out loud rather than only naming its absence.
    if t.declared_shapes_checked == 1 {
        s.push_str(
            " 1 declared field shape was checked against the source and it is what the code \
             says. A confirmed declaration may be kept as documentation or dropped back to \
             the plain field name -- both are fine once the code agrees with it.",
        );
    } else if t.declared_shapes_checked > 1 {
        s.push_str(&format!(
            " {n} declared field shapes were checked against the source and all {n} are what \
             the code says. A confirmed declaration may be kept as documentation or dropped \
             back to the plain field name -- both are fine once the code agrees with it.",
            n = t.declared_shapes_checked
        ));
    }
    s
}

/// §5.3's crate tier, told as a coverage sentence: how many real crate
/// dependencies actually cross between two differently-declared
/// components, and how they were classified.
///
/// docs/review-architecture-tier.md, findings 3 and 4:
/// - **Finding 4** decided: `dev-dependencies`/`build-dependencies` stay
///   excluded from enforcement (§5.3 is about code that ships, and a test
///   harness legitimately wires components together in ways the shipped
///   binary never does) -- but disclosed, so this sentence never again
///   claims "no crate here depends on another" when one does, just not at
///   runtime. "at runtime" below is the qualifier that keeps the base
///   sentence honest even when every real crossing is a dev/build one.
/// - **Finding 3** adds the denominator: how many crates in this workspace
///   `cargo metadata` actually reports, and how many of those no declared
///   component's anchor claims at all -- named, not left for a wildcard
///   `deny:` to imply it already covers them (`*` only ever means "any
///   *declared* component").
fn arch_detail(t: &ArchTally) -> String {
    let mut s = if t.cross_component_pairs == 0 {
        "No crate here depends on another crate that belongs to a different declared \
         component at runtime, so there was nothing to check."
            .to_string()
    } else {
        let permitted = t.cross_component_pairs - t.violations;
        let (n, verb) = if t.cross_component_pairs == 1 {
            ("1 real crate dependency".to_string(), "crosses")
        } else {
            (
                format!("{} real crate dependencies", t.cross_component_pairs),
                "cross",
            )
        };
        let mut s = format!(
            "{n} {verb} between two differently-declared components: {permitted} permitted by a \
             declared edge or by nesting, {} not permitted (reported below).",
            t.violations
        );
        if t.deny_violations > 0 {
            s.push_str(&format!(
                " {} of them also match an explicit `deny:` rule (reported below).",
                t.deny_violations
            ));
        }
        s
    };

    if t.dev_or_build_cross_component_pairs > 0 {
        let plural = if t.dev_or_build_cross_component_pairs == 1 {
            "crosses"
        } else {
            "cross"
        };
        s.push_str(&format!(
            " {} more {plural} a declared boundary only as a test or build dependency \
             (`dev-dependencies`/`build-dependencies`) — not enforced, because that code never \
             ships, but named here rather than dropped silently.",
            t.dev_or_build_cross_component_pairs
        ));
    }

    s.push_str(&format!(
        " {} of {} crates in this workspace belong to a declared component.",
        t.declared_crate_count, t.workspace_crate_count
    ));
    if !t.undeclared_crates.is_empty() {
        s.push_str(&format!(
            " Not declared, and so invisible even to a wildcard `deny:` rule: {}.",
            t.undeclared_crates.join(", ")
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
        trust_surface: None,
        open_items: None,
        not_carried_forward: vec![],
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

#[cfg(test)]
mod tests {
    /// A crate whose code sits in a module named `probe`, matching the
    /// anchor every document below uses.
    ///
    /// The module matters: a component's state is resolved *under its own
    /// anchor*, so a type sitting at the crate root is not the state of a
    /// component anchored at `probe`, however much its name matches. These
    /// fixtures used to put the type at the root and pass, which is the
    /// misfiling this rule exists to catch -- they were testing a checker
    /// that could not tell where a type lived.
    fn state_probe(lib: &str, yaml: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub mod probe;\n").unwrap();
        std::fs::write(dir.path().join("src/probe.rs"), lib).unwrap();
        std::fs::write(dir.path().join("ply.yaml"), yaml).unwrap();
        dir
    }

    const BOOK: &str = r#"
use std::collections::BTreeMap;
pub struct OrderBook {
    pub bids: BTreeMap<u64, u32>,
    ticks: Vec<u64>,
}
"#;

    /// The reason `state:` is worth having at all. A document names a type
    /// and the fields worth drawing; nothing stops whoever wrote it -- a
    /// person in a hurry, or a model that guessed -- from naming a field
    /// that does not exist. If Ply drew that as a row, the picture would be
    /// a confident lie about code nobody wrote.
    ///
    /// So a field the type does not declare is a finding, and the finding
    /// names the fields that *do* exist, because "no such field" without
    /// that list makes a typo take three guesses to fix.
    #[test]
    fn a_state_field_the_type_does_not_declare_is_refused_by_name() {
        let dir = state_probe(
            BOOK,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show: [bids, invented]\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);

        let found = diagnostics
            .iter()
            .find(|d| d.code == "A0415")
            .expect("a field nobody declared must be reported, never drawn");
        assert!(
            found.title.contains("invented"),
            "the finding must name the field that is not there: {}",
            found.title
        );
        assert!(
            found.title.contains("bids") && found.title.contains("ticks"),
            "and the fields that are, so a typo takes one guess rather than three: {}",
            found.title
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == "A0415" && d.title.contains("`bids` is")),
            "a field that really exists must not be reported"
        );
    }

    /// A type declared in two modules used to be reported as one the crate
    /// "declares no type called" -- the scanner stores a duplicate name as
    /// "ambiguous", and the one place that reads it collapsed ambiguous and
    /// absent into the same answer. So a reader was sent hunting for a type
    /// sitting right there, twice. Found 2026-09-04 declaring state on
    /// ply-core's own `diag`, which has a `Diagnostic` and so does `check`.
    #[test]
    fn a_state_type_declared_in_two_modules_says_so_rather_than_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub mod probe;\npub mod other;\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/probe.rs"),
            "pub struct Thing { pub a: u32 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/other.rs"),
            "pub struct Thing { pub b: u32 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ply.yaml"),
            "ply: 1\ncomponents:\n  probe:\n    anchor: probe::probe\n    state:\n      of: Thing\n",
        )
        .unwrap();

        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);

        let found = diagnostics
            .iter()
            .find(|d| d.code == "A0414")
            .expect("an ambiguous state type must still be refused");
        assert!(
            !found.title.contains("declares no type called that"),
            "the type is declared -- twice. Saying it does not exist sends the \
             reader looking for something that is right there:\n{}",
            found.title
        );
        for module in ["probe", "other"] {
            assert!(
                found.title.contains(module),
                "the finding has to name where the duplicates are, or there is \
                 nothing to act on:\n{}",
                found.title
            );
        }
    }

    /// The same rule one level up: a type the crate does not declare at all.
    #[test]
    fn a_state_type_the_crate_does_not_declare_is_refused_by_name() {
        let dir = state_probe(
            BOOK,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: Invented\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);

        let found = diagnostics
            .iter()
            .find(|d| d.code == "A0414")
            .expect("a type nobody declared must be reported");
        assert!(
            found.title.contains("Invented"),
            "the finding must name the type: {}",
            found.title
        );
    }

    /// The dangerous case, and the reason this needed its own rule. A
    /// workspace-root document has no `src/lib.rs` of its own, so Ply cannot
    /// resolve a `state:` claim written there -- and a claim that is never
    /// looked at, passing silently, is precisely the false clean this
    /// project exists to refuse. Measured before the rule existed: a
    /// completely invented type in the root document passed with exit 0.
    ///
    /// So an unresolvable `state:` says so. It is a warning rather than an
    /// error because the document is not wrong, it is unverifiable from
    /// here -- but it is never silence.
    #[test]
    fn a_state_claim_ply_cannot_resolve_says_so_rather_than_passing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ply.yaml"),
            "ply: 1\ncomponents:\n  book:\n    anchor: some_other_crate\n    state:\n      of: Invented\n      show: [nonsense]\n",
        )
        .unwrap();
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);

        let found = diagnostics
            .iter()
            .find(|d| d.code == "W0413")
            .expect("a state claim Ply could not check must be named, never passed in silence");
        assert!(
            found.title.contains("Invented"),
            "it must name the claim it could not check: {}",
            found.title
        );
    }

    /// The case the warning above used to be the *only* answer to. A
    /// workspace-root document has no library of its own, but its
    /// components are anchored at crates that do -- so their state is
    /// checked against those, and a lie in the root document fails the
    /// build exactly as it would in a single-crate one.
    ///
    /// Measured before this existed: an entirely invented type in Ply's own
    /// root document passed with exit 0, and then, once the warning
    /// existed, passed with a warning. Neither is checking it.
    #[test]
    fn a_state_type_in_another_crate_is_resolved_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        // A workspace root: no `src/lib.rs` here, one library crate below.
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/ledger\"]\n",
        )
        .unwrap();
        let ledger = dir.path().join("crates/ledger");
        std::fs::create_dir_all(ledger.join("src")).unwrap();
        std::fs::write(
            ledger.join("Cargo.toml"),
            "[package]\nname = \"ledger-core\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            ledger.join("src/lib.rs"),
            "pub struct Book { pub bids: u64, pub ticks: u64 }",
        )
        .unwrap();

        // The anchor names the crate by its library path, dashes
        // underscored, exactly as Rust code would reach it.
        let honest = "ply: 1\ncomponents:\n  book:\n    anchor: ledger_core\n    state:\n      of: Book\n      show: [bids]\n";
        std::fs::write(dir.path().join("ply.yaml"), honest).unwrap();
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(honest).unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);
        assert!(
            !diagnostics
                .iter()
                .any(|d| ["W0413", "A0414", "A0415"].contains(&d.code.as_str())),
            "an honest claim about a type in another crate must be checked and pass, not \
             warned about: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );

        let lying = "ply: 1\ncomponents:\n  book:\n    anchor: ledger_core\n    state:\n      of: Book\n      show: [invented]\n";
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(lying).unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);
        let found = diagnostics
            .iter()
            .find(|d| d.code == "A0415")
            .expect("a field nobody declared must fail here too, not merely be warned about");
        assert!(
            found.title.contains("`bids`") && found.title.contains("`ticks`"),
            "and it must list the fields the other crate's type really has: {}",
            found.title
        );
    }

    /// An anchor may name the crate itself rather than a module of it, and
    /// that is not a misfiling. `crates/ply-core/ply.yaml` anchors its one
    /// component at `ply_core`, which is the crate, not a `ply_core` module
    /// inside `ply_core`.
    ///
    /// Caught by running `check` on that very document after anchor-scoped
    /// resolution landed, not by review: it went from exit 0 to exit 1 with
    /// an `A0414` about a type plainly sitting right there.
    #[test]
    fn an_anchor_naming_the_crate_itself_resolves_at_its_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub struct Book { pub bids: u64 }",
        )
        .unwrap();
        let yaml = "ply: 1\ncomponents:\n  book:\n    anchor: my_crate\n    state:\n      of: Book\n      show: [bids]\n";
        std::fs::write(dir.path().join("ply.yaml"), yaml).unwrap();

        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(yaml).unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);
        assert!(
            !diagnostics
                .iter()
                .any(|d| ["W0413", "A0414", "A0415"].contains(&d.code.as_str())),
            "the anchor names this crate, so its root is where the type lives: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
    }

    /// A document telling the truth earns silence -- the check must not
    /// invent work for a correct claim.
    #[test]
    fn a_state_claim_matching_the_code_reports_nothing() {
        let dir = state_probe(
            BOOK,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show: [bids, ticks]\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);
        let state_findings: Vec<&str> = diagnostics
            .iter()
            .filter(|d| d.code == "A0414" || d.code == "A0415")
            .map(|d| d.title.as_str())
            .collect();
        assert!(
            state_findings.is_empty(),
            "a claim that matches the code is not a finding: {state_findings:?}"
        );
    }

    /// §7.1 (2026-09-04): once a field resolves against real code, a
    /// declared shape is checked against it, never drawn instead of it. A
    /// mismatch is `A0416`, naming the field, what was declared, and what
    /// the code really is -- `bids` is a `BTreeMap`, a lookup table, and the
    /// document here declares it a list.
    #[test]
    fn a_declared_shape_disagreeing_with_the_code_is_refused_by_name() {
        let dir = state_probe(
            BOOK,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show:\n        bids: list\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);

        let found = diagnostics
            .iter()
            .find(|d| d.code == "A0416")
            .expect("a declaration the code disagrees with must be reported, never drawn");
        assert!(
            found.title.contains("`bids`"),
            "the finding must name the field: {}",
            found.title
        );
        assert!(
            found.title.contains("a list") && found.title.contains("a lookup table"),
            "it must say what was declared and what the code really is, in words rather \
             than a type name: {}",
            found.title
        );
        assert!(
            found.title.contains("BTreeMap<u64, u32>"),
            "and it must name the real type as the source spells it: {}",
            found.title
        );
    }

    /// The other half: a declaration the code agrees with is not silence.
    /// It is reported as a confirmed, counted fact -- and it must not also
    /// raise `A0416`, which is reserved for a disagreement.
    #[test]
    fn a_declared_shape_agreeing_with_the_code_is_confirmed_and_counted() {
        let dir = state_probe(
            BOOK,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show:\n        bids: map\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        let tally = check_anchors(dir.path(), &doc, &mut diagnostics);

        assert!(
            !diagnostics.iter().any(|d| d.code == "A0416"),
            "a declaration that agrees with the code must not be reported as a disagreement: \
             {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
        assert_eq!(
            tally.declared_shapes_checked, 1,
            "a kept promise is counted, not passed in silence"
        );
    }

    /// A declared shape on a field the type does not have is `A0415`'s
    /// failure alone -- there is nothing to compare a shape against when
    /// the field itself does not exist, so this must not also fire `A0416`.
    #[test]
    fn a_declared_shape_on_a_missing_field_fires_a0415_only() {
        let dir = state_probe(
            BOOK,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show:\n        invented: map\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);

        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == "A0415" && d.title.contains("invented")),
            "the missing field must still be refused by name: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
        assert!(
            !diagnostics.iter().any(|d| d.code == "A0416"),
            "a field that does not exist must not also be reported as a shape mismatch: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
    }

    /// A mapping entry with no value (YAML's `ticks:`) declares a name and
    /// nothing about its shape -- the same as the plain list form. It must
    /// raise neither finding and must not be counted as a checked
    /// declaration, because nothing was actually declared.
    #[test]
    fn a_null_declared_shape_raises_no_finding() {
        let dir = state_probe(
            BOOK,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show:\n        ticks:\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        let tally = check_anchors(dir.path(), &doc, &mut diagnostics);

        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == "A0415" || d.code == "A0416"),
            "a name with no declared shape must raise no state finding: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
        assert_eq!(
            tally.declared_shapes_checked, 0,
            "nothing was declared, so nothing was checked"
        );
    }

    /// `A0416` is only reachable once a field actually resolved against
    /// real source (the same gate `A0415` uses) -- with no crate to check
    /// against at all, the claim is unverified (`W0413`), never wrongly
    /// promoted to a shape disagreement nobody could have detected.
    #[test]
    fn an_unresolvable_state_claim_never_reports_a0416() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ply.yaml"),
            "ply: 1\ncomponents:\n  book:\n    anchor: some_other_crate\n    state:\n      of: Invented\n      show:\n        nonsense: list\n",
        )
        .unwrap();
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(dir.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(dir.path(), &doc, &mut diagnostics);

        assert!(
            diagnostics.iter().any(|d| d.code == "W0413"),
            "an unresolvable claim must still say so: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
        assert!(
            !diagnostics.iter().any(|d| d.code == "A0416"),
            "there is nothing here to disagree with, so this must never fire: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
    }

    /// The case anyone forking the comparison away from `classify()` would
    /// get wrong: a presence wrapper wins over what it wraps, so
    /// `Option<Vec<u64>>` *is* "might not be there" -- to the drawing and
    /// to this comparison alike. A declared `optional` over it agrees (and
    /// is counted); a declared `list` there is a genuine `A0416`, because
    /// the wrapper is the fact that matters first. A reimplementation that
    /// matched on `RustType` and let the inner `Vec` win would fail both
    /// halves of this test at once.
    #[test]
    fn a_presence_wrapper_wins_the_comparison_exactly_as_it_wins_the_drawing() {
        const WRAPPED: &str = r#"
pub struct OrderBook {
    pub pending: Option<Vec<u64>>,
}
"#;
        let agree = state_probe(
            WRAPPED,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show:\n        pending: optional\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(agree.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        let tally = check_anchors(agree.path(), &doc, &mut diagnostics);
        assert!(
            !diagnostics.iter().any(|d| d.code == "A0416"),
            "declared `optional` agrees with `Option<Vec<u64>>` -- the wrapper wins, the \
             same rule the drawing uses: {:?}",
            diagnostics.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
        assert_eq!(tally.declared_shapes_checked, 1);

        let disagree = state_probe(
            WRAPPED,
            "ply: 1\ncomponents:\n  book:\n    anchor: probe\n    state:\n      of: OrderBook\n      show:\n        pending: list\n",
        );
        let mut diagnostics = Vec::new();
        let doc = ply_core::model::parse_document(
            &std::fs::read_to_string(disagree.path().join("ply.yaml")).unwrap(),
        )
        .unwrap();
        check_anchors(disagree.path(), &doc, &mut diagnostics);
        let found = diagnostics
            .iter()
            .find(|d| d.code == "A0416")
            .expect("declared `list` over `Option<Vec<u64>>` is a real disagreement");
        assert!(
            found.title.contains("something that might not be there")
                && found.title.contains("Option<Vec<u64>>"),
            "the finding must say the code's shape the way the drawing would: {}",
            found.title
        );
    }

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

    /// A crate that really, on disk, depends on `ply-attrs` (this
    /// workspace's own tiny proc-macro crate, path-referenced absolutely) --
    /// the crate tier's own acceptance tests need one real cross-crate
    /// `cargo metadata` dependency to classify, and this is the cheapest
    /// real one available: no network, no extra fixture crate to maintain.
    /// `ply-attrs`'s own lib-target identifier (what an anchor must name)
    /// is `ply_attrs` -- the package name with its hyphen turned to an
    /// underscore, same as every other crate's default.
    fn crate_with_real_dep(yaml: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        std::fs::write(dir.path().join("ply.yaml"), yaml).unwrap();
        let ply_attrs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("ply-attrs");
        std::fs::write(
            dir.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [dependencies]\nply-attrs = {{ path = {:?} }}\n",
                ply_attrs.display().to_string()
            ),
        )
        .unwrap();
        dir
    }

    /// The plain default-deny case: `demo` really depends on `ply_attrs`
    /// (a real `cargo metadata` edge), both are declared components, and
    /// no `->` edge says `demo` may depend on `attrs` -- `A0401`, naming
    /// both crates and both components, and saying plainly that no
    /// declared edge permits it.
    #[test]
    fn an_undeclared_cross_component_crate_dependency_is_a0401_and_fails_the_run() {
        let dir = crate_with_real_dep(
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n  attrs:\n    anchor: ply_attrs\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "A0401")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert_eq!(d.severity, "error");
        assert!(d.title.contains("ply_attrs"), "{}", d.title);
        assert!(d.title.contains("demo"), "{}", d.title);
        assert!(d.title.contains("`attrs`"), "{}", d.title);
        assert!(
            d.title.contains("no `->` edge in this document says"),
            "{}",
            d.title
        );
        assert_eq!(report.exit_code(), 1);
    }

    /// A declared `->` edge permits exactly this real dependency -- zero
    /// diagnostics, and the coverage report says the crate tier actually
    /// ran and found nothing wrong (not that it was skipped).
    #[test]
    fn a_declared_edge_permits_a_real_cross_component_dependency_and_the_run_is_clean() {
        let dir = crate_with_real_dep(
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n  attrs:\n    anchor: ply_attrs\nedges:\n  - \"demo -> attrs\"\n",
        );
        let report = check_crate(dir.path()).unwrap();
        assert!(
            report.envelope.diagnostics.is_empty(),
            "{:#?}",
            report.envelope.diagnostics
        );
        assert_eq!(report.exit_code(), 0);
        let cov = report.envelope.coverage.as_ref().unwrap();
        let checked = cov
            .checked
            .iter()
            .find(|t| t.tier == "architecture")
            .unwrap_or_else(|| panic!("{:#?}", cov.checked));
        assert!(
            checked
                .detail
                .contains("1 real crate dependency crosses between"),
            "{}",
            checked.detail
        );
        assert!(checked.detail.contains("1 permitted"), "{}", checked.detail);
    }

    /// A `deny:` rule is checked against the real graph independent of
    /// whether an edge permits the dependency -- an explicit ban fires even
    /// though the edge above would otherwise keep this clean, which is
    /// exactly what makes `A0405` a different fact from `A0401`.
    #[test]
    fn a_deny_rule_violated_by_a_real_dependency_is_a0405_even_with_a_permitting_edge() {
        let dir = crate_with_real_dep(
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n  attrs:\n    anchor: ply_attrs\n\
             edges:\n  - \"demo -> attrs\"\ndeny:\n  - \"demo -> attrs\"\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "A0405")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert_eq!(d.severity, "error");
        assert!(
            !report
                .envelope
                .diagnostics
                .iter()
                .any(|d| d.code == "A0401"),
            "the edge permits it, so A0401 must not also fire: {:#?}",
            report.envelope.diagnostics
        );
        assert_eq!(report.exit_code(), 1);
    }

    /// §5.3: a component depending on its own nested descendant's crate is
    /// never a violation, even with no edge declared at all -- containment
    /// implies permission.
    #[test]
    fn a_component_depending_on_its_own_nested_crate_is_not_a_violation() {
        let dir = crate_with_real_dep(
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    components:\n      attrs:\n        anchor: ply_attrs\n",
        );
        let report = check_crate(dir.path()).unwrap();
        assert!(
            report.envelope.diagnostics.is_empty(),
            "containment permits this with no edge declared: {:#?}",
            report.envelope.diagnostics
        );
        assert_eq!(report.exit_code(), 0);
    }

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

    /// Defect 2 (2026-08-30, "a documented way of writing contracts is
    /// accepted, then silently ignored"): `check` is the command people run
    /// first, and until now it gave no hint at all that a `requires:`/
    /// `ensures:` written directly in ply.yaml is not what `verify` actually
    /// checks -- only `#[ply::requires]`/`#[ply::ensures]` attributes on the
    /// function itself reach it. The one sentence a user needs must appear
    /// right here, on the anchors line, since that is what already reports
    /// "N of N fn claims point at a function Ply can find" for this exact
    /// claim.
    ///
    /// `seven` has `checks: [fuzz(64)]` of its own -- it is asking to be
    /// checked, unlike a boundary-only fn (2026-08-31 review, "`check` tells
    /// users of the boundary-contract feature to destroy it", see
    /// `a_boundary_only_yaml_contract_is_never_told_to_move_onto_itself`
    /// below), so this is the "move it onto an attribute" case, worded so it
    /// never claims `verify` does not read the contract at all -- it does,
    /// just not for this fn's own checks.
    #[test]
    fn a_fn_claim_with_a_yaml_only_contract_names_that_it_is_not_read() {
        let dir = crate_with(
            "pub fn seven() -> u32 { 7 }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      seven:\n        \
             checks: [fuzz(64)]\n        requires: [\"true\"]\n        ensures: [\"|result| \
             *result == 7\"]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let cov = report.envelope.coverage.as_ref().unwrap();
        assert_eq!(
            cov.checked[1].detail,
            "1 of 1 fn claims in this crate point at a function Ply can find. 1 of them also \
             writes a `requires:`/`ensures:` contract directly in ply.yaml. That contract is \
             used both ways: `verify` checks `seven` against it, alongside any \
             `#[ply::requires]`/`#[ply::ensures]` attribute written on `seven` itself, and a \
             caller of `seven` may assume it at a boundary."
        );
    }

    /// Defect 2's other half (2026-08-31 review): a fn that declares a
    /// ply.yaml contract and *no* `checks:` of its own is §5.5's
    /// boundary-contract declaration working as intended -- legacy code
    /// nobody wants to edit, given a contract from outside so a caller can
    /// assume it. Telling its author to "move the contract onto it as an
    /// attribute" is advice to delete that feature, aimed at a fn that
    /// never asked to be checked at all.
    #[test]
    fn a_boundary_only_yaml_contract_is_never_told_to_move_onto_itself() {
        let dir = crate_with(
            "pub fn legacy_rate(x: u32) -> u32 { x }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      legacy_rate:\n        \
             ensures: [\"|result| *result <= 1000\"]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let cov = report.envelope.coverage.as_ref().unwrap();
        assert_eq!(
            cov.checked[1].detail,
            "1 of 1 fn claims in this crate point at a function Ply can find. 1 of them, \
             `legacy_rate`, declares a `requires:`/`ensures:` contract in ply.yaml but asks for \
             no checks of its own -- that is deliberate: it lets any function calling \
             `legacy_rate` assume the promise is true, without Ply ever verifying that \
             `legacy_rate` itself keeps it. This is the normal way to describe legacy code you \
             do not want to edit; any caller's result will say it rests on an unchecked promise."
        );
    }

    /// Two smaller wording repairs, 2026-08-31 review: with more than one fn
    /// in a bucket, the sentence used to fall back to the singular
    /// "the function" for both cases, unnamed and grammatically wrong for a
    /// count greater than one. It must instead say "those functions" (or,
    /// for the checked case, "them"/"they").
    #[test]
    fn more_than_one_yaml_contract_fn_says_those_functions_not_the_function() {
        let dir = crate_with(
            "pub fn a() -> u32 { 1 }\npub fn b() -> u32 { 2 }\npub fn c() -> u32 { 3 }\npub fn \
             d() -> u32 { 4 }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      a:\n        \
             checks: [test]\n        ensures: [\"|result| *result >= 0\"]\n      b:\n        \
             checks: [test]\n        ensures: [\"|result| *result >= 0\"]\n      c:\n        \
             ensures: [\"|result| *result >= 0\"]\n      d:\n        ensures: [\"|result| \
             *result >= 0\"]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let cov = report.envelope.coverage.as_ref().unwrap();
        assert_eq!(
            cov.checked[1].detail,
            "4 of 4 fn claims in this crate point at a function Ply can find. 2 of them also write \
             a `requires:`/`ensures:` contract directly in ply.yaml. Those contracts are used both \
             ways: `verify` checks each function against its own, alongside any \
             `#[ply::requires]`/`#[ply::ensures]` attribute written on it, and callers may assume \
             them at a boundary. 2 of them declare a `requires:`/`ensures:` contract in ply.yaml \
             but ask for no checks of their own -- that is deliberate: it lets any function calling \
             them assume the promise is true, without Ply ever verifying that they keep it. This is \
             the normal way to describe legacy code you do not want to edit; any caller's result \
             will say it rests on these unchecked promises."
        );
    }

    /// §5.2's own MUST: "a renamed function must break CI, not silently
    /// orphan its claims" — and §5.2 asks the diagnostic to suggest the
    /// nearest name, which is what makes the break actionable rather than
    /// merely loud.
    /// The same suggestion, for a claim written under a module anchor.
    /// Anchor-relative claim keys (2026-09-04) silently broke this: the
    /// typo is matched as the user wrote it (`dottted`) against names held
    /// as crate-root paths (`schema::dotted`), which are never within edit
    /// distance of each other, so the suggestion went quiet exactly where
    /// this project had just moved all of its own claims. Found by making
    /// the typo on purpose.
    #[test]
    fn a_typo_under_a_module_anchor_still_names_the_nearest_name() {
        let dir = crate_with(
            "pub mod schema { pub fn dotted(p: &str) -> String { p.to_string() } }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    components:\n      schema:\n        anchor: demo::schema\n        fns:\n          dottted:\n            checks: [fuzz(8)]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "E0301")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert!(
            d.title.contains("The closest name Ply can see is `dotted`"),
            "the suggestion has to be the name as it would be written under \
             this anchor, ready to paste over the typo:\n{}",
            d.title
        );
    }

    /// The other way a module anchor goes wrong: the name is spelled
    /// right but the claim sits under the wrong box. Edit distance cannot
    /// see this -- `visual::examples_prose` is five characters from
    /// `visual::svg::examples_prose`, so the suggestion stayed silent and
    /// the reader was told the function does not exist when it plainly
    /// does. Found 2026-09-04 while claiming it.
    #[test]
    fn a_claim_under_the_wrong_module_is_told_where_the_function_actually_is() {
        let dir = crate_with(
            "pub mod visual { pub mod svg { pub fn examples_prose(n: usize) -> String { n.to_string() } } }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    components:\n      visual:\n        anchor: demo::visual\n        fns:\n          examples_prose:\n            checks: [fuzz(8)]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "E0301")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert!(
            d.title.contains("There is a function of that name at `svg::examples_prose`"),
            "the reader has to be told where the function actually is, relative to \
             the anchor they wrote -- and that nothing was renamed:\n{}",
            d.title
        );
        assert!(
            !d.title.contains("was renamed"),
            "nothing was renamed here; saying so sends the reader looking for a \
             change nobody made:\n{}",
            d.title
        );
    }

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

    /// A claim on a function inside a module resolves. Until 2026-08-25 it
    /// did not: this test asserted the opposite, and the sentence it
    /// asserted ("exists in this crate, but not where Ply can verify it
    /// from") described a limit that made per-function promises unusable
    /// for real legacy code, which lives in modules and files rather than
    /// at the top of `src/lib.rs`.
    #[test]
    fn a_claim_on_a_function_inside_a_module_resolves() {
        let dir = crate_with(
            "pub mod util { pub fn helper(x: u32) -> u32 { x } }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      util::helper:\n        checks: [bounded(2)]\n",
        );
        let report = check_crate(dir.path()).unwrap();
        assert!(
            report.envelope.diagnostics.is_empty(),
            "{:#?}",
            report.envelope.diagnostics
        );
        assert_eq!(report.exit_code(), 0);
    }

    /// The case that genuinely stays closed, and it gets its own sentence:
    /// Ply generates its harness at the crate root, so a private item
    /// inside a module is a name that harness cannot write. Saying "could
    /// not find" would be false and would send the user hunting for a typo
    /// that is not there.
    #[test]
    fn a_private_function_inside_a_module_says_which_of_the_two_it_is() {
        let dir = crate_with(
            "pub mod util { fn helper(x: u32) -> u32 { x } }\n",
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
                .contains("Ply found `util::helper` but cannot verify from it"),
            "{}",
            d.title
        );
        assert!(d.title.contains("private"), "{}", d.title);
    }

    /// The disagreement this fixes (adversarial review, 2026-08-27): a
    /// `Type::method` claim naming a method Ply finds and refuses for a
    /// stated reason (here, a receiver it cannot build) is not "a claim
    /// that describes nothing" -- `verify` already said so via its own
    /// pre-check. Before this test could pass, `check` had no such
    /// pre-check at all, so this exact claim counted as `unresolved` under
    /// `E0301` -- the two commands disagreeing about the same fact for the
    /// same document.
    #[test]
    fn check_does_not_deny_a_receiver_verify_can_actually_build() {
        // `check` is the command people run first. It used to report the
        // resolver's blanket "constructing a receiver is not supported yet"
        // for every method taking `&self`, while `verify` built one from the
        // type's own constructor and found real violations in the same
        // function. A user who ran `check` would conclude the feature did
        // not exist and never reach `verify`. Reproduced by hand before the
        // fix (adversarial review, 2026-08-27).
        let dir = crate_with(
            "pub struct Calc { pub v: u32 }\nimpl Calc { pub fn new(v: u32) -> Self { Self { v:              v.min(10) } }\n  pub fn value(&self) -> u32 { self.v } }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      Calc::value: {}\n",
        );
        let report = check_crate(dir.path()).unwrap();
        assert!(
            !report
                .envelope
                .diagnostics
                .iter()
                .any(|d| d.title.contains("receiver")),
            "a receiver `verify` can build must not be reported as unsupported by `check`: {:#?}",
            report.envelope.diagnostics
        );
    }

    #[test]
    fn check_still_refuses_a_receiver_that_genuinely_cannot_be_built() {
        // The other direction, so the fix above cannot become "never refuse".
        let dir = crate_with(
            "pub struct Calc { pub v: u32 }\nimpl Calc { pub fn value(&self) -> u32 { self.v } }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      Calc::value: {}\n",
        );
        let report = check_crate(dir.path()).unwrap();
        assert!(
            report
                .envelope
                .diagnostics
                .iter()
                .any(|d| d.title.contains("receiver")),
            "a type with no constructor has no receiver to build, and `check` must say so: {:#?}",
            report.envelope.diagnostics
        );
    }

    #[test]
    fn a_method_refused_for_its_receiver_counts_as_resolved_not_unresolved() {
        let dir = crate_with(
            "pub struct Bucket { n: u32 }\nimpl Bucket { pub fn n(&self) -> u32 { self.n } }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      Bucket::n: {}\n",
        );
        let report = check_crate(dir.path()).unwrap();
        assert!(
            !report
                .envelope
                .diagnostics
                .iter()
                .any(|d| d.code == "E0301"),
            "a method Ply found and refuses for a stated reason must never carry the code that \
             means \"this claim describes nothing\": {:#?}",
            report.envelope.diagnostics
        );
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.node_id == "demo::Bucket::n")
            .unwrap_or_else(|| {
                panic!(
                    "no diagnostic for demo::Bucket::n: {:#?}",
                    report.envelope.diagnostics
                )
            });
        assert_eq!(d.code, "V0507");
        assert_eq!(d.severity, "warning");
        assert!(
            d.title.contains("Bucket::n") && d.title.contains("receiver"),
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

    /// The tier §6 promises that this command does not yet fully deliver.
    /// Exact strings: a clean run's honesty is entirely carried by this
    /// sentence, so it is reviewed like the diagnostics are. The crate half
    /// of architecture (this test's fixture has no cross-component
    /// dependency at all) now runs and moves into `checked` --
    /// [`the_crate_tier_reports_it_ran_with_nothing_to_check`] covers that
    /// half; this test is only about what still doesn't run.
    #[test]
    fn the_report_names_the_tier_it_does_not_cover() {
        let dir = crate_with("pub fn clamp(x: u32) -> u32 { x }\n", CLEAN_YAML);
        let report = check_crate(dir.path()).unwrap();
        let cov = report.envelope.coverage.as_ref().unwrap();
        let names: Vec<&str> = cov.not_checked.iter().map(|t| t.tier.as_str()).collect();
        assert_eq!(names, ["item-level"]);
        assert_eq!(cov.not_checked[0].detail, ITEM_TIER_GAP);
    }

    /// The crate tier's own honest-nothing-to-check-here sentence: a
    /// single-crate fixture with no cross-component dependency at all still
    /// reports the tier *ran*, not that it was skipped.
    #[test]
    fn the_crate_tier_reports_it_ran_with_nothing_to_check() {
        let dir = crate_with("pub fn clamp(x: u32) -> u32 { x }\n", CLEAN_YAML);
        let report = check_crate(dir.path()).unwrap();
        let cov = report.envelope.coverage.as_ref().unwrap();
        let checked = cov
            .checked
            .iter()
            .find(|t| t.tier == "architecture")
            .unwrap_or_else(|| panic!("{:#?}", cov.checked));
        assert!(
            checked.detail.contains("nothing to check"),
            "{}",
            checked.detail
        );
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

    // ---- Blocker 1 (docs/review-architecture-tier.md, finding 1) ----
    //
    // "No problems found in the document" printing, exit 0, while the
    // architecture tier silently never ran at all -- reproduced here two of
    // the review's three ways (a broken manifest; a real package
    // dependency cycle). The third (`cargo` missing from `PATH`) needs a
    // real subprocess environment to mutate safely and lives in
    // `tests/e2e/tests/arch_unavailable.rs` instead.

    /// (a) A bad version requirement anywhere in the workspace's manifests
    /// makes `cargo metadata` fail outright. Before the fix this printed
    /// "No problems found in the document." and exited 0, with the failure
    /// buried as a sentence in the coverage report; now it is `A0409`, an
    /// error-severity diagnostic, so the run fails and the diagnostic is
    /// what a reader sees first.
    #[test]
    fn a_broken_manifest_makes_the_architecture_tier_a0409_not_a_clean_run() {
        let dir = crate_with("pub fn clamp(x: u32) -> u32 { x }\n", CLEAN_YAML);
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [dependencies]\nbogus = \"!!!\"\n",
        )
        .unwrap();
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "A0409")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert_eq!(d.severity, "error");
        assert!(
            d.title.contains("could not get the real dependency graph"),
            "{}",
            d.title
        );
        assert_eq!(report.exit_code(), 1, "an unchecked tier must not exit 0");
    }

    /// A tiny two-crate workspace where each crate depends normally on the
    /// other -- a real package dependency cycle, which `cargo metadata`
    /// refuses to produce a graph for at all. This is the review's own
    /// headline reproduction: the exact shape a `deny:` rule is usually
    /// written to catch is the one case that made the check stop running
    /// entirely.
    fn cycle_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crate_x\", \"crate_y\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        for (name, dep) in [("crate_x", "crate_y"), ("crate_y", "crate_x")] {
            let sub = dir.path().join(name);
            std::fs::create_dir_all(sub.join("src")).unwrap();
            std::fs::write(sub.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
            std::fs::write(
                sub.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
                     [dependencies]\n{dep} = {{ path = \"../{dep}\" }}\n"
                ),
            )
            .unwrap();
        }
        std::fs::write(
            dir.path().join("ply.yaml"),
            "ply: 1\ncomponents:\n  x:\n    anchor: crate_x\n  y:\n    anchor: crate_y\n",
        )
        .unwrap();
        dir
    }

    /// (b, the review's headline case) A real package dependency cycle:
    /// `cargo metadata` fails outright, so the crate tier cannot run --
    /// `A0409`, not a clean exit 0.
    #[test]
    fn a_package_dependency_cycle_makes_the_architecture_tier_a0409() {
        let dir = cycle_workspace();
        let report = check_crate(dir.path()).unwrap();
        let d = report
            .envelope
            .diagnostics
            .iter()
            .find(|d| d.code == "A0409")
            .unwrap_or_else(|| panic!("{:#?}", report.envelope.diagnostics));
        assert_eq!(d.severity, "error");
        assert!(
            d.title.contains("cargo metadata") || d.title.contains("cyclic"),
            "{}",
            d.title
        );
        assert_eq!(report.exit_code(), 1);
    }
}
