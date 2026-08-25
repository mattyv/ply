//! `cargo ply worklist` (§6): "unresolved markers + weak specs (W0502) +
//! stale claims (W0302)".
//!
//! **What is owed, and expected to close.** That is the whole difference
//! from `cargo ply audit`, and it is a line worth keeping sharp: `audit`
//! lists permanent trust surface — an escape, an attestation, an assumption
//! about the world outside — while everything here is work somebody
//! recorded and means to finish. An environmental assumption (§5.1's
//! `entry:`) can never be discharged by anyone, so counting it as owed
//! would pressure a user into deleting an honest declaration; it appears on
//! `audit` and nowhere here.
//!
//! Two of §6's three tiers do not exist yet, and this command says so
//! rather than letting a short list read as a short backlog: a weak spec is
//! a finding from a `mutate` run, and a stale claim needs the fingerprint
//! in `ply.lock` (Phase 1c). What it does list is:
//!
//! - **unresolved markers** (§5.6) — `ply::unresolved!` in the code and the
//!   `ply.yaml` registry, merged by id, each with its span, its enclosing
//!   function and what it blocks.
//! - **owed evidence** (§5.5) — an assumed boundary contract nothing has
//!   yet run the real callee against. `audit` lists the assumption as trust
//!   surface; this lists the evidence owed on it, because unlike the rest
//!   of that surface it closes, and the thing that closes it is one line of
//!   `ply.yaml`.
//!
//! No engines start, so this command produces no verdicts, and it never
//! fails a run for having items in it.

use std::path::Path;

use anyhow::Result;
use ply_core::diag::{Coverage, Envelope, OpenItem, Tier};
use ply_core::surface;

use crate::shared::{
    self, Loaded, empty_workspace, load_document, local_anchor_names, walk_fn_claims,
    workspace_node, wrap,
};

const PLY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The tiers §6 promises and this build cannot deliver, in the words a user
/// needs to know what a short list is not telling them. Exact strings.
const WEAK_SPEC_GAP: &str = "NOT CHECKED. `W0502` is a finding from a `mutate` run: \
     cargo-mutants changes the code, the checks run again, and a mutant that survives means the \
     spec was too weak to notice. That is engine work, and this command starts none. Ply keeps no \
     record of previous runs either, until `ply.lock` exists (Phase 1c) — so a weak spec found \
     this morning is not on this list. `cargo ply verify` reports it in the run that finds it.";
const STALE_CLAIM_GAP: &str = "NOT CHECKED. A claim is stale when the function it describes has \
     changed since its evidence was recorded, and that comparison needs the fingerprint in \
     `ply.lock` — a file this version of Ply does not write yet (Phase 1c). No `W0302` can be \
     reported here, so nothing on this list means your evidence is current.";
const CHECK_CAP_GAP: &str = "NOT ENFORCED. §5.6 caps a function containing an unresolved marker \
     at check `test`, with `W0521`. Ply does not apply that cap yet: `cargo ply verify` still \
     runs whatever the claim asks for, against a body that panics when it reaches the marker. The \
     blocking line on each marker above says what §5.6 intends, not what this build stops you \
     doing.";

/// What an empty worklist means, so nobody reads it as "nothing left to do".
const EMPTY_WORKLIST: &str = "Nothing is owed that Ply can see: no unresolved markers, in the \
     code or in `ply.yaml`, and no assumed contract waiting on evidence. That is a fact about \
     what is recorded, not a verdict about the code — see what this command could not look at, \
     below.";

/// Why a full worklist is still a green run.
const NOT_A_FAILURE: &str = "`worklist` exits 0 whether or not it has items to show: an open item \
     is work somebody recorded, not a failure, and a command that failed a build for having a \
     `TODO` in it would make deleting the `TODO` the cheapest fix. `cargo ply verify` is what \
     fails a run, and `cargo ply audit` is what lists the trust this codebase rests on \
     permanently.";

#[derive(Debug)]
pub struct WorklistReport {
    pub envelope: Envelope,
    pub document: String,
}

