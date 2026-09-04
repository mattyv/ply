//! Loading a `ply.yaml` off disk (The-Ply-Spec.md §5): read, validate its
//! keys against the §5 vocabulary (`E0204`), parse it into
//! [`crate::model::Document`], and refuse a schema version this build does
//! not speak (`E0201`).
//!
//! Until Phase 1a this module *also* carried a hand-rolled four-struct
//! subset of the format, in parallel with the full model in `tools/model`.
//! Two readers of one document is the defect §5.1a rule 1 was amended to
//! name (vetting 004 finding 7), so the subset is gone: there is now one
//! model, [`crate::model`], and every command reads the document through
//! this function.
//!
//! Still out of scope for this slice: multi-file discovery and merge (§5's
//! "files named `ply.yaml` (or `*.ply.yaml`) ... merge into one model") —
//! `load` reads exactly the path it is given.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::model::{Check, Component, Document, FnClaim, parse_check, parse_document};
use crate::schema;

/// One claim's own `checks:` strings, parsed, with `E0203` attached to
/// whichever entry failed (§5.1a rule 4). The plain-language reason comes
/// first and the code follows it — a newbie reads the sentence, a script
/// greps the code.
pub fn parsed_checks(claim: &FnClaim) -> Result<Vec<Check>> {
    claim
        .parsed_checks()
        .map_err(|reason| anyhow::anyhow!("{reason} (E0203)"))
}

/// One check string, parsed, with the same `E0203` attachment.
pub fn parse_check_string(s: &str) -> Result<Check> {
    parse_check(s).map_err(|reason| anyhow::anyhow!("{reason} (E0203)"))
}

/// §5.1a rule 1 and the rest of the load-time schema tier, **read out of
/// `schema/ply.schema.json`** rather than restated here.
///
/// This used to be a second copy of the key vocabulary living next to the
/// serde model — three descriptions of one grammar, with nothing forcing
/// them to agree. The list now comes from the schema, so deleting a key
/// there changes what Ply accepts; `crates/ply-core/tests/schema.rs` holds
/// the model to the same document from the other side.
///
/// Reports the first violation, because that is what a caller loading a
/// document can act on. `cargo ply check` calls [`crate::schema::validate`]
/// directly and reports all of them.
pub fn validate_keys(yaml_text: &str) -> Result<()> {
    let doc: serde_yaml_ng::Value = match serde_yaml_ng::from_str(yaml_text) {
        Ok(v) => v,
        // A document that is not even YAML is the parser's error to report,
        // not this validator's.
        Err(_) => return Ok(()),
    };
    match schema::validate(&doc).into_iter().next() {
        Some(v) => bail!("{}: {}", v.code, v.message),
        None => Ok(()),
    }
}

/// Loads and parses the `ply.yaml` at `path`.
///
/// Order matters and is deliberate: `E0204` key validation runs *first*, so
/// a typo'd key gets the sentence that names the nearest key Ply knows
/// rather than serde's `unknown field` line, which suggests nothing.
pub fn load(path: &Path) -> Result<Document> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading ply.yaml at {}", path.display()))?;
    load_str(&text).with_context(|| format!("reading ply.yaml at {}", path.display()))
}

/// [`load`] over already-read text, for callers that have the document in
/// hand (tests, and `check`, which reads the file once and reports on it).
pub fn load_str(text: &str) -> Result<Document> {
    validate_keys(text)?;
    let doc = parse_document(text).map_err(|e| anyhow::anyhow!("{e}"))?;
    if doc.ply != 1 {
        bail!(
            "E0201: unsupported `ply:` schema version {} (expected 1)",
            doc.ply
        );
    }
    Ok(doc)
}

/// One cross-document link, derived rather than declared (The-Ply-Spec.md
/// §7.1 "hollow means nothing inside; collapsed means plenty inside,
/// folded" -- a link supplies the "plenty inside" from a different file,
/// with no `include:` key anywhere: a design pass considered one and
/// rejected it in favour of deriving the link from anchors alone).
///
/// A component in document A links to document B when B's own top-level
/// component anchor equals, or sits under, A's component's anchor -- so
/// `target` is a clone of that one matching top-level component from B,
/// which is all a renderer needs to draw B's contents count and badges
/// without ever reading B's file itself (The-Ply-Spec.md's "a renderer's
/// only input is the §8 envelope -- no side channel").
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLink {
    /// B's path, for display -- stripped of a leading `./` so a drawing
    /// built with `root: "."` reads `crates/ply-core/ply.yaml`, never
    /// `./crates/ply-core/ply.yaml`.
    pub target_path: String,
    pub target: Component,
}

