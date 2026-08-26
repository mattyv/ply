//! The-Ply-Spec.md §5.3, first paragraph only: the **crate tier** of
//! architecture semantics — the exact, sound half. From `cargo metadata`,
//! the real crate dependency graph; a dependency between two crates that
//! belong to different declared components, with no `->` edge permitting
//! it, is `A0401` (an error — this tier does not hedge). A `deny:` pattern
//! matched against that same graph is `A0405`. Containment — a component
//! and its own descendant — is always permitted with no edge declared.
//!
//! The **item tier** (§5.3's second paragraph: `calls`, `calls_dyn`,
//! `touches_cap`, `mutates`, profile bans — all from the approximate
//! syn-backed extractor) is a separate, later milestone and is not built
//! here. `crates/ply-cli/src/check.rs` says so in its own coverage report,
//! the same way it already says the architecture tier didn't exist at all
//! before this module.
//!
//! `W0409` (a redundant edge between a component and its own descendant)
//! is a document-local fact that needs no crate graph at all — it already
//! lives in [`crate::check`] and is unaffected by this module.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::model::{Component, Document, EdgeKind, parse_deny, parse_edge};

/// One real dependency `cargo metadata` resolved: the crate `from` links
/// directly against the crate `to`. Both names are each crate's identity
/// per [`crate_identity_name`] — the same spelling a `ply.yaml` anchor uses:
/// a package with a lib target uses that target's own name
/// (`ply_fixture_passing`, never the hyphenated package name
/// `ply-fixture-passing`); a package with no lib target at all (a pure
/// `[[bin]]` crate) uses its package name normalised the same way
/// (`ply-cli` -> `ply_cli`), never its binary target's name (`cargo-ply`).
/// This is a *normal* (runtime) dependency only: a `dev-dependencies` or
/// `build-dependencies` entry is test/build tooling, not code that runs in
/// the shipped crate, so it is not part of the graph this tier reasons
/// about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CrateDependency {
    pub from: String,
    pub to: String,
}

/// `cargo metadata` could not be run, or its output was not the shape this
/// reads — a tool problem, never a finding about the document.
#[derive(Debug)]
pub struct MetadataError(pub String);

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MetadataError {}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetaPackage>,
    resolve: Option<MetaResolve>,
}

#[derive(Deserialize)]
struct MetaPackage {
    id: String,
    /// The package name Cargo reports (hyphenated form, e.g. `ply-cli`) --
    /// used only as a fallback identity for a package with no lib target,
    /// via [`normalized_package_name`]. A package that also has a lib
    /// target is identified by that instead (`lib_target_name` wins).
    #[serde(default)]
    name: String,
    #[serde(default)]
    targets: Vec<MetaTarget>,
}

#[derive(Deserialize)]
struct MetaTarget {
    kind: Vec<String>,
    name: String,
}

#[derive(Deserialize)]
struct MetaResolve {
    nodes: Vec<MetaNode>,
}

#[derive(Deserialize)]
struct MetaNode {
    id: String,
    #[serde(default)]
    deps: Vec<MetaDep>,
}

#[derive(Deserialize)]
struct MetaDep {
    pkg: String,
    #[serde(default)]
    dep_kinds: Vec<MetaDepKind>,
}

#[derive(Deserialize)]
struct MetaDepKind {
    kind: Option<String>,
}

/// The lib-target identifier `cargo metadata` reports for a package — the
/// name a `ply.yaml` anchor would use to mean this crate — or `None` for a
/// package that carries no library target at all (a pure `[[bin]]` crate).
/// `None` here does *not* mean the package has no identity: a bin-only
/// package is still a crate a component can own (this repo's own
/// `ply-cli` is one), it is just named by [`normalized_package_name`]
/// instead — see [`crate_identity_name`], which every caller in this
/// module uses rather than calling this function directly.
fn lib_target_name(pkg: &MetaPackage) -> Option<&str> {
    pkg.targets
        .iter()
        .find(|t| {
            t.kind.iter().any(|k| {
                matches!(
                    k.as_str(),
                    "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
                )
            })
        })
        .map(|t| t.name.as_str())
}

