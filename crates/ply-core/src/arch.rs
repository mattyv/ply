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

use std::collections::{BTreeSet, HashMap, HashSet};
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
    /// Package ids that are members of *this* workspace -- as opposed to
    /// `packages`, which also lists every resolved third-party dependency.
    /// Finding 3 (docs/review-architecture-tier.md): the coverage line's
    /// denominator is "how many crates in this workspace", so it is this
    /// field, not the full package list, that answers it. Absent in no
    /// real `cargo metadata` output this tool supports, but defaulted to
    /// empty rather than failing the parse if a future format ever omits
    /// it -- an empty denominator is a visible zero, not a crash.
    #[serde(default)]
    workspace_members: Vec<String>,
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

/// Everything the crate tier reads out of one `cargo metadata` run, beyond
/// the plain dependency edges [`CrateDependency`] always carried: which
/// identity strings actually name a real crate at all (finding 2's family:
/// an anchor that names none of them owns nothing, silently, unless this is
/// consulted), which of those are members of *this* workspace rather than
/// a resolved third-party dependency (finding 3's denominator), and which
/// identity string two *different* real crates both answer to (finding 6:
/// two workspace crates sharing a library name collapse to one identity,
/// and a dependency graph edge naming that identity cannot be trusted to
/// mean either crate in particular).
#[derive(Debug, Clone, Default)]
pub struct WorkspaceGraph {
    /// Every *normal* (runtime) dependency edge -- code that ships.
    pub edges: Vec<CrateDependency>,
    /// Every dependency edge that exists **only** as a `dev-dependencies`
    /// or `build-dependencies` entry (finding 4). Deliberately excluded
    /// from `edges` and from crate-tier enforcement (§5.3 is about code
    /// that ships), but kept here rather than dropped outright, so a
    /// caller can disclose them instead of printing a sentence ("no crate
    /// here depends on another...") that a real dev-dependency crossing
    /// makes false.
    pub dev_or_build_edges: Vec<CrateDependency>,
    /// Every identity string [`crate_identity_name`] produces for some
    /// package `cargo metadata` knows about -- workspace member or
    /// resolved dependency alike. An anchor whose crate segment is not in
    /// here names nothing real (finding 2a).
    pub all_crate_identities: BTreeSet<String>,
    /// The identity strings of packages that are members of *this*
    /// workspace (`cargo metadata`'s own `workspace_members`) -- the
    /// denominator finding 3 asks for.
    pub workspace_crate_identities: BTreeSet<String>,
    /// An identity string that two or more *different* packages both
    /// produce (finding 6). A dependency edge naming it is excluded from
    /// classification entirely -- see [`check_architecture`] -- because
    /// there is no way to tell, from the identity alone, which of the
    /// colliding crates it actually refers to, and guessing is how the
    /// review's ws8 fixture got told a dependency existed that did not.
    pub ambiguous_identities: BTreeSet<String>,
}

/// Runs `cargo metadata --format-version=1` in `crate_dir` and returns the
/// real, resolved dependency graph plus the identity facts above — not
/// merely what a `Cargo.toml` declares (a `resolve` walk reflects which
/// optional dependencies actually got activated; a plain read of
/// `dependencies:` would not). The actual classification (identity
/// resolution, normal/dev/build split, ambiguity detection) is
/// [`graph_from_metadata`], a pure function kept separate so a test can
/// drive it directly against a crafted `Metadata` value with no
/// `cargo metadata` process in the loop.
pub fn crate_dependency_graph(crate_dir: &Path) -> Result<WorkspaceGraph, MetadataError> {
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
/// map and so dropping every dependency that originates from it -- plus
/// the identity facts [`WorkspaceGraph`] carries.
fn graph_from_metadata(meta: &Metadata) -> WorkspaceGraph {
    let mut name_by_id: HashMap<&str, String> = HashMap::new();
    let mut ids_by_identity: HashMap<String, Vec<&str>> = HashMap::new();
    for pkg in &meta.packages {
        let identity = crate_identity_name(pkg);
        ids_by_identity
            .entry(identity.clone())
            .or_default()
            .push(pkg.id.as_str());
        name_by_id.insert(pkg.id.as_str(), identity);
    }

    let ambiguous_identities: BTreeSet<String> = ids_by_identity
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(name, _)| name)
        .collect();
    let all_crate_identities: BTreeSet<String> = name_by_id.values().cloned().collect();
    let workspace_crate_identities: BTreeSet<String> = meta
        .workspace_members
        .iter()
        .filter_map(|id| name_by_id.get(id.as_str()).cloned())
        .collect();

    let mut edges = Vec::new();
    let mut dev_or_build_edges = Vec::new();
    if let Some(resolve) = &meta.resolve {
        for node in &resolve.nodes {
            let Some(from) = name_by_id.get(node.id.as_str()) else {
                continue;
            };
            for dep in &node.deps {
                let Some(to) = name_by_id.get(dep.pkg.as_str()) else {
                    continue;
                };
                let is_normal =
                    dep.dep_kinds.is_empty() || dep.dep_kinds.iter().any(|k| k.kind.is_none());
                let d = CrateDependency {
                    from: from.clone(),
                    to: to.clone(),
                };
                if is_normal {
                    edges.push(d);
                } else {
                    dev_or_build_edges.push(d);
                }
            }
        }
    }
    WorkspaceGraph {
        edges,
        dev_or_build_edges,
        all_crate_identities,
        workspace_crate_identities,
        ambiguous_identities,
    }
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
    /// Every (qualified component path, crate name its anchor's first
    /// segment names) pair, **in document walk order and not deduped** --
    /// unlike `crate_owner`, which keeps only the first claim to each
    /// crate name. This is what lets [`check_architecture`] tell "this
    /// component owns nothing because its anchor names no real crate"
    /// (finding 2a) apart from "this component owns nothing because
    /// another declared component already claimed its crate first"
    /// (finding 2b/2c) -- `crate_owner` alone cannot distinguish either
    /// case from ordinary success, since both leave the crate simply
    /// absent or pointed elsewhere in that map.
    pub component_crate_claims: Vec<(String, String)>,
    leaf_index: HashMap<String, Vec<String>>,
    all_qualified: HashSet<String>,
}