/// Every derived link a document's top-level components resolved, keyed by
/// the component's own bare top-level name. Nested components are never
/// candidates -- see [`derive_links`]'s doc comment for why.
pub type LinkIndex = BTreeMap<String, ResolvedLink>;

/// One of the four named ways a candidate link can fail to form, attached
/// to the *including* component rather than the document it was reaching
/// for: a reader sees this on the box that tried to link, not on a file
/// that (from this document's point of view) may not even parse.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkFinding {
    pub code: &'static str,
    pub severity: &'static str,
    pub component_path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkSet {
    pub links: LinkIndex,
    pub findings: Vec<LinkFinding>,
}

/// Derives every cross-document link this document's top-level components
/// resolve to, purely from anchors and real crate directories under
/// `root` -- the same argument [`crate::harness::resolve_state_fields`]
/// takes, and for the same reason: an anchor is resolved against the real
/// crate tree, never against either document's own text.
///
/// Only **top-level** components are ever candidates. A nested component's
/// anchor (`ply_core::kernel`) shares its crate with the document it
/// already lives in, so treating it as a fresh candidate would make every
/// module in `crates/ply-core/ply.yaml` "discover" that very document on
/// every single run -- the degenerate zero-hop case every crate-rooted
/// document's own top-level component already has to refuse silently
/// (below), just twenty times over for no reason.
///
/// Four things stop a candidate from becoming a link, and each is a named,
/// tested outcome rather than a silent guess: the target document exists
/// but cannot be read or does not parse (`A0417`); its top-level anchor no
/// longer sits under this component's anchor (`W0532`, "drifted" rather
/// than vanishing with no trace); resolving it would eventually revisit a
/// document already on this chain (`W0534`, a cycle); or another component
/// in this same document already claimed it (`W0533`, at most one link per
/// target). A crate that simply has no `ply.yaml` of its own is the
/// ordinary case and produces neither a link nor a finding.
pub fn derive_links(doc: &Document, root: &Path) -> LinkSet {
    let root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    let this_doc = root.join("ply.yaml");
    let this_doc = this_doc.canonicalize().unwrap_or(this_doc);
    let crates = crate::harness::workspace_library_crates(root);

    let mut set = LinkSet::default();
    let mut claimed: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (name, comp) in &doc.components {
        resolve_one(name, comp, &crates, &this_doc, &mut claimed, &mut set);
    }
    set
}

/// The `ply.yaml` a component's anchor would name, if its crate segment
/// names a real crate this workspace has. `None` is not a defect: an
/// anchor naming no crate at all is `A0410`'s problem, not this rule's.
fn candidate_path(comp: &Component, crates: &BTreeMap<String, PathBuf>) -> Option<PathBuf> {
    let crate_name = comp.anchor.split("::").next()?;
    crates.get(crate_name).map(|dir| dir.join("ply.yaml"))
}

/// Whether `anchor` is `floor` itself or a `::`-descendant of it -- the one
/// relation this whole feature is built on.
fn anchor_under(anchor: &str, floor: &str) -> bool {
    anchor == floor || anchor.starts_with(&format!("{floor}::"))
}

fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