/// Cargo's own hyphen-to-underscore rule for turning a package name into an
/// identifier (`ply-cli` -> `ply_cli`) -- the same normalisation Cargo
/// applies when it derives a default lib/bin identifier from `[package]
/// name`, so this is not a Ply-invented convention.
fn normalized_package_name(name: &str) -> String {
    name.replace('-', "_")
}

/// The identifier this tier uses to mean one crate — the name a `ply.yaml`
/// anchor must use to own it. A package with a library target is named by
/// that (the importable name, which can differ from the package name, as
/// `tests/fixtures/archtier` deliberately does); a package with no library
/// target at all — a pure `[[bin]]` crate — is still a crate a component
/// can own, identified by its own package name, normalised the way Cargo
/// normalises one. A binary target's own name (`cargo-ply`) is never used:
/// it names an *output artifact*, not the crate, and a package may carry
/// several.
fn crate_identity_name(pkg: &MetaPackage) -> String {
    match lib_target_name(pkg) {
        Some(lib_name) => lib_name.to_string(),
        None => normalized_package_name(&pkg.name),
    }
}

/// Runs `cargo metadata --format-version=1` in `crate_dir` and returns
/// every *normal* (runtime) dependency edge between two crates each
/// identified by [`crate_identity_name`] — the real, resolved graph, not
/// merely what a `Cargo.toml` declares (a `resolve` walk reflects which
/// optional dependencies actually got activated; a plain read of
/// `dependencies:` would not). The actual classification (identity
/// resolution, normal-dependency filtering) is [`graph_from_metadata`], a
/// pure function kept separate so a test can drive it directly against a
/// crafted `Metadata` value with no `cargo metadata` process in the loop.
pub fn crate_dependency_graph(crate_dir: &Path) -> Result<Vec<CrateDependency>, MetadataError> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version=1")
        .current_dir(crate_dir)
        .output()
        .map_err(|e| {
            MetadataError(format!(
                "could not run `cargo metadata` in {}: {e}",
                crate_dir.display()
            ))
        })?;
    if !output.status.success() {
        return Err(MetadataError(format!(
            "`cargo metadata` failed in {}: {}",
            crate_dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let meta: Metadata = serde_json::from_slice(&output.stdout).map_err(|e| {
        MetadataError(format!(
            "could not read `cargo metadata`'s output in {}: {e}",
            crate_dir.display()
        ))
    })?;
    Ok(graph_from_metadata(&meta))
}

/// The pure half of [`crate_dependency_graph`]: every real dependency
/// edge, both ends named by [`crate_identity_name`] -- so a bin-only
/// package (no `[lib]` at all) is still identified, by its own normalised
/// package name, rather than silently having no entry in the id-to-name
/// map and so dropping every dependency that originates from it.
fn graph_from_metadata(meta: &Metadata) -> Vec<CrateDependency> {
    let mut name_by_id: HashMap<&str, String> = HashMap::new();
    for pkg in &meta.packages {
        name_by_id.insert(pkg.id.as_str(), crate_identity_name(pkg));
    }

    let mut edges = Vec::new();
    if let Some(resolve) = &meta.resolve {
        for node in &resolve.nodes {
            let Some(from) = name_by_id.get(node.id.as_str()) else {
                continue;
            };
            for dep in &node.deps {
                let is_normal =
                    dep.dep_kinds.is_empty() || dep.dep_kinds.iter().any(|k| k.kind.is_none());
                if !is_normal {
                    continue;
                }
                let Some(to) = name_by_id.get(dep.pkg.as_str()) else {
                    continue;
                };
                edges.push(CrateDependency {
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
    }
    edges
}

/// Resolves crate names (an anchor's segment before its first `::`, or the
/// whole anchor when it has none) to the declared component that owns them,
/// and answers the two questions the crate tier needs about the declared
/// component tree: does an edge permit a crossing, and does containment.
///
/// Built once per document and reused across every real dependency pair —
/// re-walking the component tree per pair would be quadratic for no
/// reason.
pub struct ComponentIndex {
    /// crate lib-ident name -> qualified dotted path of the declared
    /// component that owns it (the outermost component whose anchor names
    /// that crate's root; document order, first one wins on a collision).
    pub crate_owner: HashMap<String, String>,
    leaf_index: HashMap<String, Vec<String>>,
    all_qualified: HashSet<String>,
}

impl ComponentIndex {
    pub fn build(doc: &Document) -> Self {
        let mut crate_owner = HashMap::new();
        let mut leaf_index: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_qualified = HashSet::new();

        fn walk(
            qualified: &str,
            leaf: &str,
            comp: &Component,
            crate_owner: &mut HashMap<String, String>,
            leaf_index: &mut HashMap<String, Vec<String>>,
            all_qualified: &mut HashSet<String>,
        ) {
            leaf_index
                .entry(leaf.to_string())
                .or_default()
                .push(qualified.to_string());
            all_qualified.insert(qualified.to_string());

            let crate_name = comp
                .anchor
                .split("::")
                .next()
                .unwrap_or(&comp.anchor)
                .to_string();
            crate_owner
                .entry(crate_name)
                .or_insert_with(|| qualified.to_string());

            for (child_name, nested) in &comp.components {
                let nested_qualified = format!("{qualified}.{child_name}");
                walk(
                    &nested_qualified,
                    child_name,
                    nested,
                    crate_owner,
                    leaf_index,
                    all_qualified,
                );
            }
        }

        for (name, comp) in &doc.components {
            walk(
                name,
                name,
                comp,
                &mut crate_owner,
                &mut leaf_index,
                &mut all_qualified,
            );
        }

        Self {
            crate_owner,
            leaf_index,
            all_qualified,
        }
    }

    /// Resolves one edge/deny endpoint token to the single qualified
    /// component path it must mean — `None` for `*`, for a bare name that
    /// is ambiguous (already reported as `E0206` by the document tier) or
    /// resolves to nothing, or for a dotted path naming no real component.
    /// Mirrors `ply_core::check`'s own token resolution (§5.1a rule 6) —
    /// duplicated here in miniature rather than shared, so this module
    /// stays self-contained.
    fn resolve(&self, token: &str) -> Option<String> {
        if token == "*" {
            return None;
        }
        if token.contains('.') {
            return self
                .all_qualified
                .contains(token)
                .then(|| token.to_string());
        }
        match self.leaf_index.get(token) {
            Some(paths) if paths.len() == 1 => Some(paths[0].clone()),
            _ => None,
        }
    }

    /// `ancestor` is a strict prefix of `other` on a `.` boundary — the
    /// same nesting test `ply_core::check::is_strict_ancestor` applies to
    /// edge redundancy.
    fn is_strict_ancestor(ancestor: &str, other: &str) -> bool {
        other.len() > ancestor.len()
            && other.starts_with(ancestor)
            && other.as_bytes()[ancestor.len()] == b'.'
    }

    /// §5.3: "containment implies permission" — a component may always
    /// depend on its own descendant, and the descendant on it, with no
    /// edge declared — plus a literal `->` edge naming exactly this
    /// ordered pair.
    pub fn permitted(&self, doc: &Document, from: &str, to: &str) -> bool {
        if Self::is_strict_ancestor(from, to) || Self::is_strict_ancestor(to, from) {
            return true;
        }
        doc.edges
            .iter()
            .filter_map(|e| parse_edge(e).ok())
            .any(|e| {
                matches!(e.kind, EdgeKind::Call)
                    && self.resolve(&e.from).as_deref() == Some(from)
                    && self.resolve(&e.to).as_deref() == Some(to)
            })
    }

    /// Every `deny:` entry (by its position in `doc.deny`, plus its
    /// original source text) whose pattern matches this ordered
    /// (component, component) pair and whose `except` list does not name
    /// `from`.
    pub fn matching_deny<'a>(
        &self,
        doc: &'a Document,
        from: &str,
        to: &str,
    ) -> Vec<(usize, &'a str)> {
        let mut out = Vec::new();
        for (i, d) in doc.deny.iter().enumerate() {
            let Ok(deny) = parse_deny(d) else {
                continue;
            };
            let from_matches =
                deny.from == "*" || self.resolve(&deny.from).as_deref() == Some(from);
            let to_matches = deny.to == "*" || self.resolve(&deny.to).as_deref() == Some(to);
            if !(from_matches && to_matches) {
                continue;
            }
            let excepted = deny
                .except
                .iter()
                .any(|e| self.resolve(e).as_deref() == Some(from));
            if excepted {
                continue;
            }
            out.push((i, d.as_str()));
        }
        out
    }
}

/// One crate-tier finding, independent of the §8 envelope shape — the
/// caller (`cargo ply check`) wraps this into a full `diag::Diagnostic`
/// the same way `ply_core::check::Diagnostic` is wrapped by
/// `document_diag`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchFinding {
    pub code: &'static str,
    pub message: String,
    pub node_id: String,
}

/// What the crate tier actually looked at, for the coverage report: how
/// many real crate-dependency pairs crossed between two differently-owned
/// declared components, and how many of those were flagged.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArchTally {
    pub cross_component_pairs: usize,
    pub violations: usize,
    pub deny_violations: usize,
}

fn a0401_message(from_crate: &str, to_crate: &str, from_comp: &str, to_comp: &str) -> String {
    format!(
        "crate `{from_crate}` depends on crate `{to_crate}`. `{from_crate}` belongs to the \
         `{from_comp}` component and `{to_crate}` belongs to `{to_comp}`, and no `->` edge in \
         this document says `{from_comp}` may depend on `{to_comp}` — so this dependency \
         crosses a boundary nothing allows. Add \"{from_comp} -> {to_comp}\" under `edges:` if \
         this is intended, or remove the dependency."
    )
}

fn a0405_message(
    from_crate: &str,
    to_crate: &str,
    from_comp: &str,
    to_comp: &str,
    deny_str: &str,
) -> String {
    let deny_str = deny_str.trim();
    format!(
        "crate `{from_crate}` (component `{from_comp}`) depends on crate `{to_crate}` \
         (component `{to_comp}`), and this matches the rule \"{deny_str}\" under `deny:`, which \
         forbids it. Remove the dependency, or change the `deny:` rule if this crossing should \
         be allowed."
    )
}

/// §5.3's crate tier, run over an already-fetched dependency graph (kept
/// separate from [`crate_dependency_graph`] so this half — the actual
/// classification rule — is a pure function a test can drive directly,
/// with no `cargo metadata` process in the loop).
pub fn check_architecture(
    doc: &Document,
    graph: &[CrateDependency],
) -> (Vec<ArchFinding>, ArchTally) {
    let index = ComponentIndex::build(doc);
    let mut findings = Vec::new();
    let mut tally = ArchTally::default();

    for dep in graph {
        let Some(from_comp) = index.crate_owner.get(&dep.from) else {
            continue;
        };
        let Some(to_comp) = index.crate_owner.get(&dep.to) else {
            continue;
        };
        if from_comp == to_comp {
            continue;
        }
        tally.cross_component_pairs += 1;

        if !index.permitted(doc, from_comp, to_comp) {
            tally.violations += 1;
            findings.push(ArchFinding {
                code: "A0401",
                message: a0401_message(&dep.from, &dep.to, from_comp, to_comp),
                node_id: format!("{from_comp}->{to_comp}"),
            });
        }

        for (deny_idx, deny_str) in index.matching_deny(doc, from_comp, to_comp) {
            tally.deny_violations += 1;
            findings.push(ArchFinding {
                code: "A0405",
                message: a0405_message(&dep.from, &dep.to, from_comp, to_comp, deny_str),
                node_id: format!("deny[{deny_idx}]"),
            });
        }
    }

    (findings, tally)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_document;

    fn doc(yaml: &str) -> Document {
        parse_document(yaml).unwrap()
    }

    fn meta_target(kind: &str, name: &str) -> MetaTarget {
        MetaTarget {
            kind: vec![kind.to_string()],
            name: name.to_string(),
        }
    }

    /// A package with both a lib and a bin target is identified by its lib
    /// name, never its bin name -- `crate_identity_name` (and
    /// `lib_target_name` underneath it) must not pick whichever target
    /// happens to be found first, or the last one, but always the lib.
    #[test]
    fn crate_identity_prefers_the_lib_target_over_a_bin_target() {
        let pkg = MetaPackage {
            id: "id1".into(),
            name: "ply-fixture-dual".into(),
            targets: vec![
                meta_target("bin", "dual-cli"),
                meta_target("lib", "dual_lib"),
            ],
        };
        assert_eq!(crate_identity_name(&pkg), "dual_lib");
    }

    /// A package with no lib target at all -- a pure `[[bin]]` crate, the
    /// exact shape `ply-cli` itself has (`cargo-ply`, `['bin']`) -- is
    /// still identified: by its own package name, normalised the way
    /// Cargo normalises one (hyphens to underscores), never by its binary
    /// target's own name. Before the fix this had no identity at all, so
    /// every dependency originating from it was silently dropped from the
    /// graph.
    #[test]
    fn crate_identity_falls_back_to_the_normalized_package_name_for_a_bin_only_crate() {
        let pkg = MetaPackage {
            id: "id2".into(),
            name: "ply-cli".into(),
            targets: vec![meta_target("bin", "cargo-ply")],
        };
        assert_eq!(crate_identity_name(&pkg), "ply_cli");
    }

    /// The regression this whole defect was: `graph_from_metadata` (the
    /// pure half of `crate_dependency_graph`) must include a real
    /// dependency whose *source* crate has no lib target at all -- before
    /// the fix, such a crate had no entry in the id-to-name map, so every
    /// edge originating from it was silently dropped and the graph came
    /// back looking clean.
    #[test]
    fn graph_from_metadata_includes_a_dependency_from_a_bin_only_crate() {
        let meta = Metadata {
            packages: vec![
                MetaPackage {
                    id: "top".into(),
                    name: "ply-cli".into(),
                    targets: vec![meta_target("bin", "cargo-ply")],
                },
                MetaPackage {
                    id: "core".into(),
                    name: "ply-core".into(),
                    targets: vec![meta_target("lib", "ply_core")],
                },
            ],
            resolve: Some(MetaResolve {
                nodes: vec![
                    MetaNode {
                        id: "top".into(),
                        deps: vec![MetaDep {
                            pkg: "core".into(),
                            dep_kinds: vec![],
                        }],
                    },
                    MetaNode {
                        id: "core".into(),
                        deps: vec![],
                    },
                ],
            }),
        };
        let graph = graph_from_metadata(&meta);
        assert_eq!(
            graph,
            vec![CrateDependency {
                from: "ply_cli".into(),
                to: "ply_core".into(),
            }],
            "the bin-only crate's own dependency must not be dropped: {graph:?}"
        );
    }

    /// The invariant this tier exists to guarantee (§5.3/D4: "default-deny
    /// between declared components"): walking the *real* classification
    /// output, every real dependency between two crates owned by different
    /// declared components is either permitted (an edge, or containment)
    /// or produces exactly one `A0401` — never both, never neither. This
    /// is the one check that would catch a *new* containment or edge shape
    /// silently falling through unclassified, the way
    /// `every_painted_element_resolves_a_style_rule` catches a construct
    /// the renderer forgot to style.
    ///
    /// Until 2026-08-26 this test called `index.permitted(...)` for its own
    /// "expected" side and then compared that to whether `check_architecture`
    /// (which is itself implemented in terms of `index.permitted`) had
    /// flagged the pair — both sides were the same logic wearing two hats,
    /// so a change that broke containment or edge resolution would break
    /// both sides identically and the test would stay green. §9 as amended
    /// calls that "a test that cannot fail": it went unnoticed exactly this
    /// way when containment was disabled in review. The fix is below: the
    /// expected side is now hand-written from the document's literal
    /// shape — the crate-to-component mapping and the containment/edge
    /// facts are spelled out as data in this test, not fetched from
    /// `ComponentIndex` at all — so a real regression in either mapping or
    /// containment/edge resolution changes only the code side, and the two
    /// sides can disagree.
    #[test]
    fn every_cross_component_dependency_is_either_permitted_or_flagged_never_both() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
    components:
      c:
        anchor: crate_c
  b:
    anchor: crate_b
  d:
    anchor: crate_d
edges:
  - "b -> a"
"#);
        // Every ordered pair among 4 crates, several of which have no
        // declared component at all (`crate_x`) -- those must be ignored,
        // not crash or misclassify.
        let crates = ["crate_a", "crate_b", "crate_c", "crate_d", "crate_x"];
        let mut graph = Vec::new();
        for &from in &crates {
            for &to in &crates {
                if from != to {
                    graph.push(CrateDependency {
                        from: from.to_string(),
                        to: to.to_string(),
                    });
                }
            }
        }

        let (findings, _tally) = check_architecture(&document, &graph);

        // The oracle: hand-derived from the YAML literal above, not from
        // any production type. `crate_x` owns no declared component at
        // all, so it is simply absent from this map -- exactly mirroring
        // what §5.3 says an undeclared crate is: out of scope, not a
        // violation and not a permission.
        fn expected_component_for_crate(crate_name: &str) -> Option<&'static str> {
            match crate_name {
                "crate_a" => Some("a"),
                "crate_b" => Some("b"),
                "crate_c" => Some("a.c"),
                "crate_d" => Some("d"),
                _ => None,
            }
        }
        // The document declares exactly one nesting line (`a` contains
        // `a.c`) and exactly one edge (`b -> a`) -- written out here as
        // plain pairs, independent of `ComponentIndex::is_strict_ancestor`
        // or `ComponentIndex::permitted`.
        fn expected_permitted(from_comp: &str, to_comp: &str) -> bool {
            let is_containment = matches!((from_comp, to_comp), ("a", "a.c") | ("a.c", "a"));
            let is_declared_edge = (from_comp, to_comp) == ("b", "a");
            is_containment || is_declared_edge
        }

        for dep in &graph {
            let (Some(from_comp), Some(to_comp)) = (
                expected_component_for_crate(&dep.from),
                expected_component_for_crate(&dep.to),
            ) else {
                continue; // crate_x: no declared component, nothing to classify
            };
            if from_comp == to_comp {
                continue;
            }
            let is_permitted = expected_permitted(from_comp, to_comp);
            let is_flagged = findings
                .iter()
                .any(|f| f.code == "A0401" && f.node_id == format!("{from_comp}->{to_comp}"));
            assert!(
                is_permitted != is_flagged,
                "pair {}->{} ({from_comp}->{to_comp}) must be either permitted or flagged, \
                 never both and never neither -- permitted={is_permitted} flagged={is_flagged}",
                dep.from,
                dep.to
            );
        }
    }

    /// A crate whose component has no edge and no containment relationship
    /// to the crate it depends on -- the plain default-deny case -- is
    /// `A0401`, naming both crates and both components, and saying plainly
    /// that no edge permits it.
    #[test]
    fn undeclared_cross_component_dependency_is_a0401_naming_both_sides() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
"#);
        let graph = vec![CrateDependency {
            from: "crate_b".into(),
            to: "crate_a".into(),
        }];
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.violations, 1);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.code, "A0401");
        assert!(f.message.contains("crate_a"), "{}", f.message);
        assert!(f.message.contains("crate_b"), "{}", f.message);
        assert!(
            f.message.contains("`a`"),
            "must name the `a` component: {}",
            f.message
        );
        assert!(
            f.message.contains("`b`"),
            "must name the `b` component: {}",
            f.message
        );
        assert!(
            f.message.contains("no `->` edge in this document says"),
            "must plainly say no declared edge permits it: {}",
            f.message
        );
    }

    /// A declared `->` edge permits the exact real dependency it names --
    /// no `A0401`.
    #[test]
    fn a_declared_edge_permits_the_matching_dependency() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