/// What an edge/deny endpoint token resolves to. Split out from a plain
/// `Option<String>` so [`check_architecture`] can tell "resolves to
/// nothing at all" (finding 2d: a typo naming no declared component)
/// apart from "ambiguous" (already `E0206` from the document tier) --
/// collapsing both into `None`, as the old `resolve` did, is exactly how
/// a dangling `deny: ["b -> nosuch"]` went unreported: `matching_deny`
/// simply found no match and called that a clean run.
enum TokenResolution {
    Wildcard,
    Resolved(String),
    Ambiguous,
    NotFound,
}

impl ComponentIndex {
    pub fn build(doc: &Document) -> Self {
        let mut crate_owner = HashMap::new();
        let mut component_crate_claims = Vec::new();
        let mut leaf_index: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_qualified = HashSet::new();

        fn walk(
            qualified: &str,
            leaf: &str,
            comp: &Component,
            crate_owner: &mut HashMap<String, String>,
            component_crate_claims: &mut Vec<(String, String)>,
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
            component_crate_claims.push((qualified.to_string(), crate_name.clone()));
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
                    component_crate_claims,
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
                &mut component_crate_claims,
                &mut leaf_index,
                &mut all_qualified,
            );
        }

        Self {
            crate_owner,
            component_crate_claims,
            leaf_index,
            all_qualified,
        }
    }

    /// Classifies one edge/deny endpoint token -- see [`TokenResolution`].
    /// Mirrors `ply_core::check`'s own token resolution (§5.1a rule 6) —
    /// duplicated here in miniature rather than shared, so this module
    /// stays self-contained.
    fn classify(&self, token: &str) -> TokenResolution {
        if token == "*" {
            return TokenResolution::Wildcard;
        }
        if token.contains('.') {
            return if self.all_qualified.contains(token) {
                TokenResolution::Resolved(token.to_string())
            } else {
                TokenResolution::NotFound
            };
        }
        match self.leaf_index.get(token) {
            Some(paths) if paths.len() == 1 => TokenResolution::Resolved(paths[0].clone()),
            Some(paths) if paths.len() > 1 => TokenResolution::Ambiguous,
            _ => TokenResolution::NotFound,
        }
    }

    /// Resolves one edge/deny endpoint token to the single qualified
    /// component path it must mean — `None` for `*`, for a bare name that
    /// is ambiguous (already reported as `E0206` by the document tier) or
    /// resolves to nothing (`A0413`, see [`check_architecture`]), or for a
    /// dotted path naming no real component.
    fn resolve(&self, token: &str) -> Option<String> {
        match self.classify(token) {
            TokenResolution::Resolved(path) => Some(path),
            TokenResolution::Wildcard | TokenResolution::Ambiguous | TokenResolution::NotFound => {
                None
            }
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
#[derive(Debug, Clone, Default)]
pub struct ArchTally {
    pub cross_component_pairs: usize,
    pub violations: usize,
    pub deny_violations: usize,
    /// Finding 4: real crossings that exist **only** as a `dev-dependencies`
    /// or `build-dependencies` entry -- excluded from `violations` above
    /// (this tier enforces boundaries on code that ships), but counted
    /// here so the coverage sentence can say so instead of reporting "no
    /// crate here depends on another" when one genuinely does, just not
    /// at runtime.
    pub dev_or_build_cross_component_pairs: usize,
    /// Finding 3's denominator: how many crates in this workspace `cargo
    /// metadata` reports, and how many of those are claimed by some
    /// declared component's anchor (regardless of whether that claim
    /// itself is valid -- `undeclared_crates` below is the complement of
    /// this count, not of "components with a working anchor").
    pub workspace_crate_count: usize,
    pub declared_crate_count: usize,
    /// The workspace crates in `workspace_crate_count` that no declared
    /// component's anchor names at all -- sorted, so the report is
    /// deterministic. `tests/e2e` in this repo's own `ply.yaml` is exactly
    /// this today (finding 3).
    pub undeclared_crates: Vec<String>,
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

/// Finding 2a: a component's anchor names a crate `cargo metadata` has
/// never heard of. Left unreported, this silently owns nothing -- exactly
/// the "renamed function must break CI" rule §5.2 already applies to a fn
/// claim, missing here for a component's own anchor.
fn a0410_message(qualified: &str, crate_name: &str) -> String {
    format!(
        "component `{qualified}` is anchored at `{crate_name}`, but Ply cannot find a crate \
         called that anywhere in this workspace's real dependency graph. This component \
         therefore owns no crate at all, and every edge or `deny:` rule written for `{qualified}` \
         is silently inert. Check the anchor for a typo, or for a crate that was renamed after \
         this document was written."
    )
}

/// Finding 2b/2c: two declared components' anchors resolve to the same
/// crate identity -- a literal duplicate anchor, or a component anchored
/// at a module inside a crate another component already claims whole (the
/// crate tier only ever sees an anchor's first path segment). Whichever
/// was declared first in the document owns the crate; this one owns
/// nothing, silently, unless reported.
fn a0411_message(qualified: &str, crate_name: &str, first_owner: &str) -> String {
    format!(
        "component `{qualified}` is anchored at `{crate_name}`, but component `{first_owner}` is \
         declared first and already claims that same crate. Ply keeps only the first claim, so \
         `{qualified}` owns no crate at all here: every edge or `deny:` rule written for it never \
         fires. Two components cannot own the same crate — anchor one of them elsewhere, or \
         merge the two."
    )
}

/// Finding 6: two *different* crates in this workspace build a library by
/// the same name, so the identity a dependency graph edge carries cannot
/// be trusted to mean either one in particular. Reported once per
/// component anchored at the colliding name, rather than guessing which
/// real crate it means — a wrong guess is what let the review's ws8
/// fixture see a dependency that did not exist.
fn a0412_message(qualified: &str, crate_name: &str) -> String {
    format!(
        "component `{qualified}` is anchored at `{crate_name}`, but two different crates in this \
         workspace both build a library called `{crate_name}`. Ply cannot tell which one a real \
         dependency on `{crate_name}` refers to, so no crossing involving `{qualified}` is \
         checked against the crate dependency graph until the name collision is resolved — \
         rename one of the two library targets so each has its own identity."
    )
}

/// Finding 2d: an edge or `deny:` line names a component that does not
/// exist at all -- not ambiguous (that is `E0206`, from the document
/// tier), just absent. Left unreported, the line is silently inert: a
/// `deny: ["b -> nosuch"]` never matches anything, and the run reads as
/// clean.
fn a0413_message(construct_str: &str, token: &str) -> String {
    let construct_str = construct_str.trim();
    format!(
        "\"{construct_str}\" names {token:?}, but no component declared under `components:` in \
         this document is called that (and it is not `*`). This line can never match anything, \
         so it is silently inert. Check the spelling against the component names this document \
         declares."
    )
}

/// §5.3's crate tier, run over an already-fetched dependency graph (kept
/// separate from [`crate_dependency_graph`] so this half — the actual
/// classification rule — is a pure function a test can drive directly,
/// with no `cargo metadata` process in the loop).
pub fn check_architecture(doc: &Document, graph: &WorkspaceGraph) -> (Vec<ArchFinding>, ArchTally) {
    let index = ComponentIndex::build(doc);
    let mut findings = Vec::new();
    let mut tally = ArchTally::default();

    // Finding 2's family (2a/2b/2c) and finding 6: diagnose every
    // component whose anchor claims a crate incorrectly, before
    // classifying a single real dependency through it. Order matters:
    // ambiguity is checked first, because an ambiguous identity existing
    // at all (finding 6) is a stronger and more specific fact than merely
    // "some component already claimed it first" (2b/2c).
    let mut first_claim: HashMap<&str, &str> = HashMap::new();
    for (qualified, crate_name) in &index.component_crate_claims {
        if graph.ambiguous_identities.contains(crate_name) {
            findings.push(ArchFinding {
                code: "A0412",
                message: a0412_message(qualified, crate_name),
                node_id: qualified.clone(),
            });
            continue;
        }
        if !graph.all_crate_identities.contains(crate_name) {
            findings.push(ArchFinding {
                code: "A0410",
                message: a0410_message(qualified, crate_name),
                node_id: qualified.clone(),
            });
            continue;
        }
        match first_claim.get(crate_name.as_str()) {
            None => {
                first_claim.insert(crate_name.as_str(), qualified.as_str());
            }
            // §5.3: "containment implies permission" -- a component and its
            // own strict ancestor/descendant sharing the same crate anchor
            // is redundant (the same way an explicit edge between them is
            // `W0409`), not the silent-takeover finding 2b/2c describe.
            // The defect there is two *unrelated* components colliding;
            // nesting already grants full mutual permission regardless of
            // which of the two "owns" the crate in `crate_owner`, so
            // flagging it here would only be noise on a legitimate pattern
            // (a nested component re-describing its own parent's crate).
            Some(owner)
                if !ComponentIndex::is_strict_ancestor(owner, qualified)
                    && !ComponentIndex::is_strict_ancestor(qualified, owner) =>
            {
                findings.push(ArchFinding {
                    code: "A0411",
                    message: a0411_message(qualified, crate_name, owner),
                    node_id: qualified.clone(),
                });
            }
            Some(_) => {}
        }
    }

    // Finding 2d: an edge or `deny:` endpoint (or `except` name) that
    // resolves to nothing at all — a typo, not an ambiguity (`E0206`
    // already covers that from the document tier). Externals share the
    // component-reference namespace (docs/plans/external-elements.md §3)
    // but this index knows nothing about them, so they are exempted here
    // rather than misreported as nonexistent.
    let externals: HashSet<&str> = doc.externals.keys().map(String::as_str).collect();
    let mut check_token = |construct_str: &str, token: &str, node_id: String| {
        if token == "*" || externals.contains(token) {
            return;
        }
        if matches!(index.classify(token), TokenResolution::NotFound) {
            findings.push(ArchFinding {
                code: "A0413",
                message: a0413_message(construct_str, token),
                node_id,
            });
        }
    };
    for (i, e) in doc.edges.iter().enumerate() {
        if let Ok(edge) = parse_edge(e) {
            check_token(e, &edge.from, format!("edges[{i}]"));
            check_token(e, &edge.to, format!("edges[{i}]"));
        }
    }
    for (i, d) in doc.deny.iter().enumerate() {
        if let Ok(deny) = parse_deny(d) {
            check_token(d, &deny.from, format!("deny[{i}]"));
            check_token(d, &deny.to, format!("deny[{i}]"));
            for except in &deny.except {
                check_token(d, except, format!("deny[{i}]"));
            }
        }
    }

    // The crate tier proper: every *normal* dependency, classified.
    // Finding 6: a pair touching an ambiguous identity is excluded here
    // (already reported above) rather than attributed to whichever
    // component happened to claim that name — attributing it is the
    // false A0401/A0405 the review found.
    for dep in &graph.edges {
        if graph.ambiguous_identities.contains(&dep.from)
            || graph.ambiguous_identities.contains(&dep.to)
        {
            continue;
        }
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

    // Finding 4: dev/build-only crossings are never enforced (§5.3 is
    // about code that ships), but disclosed — counted here rather than
    // silently dropped, so the coverage sentence never claims "no crate
    // here depends on another" when one does, just not at runtime.
    for dep in &graph.dev_or_build_edges {
        if graph.ambiguous_identities.contains(&dep.from)
            || graph.ambiguous_identities.contains(&dep.to)
        {
            continue;
        }
        let Some(from_comp) = index.crate_owner.get(&dep.from) else {
            continue;
        };
        let Some(to_comp) = index.crate_owner.get(&dep.to) else {
            continue;
        };
        if from_comp == to_comp {
            continue;
        }
        tally.dev_or_build_cross_component_pairs += 1;
    }

    // Finding 3: the coverage line's denominator.
    tally.workspace_crate_count = graph.workspace_crate_identities.len();
    let mut undeclared: Vec<String> = graph
        .workspace_crate_identities
        .iter()
        .filter(|c| !index.crate_owner.contains_key(c.as_str()))
        .cloned()
        .collect();
    undeclared.sort();
    tally.declared_crate_count = tally.workspace_crate_count - undeclared.len();
    tally.undeclared_crates = undeclared;

    (findings, tally)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_document;

    fn doc(yaml: &str) -> Document {
        parse_document(yaml).unwrap()
    }

    /// A `WorkspaceGraph` for a test that only cares about the plain
    /// classification rule (containment/edges/deny) -- every crate any
    /// edge mentions is treated as a real, known, unambiguous workspace
    /// crate, so a test written before `WorkspaceGraph` existed does not
    /// have to also spell out crate existence separately.
    fn graph_of(edges: Vec<CrateDependency>) -> WorkspaceGraph {
        let mut crates = BTreeSet::new();
        for e in &edges {
            crates.insert(e.from.clone());
            crates.insert(e.to.clone());
        }
        WorkspaceGraph {
            edges,
            dev_or_build_edges: vec![],
            all_crate_identities: crates.clone(),
            workspace_crate_identities: crates,
            ambiguous_identities: BTreeSet::new(),
        }
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
            workspace_members: vec!["top".into(), "core".into()],
        };
        let graph = graph_from_metadata(&meta);
        assert_eq!(
            graph.edges,
            vec![CrateDependency {
                from: "ply_cli".into(),
                to: "ply_core".into(),
            }],
            "the bin-only crate's own dependency must not be dropped: {graph:?}"
        );
    }

    /// Finding 7's mutation table, row 5: a dependency node whose `deps`
    /// list names a package id `cargo metadata` never described in
    /// `packages` (should not happen in real output, but this is exactly
    /// the shape a defensive `continue` guards against) must be **excluded**
    /// from the graph, never fall back to its raw, un-normalised cargo id --
    /// a graph edge with a garbage identity can never match a real crate
    /// owner, so it would silently vanish from classification either way,
    /// but a raw id leaking into `all_crate_identities`/`edges` is exactly
    /// the kind of silent widening this suite is supposed to catch.
    #[test]
    fn graph_from_metadata_excludes_a_dependency_on_an_unknown_package_id() {
        let meta = Metadata {
            packages: vec![MetaPackage {
                id: "top".into(),
                name: "top".into(),
                targets: vec![meta_target("lib", "top")],
            }],
            resolve: Some(MetaResolve {
                nodes: vec![MetaNode {
                    id: "top".into(),
                    deps: vec![MetaDep {
                        pkg: "not-in-packages-at-all".into(),
                        dep_kinds: vec![],
                    }],
                }],
            }),
            workspace_members: vec!["top".into()],
        };
        let graph = graph_from_metadata(&meta);
        assert!(
            graph.edges.is_empty(),
            "an edge to an unresolvable package id must be dropped, not kept with a raw id: \
             {graph:?}"
        );
        assert!(
            !graph
                .all_crate_identities
                .contains("not-in-packages-at-all"),
            "the raw cargo id must never leak into the known-identities set: {graph:?}"
        );
    }

    /// Finding 4: a dependency that exists **only** as a `dev-dependencies`
    /// or `build-dependencies` entry is excluded from `edges` (this tier
    /// enforces boundaries on code that ships) but must still show up
    /// somewhere -- `dev_or_build_edges` -- rather than being dropped as
    /// though it never existed. Deleting this classification's `if
    /// is_normal` branch entirely (finding 7's row 1) collapses this
    /// distinction and both dev/build dependencies below would wrongly
    /// enter `edges`.
    #[test]
    fn graph_from_metadata_separates_dev_and_build_only_dependencies() {
        let meta = Metadata {
            packages: vec![
                MetaPackage {
                    id: "x".into(),
                    name: "x".into(),
                    targets: vec![meta_target("lib", "x")],
                },
                MetaPackage {
                    id: "y".into(),
                    name: "y".into(),
                    targets: vec![meta_target("lib", "y")],
                },
                MetaPackage {
                    id: "z".into(),
                    name: "z".into(),
                    targets: vec![meta_target("lib", "z")],
                },
            ],
            resolve: Some(MetaResolve {
                nodes: vec![
                    MetaNode {
                        id: "x".into(),
                        deps: vec![
                            MetaDep {
                                pkg: "y".into(),
                                dep_kinds: vec![MetaDepKind {
                                    kind: Some("dev".into()),
                                }],
                            },
                            MetaDep {
                                pkg: "z".into(),
                                dep_kinds: vec![MetaDepKind {
                                    kind: Some("build".into()),
                                }],
                            },
                        ],
                    },
                    MetaNode {
                        id: "y".into(),
                        deps: vec![],
                    },
                    MetaNode {
                        id: "z".into(),
                        deps: vec![],
                    },
                ],
            }),
            workspace_members: vec!["x".into(), "y".into(), "z".into()],
        };
        let graph = graph_from_metadata(&meta);
        assert!(graph.edges.is_empty(), "{graph:?}");
        assert_eq!(
            graph.dev_or_build_edges,
            vec![
                CrateDependency {
                    from: "x".into(),
                    to: "y".into(),
                },
                CrateDependency {
                    from: "x".into(),
                    to: "z".into(),
                },
            ],
            "{graph:?}"
        );
    }

    /// Finding 6: two different packages whose `crate_identity_name` comes
    /// out the same (two `[lib] name = "shared"` targets) must be reported
    /// as ambiguous, not silently collapsed to one identity.
    #[test]
    fn graph_from_metadata_flags_two_packages_sharing_one_crate_identity() {
        let meta = Metadata {
            packages: vec![
                MetaPackage {
                    id: "left".into(),
                    name: "left".into(),
                    targets: vec![meta_target("lib", "shared")],
                },
                MetaPackage {
                    id: "right".into(),
                    name: "right".into(),
                    targets: vec![meta_target("lib", "shared")],
                },
                MetaPackage {
                    id: "onlyone".into(),
                    name: "onlyone".into(),
                    targets: vec![meta_target("lib", "onlyone")],
                },
            ],
            resolve: Some(MetaResolve { nodes: vec![] }),
            workspace_members: vec!["left".into(), "right".into(), "onlyone".into()],
        };
        let graph = graph_from_metadata(&meta);
        assert_eq!(
            graph.ambiguous_identities,
            BTreeSet::from(["shared".to_string()]),
            "{graph:?}"
        );
    }

    /// Finding 3's denominator: `workspace_crate_identities` is only the
    /// packages `cargo metadata` reports as workspace members -- a
    /// resolved third-party dependency (present in `packages`, absent from
    /// `workspace_members`) must not inflate it.
    #[test]
    fn workspace_crate_identities_excludes_a_resolved_third_party_dependency() {
        let meta = Metadata {
            packages: vec![
                MetaPackage {
                    id: "top".into(),
                    name: "top".into(),
                    targets: vec![meta_target("lib", "top")],
                },
                MetaPackage {
                    id: "serde".into(),
                    name: "serde".into(),
                    targets: vec![meta_target("lib", "serde")],
                },
            ],
            resolve: Some(MetaResolve {
                nodes: vec![
                    MetaNode {
                        id: "top".into(),
                        deps: vec![MetaDep {
                            pkg: "serde".into(),
                            dep_kinds: vec![],
                        }],
                    },
                    MetaNode {
                        id: "serde".into(),
                        deps: vec![],
                    },
                ],
            }),
            workspace_members: vec!["top".into()],
        };
        let graph = graph_from_metadata(&meta);
        assert_eq!(
            graph.workspace_crate_identities,
            BTreeSet::from(["top".to_string()]),
            "serde is a real dependency but not a workspace member: {graph:?}"
        );
        assert!(graph.all_crate_identities.contains("serde"));
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
        let mut edges = Vec::new();
        for &from in &crates {
            for &to in &crates {
                if from != to {
                    edges.push(CrateDependency {
                        from: from.to_string(),
                        to: to.to_string(),
                    });
                }
            }
        }
        let graph = graph_of(edges);

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

        for dep in &graph.edges {
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
        let graph = graph_of(vec![CrateDependency {
            from: "crate_b".into(),
            to: "crate_a".into(),
        }]);
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
        let graph = graph_of(vec![CrateDependency {
            from: "crate_b".into(),
            to: "crate_a".into(),
        }]);
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
        let graph = graph_of(vec![CrateDependency {
            from: "crate_a".into(),
            to: "crate_c".into(),
        }]);
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
        let graph = graph_of(vec![CrateDependency {
            from: "crate_c".into(),
            to: "crate_a".into(),
        }]);
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
        let graph = graph_of(vec![CrateDependency {
            from: "crate_b".into(),
            to: "crate_a".into(),
        }]);
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
        let graph = graph_of(vec![
            CrateDependency {
                from: "crate_b".into(),
                to: "crate_a".into(),
            },
            CrateDependency {
                from: "crate_m".into(),
                to: "crate_a".into(),
            },
        ]);
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
        let graph = graph_of(vec![CrateDependency {
            from: "crate_a".into(),
            to: "serde".into(),
        }]);
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.cross_component_pairs, 0);
        assert!(findings.is_empty(), "{findings:?}");
    }

    // ---- Review findings 2, 3, 4, 6 (docs/review-architecture-tier.md) ----

    /// Finding 2a: a component's anchor names a crate that does not exist
    /// anywhere in the real dependency graph -- a rename, or a plain typo.
    /// Before the fix this component silently owned nothing at all; now it
    /// is `A0410`, naming the component and the crate it cannot find.
    #[test]
    fn an_anchor_naming_a_nonexistent_crate_is_a0410() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_typo
  b:
    anchor: crate_b
"#);
        let graph = graph_of(vec![]);
        // `crate_b` is real (it's a component's own anchor) even though no
        // edge mentions it -- construct the graph's identity set directly
        // rather than through `graph_of`, which only knows about crates an
        // edge names.
        let graph = WorkspaceGraph {
            all_crate_identities: BTreeSet::from(["crate_b".to_string()]),
            workspace_crate_identities: BTreeSet::from(["crate_b".to_string()]),
            ..graph
        };
        let (findings, _tally) = check_architecture(&document, &graph);
        let f = findings
            .iter()
            .find(|f| f.code == "A0410")
            .unwrap_or_else(|| panic!("{findings:?}"));
        assert!(f.message.contains("`a`"), "{}", f.message);
        assert!(f.message.contains("crate_typo"), "{}", f.message);
        assert_eq!(f.node_id, "a");
    }

    /// Finding 2b: two components literally anchored at the same crate.
    /// Ply keeps the first declaration (§5.3's own documented rule) but
    /// must now *say* the second owns nothing, rather than leaving its
    /// `deny:` rule silently inert.
    #[test]
    fn two_components_anchored_at_the_same_crate_is_a0411_on_the_second() {
        let document = doc(r#"
ply: 1
components:
  a_public:
    anchor: crate_shared
  a_internal:
    anchor: crate_shared
  c:
    anchor: crate_c
deny:
  - "c -> a_internal"
"#);
        let graph = WorkspaceGraph {
            all_crate_identities: BTreeSet::from([
                "crate_shared".to_string(),
                "crate_c".to_string(),
            ]),
            workspace_crate_identities: BTreeSet::from([
                "crate_shared".to_string(),
                "crate_c".to_string(),
            ]),
            ..graph_of(vec![CrateDependency {
                from: "crate_c".into(),
                to: "crate_shared".into(),
            }])
        };
        let (findings, _tally) = check_architecture(&document, &graph);
        let f = findings
            .iter()
            .find(|f| f.code == "A0411")
            .unwrap_or_else(|| panic!("{findings:?}"));
        assert_eq!(
            f.node_id, "a_internal",
            "the *second* declaration owns nothing"
        );
        assert!(f.message.contains("a_public"), "{}", f.message);
        assert!(
            !findings.iter().any(|other| other.node_id == "a_public"),
            "the first declaration is not itself flagged: {findings:?}"
        );
        // The ban attached to the shadowed component must not silently
        // apply to the crate the *first* component actually owns -- since
        // `a_internal` owns nothing, `c -> a_internal` never matches the
        // real `c -> crate_shared` dependency, so no A0405 fires either.
        assert!(
            !findings.iter().any(|f| f.code == "A0405"),
            "a ban on a shadowed component must not fire: {findings:?}"
        );
    }

    /// Finding 2c: a component anchored at a *module* inside a crate that
    /// another component already claims whole. The crate tier only ever
    /// reads an anchor's first `::`-segment, so `crate_a::ratemod` and
    /// `crate_a` collide on the same crate identity exactly the way two
    /// literal duplicates do (finding 2b) -- same code path, different
    /// surface shape, so it gets its own fixture per §9.
    #[test]
    fn a_module_anchored_component_collides_with_a_crate_anchored_one() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  a_ratemod:
    anchor: crate_a::ratemod
"#);
        let graph = graph_of(vec![]);
        let graph = WorkspaceGraph {
            all_crate_identities: BTreeSet::from(["crate_a".to_string()]),
            workspace_crate_identities: BTreeSet::from(["crate_a".to_string()]),
            ..graph
        };
        let (findings, _tally) = check_architecture(&document, &graph);
        let f = findings
            .iter()
            .find(|f| f.code == "A0411")
            .unwrap_or_else(|| panic!("{findings:?}"));
        assert_eq!(f.node_id, "a_ratemod");
        assert!(f.message.contains("`a`"), "{}", f.message);
    }

    /// Finding 7's mutation-table row 4: the crate name a component's
    /// anchor claims is its *first* `::`-segment, never its last -- an
    /// anchor of `outer::inner` claims crate `outer`, not `inner`.
    #[test]
    fn component_crate_claim_uses_the_first_anchor_segment_not_the_last() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: outer::inner::deepest
"#);
        let index = ComponentIndex::build(&document);
        assert_eq!(
            index.component_crate_claims,
            vec![("a".to_string(), "outer".to_string())]
        );
    }

    /// Finding 7's mutation-table row 3: `crate_owner` -- the map the
    /// crate tier's own classification loop actually reads to attribute a
    /// real dependency to a component -- must keep the *first* declared
    /// component for a crate name, never the last, exactly as its own doc
    /// comment states. This is a separate fact from `A0411` firing on the
    /// second declaration (that pins the diagnostic; this pins the map the
    /// diagnostic is *about*), and a mutation that swapped which one wins
    /// would leave `A0411` firing on the *wrong* component while real
    /// dependencies got silently misattributed.
    #[test]
    fn crate_owner_keeps_the_first_declared_component_not_the_last() {
        let document = doc(r#"
ply: 1
components:
  a_public:
    anchor: crate_shared
  a_internal:
    anchor: crate_shared
"#);
        let index = ComponentIndex::build(&document);
        assert_eq!(
            index.crate_owner.get("crate_shared"),
            Some(&"a_public".to_string())
        );
    }

    /// Finding 2d: an edge naming a component that does not exist at all
    /// (as opposed to ambiguously -- that's `E0206`, from the document
    /// tier) is silently inert today: `matching_deny`/`permitted` simply
    /// find no match, and the run reads as clean. `A0413` says so instead.
    #[test]
    fn an_edge_naming_a_nonexistent_component_is_a0413() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
edges:
  - "b -> nosuch"
"#);
        let graph = graph_of(vec![]);
        let (findings, _tally) = check_architecture(&document, &graph);
        let f = findings
            .iter()
            .find(|f| f.code == "A0413")
            .unwrap_or_else(|| panic!("{findings:?}"));
        assert!(f.message.contains("nosuch"), "{}", f.message);
        assert_eq!(f.node_id, "edges[0]");
    }

    /// The `deny:` sibling of the same defect: a typo in a ban is a ban
    /// that never fires, and until now nothing said so.
    #[test]
    fn a_deny_rule_naming_a_nonexistent_component_is_a0413() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
deny:
  - "b -> nosuch"
"#);
        let graph = graph_of(vec![]);
        let (findings, _tally) = check_architecture(&document, &graph);
        let f = findings
            .iter()
            .find(|f| f.code == "A0413")
            .unwrap_or_else(|| panic!("{findings:?}"));
        assert!(f.message.contains("nosuch"), "{}", f.message);
        assert_eq!(f.node_id, "deny[0]");
    }

    /// `*` is never flagged as a nonexistent component (it is the
    /// wildcard, §5's own micro-syntax), and neither is a declared
    /// external -- externals share the reference namespace but are not
    /// components, and are somebody else's rule (`E0207`) to enforce.
    #[test]
    fn wildcard_and_external_edge_endpoints_are_never_a0413() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
externals:
  customer:
    note: "a paying user"
deny:
  - "* -> a"
"#);
        let graph = graph_of(vec![]);
        let (findings, _tally) = check_architecture(&document, &graph);
        assert!(!findings.iter().any(|f| f.code == "A0413"), "{findings:?}");
    }

    /// Finding 6: two different crates sharing a library name must never
    /// be silently attributed to whichever component happened to claim
    /// that name -- the review's own ws8 reproduction. `leftside` really
    /// depends on nothing here (its only real dependency is on the
    /// *right*-hand crate that shares its identity), so no `A0401`/`A0405`
    /// may fire; instead `A0412` reports the ambiguity itself.
    #[test]
    fn an_ambiguous_crate_identity_is_a0412_and_never_silently_attributed() {
        let document = doc(r#"
ply: 1
components:
  leftside:
    anchor: shared
  user_r:
    anchor: crate_user_r
deny:
  - "user_r -> leftside"
"#);
        let graph = WorkspaceGraph {
            edges: vec![CrateDependency {
                from: "crate_user_r".into(),
                to: "shared".into(),
            }],
            dev_or_build_edges: vec![],
            all_crate_identities: BTreeSet::from([
                "shared".to_string(),
                "crate_user_r".to_string(),
            ]),
            workspace_crate_identities: BTreeSet::from([
                "shared".to_string(),
                "crate_user_r".to_string(),
            ]),
            ambiguous_identities: BTreeSet::from(["shared".to_string()]),
        };
        let (findings, tally) = check_architecture(&document, &graph);
        assert!(
            !findings
                .iter()
                .any(|f| f.code == "A0401" || f.code == "A0405"),
            "an ambiguous identity must never be silently attributed to either real crate: \
             {findings:?}"
        );
        assert_eq!(
            tally.cross_component_pairs, 0,
            "the pair must not even be counted as classified: {tally:?}"
        );
        let f = findings
            .iter()
            .find(|f| f.code == "A0412")
            .unwrap_or_else(|| panic!("{findings:?}"));
        assert_eq!(f.node_id, "leftside");
        assert!(f.message.contains("shared"), "{}", f.message);
    }

    /// Finding 7's mutation-table row 2: a *dashed* (`~>`, data-flow) edge
    /// must never permit a real crate dependency the way a solid `->`
    /// edge does -- every solid arrow is a checked claim and every dashed
    /// one is declared-not-checked (§5.3), so treating them the same here
    /// would let a `~>` edge silently license a real crossing.
    #[test]
    fn a_dashed_flow_edge_does_not_permit_a_real_dependency() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
edges:
  - "b ~> a : SomeType"
"#);
        let graph = graph_of(vec![CrateDependency {
            from: "crate_b".into(),
            to: "crate_a".into(),
        }]);
        let (findings, tally) = check_architecture(&document, &graph);
        assert_eq!(
            tally.violations, 1,
            "a dashed edge must not permit the real dependency: {findings:?}"
        );
        assert!(findings.iter().any(|f| f.code == "A0401"), "{findings:?}");
    }

    /// Finding 4: a dependency that exists only as a `dev-dependencies` or
    /// `build-dependencies` entry is not enforced (no `A0401`), but it must
    /// be counted rather than vanish -- the coverage sentence built from
    /// this tally is what stops "no crate here depends on another" from
    /// being printed when one, in fact, does (just not at runtime).
    #[test]
    fn a_dev_dependency_crossing_is_counted_but_not_enforced() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
  b:
    anchor: crate_b