fn resolve_one(
    component_path: &str,
    comp: &Component,
    crates: &BTreeMap<String, PathBuf>,
    this_doc: &Path,
    claimed: &mut BTreeMap<PathBuf, String>,
    set: &mut LinkSet,
) {
    let Some(candidate) = candidate_path(comp, crates) else {
        return;
    };
    if std::fs::symlink_metadata(&candidate).is_err() {
        // No `ply.yaml` sits there at all -- the ordinary case for a crate
        // with no self-description, never a finding.
        return;
    }
    let canonical = candidate.canonicalize().unwrap_or_else(|_| candidate.clone());
    if canonical == *this_doc {
        // This component's crate *is* the document being derived -- not a
        // link to "another" document, just this one naming its own crate.
        // Silent, on purpose: every crate-rooted `ply.yaml`'s own top-level
        // component would otherwise "discover" itself on every run.
        return;
    }
    let target_doc = match std::fs::read_to_string(&candidate) {
        Ok(text) => match load_str(&text) {
            Ok(target_doc) => target_doc,
            Err(e) => {
                set.findings.push(LinkFinding {
                    code: "A0417",
                    severity: "error",
                    component_path: component_path.to_string(),
                    message: format!(
                        "component `{component_path}` is anchored at `{}`, whose crate has \
                         its own `{}`, but it does not read as a valid ply.yaml document: {e}. \
                         This box draws its own declared interior instead of the link, and the \
                         run continues.",
                        comp.anchor,
                        candidate.display()
                    ),
                });
                return;
            }
        },
        Err(e) => {
            set.findings.push(LinkFinding {
                code: "A0417",
                severity: "error",
                component_path: component_path.to_string(),
                message: format!(
                    "component `{component_path}` is anchored at `{}`, whose crate has its \
                     own `{}`, but it could not be read: {e}. This box draws its own declared \
                     interior instead of the link, and the run continues.",
                    comp.anchor,
                    candidate.display()
                ),
            });
            return;
        }
    };
    let Some((_, top)) = target_doc
        .components
        .iter()
        .find(|(_, c)| anchor_under(&c.anchor, &comp.anchor))
    else {
        // Rule 4: anchor drift. Say so rather than letting the link vanish
        // with no trace -- a reader who expected this box to point
        // somewhere should learn why it stopped, not just find that it did.
        if let Some((_, first)) = target_doc.components.iter().next() {
            set.findings.push(LinkFinding {
                code: "W0532",
                severity: "warning",
                component_path: component_path.to_string(),
                message: format!(
                    "component `{component_path}` is anchored at `{}`, and its crate has its \
                     own `{}`, but that document's own top-level anchor `{}` no longer sits \
                     under `{}` -- not linked. Realign one of the two anchors to relink them.",
                    comp.anchor,
                    candidate.display(),
                    first.anchor,
                    comp.anchor
                ),
            });
        }
        return;
    };
    let chain = vec![this_doc.to_path_buf(), canonical.clone()];
    if would_cycle(&target_doc, crates, &chain) {
        set.findings.push(LinkFinding {
            code: "W0534",
            severity: "warning",
            component_path: component_path.to_string(),
            message: format!(
                "component `{component_path}` would link to `{}`, but following that \
                 document's own further links eventually leads back to a document already in \
                 this chain -- not linked, so the drawing never has to walk a loop to find out.",
                candidate.display()
            ),
        });
        return;
    }
    if let Some(owner) = claimed.get(&canonical) {
        set.findings.push(LinkFinding {
            code: "W0533",
            severity: "warning",
            component_path: component_path.to_string(),
            message: format!(
                "component `{component_path}` would also link to `{}`, but component `{owner}` \
                 already claimed it -- a document links to another at most once, so this box \
                 draws its own declared interior instead.",
                candidate.display()
            ),
        });
        return;
    }
    claimed.insert(canonical, component_path.to_string());
    set.links.insert(
        component_path.to_string(),
        ResolvedLink {
            target_path: display_path(&candidate),
            target: top.clone(),
        },
    );
}