edges:
  - "b -> a"
"#);
        let graph = vec![CrateDependency {
            from: "crate_b".into(),
            to: "crate_a".into(),
        }];
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.violations, 0);
        assert!(findings.is_empty(), "{findings:?}");
    }

    /// §5.3: containment implies permission with no edge declared at all --
    /// a component depending on its own descendant is never a violation.
    #[test]
    fn containment_permits_a_dependency_on_a_descendant_with_no_edge() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
    components:
      c:
        anchor: crate_c
"#);
        let graph = vec![CrateDependency {
            from: "crate_a".into(),
            to: "crate_c".into(),
        }];
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.violations, 0);
        assert!(
            findings.is_empty(),
            "containment permits this with no edge declared: {findings:?}"
        );
    }

    /// Containment also runs the other way -- a descendant depending back
    /// on its ancestor is equally permitted.
    #[test]
    fn containment_permits_a_descendant_depending_back_on_its_ancestor() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
    components:
      c:
        anchor: crate_c
"#);
        let graph = vec![CrateDependency {
            from: "crate_c".into(),
            to: "crate_a".into(),
        }];
        let (findings, _tally) = check_architecture(&document, &graph);
        assert!(findings.is_empty(), "{findings:?}");
    }

    /// A `deny:` pattern is checked against the real graph independent of
    /// whether a permitting edge exists -- an explicit ban overrides a
    /// permission. This is what makes `A0401` and `A0405` two different
    /// facts rather than the same finding twice.
    #[test]
    fn a_permitted_dependency_can_still_violate_an_explicit_deny_rule() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