"#);
        let graph = WorkspaceGraph {
            edges: vec![],
            dev_or_build_edges: vec![CrateDependency {
                from: "crate_b".into(),
                to: "crate_a".into(),
            }],
            all_crate_identities: BTreeSet::from(["crate_a".to_string(), "crate_b".to_string()]),
            workspace_crate_identities: BTreeSet::from([
                "crate_a".to_string(),
                "crate_b".to_string(),
            ]),
            ambiguous_identities: BTreeSet::new(),
        };
        let (findings, tally) = check_architecture(&document, &graph);
        assert!(
            findings.is_empty(),
            "a dev-only crossing must not be enforced: {findings:?}"
        );
        assert_eq!(tally.cross_component_pairs, 0);
        assert_eq!(tally.dev_or_build_cross_component_pairs, 1);
    }

    /// Finding 3: the coverage denominator -- a workspace crate no
    /// component's anchor names at all is counted and named, the way
    /// `tests/e2e` is invisible in this repo's own `ply.yaml` today.
    #[test]
    fn undeclared_workspace_crates_are_counted_and_named() {
        let document = doc(r#"
ply: 1
components:
  a:
    anchor: crate_a
"#);
        let graph = WorkspaceGraph {
            edges: vec![],
            dev_or_build_edges: vec![],
            all_crate_identities: BTreeSet::from([
                "crate_a".to_string(),
                "crate_extra".to_string(),
            ]),
            workspace_crate_identities: BTreeSet::from([
                "crate_a".to_string(),
                "crate_extra".to_string(),
            ]),
            ambiguous_identities: BTreeSet::new(),
        };
        let (_findings, tally) = check_architecture(&document, &graph);
        assert_eq!(tally.workspace_crate_count, 2);
        assert_eq!(tally.declared_crate_count, 1);
        assert_eq!(tally.undeclared_crates, vec!["crate_extra".to_string()]);
    }
}