/// Whether resolving any of `target_doc`'s own top-level components as a
/// link -- the same relation [`resolve_one`] just used to reach
/// `target_doc` itself -- ever leads back to a document already in
/// `visited`. Real in this repository only as the two-file fixture in this
/// module's own tests builds directly: today's two documents are one hop
/// apart, so nothing here can construct a longer chain on its own. The
/// guard is unconditional anyway, because a cycle is a property of the
/// *graph* a set of `ply.yaml` files forms, not of how deep this
/// repository happens to nest today.
///
/// A sibling that is broken, drifted, or simply absent just ends the chase
/// quietly at that branch -- that is a different document's own problem,
/// reported when *it* is checked, never smeared onto the component that
/// merely chased through it looking for a cycle.
fn would_cycle(
    target_doc: &Document,
    crates: &BTreeMap<String, PathBuf>,
    visited: &[PathBuf],
) -> bool {
    target_doc.components.values().any(|sibling| {
        let Some(candidate) = candidate_path(sibling, crates) else {
            return false;
        };
        if std::fs::symlink_metadata(&candidate).is_err() {
            return false;
        }
        let Ok(canonical) = candidate.canonicalize() else {
            return false;
        };
        // The degenerate case: `sibling` names its own document's crate,
        // exactly like [`resolve_one`]'s own self-check. Not a cycle --
        // every crate-rooted document has a component shaped like this.
        if visited.last() == Some(&canonical) {
            return false;
        }
        if visited.contains(&canonical) {
            return true;
        }
        let Ok(text) = std::fs::read_to_string(&candidate) else {
            return false;
        };
        let Ok(next_doc) = load_str(&text) else {
            return false;
        };
        if !next_doc
            .components
            .values()
            .any(|c| anchor_under(&c.anchor, &sibling.anchor))
        {
            return false;
        }
        let mut chain = visited.to_vec();
        chain.push(canonical);
        would_cycle(&next_doc, crates, &chain)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::check::mutate_lacks_kill_signal;

    #[test]
    fn parses_bounded_check() {
        assert_eq!(parse_check_string("bounded(8)").unwrap(), Check::Bounded(8));
    }

    #[test]
    fn rejects_bounded_out_of_range() {
        assert!(parse_check_string("bounded(65)").is_err());
        assert!(parse_check_string("bounded(0)").is_err());
    }

    /// The `E0203` code follows the plain sentence rather than replacing it
    /// (CLAUDE.md's newbie bar): before Phase 1a the verify path had its own
    /// terser wording for the same defect.
    #[test]
    fn an_out_of_range_bound_says_why_before_it_says_the_code() {
        let err = parse_check_string("bounded(0)").unwrap_err().to_string();
        assert_eq!(
            err,
            "\"bounded(0)\" is not a valid check: the number is how many times loops are \
             unrolled during the proof, and it must be between 1 and 64 — a bound of 0 would \
             prove nothing (E0203)"
        );
    }

    #[test]
    fn mutate_alone_is_e0504() {
        assert!(mutate_lacks_kill_signal(&[Check::Mutate]));
    }

    #[test]
    fn mutate_with_fuzz_is_fine() {
        assert!(!mutate_lacks_kill_signal(&[
            Check::Fuzz(256),
            Check::Mutate
        ]));
    }

    #[test]
    fn mutate_with_test_is_fine() {
        assert!(!mutate_lacks_kill_signal(&[Check::Test, Check::Mutate]));
    }

    #[test]
    fn no_mutate_at_all_is_fine() {
        assert!(!mutate_lacks_kill_signal(&[Check::Bounded(2)]));
    }

    /// vetting 004 finding 7, the half that made it silent: `ensures:` was
    /// eaten by serde on the verify path while `ply-check` enforced
    /// §5.1a rule 1 on the very same document.
    #[test]
    fn a_typo_in_a_fn_key_is_e0204_with_the_nearest_key_named() {
        let err = validate_keys(
            r#"
ply: 1
components:
  withdrawal:
    anchor: withdrawal
    fns:
      fee_cents:
        ensure:
          - "|result| *result <= amount_cents"
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("E0204"), "{err}");
        assert!(err.contains("`ensure:`"), "{err}");
        assert!(err.contains("Did you mean `ensures`?"), "{err}");
        assert!(
            err.contains("components.withdrawal.fns.fee_cents.ensure"),
            "the message must say where the key is: {err}"
        );
    }

    /// The whole §5 grammar must survive, not just the subset `verify`
    /// acts on -- one document, three tools (vetting 004's own ply.yaml has
    /// `pure:` on a component and `edges:` at the top level).
    #[test]
    fn the_full_section_5_grammar_is_accepted_even_where_verify_ignores_it() {
        validate_keys(
            r#"
ply: 1
components:
  ledger:
    anchor: ledger
  withdrawal:
    anchor: withdrawal
    pure: true
    strict: false
    profile: core
    checks: [bounded(2)]
    fns:
      fee_cents:
        checks: [bounded(2)]
        mode: check
        requires: ["x < 10"]
        ensures: ["|result| *result <= x"]
        examples: ["fee_cents(1, 1) == 0"]
        check_with: { T: u64 }
        entry: [stripe]
        trusted:
          - claim: "loom-checked"
            evidence: "tests/loom.rs"
        unresolved:
          - id: 147
            note: "employee discount undecided"
externals:
  stripe:
    note: "the payment processor"
edges:
  - withdrawal -> ledger
deny:
  - "* -> ledger"
profiles:
  core: []
unresolved:
  - id: 9
    note: "open"
"#,
        )
        .unwrap();
    }

    #[test]
    fn an_unknown_top_level_key_is_caught_too() {
        let err = validate_keys(
            "ply: 1
component:
  x: 1
",
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("E0204") && err.contains("`components`"),
            "{err}"
        );
    }
    #[test]
    fn loads_minimal_ply_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ply.yaml");
        std::fs::write(
            &path,
            r#"
ply: 1
components:
  clamp:
    anchor: ply_fixture_clamp
    fns:
      clamp:
        checks: [bounded(2)]
"#,
        )
        .unwrap();
        let file = load(&path).unwrap();
        assert_eq!(file.ply, 1);
        let comp = file.components.get("clamp").unwrap();
        assert_eq!(comp.anchor, "ply_fixture_clamp");
        let fn_claim = comp.fns.get("clamp").unwrap();
        assert_eq!(parsed_checks(fn_claim).unwrap(), vec![Check::Bounded(2)]);
    }

    /// The four rules a derived link is refused by (The-Ply-Spec.md §7.1,
    /// the derive-document-links brief), plus the one degenerate case that
    /// looks like a cycle but is not: a crate-rooted document's own
    /// top-level component naming its own crate.
    mod derive_links_tests {
        use super::*;
        use std::path::Path;

        /// A minimal real crate on disk -- `Cargo.toml` with just enough
        /// to be found by [`crate::harness::workspace_library_crates`], and
        /// `src/lib.rs` so it counts as a library crate at all. `ply_yaml`
        /// is written alongside it when given, exactly where a component
        /// anchored at this crate's name would look for one.
        fn write_crate(dir: &Path, crate_name: &str, ply_yaml: Option<&str>) {
            let crate_dir = dir.join(crate_name);
            std::fs::create_dir_all(crate_dir.join("src")).unwrap();
            std::fs::write(
                crate_dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
                ),
            )
            .unwrap();
            std::fs::write(crate_dir.join("src/lib.rs"), "").unwrap();
            if let Some(yaml) = ply_yaml {
                std::fs::write(crate_dir.join("ply.yaml"), yaml).unwrap();
            }
        }

        /// Writes `text` as the outer document at `dir/ply.yaml` too, so
        /// the self-reference check has a real file to canonicalise
        /// against -- exactly the shape a loaded document has in
        /// production.
        fn outer_doc(dir: &Path, text: &str) -> Document {
            std::fs::write(dir.join("ply.yaml"), text).unwrap();
            parse_document(text).unwrap()
        }

        #[test]
        fn a_crate_with_its_own_ply_yaml_forms_a_link() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(
                dir.path(),
                "inner_lib",
                Some(
                    "ply: 1\ncomponents:\n  inner:\n    anchor: inner_lib\n    fns:\n      go:\n        checks: [bounded(2)]\n",
                ),
            );
            let outer = outer_doc(
                dir.path(),
                "ply: 1\ncomponents:\n  core:\n    anchor: inner_lib\n",
            );

            let set = derive_links(&outer, dir.path());

            assert!(set.findings.is_empty(), "{:?}", set.findings);
            let link = set.links.get("core").expect("core should link");
            // `dir.path()` is itself absolute (a tempdir), so the display
            // path is too here -- production always passes a workspace-
            // relative root, which is what makes `target_path` read
            // `crates/ply-core/ply.yaml` rather than an absolute host path.
            assert!(
                link.target_path.ends_with("inner_lib/ply.yaml"),
                "{}",
                link.target_path
            );
            assert_eq!(link.target.anchor, "inner_lib");
            assert!(link.target.fns.contains_key("go"));
        }

        #[test]
        fn a_crate_with_no_ply_yaml_of_its_own_is_silent() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(dir.path(), "bare_lib", None);
            let outer = outer_doc(
                dir.path(),
                "ply: 1\ncomponents:\n  core:\n    anchor: bare_lib\n",
            );

            let set = derive_links(&outer, dir.path());

            assert!(set.links.is_empty());
            assert!(set.findings.is_empty());
        }

        /// Rule 1, first half: the candidate exists (`symlink_metadata`
        /// sees it) but is not a readable file -- a stray directory named
        /// `ply.yaml`, standing in for any I/O failure. A finding, not a
        /// panic and not an `Err` out of [`derive_links`] itself.
        #[test]
        fn a_target_that_cannot_be_read_is_a_finding_not_an_abort() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(dir.path(), "broken_lib", None);
            std::fs::create_dir(dir.path().join("broken_lib/ply.yaml")).unwrap();
            let outer = outer_doc(
                dir.path(),
                "ply: 1\ncomponents:\n  core:\n    anchor: broken_lib\n",
            );

            let set = derive_links(&outer, dir.path());

            assert!(set.links.is_empty());
            assert_eq!(set.findings.len(), 1);
            assert_eq!(set.findings[0].code, "A0417");
            assert_eq!(set.findings[0].severity, "error");
            assert_eq!(set.findings[0].component_path, "core");
        }

        /// Rule 1, second half: the candidate is a real file but fails
        /// `load_str` (here, an unknown key -- `E0204`).
        #[test]
        fn a_target_that_does_not_parse_is_a_finding_not_an_abort() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(
                dir.path(),
                "bad_lib",
                Some("ply: 1\nbogus_top_level_key: 1\n"),
            );
            let outer = outer_doc(
                dir.path(),
                "ply: 1\ncomponents:\n  core:\n    anchor: bad_lib\n",
            );

            let set = derive_links(&outer, dir.path());

            assert!(set.links.is_empty());
            assert_eq!(set.findings.len(), 1);
            assert_eq!(set.findings[0].code, "A0417");
            assert_eq!(set.findings[0].severity, "error");
        }

        /// Rule 4: the target exists and parses, but its own top-level
        /// anchor no longer sits under the linking component's anchor. The
        /// link does not form, and it says why rather than vanishing.
        #[test]
        fn a_drifted_anchor_does_not_link_but_says_why() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(
                dir.path(),
                "drift_lib",
                Some("ply: 1\ncomponents:\n  inner:\n    anchor: somewhere_else\n"),
            );
            let outer = outer_doc(
                dir.path(),
                "ply: 1\ncomponents:\n  core:\n    anchor: drift_lib\n",
            );

            let set = derive_links(&outer, dir.path());

            assert!(set.links.is_empty());
            assert_eq!(set.findings.len(), 1);
            assert_eq!(set.findings[0].code, "W0532");
            assert_eq!(set.findings[0].severity, "warning");
        }

        /// Rule 3: two components in the same document would both claim
        /// the same target. Only the first (declaration order) links; the
        /// second is a diagnostic, not a second copy of the same box.
        #[test]
        fn two_components_claiming_the_same_target_link_only_the_first() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(
                dir.path(),
                "shared_lib",
                Some("ply: 1\ncomponents:\n  inner:\n    anchor: shared_lib\n"),
            );
            let outer = outer_doc(
                dir.path(),
                "ply: 1\ncomponents:\n  first:\n    anchor: shared_lib\n  second:\n    anchor: shared_lib\n",
            );

            let set = derive_links(&outer, dir.path());

            assert!(set.links.contains_key("first"));
            assert!(!set.links.contains_key("second"));
            assert_eq!(set.findings.len(), 1);
            assert_eq!(set.findings[0].code, "W0533");
            assert_eq!(set.findings[0].component_path, "second");
        }

        /// Rule 2: a real, two-file cycle -- `crate_a`'s document links
        /// onward to `crate_b`, and `crate_b`'s document links back to
        /// `crate_a`. The chain is refused with a named finding rather than
        /// walked forever.
        #[test]
        fn a_chain_that_leads_back_into_itself_is_refused() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(
                dir.path(),
                "crate_a",
                Some(
                    "ply: 1\ncomponents:\n  a:\n    anchor: crate_a\n  onward:\n    anchor: crate_b\n",
                ),
            );
            write_crate(
                dir.path(),
                "crate_b",
                Some(
                    "ply: 1\ncomponents:\n  b:\n    anchor: crate_b\n  back:\n    anchor: crate_a\n",
                ),
            );
            let outer = outer_doc(
                dir.path(),
                "ply: 1\ncomponents:\n  root_link:\n    anchor: crate_a\n",
            );

            let set = derive_links(&outer, dir.path());

            assert!(set.links.is_empty(), "{:?}", set.links);
            assert_eq!(set.findings.len(), 1);
            assert_eq!(set.findings[0].code, "W0534");
            assert_eq!(set.findings[0].component_path, "root_link");
        }

        /// Not a cycle: a crate-rooted document's own top-level component
        /// naming its own crate is the same file, not "another" document.
        /// Every document like `crates/ply-core/ply.yaml` hits this on
        /// every run, so it must stay silent rather than reporting a
        /// self-referential finding on itself forever.
        #[test]
        fn a_top_level_component_naming_its_own_documents_crate_is_silent() {
            let dir = tempfile::tempdir().unwrap();
            write_crate(dir.path(), "self_lib", None);
            let text = "ply: 1\ncomponents:\n  core:\n    anchor: self_lib\n";
            std::fs::write(dir.path().join("self_lib/ply.yaml"), text).unwrap();
            let doc = parse_document(text).unwrap();

            let set = derive_links(&doc, &dir.path().join("self_lib"));

            assert!(set.links.is_empty());
            assert!(set.findings.is_empty(), "{:?}", set.findings);
        }
    }
}