edges:
  - "b -> a"
deny:
  - "b -> a"
"#);
        let graph = vec![CrateDependency {
            from: "crate_b".into(),
            to: "crate_a".into(),
        }];
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.violations, 0, "the edge permits it, so no A0401");
        assert_eq!(tally.deny_violations, 1);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "A0405");
        assert!(
            findings[0].message.contains("crate_a"),
            "{}",
            findings[0].message
        );
        assert!(
            findings[0].message.contains("crate_b"),
            "{}",
            findings[0].message
        );
    }

    /// A wildcard `deny` pattern with an `except` list exempts the named
    /// component but still catches everyone else.
    #[test]
    fn a_wildcard_deny_with_except_exempts_only_the_named_component() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
  m:
    anchor: crate_m
edges:
  - "b -> a"
  - "m -> a"
deny:
  - "* -> a except m"
"#);
        let graph = vec![
            CrateDependency {
                from: "crate_b".into(),
                to: "crate_a".into(),
            },
            CrateDependency {
                from: "crate_m".into(),
                to: "crate_a".into(),
            },
        ];
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.deny_violations, 1, "{findings:?}");
        let f = findings.iter().find(|f| f.code == "A0405").unwrap();
        assert!(f.message.contains("crate_b"), "{}", f.message);
        assert!(
            !findings.iter().any(|f| f.message.contains("crate_m")),
            "m is excepted: {findings:?}"
        );
    }

    /// A dependency on a crate that owns no declared component at all
    /// (an ordinary external dependency, e.g. `serde`) is simply outside
    /// this tier's scope -- not a violation, not counted.
    #[test]
    fn a_dependency_on_an_undeclared_crate_is_ignored() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
"#);
        let graph = vec![CrateDependency {
            from: "crate_a".into(),
            to: "serde".into(),
        }];
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.cross_component_pairs, 0);
        assert!(findings.is_empty(), "{findings:?}");
    }
}