impl WorklistReport {
    /// The tiers, in the order they are reported.
    pub const TIERS: [&'static str; 2] = ["unresolved_marker", "owed_evidence"];

    /// §6's exit codes: 0 clean *and* 0 with a full list (see
    /// [`NOT_A_FAILURE`]), 1 a document that will not load, 2 tool error.
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

fn tier_heading(kind: &str) -> String {
    match kind {
        "unresolved_marker" => "unresolved markers",
        "owed_evidence" => "owed evidence",
        other => other,
    }
    .to_string()
}

pub fn worklist_crate(crate_dir: &Path) -> Result<WorklistReport> {
    let yaml_path = crate_dir.join("ply.yaml");
    let document = yaml_path.display().to_string();

    let doc = match load_document(&yaml_path, "worklist")? {
        Loaded::Document(doc) => *doc,
        Loaded::SchemaViolations(violations) => {
            return Ok(WorklistReport {
                envelope: Envelope {
                    command: "worklist".into(),
                    ply_version: PLY_VERSION.into(),
                    root: empty_workspace(),
                    diagnostics: violations,
                    coverage: Some(unread_coverage()),
                    trust_surface: None,
                    // Absent, not empty: this run never got to look.
                    open_items: None,
                },
                document,
            });
        }
    };

    let local_anchors = local_anchor_names(crate_dir);
    let scanned = surface::scan_crate(crate_dir);

    // Which fn claims exist, so a marker can say what it caps -- and which
    // registry entries exist, so a marker written in both places is one
    // item rather than two.
    let mut claimed: Vec<ClaimedFn> = Vec::new();
    let mut registry: Vec<(u64, String, String)> = Vec::new(); // (id, note, node id)
    walk_fn_claims(&doc, |c| {
        let governing = c.governing_checks();
        claimed.push(ClaimedFn {
            name: c.fn_name.clone(),
            node_id: c.node_id(),
            governing: governing.map(|g| (g.checks.to_vec(), g.from_component.map(str::to_string))),
        });
        for entry in &c.claim.unresolved {
            registry.push((entry.id, entry.note.clone(), c.node_id()));
        }
    });
    for entry in &doc.unresolved {
        registry.push((entry.id, entry.note.clone(), "ply.yaml".to_string()));
    }

    let mut items: Vec<OpenItem> = Vec::new();
    let mut merged: Vec<u64> = Vec::new();

    for m in &scanned.markers {
        let claim = m
            .enclosing_fn
            .as_ref()
            .and_then(|f| claimed.iter().find(|c| &c.name == f));
        // The registry's note is the fuller of the two by design (§5.1
        // calls a fn's `unresolved:` "registry links for markers in this
        // fn"), so it wins the merge, and the marker keeps the span.
        let registered =
            m.id.and_then(|id| registry.iter().find(|(rid, _, _)| *rid == id));
        if let Some((id, _, _)) = registered {
            merged.push(*id);
        }
        let note = registered
            .map(|(_, note, _)| note.clone())
            .or_else(|| m.note.clone());
        items.push(marker_item(m, claim, note.as_deref()));
    }

    for (id, note, node_id) in &registry {
        if merged.contains(id) {
            continue;
        }
        items.push(registry_item(*id, note, node_id));
    }

    for a in shared::assumed_contracts(crate_dir, &doc, &local_anchors) {
        items.push(owed_evidence_item(&a));
    }

    items.sort_by_key(|i| {
        (
            WorklistReport::TIERS
                .iter()
                .position(|t| *t == i.kind)
                .unwrap_or(usize::MAX),
            i.id.unwrap_or(u64::MAX),
            i.node_id.clone(),
        )
    });

    let markers = items
        .iter()
        .filter(|i| i.kind == "unresolved_marker")
        .count();
    let owed = items.iter().filter(|i| i.kind == "owed_evidence").count();

    Ok(WorklistReport {
        envelope: Envelope {
            command: "worklist".into(),
            ply_version: PLY_VERSION.into(),
            root: workspace_node(&doc),
            diagnostics: vec![],
            coverage: Some(read_coverage(&document, markers, owed)),
            trust_surface: None,
            open_items: Some(items),
        },
        document,
    })
}

/// One fn claim, as the line under a marker needs it: what it is called,
/// what it is called in a §7 tree, and the checks that govern it —
/// resolved through the one shared §5.1 resolution, so a fn that takes its
/// checks from its component is not reported as declaring none.
struct ClaimedFn {
    name: String,
    node_id: String,
    /// The governing list and the component it was written on when the fn
    /// wrote none of its own; `None` when no list is written anywhere.
    governing: Option<(Vec<String>, Option<String>)>,
}

fn marker_item(
    m: &ply_core::surface::Marker,
    claim: Option<&ClaimedFn>,
    note: Option<&str>,
) -> OpenItem {
    let id_text = match m.id {
        Some(id) => format!("#{id}"),
        None => "with no id Ply could read".to_string(),
    };
    let note_text = match note {
        Some(n) => format!("“{n}”"),
        None => "no note was written with it".to_string(),
    };
    let (node_id, blocking) = match claim {
        Some(c) => {
            let node_id = &c.node_id;
            // §5.1: a fn that writes no `checks:` of its own runs whatever
            // its component declares for everything inside it. Saying "no
            // checks of its own" about such a fn is true of the line and
            // false about the run.
            let asks = match &c.governing {
                None => "it declares no checks of its own".to_string(),
                Some((checks, None)) if checks.is_empty() => {
                    "its `checks:` list is empty, so nothing runs against it".to_string()
                }
                Some((checks, Some(from))) if checks.is_empty() => format!(
                    "the component `{from}` declares an empty list as the default for everything \
                     inside it, so nothing runs against it"
                ),
                Some((checks, None)) => format!("it claims `{}`", checks.join(", ")),
                Some((checks, Some(from))) => format!(
                    "it claims `{}`, the default the component `{from}` sets for everything \
                     inside it",
                    checks.join(", ")
                ),
            };
            (
                node_id.clone(),
                format!("§5.6 caps `{node_id}` at check `test` while this stands; {asks}."),
            )
        }
        None => (
            m.enclosing_fn.clone().unwrap_or_else(|| m.file.clone()),
            match &m.enclosing_fn {
                Some(f) => format!(
                    "Nothing: no claim in `ply.yaml` names `{f}`, so no check is capped by it."
                ),
                None => "Nothing: this marker is not inside a function.".to_string(),
            },
        ),
    };
    OpenItem {
        kind: "unresolved_marker".into(),
        id: m.id,
        node_id,
        where_: Some(format!("{}:{}:{}", m.file, m.line, m.col)),
        blocking,
        detail: format!(
            "An unresolved marker {id_text} stands where the code for a decision nobody has made \
             yet would go: {note_text}. Reaching that line panics: the macro expands to \
             `unimplemented!` in every build, dev and prod alike, which is what keeps the gap from \
             shipping quietly. §5.6 caps a function that contains one at check `test`, because a \
             body that panics where the decision is missing cannot support anything stronger — \
             Ply does not enforce that cap yet (see below). This closes when somebody makes the \
             decision and writes the code. (§5.6)"
        ),
    }
}

fn registry_item(id: u64, note: &str, node_id: &str) -> OpenItem {
    OpenItem {
        kind: "unresolved_marker".into(),
        id: Some(id),
        node_id: node_id.into(),
        where_: None,
        blocking: "Nothing: there is no code behind this entry to cap.".into(),
        detail: format!(
            "Unresolved #{id} — “{note}” — is recorded in `ply.yaml` with no marker in the code, \
             so nothing panics and no check is capped. It is a decision somebody wrote down so it \
             would not be forgotten, and it closes when the decision is made. If the code for it \
             does exist somewhere unfinished, a `ply::unresolved!({id}, …)` at that line is what \
             connects the two. (§5.6)"
        ),
    }
}

fn owed_evidence_item(a: &shared::AssumedContract) -> OpenItem {
    let (caller, callee, contract) = (&a.caller_fn, &a.callee, &a.contract);
    let discharge = match (a.callee_checks.first(), a.callee_anchor.as_deref()) {
        (Some(check), None) => format!(
            "Its `ply.yaml` entry already asks for `{check}`: run `cargo ply verify` and the \
             promise is measured against the real body."
        ),
        // `cargo ply verify` checks one crate at a time, so a check
        // declared for a function in another package is read for its
        // promise and declined for its checks (`W0303`). Advice that does
        // not say where to run it is advice this tool will refuse.
        (Some(check), Some(anchor)) => format!(
            "Its `ply.yaml` entry already asks for `{check}` — run `cargo ply verify` inside the \
             `{anchor}` crate, where that function lives, and the promise is measured against the \
             real body. This run checks one crate at a time, so from here that entry's promise is \
             read and its checks are skipped."
        ),
        (None, None) => format!(
            "To close it, add `checks: [fuzz(256)]` to its `ply.yaml` entry — fuzzing crosses a \
             legacy boundary by simply calling the code, so it tests the promise against the real \
             `{callee}`."
        ),
        (None, Some(anchor)) => format!(
            "To close it, add `checks: [fuzz(256)]` to its `ply.yaml` entry and run `cargo ply \
             verify` inside the `{anchor}` crate, which is where that function lives — fuzzing \
             crosses a legacy boundary by simply calling the code, so it tests the promise \
             against the real `{callee}`. Adding the check changes nothing in this crate: \
             `cargo ply verify` checks one crate at a time and will decline to run it from here. \
             If you would rather not leave this crate, pass what `{callee}` returns into \
             `{caller}` as a parameter instead: the value becomes the caller's own data and there \
             is no promise left to owe."
        ),
    };
    OpenItem {
        kind: "owed_evidence".into(),
        id: None,
        node_id: a.caller_node_id.clone(),
        where_: Some(a.where_text.clone()),
        blocking: format!(
            "`{}` keeps a `conditional` verdict until the promise made for `{callee}` is checked \
             against the real body.",
            a.caller_node_id
        ),
        detail: format!(
            "`{caller}`'s proof stands on a promise `ply.yaml` makes for `{callee}` — {contract} — \
             and nothing has run the real `{callee}` against it. That is what `owed-evidence` \
             means: trust that is never checked is green paint. Unlike the rest of the trust \
             surface this one closes, and cheaply. {discharge} (§5.5)"
        ),
    }
}

fn unread_coverage() -> Coverage {
    Coverage {
        checked: vec![
            Tier {
                tier: "markers".into(),
                detail: "NOT REACHED. The document did not pass the schema, so there was nothing \
                         well-formed to read a worklist out of."
                    .into(),
            },
            Tier {
                tier: "owed evidence".into(),
                detail: "NOT REACHED. Same reason.".into(),
            },
        ],
        not_checked: gaps(),
    }
}

fn read_coverage(document: &str, markers: usize, owed: usize) -> Coverage {
    Coverage {
        checked: vec![
            Tier {
                tier: "markers".into(),
                detail: format!(
                    "`ply::unresolved!` in every `.rs` file under `src/`, and the registry in \
                     {document}, merged by id: {markers} in total."
                ),
            },
            Tier {
                tier: "owed evidence".into(),
                detail: format!(
                    "Every assumed boundary contract, read from the call graph the same way \
                     `cargo ply audit` reads it: {owed} waiting on evidence."
                ),
            },
        ],
        not_checked: gaps(),
    }
}

fn gaps() -> Vec<Tier> {
    vec![
        Tier {
            tier: "weak specs (W0502)".into(),
            detail: WEAK_SPEC_GAP.into(),
        },
        Tier {
            tier: "stale claims (W0302)".into(),
            detail: STALE_CLAIM_GAP.into(),
        },
        Tier {
            tier: "check cap (W0521)".into(),
            detail: CHECK_CAP_GAP.into(),
        },
    ]
}

/// The one line a reader scans before deciding whether to read the
/// paragraph under it.
fn item_line(item: &OpenItem) -> String {
    // An item with an id leads with it -- that is how people refer to
    // these ("what happened to 147?") -- and one without leads with the
    // node, never with a stray dash where the id would have been.
    let id = match item.id {
        Some(id) => format!("#{id} — "),
        None => String::new(),
    };
    let at = match &item.where_ {
        Some(w) => format!(" (at {w})"),
        None => String::new(),
    };
    format!("{id}`{}`{at}", item.node_id)
}

pub fn print_human(report: &WorklistReport) {
    println!("cargo ply worklist — {}", report.document);
    println!();
    let cov = report
        .envelope
        .coverage
        .as_ref()
        .expect("worklist sets coverage");
    for tier in &cov.checked {
        println!("  {:<15}{}", tier.tier, wrap(&tier.detail, 17));
    }
    println!();

    match report.envelope.open_items.as_ref() {
        None => println!("  The worklist was not read (see above)."),
        Some(items) if items.is_empty() => println!("{}", wrap(EMPTY_WORKLIST, 0)),
        Some(items) => {
            println!("What is owed — recorded by somebody, and expected to close:");
            for kind in WorklistReport::TIERS {
                let tier: Vec<&OpenItem> = items.iter().filter(|i| i.kind == kind).collect();
                if tier.is_empty() {
                    continue;
                }
                println!();
                println!("  {} ({})", tier_heading(kind), tier.len());
                for item in tier {
                    println!("    {}", item_line(item));
                    println!("      {}", wrap(&item.detail, 6));
                    println!("      blocks: {}", wrap(&item.blocking, 14));
                }
            }
        }
    }
    println!();
    println!("What this command did NOT look at:");
    for tier in &cov.not_checked {
        println!("  {:<21}{}", tier.tier, wrap(&tier.detail, 23));
    }
    println!();
    println!("{}", wrap(NOT_A_FAILURE, 0));
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn items(report: &WorklistReport) -> Vec<ply_core::diag::OpenItem> {
        report.envelope.open_items.clone().unwrap()
    }

    /// §5.6: "`ply worklist` lists every marker (macro or `ply.yaml`
    /// registry) with its span, enclosing component, and blocking status."
    /// All three, in the words a reader needs — a marker with no span is a
    /// hunt, and one with no blocking status is a nag.
    #[test]
    fn a_marker_in_the_code_is_listed_with_its_span_its_fn_and_what_it_blocks() {
        let dir = crate_with(
            "pub fn discount(pct: u32) -> u32 {\n    \
             ply::unresolved!(147, \"employee discount undecided\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      discount:\n        checks: [bounded(2)]\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        let items = items(&report);
        assert_eq!(items.len(), 1, "{items:#?}");
        let m = &items[0];
        assert_eq!(m.kind, "unresolved_marker");
        assert_eq!(m.id, Some(147));
        assert_eq!(m.node_id, "demo::discount");
        assert_eq!(m.where_.as_deref(), Some("src/lib.rs:2:5"));
        assert_eq!(
            m.blocking,
            "§5.6 caps `demo::discount` at check `test` while this stands; it claims `bounded(2)`."
        );
        assert!(
            m.detail.contains("employee discount undecided"),
            "{}",
            m.detail
        );
        assert!(
            m.detail.contains(
                "Reaching that line panics: the macro expands to `unimplemented!` in every build, \
                 dev and prod alike"
            ),
            "{}",
            m.detail
        );
        assert!(
            m.detail.contains("Ply does not enforce that cap yet"),
            "a blocking status this build does not enforce must say so on the line that claims \
             it: {}",
            m.detail
        );
    }

    /// A marker in a function nobody claims is still an open decision. It
    /// blocks no claim, and saying that is what stops the list reading as
    /// a pile of equally urgent things.
    #[test]
    fn a_marker_in_an_unclaimed_fn_says_it_blocks_no_claim() {
        let dir = crate_with(
            "pub fn helper() -> u32 {\n    ply::unresolved!(9, \"tier table TBD\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        assert_eq!(
            items(&report)[0].blocking,
            "Nothing: no claim in `ply.yaml` names `helper`, so no check is capped by it."
        );
    }

    /// §5.6's other half: the registry. An entry with no marker in the code
    /// is a decision somebody wrote down so it would not be forgotten.
    #[test]
    fn a_registry_entry_with_no_marker_in_the_code_is_listed_as_such() {
        let dir = crate_with(
            "pub fn quote() -> u32 { 1 }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\nunresolved:\n  - { id: 151, note: \"settlement rounding rule TBD\" }\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        let items = items(&report);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, Some(151));
        assert_eq!(items[0].node_id, "ply.yaml");
        assert_eq!(items[0].where_, None);
        assert!(
            items[0].detail.contains(
                "recorded in `ply.yaml` with no marker in the code, so nothing panics and no \
                 check is capped"
            ),
            "{}",
            items[0].detail
        );
    }

    /// One decision, written in both places, is one item. Listing it twice
    /// would make a codebase that documents its markers look worse than one
    /// that does not.
    #[test]
    fn a_marker_and_its_registry_entry_are_one_item_not_two() {
        let dir = crate_with(
            "pub fn discount(pct: u32) -> u32 {\n    ply::unresolved!(147, \"in code\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      discount:\n        checks: [test]\n        unresolved:\n          - { id: 147, note: \"employee discount undecided\" }\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        let items = items(&report);
        assert_eq!(items.len(), 1, "{items:#?}");
        assert_eq!(items[0].where_.as_deref(), Some("src/lib.rs:2:5"));
        assert!(
            items[0].detail.contains("employee discount undecided"),
            "the registry's note is the fuller one, so it is the one that survives the merge: {}",
            items[0].detail
        );
    }

    /// §5.1: a component's `checks:` is the default for every fn inside
    /// it, nested components included. The line under a marker says what
    /// the marker holds up, so it has to name the check that would actually
    /// run -- reading only the fn's own (absent) list reported "no checks"
    /// for a fn the document does check.
    #[test]
    fn a_marker_in_a_fn_that_inherits_its_checks_names_the_check_that_would_run() {
        let dir = crate_with(
            "pub fn discount(pct: u32) -> u32 {\n    \
             ply::unresolved!(147, \"employee discount undecided\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    checks: [bounded(2)]\n    components:\n      pricing:\n        anchor: demo\n        fns:\n          discount: {}\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        assert_eq!(
            items(&report)[0].blocking,
            "§5.6 caps `demo.pricing::discount` at check `test` while this stands; it claims \
             `bounded(2)`, the default the component `demo` sets for everything inside it."
        );
    }

    /// The other half of the same misreading: a fn that inherits `fuzz(64)`
    /// never has its callee stubbed, because fuzzing crosses a legacy
    /// boundary by running the real code (§5.5). Nothing is assumed, so
    /// nothing is owed -- and an open item nobody owes is work invented by
    /// the tool.
    #[test]
    fn a_component_default_that_asks_for_fuzzing_owes_no_evidence() {
        let dir = crate_with(
            "pub fn legacy_rate(tier: u8) -> u32 { if tier == 0 { 150 } else { 90 } }\n\
             #[ply::requires(amount <= 100)]\n\
             #[ply::ensures(|result| *result <= amount)]\n\
             pub fn tiered_fee(amount: u32, tier: u8) -> u32 { legacy_rate(tier).min(amount) }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    checks: [fuzz(64)]\n    fns:\n      legacy_rate:\n        checks: []\n        ensures:\n          - \"|result| *result <= 10_000\"\n      tiered_fee: {}\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        assert!(
            items(&report).iter().all(|i| i.kind != "owed_evidence"),
            "{:#?}",
            items(&report)
        );
    }

    /// The two ways a document can say "check nothing" (§5.4c) read
    /// differently to somebody looking for the line that says it: one is on
    /// the fn, the other on the component above it.
    #[test]
    fn an_empty_checks_list_says_which_line_it_was_written_on() {
        let lib = "pub fn discount(pct: u32) -> u32 {\n    \
                   ply::unresolved!(147, \"employee discount undecided\");\n}\n";
        let own = crate_with(
            lib,
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      discount:\n        checks: []\n",
        );
        assert_eq!(
            items(&worklist_crate(own.path()).unwrap())[0].blocking,
            "§5.6 caps `demo::discount` at check `test` while this stands; its `checks:` list is \
             empty, so nothing runs against it."
        );

        let inherited = crate_with(
            lib,
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    checks: []\n    fns:\n      discount: {}\n",
        );
        assert_eq!(
            items(&worklist_crate(inherited.path()).unwrap())[0].blocking,
            "§5.6 caps `demo::discount` at check `test` while this stands; the component `demo` \
             declares an empty list as the default for everything inside it, so nothing runs \
             against it."
        );
    }

    /// §5.5's honesty condition 3: an assumed contract is *owed evidence*
    /// until something exercises it, and it closes when the cheap tier runs
    /// — which is exactly what an open item is. The assumption itself is
    /// permanent trust surface (`audit`); the evidence owed on it is work.
    #[test]
    fn an_assumed_contract_is_owed_evidence_and_says_what_would_close_it() {
        let dir = crate_with(
            "pub fn legacy_rate(tier: u8) -> u32 { if tier == 0 { 150 } else { 90 } }\n\
             #[ply::requires(amount <= 100)]\n\
             #[ply::ensures(|result| *result <= amount)]\n\
             pub fn tiered_fee(amount: u32, tier: u8) -> u32 { legacy_rate(tier).min(amount) }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      legacy_rate:\n        ensures:\n          - \"|result| *result <= 10_000\"\n      tiered_fee:\n        checks: [bounded(2)]\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        let owed: Vec<_> = items(&report)
            .into_iter()
            .filter(|i| i.kind == "owed_evidence")
            .collect();
        assert_eq!(owed.len(), 1, "{owed:#?}");
        assert_eq!(owed[0].node_id, "demo::tiered_fee");
        assert_eq!(
            owed[0].blocking,
            "`demo::tiered_fee` keeps a `conditional` verdict until the promise made for \
             `legacy_rate` is checked against the real body."
        );
        assert!(
            owed[0]
                .detail
                .contains("add `checks: [fuzz(256)]` to its `ply.yaml` entry"),
            "{}",
            owed[0].detail
        );
    }

    /// The same assumption across a crate boundary, which is the case
    /// §5.5's second branch exists for: the callee is old code in another
    /// package. Advice that stops at "add a check to its entry" is advice
    /// `cargo ply verify`, run from here, then declines (`W0303`) -- each
    /// command right on its own, the pair of them a circle. It has to say
    /// where that check can actually be run.
    #[test]
    fn advice_for_a_callee_in_another_package_says_where_that_advice_has_to_be_run() {
        let root = tempfile::tempdir().unwrap();
        let ledger = root.path().join("ledger");
        std::fs::create_dir_all(ledger.join("src")).unwrap();
        std::fs::write(
            ledger.join("src/lib.rs"),
            "pub mod fees {\n    pub fn bps_for_tier(tier: u8) -> u32 { if tier == 0 { 150 } else { 90 } }\n}\n",
        )
        .unwrap();
        std::fs::write(
            ledger.join("Cargo.toml"),
            "[package]\nname = \"ledger\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let demo = root.path().join("demo");
        std::fs::create_dir_all(demo.join("src")).unwrap();
        std::fs::write(
            demo.join("src/lib.rs"),
            "#[ply::requires(amount <= 100)]\n\
             #[ply::ensures(|result| *result <= amount)]\n\
             pub fn tiered_fee(amount: u32, tier: u8) -> u32 { ledger::fees::bps_for_tier(tier).min(amount) }\n",
        )
        .unwrap();
        std::fs::write(
            demo.join("ply.yaml"),
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      tiered_fee:\n        checks: [bounded(2)]\n  ledger:\n    anchor: ledger\n    fns:\n      fees::bps_for_tier:\n        ensures:\n          - \"|result| *result <= 10_000\"\n",
        )
        .unwrap();
        std::fs::write(
            demo.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [dependencies]\nledger = { path = \"../ledger\" }\n",
        )
        .unwrap();

        let report = worklist_crate(&demo).unwrap();
        let owed: Vec<_> = items(&report)
            .into_iter()
            .filter(|i| i.kind == "owed_evidence")
            .collect();
        assert_eq!(owed.len(), 1, "{owed:#?}");
        assert!(
            owed[0]
                .detail
                .contains("run `cargo ply verify` inside the `ledger` crate"),
            "the advice has to name the package that check would have to be run in: {}",
            owed[0].detail
        );
        assert!(
            owed[0].detail.contains(
                "pass what `ledger::fees::bps_for_tier` returns into \
                                     `tiered_fee` as a parameter"
            ),
            "and it should offer the route that needs no second crate at all: {}",
            owed[0].detail
        );
    }

    /// The line the adversarial review drew, and the one this command must
    /// not blur: an environmental assumption can never be discharged, so
    /// counting it as owed would pressure a user into deleting an honest
    /// declaration. It belongs to `audit` and appears nowhere here.
    #[test]
    fn an_environmental_assumption_is_never_an_open_item() {
        let dir = crate_with(
            "#[ply::requires(tick > 0)]\npub fn quote(tick: u32) -> u32 { tick }\n",
            "ply: 1\nexternals:\n  venue:\n    note: \"the exchange: accepts orders, returns fills\"\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      quote:\n        checks: [test]\n        requires:\n          - \"tick > 0\"\n        entry: [venue]\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        assert!(items(&report).is_empty(), "{:#?}", items(&report));
        assert!(
            EMPTY_WORKLIST.contains("Nothing is owed that Ply can see"),
            "{EMPTY_WORKLIST}"
        );
    }

    /// The two tiers §6 promises that this build cannot deliver, in the
    /// same `coverage.not_checked` place `check` puts its own — plus the
    /// cap this build does not enforce, which is a promise made on every
    /// marker line above.
    #[test]
    fn the_report_names_the_tiers_it_cannot_deliver() {
        let dir = crate_with(
            "pub fn quote() -> u32 { 1 }\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        let cov = report.envelope.coverage.as_ref().unwrap();
        let names: Vec<&str> = cov.not_checked.iter().map(|t| t.tier.as_str()).collect();
        assert_eq!(
            names,
            [
                "weak specs (W0502)",
                "stale claims (W0302)",
                "check cap (W0521)"
            ]
        );
        assert_eq!(cov.not_checked[0].detail, WEAK_SPEC_GAP);
        assert_eq!(cov.not_checked[1].detail, STALE_CLAIM_GAP);
        assert_eq!(cov.not_checked[2].detail, CHECK_CAP_GAP);
        assert!(STALE_CLAIM_GAP.contains("ply.lock"));
        assert!(WEAK_SPEC_GAP.contains("mutate"));
    }

    /// An open item is work somebody recorded, not a failure. A `worklist`
    /// that exited non-zero for having items would be a `verify` with worse
    /// manners, and would make deleting the marker the cheapest fix.
    #[test]
    fn a_run_with_open_items_still_exits_zero_and_says_why() {
        let dir = crate_with(
            "pub fn quote() -> u32 {\n    ply::unresolved!(1, \"undecided\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        assert_eq!(items(&report).len(), 1);
        assert_eq!(report.exit_code(), 0);
        assert!(
            NOT_A_FAILURE.contains(
                "`worklist` exits 0 whether or not it has items to show: an open item is work \
                 somebody recorded, not a failure"
            ),
            "{NOT_A_FAILURE}"
        );
    }

    /// §8's envelope, with `worklist` in the command field and the items as
    /// data.
    #[test]
    fn the_envelope_is_the_section_8_shape_with_the_open_items_as_data() {
        let dir = crate_with(
            "pub fn quote() -> u32 {\n    ply::unresolved!(151, \"undecided\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      quote:\n        checks: [test]\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&report.envelope.to_json_pretty()).unwrap();
        assert_eq!(json["command"], "worklist");
        assert_eq!(json["root"]["verdict"], "unclaimed");
        let open = json["open_items"].as_array().unwrap();
        assert_eq!(open[0]["kind"], "unresolved_marker");
        assert_eq!(open[0]["id"], 151);
        assert_eq!(open[0]["node_id"], "demo::quote");
        assert_eq!(open[0]["where"], "src/lib.rs:2:5");
        assert!(open[0]["blocking"].is_string());
    }

    /// The line a reader scans first. An item with an id leads with it —
    /// that is how people refer to these ("what happened to 147?") — and
    /// one without leads with the node, never with a stray dash where the
    /// id would have been.
    #[test]
    fn the_scanned_line_leads_with_the_id_where_there_is_one() {
        let dir = crate_with(
            "pub fn quote() -> u32 {\n    ply::unresolved!(147, \"undecided\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n    fns:\n      quote:\n        checks: [test]\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        assert_eq!(
            item_line(&items(&report)[0]),
            "#147 — `demo::quote` (at src/lib.rs:2:5)"
        );

        let owed = OpenItem {
            kind: "owed_evidence".into(),
            id: None,
            node_id: "demo::tiered_fee".into(),
            where_: Some("line 5, column 51".into()),
            blocking: String::new(),
            detail: String::new(),
        };
        assert_eq!(
            item_line(&owed),
            "`demo::tiered_fee` (at line 5, column 51)"
        );
    }

    /// Markers first, in id order, then owed evidence: the ids are how
    /// people talk about these ("what happened to 147?"), so a list that
    /// reordered them between runs would be unusable.
    #[test]
    fn items_are_ordered_by_tier_then_by_id() {
        let dir = crate_with(
            "pub fn quote() -> u32 {\n    ply::unresolved!(9, \"b\");\n    ply::unresolved!(2, \"a\");\n}\n",
            "ply: 1\ncomponents:\n  demo:\n    anchor: demo\n",
        );
        let report = worklist_crate(dir.path()).unwrap();
        let ids: Vec<Option<u64>> = items(&report).iter().map(|i| i.id).collect();
        assert_eq!(ids, [Some(2), Some(9)]);
    }

    #[test]
    fn a_missing_document_is_a_tool_error_naming_the_path_it_looked_at() {
        let dir = tempfile::tempdir().unwrap();
        let err = worklist_crate(dir.path()).unwrap_err().to_string();
        assert!(err.contains("could not read a ply.yaml at"), "{err}");
        assert!(err.contains("cargo ply worklist"), "{err}");
    }
}
