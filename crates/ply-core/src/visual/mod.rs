//! Editor-neutral visual snapshots and their atomic on-disk publication.
//!
//! This module is the boundary clients consume. It combines the declared
//! `ply.yaml` picture with a completed, already-classified result envelope;
//! clients never read or interpret `ply.yaml` or `ply.lock` themselves.

pub mod layout;
pub mod state_shapes;
pub mod svg;
pub mod transcript;

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::diag::{Diagnostic, Envelope, Node, Span};
use crate::model::Document;

pub const VISUAL_PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_RETAINED_RUNS: usize = 20;

#[derive(Debug)]
pub enum VisualEnvelopeError {
    Json(serde_json::Error),
    Io(std::io::Error),
    UnsupportedVersion(u64),
    Invalid(String),
    Render(svg::RenderError),
}

impl fmt::Display for VisualEnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "visual JSON is invalid: {error}"),
            Self::Io(error) => write!(f, "visual publication failed: {error}"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "visual protocol version {version} is not supported; this build supports version {VISUAL_PROTOCOL_VERSION}"
            ),
            Self::Invalid(message) => write!(f, "visual data is invalid: {message}"),
            Self::Render(error) => write!(f, "visual rendering failed: {error}"),
        }
    }
}

impl std::error::Error for VisualEnvelopeError {}

impl From<serde_json::Error> for VisualEnvelopeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<std::io::Error> for VisualEnvelopeError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<svg::RenderError> for VisualEnvelopeError {
    fn from(value: svg::RenderError) -> Self {
        Self::Render(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Clean,
    Violation,
    Timeout,
    MissingEvidence,
    NarrowedEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RootIdentity {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ToolIdentity {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RunMetadata {
    pub id: String,
    pub completed_at: String,
    pub root: RootIdentity,
    pub tool: ToolIdentity,
    pub outcome: RunOutcome,
}

/// An exact, workspace-relative source range. Lines and columns are zero-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub file: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

impl SourceLocation {
    pub fn point(file: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            start_line: line,
            start_column: column,
            end_line: line,
            end_column: column,
        }
    }

    fn from_span(span: &Span) -> Self {
        Self {
            file: span.file.clone(),
            start_line: span.start[0],
            start_column: span.start[1],
            end_line: span.end[0],
            end_column: span.end[1],
        }
    }

    pub fn validate(&self) -> Result<(), VisualEnvelopeError> {
        if !is_portable_workspace_relative(&self.file, false) {
            return Err(VisualEnvelopeError::Invalid(format!(
                "source file {:?} is not workspace-relative",
                self.file
            )));
        }
        if (self.end_line, self.end_column) < (self.start_line, self.start_column) {
            return Err(VisualEnvelopeError::Invalid(format!(
                "source range for {:?} ends before it starts",
                self.file
            )));
        }
        Ok(())
    }
}

/// The four states a client's evidence filter can hide items by. Published
/// explicitly so a client (the Ply Visual viewer's Earned/Gap/Violation
/// checkboxes) never has to re-derive it by pattern-matching the real
/// verdict strings (`bounded(2)`, `fuzzed(64)`, `unclaimed`, `tool_error`,
/// ...) itself -- Ply already computes this exact classification for its
/// own SVG styling (`svg::classify_evidence`), and a second implementation
/// of the same rule in a client is precisely the kind of drift this field
/// exists to prevent.
pub const EVIDENCE_STATES: [&str; 4] = ["declared", "earned", "gap", "violation"];

/// The default for an envelope published before this field existed:
/// "declared" is the one state none of the viewer's three checkboxes ever
/// hides, so an element whose real state no old run recorded stays visible
/// rather than being silently hidden by a guessed classification.
fn default_evidence_state() -> String {
    "declared".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementEvidence {
    pub verdict: String,
    pub statuses: Vec<String>,
    pub reused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cases: Option<u32>,
    /// One of [`EVIDENCE_STATES`]: exactly what the viewer's Earned/Gap/
    /// Violation checkboxes filter on. See `svg::classify_evidence` for the
    /// one place this is computed.
    #[serde(default = "default_evidence_state")]
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualElement {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<String>,
    pub evidence: ElementEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
    pub diagnostic_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualDiagnostic {
    pub id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
}

/// The same document drawn again with everything below `depth` folded into
/// its containing box.
///
/// A client that wants less detail on screen has two ways to get it, and only
/// one of them works. Hiding parts of the full drawing leaves every box at
/// the size its hidden contents needed, so pulling back produces large empty
/// rectangles -- the shape a reader pulled back to get away from. Drawing it
/// again at that level lays it out properly. Ply can already do that, so the
/// envelope carries the results rather than making a client ask for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FoldedDrawing {
    /// Boxes nested this many levels or deeper are folded away. Top-level
    /// boxes are level 1, so `depth: 1` draws only the outermost boxes.
    pub depth: usize,
    pub svg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct VisualEnvelope {
    pub protocol_version: u32,
    pub run: RunMetadata,
    pub svg: String,
    pub elements: BTreeMap<String, VisualElement>,
    pub diagnostics: Vec<VisualDiagnostic>,
    /// Shallower drawings of the same document, shallowest first. Empty when
    /// nothing nests deeply enough for folding to change anything -- a client
    /// that finds no entry for the level it wants should draw the full one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folded: Vec<FoldedDrawing>,
}

impl VisualEnvelope {
    pub fn from_json(json: &str) -> Result<Self, VisualEnvelopeError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        reject_unknown_version(&value)?;
        let envelope: Self = serde_json::from_value(value)?;
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("VisualEnvelope always serializes")
    }

    pub fn validate(&self) -> Result<(), VisualEnvelopeError> {
        if self.protocol_version != VISUAL_PROTOCOL_VERSION {
            return Err(VisualEnvelopeError::UnsupportedVersion(
                self.protocol_version.into(),
            ));
        }
        validate_run_id(&self.run.id)?;
        validate_rfc3339_utc(&self.run.completed_at)?;
        require_non_empty("run.root.path", &self.run.root.path)?;
        if !is_portable_workspace_relative(&self.run.root.path, true) {
            return Err(VisualEnvelopeError::Invalid(format!(
                "root path {:?} is not a portable workspace-relative path",
                self.run.root.path
            )));
        }
        require_non_empty("run.tool.name", &self.run.tool.name)?;
        require_non_empty("run.tool.version", &self.run.tool.version)?;
        require_non_empty("svg", &self.svg)?;
        for drawing in &self.folded {
            if drawing.depth == 0 {
                return Err(VisualEnvelopeError::Invalid(
                    "a folded drawing at depth 0 would select nothing: nesting levels start                      at 1 for top-level boxes"
                        .into(),
                ));
            }
            require_non_empty("folded svg", &drawing.svg)?;
        }
        let diagnostic_ids = self
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if diagnostic_ids.len() != self.diagnostics.len() {
            return Err(VisualEnvelopeError::Invalid(
                "diagnostic ids must be unique".into(),
            ));
        }
        for (key, element) in &self.elements {
            require_non_empty("element id", &element.id)?;
            require_non_empty("element kind", &element.kind)?;
            require_non_empty("element label", &element.label)?;
            require_non_empty("element evidence verdict", &element.evidence.verdict)?;
            if !EVIDENCE_STATES.contains(&element.evidence.state.as_str()) {
                return Err(VisualEnvelopeError::Invalid(format!(
                    "element {:?} has evidence state {:?}, which is none of {EVIDENCE_STATES:?}",
                    element.id, element.evidence.state
                )));
            }
            for status in &element.evidence.statuses {
                require_non_empty("element evidence status", status)?;
            }
            if let Some(engine) = &element.evidence.engine {
                require_non_empty("element evidence engine", engine)?;
            }
            if let Some(seed) = &element.evidence.seed {
                require_non_empty("element evidence seed", seed)?;
            }
            if key != &element.id {
                return Err(VisualEnvelopeError::Invalid(format!(
                    "element key {key:?} does not match its id {:?}",
                    element.id
                )));
            }
            if let Some(source) = &element.source {
                source.validate()?;
            }
            if let Some(parent) = &element.parent_id
                && !self.elements.contains_key(parent)
            {
                return Err(VisualEnvelopeError::Invalid(format!(
                    "element {:?} names missing parent {parent:?}",
                    element.id
                )));
            }
            for diagnostic_id in &element.diagnostic_ids {
                require_non_empty("element diagnostic id", diagnostic_id)?;
                if !diagnostic_ids.contains(diagnostic_id.as_str()) {
                    return Err(VisualEnvelopeError::Invalid(format!(
                        "element {:?} names missing diagnostic {diagnostic_id:?}",
                        element.id
                    )));
                }
            }
        }
        for diagnostic in &self.diagnostics {
            require_non_empty("diagnostic id", &diagnostic.id)?;
            require_non_empty("diagnostic code", &diagnostic.code)?;
            require_non_empty("diagnostic severity", &diagnostic.severity)?;
            require_non_empty("diagnostic message", &diagnostic.message)?;
            if let Some(source) = &diagnostic.source {
                source.validate()?;
            }
            if let Some(element_id) = &diagnostic.element_id
                && !self.elements.contains_key(element_id)
            {
                return Err(VisualEnvelopeError::Invalid(format!(
                    "diagnostic {:?} names missing element {element_id:?}",
                    diagnostic.id
                )));
            }
        }
        Ok(())
    }
}

fn reject_unknown_version(value: &serde_json::Value) -> Result<(), VisualEnvelopeError> {
    let version = value
        .get("protocolVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| VisualEnvelopeError::Invalid("protocolVersion must be 1".into()))?;
    if version != u64::from(VISUAL_PROTOCOL_VERSION) {
        return Err(VisualEnvelopeError::UnsupportedVersion(version));
    }
    Ok(())
}

/// A stable semantic ID, independent of traversal order and run identity.
/// Whether a component declares nothing that could ever earn evidence.
///
/// Not simply "no functions and no children": a component that says what
/// must always be true of the structure it holds has made a claim a run can
/// be wrong about, so calling it "a sketch waiting for claims" is false --
/// and it was, printed directly beneath the promise itself, until `holds:`
/// arrived and this predicate stopped being three copies of one expression.
pub fn is_hollow(comp: &crate::model::Component) -> bool {
    comp.fns.is_empty()
        && comp.components.is_empty()
        && comp.state.as_ref().is_none_or(|st| st.holds.is_empty())
}

pub fn stable_element_id(kind: &str, semantic_key: &str) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"ply-visual-element-v1\0");
    hash.update(kind.as_bytes());
    hash.update(b"\0");
    hash.update(semantic_key.as_bytes());
    let prefix = kind
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect::<String>();
    let prefix = if prefix.is_empty() {
        "element"
    } else {
        &prefix
    };
    let digest = hash.finalize().to_hex();
    format!("ply-{prefix}-{}", &digest[..16])
}

pub fn outcome_of(envelope: &Envelope) -> RunOutcome {
    fn visit(node: &Node, flags: &mut [bool; 4]) {
        flags[0] |= node.verdict == "violation";
        flags[1] |= node.verdict == "timeout" || node.statuses.iter().any(|s| s == "timeout");
        flags[2] |= crate::diag::is_absence(&node.verdict)
            || node.statuses.iter().any(|s| crate::diag::is_absence(s));
        flags[3] |= node.statuses.iter().any(|s| s == "partial-history");
        for child in &node.children {
            visit(child, flags);
        }
    }
    let mut flags = [false; 4];
    visit(&envelope.root, &mut flags);
    if flags[0] {
        RunOutcome::Violation
    } else if flags[1] {
        RunOutcome::Timeout
    } else if flags[2] {
        RunOutcome::MissingEvidence
    } else if flags[3] {
        RunOutcome::NarrowedEvidence
    } else {
        RunOutcome::Clean
    }
}

pub fn completed_run_metadata(
    _root: &Path,
    tool_version: &str,
    outcome: RunOutcome,
) -> RunMetadata {
    let now = SystemTime::now();
    let duration = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    RunMetadata {
        id: format!(
            "{}-{:09}-{}",
            duration.as_secs(),
            duration.subsec_nanos(),
            std::process::id()
        ),
        completed_at: rfc3339_utc(now),
        root: RootIdentity {
            // The snapshot is rooted at the `ply.yaml` directory that owns
            // `target/ply`; spelling the identity relative to itself keeps
            // the public artifact portable even when the CLI received an
            // absolute host path.
            path: ".".into(),
        },
        tool: ToolIdentity {
            name: "cargo-ply".into(),
            version: tool_version.into(),
        },
        outcome,
    }
}

/// Build the portable contract from one completed semantic result.
pub fn build_visual_envelope(
    document: &Document,
    result: &Envelope,
    run: RunMetadata,
) -> Result<VisualEnvelope, VisualEnvelopeError> {
    build_visual_envelope_with_sources(document, result, run, &BTreeMap::new())
}

/// Build the same navigable visual contract before code or evidence exists.
/// Every declared item is present and explicitly unanswered; verification can
/// later replace those states without changing identity or hierarchy.
///
/// `state_fields` is what a component's declared `state:` type really
/// holds, read from source by the caller (`crate::harness::resolve_state_fields`)
/// -- `None` when there is no source root to read it from. Without it every
/// component that declares `state:` draws honestly as unresolved even when
/// the code sits right there, which is the one thing this function cannot
/// tell on its own: it only ever sees the document.
pub fn build_declared_visual_envelope(
    document: &Document,
    run: RunMetadata,
    options: &svg::RenderOptions,
    state_fields: Option<&crate::harness::StateFieldIndex>,
) -> Result<VisualEnvelope, VisualEnvelopeError> {
    build_declared_visual_envelope_with_links(document, run, options, state_fields, None)
}

/// [`build_declared_visual_envelope`], plus every derived cross-document
/// link (`crate::config::derive_links`) — resolved by the caller, the same
/// way `state_fields` already is, since deriving one means reading real
/// crate directories off disk and this function only ever sees the parsed
/// document.
pub fn build_declared_visual_envelope_with_links(
    document: &Document,
    run: RunMetadata,
    options: &svg::RenderOptions,
    state_fields: Option<&crate::harness::StateFieldIndex>,
    links: Option<&crate::config::LinkIndex>,
) -> Result<VisualEnvelope, VisualEnvelopeError> {
    fn component_node(path: &str, component: &crate::model::Component) -> Node {
        let mut children = component
            .fns
            .iter()
            .map(|(name, claim)| Node {
                id: name.clone(),
                kind: "fn".into(),
                verdict: "unclaimed".into(),
                contract: crate::diag::Contract {
                    requires: claim.requires.clone(),
                    ensures: claim.ensures.clone(),
                },
                ..Node::default()
            })
            .collect::<Vec<_>>();
        children.extend(
            component
                .components
                .iter()
                .map(|(name, child)| component_node(&format!("{path}.{name}"), child)),
        );
        Node {
            id: path.into(),
            kind: "component".into(),
            verdict: "unclaimed".into(),
            children,
            ..Node::default()
        }
    }

    let result = Envelope {
        command: "render".into(),
        ply_version: document.ply.to_string(),
        root: Node {
            id: "workspace".into(),
            kind: "workspace".into(),
            verdict: "unclaimed".into(),
            children: document
                .components
                .iter()
                .map(|(name, component)| component_node(name, component))
                .collect(),
            ..Node::default()
        },
        diagnostics: vec![],
        coverage: None,
        trust_surface: None,
        open_items: None,
        not_carried_forward: vec![],
    };
    // The caller cannot know what this outcome should be: the tree is built
    // right here, from the declarations alone, and nothing in it has been
    // checked. Deriving it rather than trusting the argument is what stops a
    // client from reading `clean` off a document where every item is still
    // unclaimed and colouring its badge green.
    let run = RunMetadata {
        outcome: outcome_of(&result),
        ..run
    };
    let mut visual = build_visual_envelope(document, &result, run)?;
    visual.svg = svg::render_svg_with_evidence_state_options_and_links(
        document,
        &visual.elements,
        &[],
        options,
        state_fields,
        links,
    )?;
    visual.folded = folded_drawings(
        document,
        &visual.elements,
        &[],
        options,
        state_fields,
        links,
    )?;
    visual.validate()?;
    Ok(visual)
}

/// Build the portable visual contract with the exact source ranges captured
/// from the same verification snapshot. The map is keyed by Ply's qualified
/// claim id (`component.path::function-key`), never by a display label.
pub fn build_visual_envelope_with_sources(
    document: &Document,
    result: &Envelope,
    run: RunMetadata,
    source_map: &BTreeMap<String, Span>,
) -> Result<VisualEnvelope, VisualEnvelopeError> {
    let mut elements = BTreeMap::new();
    let mut semantic_ids = BTreeMap::new();
    collect_elements(
        &result.root,
        None,
        None,
        &result.diagnostics,
        source_map,
        &mut elements,
        &mut semantic_ids,
    )?;
    let diagnostics = result
        .diagnostics
        .iter()
        .enumerate()
        .map(|(index, diagnostic)| visual_diagnostic(index, diagnostic, &semantic_ids))
        .collect::<Vec<_>>();
    let svg = svg::render_svg_with_evidence(document, &elements, &diagnostics)?;
    // No source root travels with a completed run's evidence, so there is
    // nothing to resolve state fields against here -- `None` keeps this
    // path exactly as it was before state resolution existed.
    let folded = folded_drawings(
        document,
        &elements,
        &diagnostics,
        &svg::RenderOptions::default(),
        None,
        None,
    )?;
    let envelope = VisualEnvelope {
        protocol_version: VISUAL_PROTOCOL_VERSION,
        run,
        svg,
        elements,
        diagnostics,
        folded,
    };
    envelope.validate()?;
    Ok(envelope)
}

/// Draw the document again at every level shallower than it actually nests.
///
/// Only levels that change something are kept. A drawing identical to the one
/// the caller already has is pure weight in the envelope, and a client reading
/// the list would have no way to tell a real choice from a repeat.
///
/// `state_fields` is threaded through unchanged to every re-render: a
/// component that stays fully expanded at a shallower depth must show
/// exactly the resolved state row the full drawing showed it, never fall
/// back to "unresolved" just because this is a re-render rather than the
/// first one.
fn folded_drawings(
    document: &Document,
    elements: &BTreeMap<String, VisualElement>,
    diagnostics: &[VisualDiagnostic],
    base: &svg::RenderOptions,
    state_fields: Option<&crate::harness::StateFieldIndex>,
    links: Option<&crate::config::LinkIndex>,
) -> Result<Vec<FoldedDrawing>, VisualEnvelopeError> {
    // A reader who already narrowed the drawing by hand has made this choice
    // themselves; offering alternatives to a selection would silently undo it.
    if base.depth.is_some() || base.focus.is_some() || !base.collapse.is_empty() {
        return Ok(Vec::new());
    }
    let full = svg::render_svg_with_evidence_state_options_and_links(
        document,
        elements,
        diagnostics,
        base,
        state_fields,
        links,
    )?;
    let mut folded = Vec::new();
    for depth in 1..nesting_levels(document) {
        let options = svg::RenderOptions {
            depth: Some(depth),
            ..base.clone()
        };
        let svg = svg::render_svg_with_evidence_state_options_and_links(
            document,
            elements,
            diagnostics,
            &options,
            state_fields,
            links,
        )?;
        if svg != full {
            folded.push(FoldedDrawing { depth, svg });
        }
    }
    Ok(folded)
}

/// How many levels of boxes the document actually has: 1 when no component
/// contains another.
fn nesting_levels(document: &Document) -> usize {
    fn deepest(component: &crate::model::Component) -> usize {
        1 + component
            .components
            .values()
            .map(deepest)
            .max()
            .unwrap_or(0)
    }
    document.components.values().map(deepest).max().unwrap_or(0)
}

/// Maps the renderer's own classification to the four strings a client is
/// promised. Reuses `svg::classify_evidence` rather than re-reading
/// `verdict`/`statuses` here, so there is exactly one place in Ply that
/// decides what a verdict *means* display-wise.
///
/// `Stale` maps to `"gap"`: a stale result is answered by nothing current
/// (the code moved on since the check ran), so calling it `"earned"` would
/// overclaim, and it was never merely undeclared or a broken rule, so
/// neither `"declared"` nor `"violation"` fits either -- `"gap"` is Ply's
/// existing word for "answered by nothing current" (§ absence verdicts:
/// timeout, engine-missing, inconclusive, tool_error, unsupported*). In
/// practice this arm is unreached: `registry.rs` records that `stale` is
/// not a status any pipeline code emits today, and the only reader of it is
/// this same dead renderer styling -- but the classifier still has the
/// arm, so the mapping has to cover it honestly rather than pretend it
/// cannot occur.
fn evidence_state(evidence: &ElementEvidence) -> &'static str {
    match svg::classify_evidence(evidence) {
        svg::DisplayState::Violated => "violation",
        svg::DisplayState::Declared => "declared",
        svg::DisplayState::Unanswered | svg::DisplayState::Stale => "gap",
        svg::DisplayState::Earned { .. } => "earned",
    }
}

fn collect_elements(
    node: &Node,
    parent_id: Option<&str>,
    parent_semantic_key: Option<&str>,
    diagnostics: &[Diagnostic],
    source_map: &BTreeMap<String, Span>,
    out: &mut BTreeMap<String, VisualElement>,
    semantic_ids: &mut BTreeMap<String, String>,
) -> Result<(), VisualEnvelopeError> {
    // A leaf's key is qualified by its parent; a container's is its own id.
    // `state` joined `fn` here when it arrived (2026-09-04): two components
    // may each promise something about the same type, and an unqualified
    // key gives both the same element id.
    let semantic_key = if node.kind == "fn" || node.kind == "state" {
        parent_semantic_key
            .map(|parent| format!("{parent}::{}", node.id))
            .unwrap_or_else(|| node.id.clone())
    } else {
        node.id.clone()
    };
    let id = stable_element_id(&node.kind, &semantic_key);
    let attached = diagnostics
        .iter()
        .enumerate()
        .filter(|(_, diagnostic)| diagnostic.node_id == semantic_key)
        .collect::<Vec<_>>();
    let source = source_map
        .get(&semantic_key)
        .or_else(|| {
            attached
                .iter()
                .find_map(|(_, diagnostic)| diagnostic.primary_span.as_ref())
        })
        .map(SourceLocation::from_span);
    let diagnostic_ids = attached
        .iter()
        .map(|(index, diagnostic)| diagnostic_id(*index, diagnostic))
        .collect();
    let declaration = (!node.contract.is_empty()).then(|| {
        node.contract
            .requires
            .iter()
            .map(|clause| format!("Input (requires): {clause}"))
            .chain(
                node.contract
                    .ensures
                    .iter()
                    .map(|clause| format!("Postcondition (ensures): {clause}")),
            )
            .collect::<Vec<_>>()
            .join("\n")
    });
    let mut evidence = ElementEvidence {
        verdict: node.verdict.clone(),
        statuses: node.statuses.clone(),
        reused: node.reused,
        engine: node
            .evidence
            .as_ref()
            .map(|evidence| evidence.engine.clone()),
        seed: node
            .evidence
            .as_ref()
            .and_then(|evidence| evidence.seed.clone()),
        cases: node.evidence.as_ref().and_then(|evidence| evidence.cases),
        // Placeholder: `evidence_state` below reuses `svg::classify_evidence`,
        // which reads verdict/statuses off this same struct, so the struct
        // has to exist before the state it publishes can be computed.
        state: String::new(),
    };
    evidence.state = evidence_state(&evidence).to_string();
    if semantic_ids
        .insert(semantic_key.clone(), id.clone())
        .is_some()
    {
        return Err(VisualEnvelopeError::Invalid(format!(
            "semantic element key {semantic_key:?} appears more than once"
        )));
    }
    if out
        .insert(
            id.clone(),
            VisualElement {
                id: id.clone(),
                kind: node.kind.clone(),
                label: if node.kind == "component" {
                    node.id.rsplit('.').next().unwrap_or(&node.id).to_string()
                } else {
                    node.id.clone()
                },
                parent_id: parent_id.map(ToOwned::to_owned),
                declaration,
                evidence,
                source,
                diagnostic_ids,
            },
        )
        .is_some()
    {
        return Err(VisualEnvelopeError::Invalid(format!(
            "stable element id {id:?} appears more than once"
        )));
    }
    for child in &node.children {
        collect_elements(
            child,
            Some(&id),
            Some(&semantic_key),
            diagnostics,
            source_map,
            out,
            semantic_ids,
        )?;
    }
    Ok(())
}

fn diagnostic_id(index: usize, diagnostic: &Diagnostic) -> String {
    stable_element_id(
        "diagnostic",
        &format!("{}\0{}\0{index}", diagnostic.code, diagnostic.node_id),
    )
}

fn visual_diagnostic(
    index: usize,
    diagnostic: &Diagnostic,
    semantic_ids: &BTreeMap<String, String>,
) -> VisualDiagnostic {
    VisualDiagnostic {
        id: diagnostic_id(index, diagnostic),
        code: diagnostic.code.clone(),
        severity: diagnostic.severity.clone(),
        message: diagnostic.title.clone(),
        element_id: semantic_ids.get(&diagnostic.node_id).cloned(),
        source: diagnostic
            .primary_span
            .as_ref()
            .map(SourceLocation::from_span),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RunIndexEntry {
    pub id: String,
    pub path: String,
    pub completed_at: String,
    pub outcome: RunOutcome,
}

impl RunIndexEntry {
    pub fn path_is_safe(&self) -> bool {
        self.path == format!("views/{}/visual.json", self.id)
            && is_portable_workspace_relative(&self.path, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ViewIndex {
    pub protocol_version: u32,
    pub current_run: String,
    pub runs: Vec<RunIndexEntry>,
}

impl ViewIndex {
    pub fn from_json(json: &str) -> Result<Self, VisualEnvelopeError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        reject_unknown_version(&value)?;
        let index: Self = serde_json::from_value(value)?;
        index.validate()?;
        Ok(index)
    }

    pub fn validate(&self) -> Result<(), VisualEnvelopeError> {
        if self.protocol_version != VISUAL_PROTOCOL_VERSION {
            return Err(VisualEnvelopeError::UnsupportedVersion(
                self.protocol_version.into(),
            ));
        }
        validate_run_id(&self.current_run)?;
        if !self.runs.iter().any(|run| run.id == self.current_run) {
            return Err(VisualEnvelopeError::Invalid(
                "currentRun does not name an indexed run".into(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for run in &self.runs {
            validate_run_id(&run.id)?;
            if !seen.insert(&run.id) {
                return Err(VisualEnvelopeError::Invalid(format!(
                    "run {:?} appears more than once in the index",
                    run.id
                )));
            }
            validate_rfc3339_utc(&run.completed_at)?;
            if !run.path_is_safe() {
                return Err(VisualEnvelopeError::Invalid(format!(
                    "run path {:?} is not the safe path for {:?}",
                    run.path, run.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Publication {
    pub artifact: PathBuf,
    pub index: PathBuf,
    /// The index and artifact are committed even when stale directories could
    /// not be pruned. Callers must surface this warning without calling the
    /// publication itself a failure.
    pub warning: Option<CleanupWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupWarning {
    pub failures: Vec<String>,
}

impl fmt::Display for CleanupWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the index was committed, but cleanup was incomplete: {}",
            self.failures.join("; ")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleanup {
    /// Number of runs removed from the authoritative index.
    pub removed: usize,
    /// Disk paths which could not safely be pruned after the index commit.
    pub warning: Option<CleanupWarning>,
}

pub struct VisualPublisher {
    root: PathBuf,
}

impl VisualPublisher {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn ply_dir(&self) -> PathBuf {
        self.root.join("target/ply")
    }

    pub fn read_index(&self) -> Result<Option<ViewIndex>, VisualEnvelopeError> {
        let path = self.ply_dir().join("view.json");
        match fs::read_to_string(path) {
            Ok(json) => ViewIndex::from_json(&json).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn publish(
        &self,
        envelope: &VisualEnvelope,
        retain: usize,
    ) -> Result<Publication, VisualEnvelopeError> {
        if retain == 0 {
            return Err(VisualEnvelopeError::Invalid(
                "retention must keep at least the current run".into(),
            ));
        }
        if !self.root.join("ply.yaml").is_file() {
            return Err(VisualEnvelopeError::Invalid(format!(
                "{} does not contain ply.yaml",
                self.root.display()
            )));
        }
        envelope.validate()?;
        let artifact_json = serde_json::to_vec_pretty(envelope)?;
        let ply_dir = self.ply_dir();
        let views_dir = ply_dir.join("views");
        self.refuse_symlinked_publication_tree()?;
        fs::create_dir_all(&ply_dir)?;
        self.refuse_symlinked_publication_tree()?;
        let _lock = self.acquire_publication_lock()?;
        self.refuse_symlinked_publication_tree()?;
        fs::create_dir_all(&views_dir)?;
        self.refuse_symlinked_publication_tree()?;

        // This read belongs inside the lock. Reading before acquiring it can
        // lose a concurrently-published run even when both later swaps are
        // individually atomic.
        let old_index = self.read_index()?;
        if old_index
            .as_ref()
            .is_some_and(|index| index.runs.iter().any(|run| run.id == envelope.run.id))
        {
            return Err(VisualEnvelopeError::Invalid(format!(
                "visual run {:?} already exists and is immutable",
                envelope.run.id
            )));
        }

        let run_dir = views_dir.join(&envelope.run.id);
        if fs::symlink_metadata(&run_dir).is_ok() {
            return Err(VisualEnvelopeError::Invalid(format!(
                "visual run {:?} already exists and is immutable",
                envelope.run.id
            )));
        }
        fs::create_dir(&run_dir)?;
        let artifact = run_dir.join("visual.json");
        let artifact_temp = run_dir.join(format!(".visual.json.{}.tmp", std::process::id()));
        if let Err(error) = write_new_and_rename(&artifact_temp, &artifact, &artifact_json) {
            let _ = fs::remove_file(&artifact_temp);
            let _ = fs::remove_dir(&run_dir);
            return Err(error.into());
        }

        let mut runs = old_index.map_or_else(Vec::new, |index| index.runs);
        runs.retain(|run| run.id != envelope.run.id);
        runs.push(RunIndexEntry {
            id: envelope.run.id.clone(),
            path: format!("views/{}/visual.json", envelope.run.id),
            completed_at: envelope.run.completed_at.clone(),
            outcome: envelope.run.outcome.clone(),
        });
        runs.sort_by(|a, b| a.completed_at.cmp(&b.completed_at).then(a.id.cmp(&b.id)));
        let removed = if runs.len() > retain {
            runs.drain(..runs.len() - retain).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let index = ViewIndex {
            protocol_version: VISUAL_PROTOCOL_VERSION,
            current_run: envelope.run.id.clone(),
            runs,
        };
        index.validate()?;
        let index_json = serde_json::to_vec_pretty(&index)?;
        let index_path = ply_dir.join("view.json");
        let index_temp = ply_dir.join(format!(".view.json.{}.tmp", std::process::id()));
        if let Err(error) = write_new_and_rename(&index_temp, &index_path, &index_json) {
            let _ = fs::remove_file(&index_temp);
            let _ = fs::remove_file(&artifact);
            let _ = fs::remove_dir(&run_dir);
            return Err(error.into());
        }

        let warning = prune_run_directories(&views_dir, &removed, &index.current_run);
        Ok(Publication {
            artifact,
            index: index_path,
            warning,
        })
    }

    pub fn cleanup(&self, retain: usize) -> Result<Cleanup, VisualEnvelopeError> {
        if retain == 0 {
            return Err(VisualEnvelopeError::Invalid(
                "retention must keep at least the current run".into(),
            ));
        }
        if !self.root.join("ply.yaml").is_file() {
            return Err(VisualEnvelopeError::Invalid(format!(
                "{} does not contain ply.yaml",
                self.root.display()
            )));
        }
        self.refuse_symlinked_publication_tree()?;
        if !self.ply_dir().exists() {
            return Ok(Cleanup {
                removed: 0,
                warning: None,
            });
        }
        let _lock = self.acquire_publication_lock()?;
        self.refuse_symlinked_publication_tree()?;
        let Some(mut index) = self.read_index()? else {
            return Ok(Cleanup {
                removed: 0,
                warning: None,
            });
        };
        index
            .runs
            .sort_by(|a, b| a.completed_at.cmp(&b.completed_at).then(a.id.cmp(&b.id)));
        let mut removed = Vec::new();
        while index.runs.len() > retain {
            let candidate = index
                .runs
                .iter()
                .position(|run| run.id != index.current_run)
                .ok_or_else(|| {
                    VisualEnvelopeError::Invalid("retention would remove the current run".into())
                })?;
            removed.push(index.runs.remove(candidate));
        }
        if removed.is_empty() {
            return Ok(Cleanup {
                removed: 0,
                warning: None,
            });
        }
        index.validate()?;
        let bytes = serde_json::to_vec_pretty(&index)?;
        let ply_dir = self.ply_dir();
        let index_path = ply_dir.join("view.json");
        let temp = ply_dir.join(format!(".view.json.{}.tmp", std::process::id()));
        write_new_and_rename(&temp, &index_path, &bytes)?;
        let warning = prune_run_directories(&ply_dir.join("views"), &removed, &index.current_run);
        Ok(Cleanup {
            removed: removed.len(),
            warning,
        })
    }

    fn refuse_symlinked_publication_tree(&self) -> Result<(), VisualEnvelopeError> {
        let ply_dir = self.ply_dir();
        refuse_symlink(&self.root.join("target"))?;
        refuse_symlink(&ply_dir)?;
        refuse_symlink(&ply_dir.join("views"))?;
        refuse_symlink(&ply_dir.join("view.json"))?;
        refuse_symlink(&ply_dir.join(".publication.lock"))
    }

    fn acquire_publication_lock(&self) -> Result<File, VisualEnvelopeError> {
        let path = self.ply_dir().join(".publication.lock");
        refuse_symlink(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.lock()?;
        // Catch a lock-path symlink substituted between the first metadata
        // check and open. Stable path-based APIs cannot pin every ancestor
        // against a hostile rename, so run deletion separately uses
        // symlink_metadata and remove_dir_all's no-follow guarantee.
        refuse_symlink(&path)?;
        Ok(file)
    }
}

fn prune_run_directories(
    views_dir: &Path,
    removed: &[RunIndexEntry],
    current_run: &str,
) -> Option<CleanupWarning> {
    let mut failures = Vec::new();
    for old in removed {
        if !old.path_is_safe() || old.id == current_run {
            continue;
        }
        let dir = views_dir.join(&old.id);
        match fs::symlink_metadata(&dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", dir.display())),
            Ok(metadata) if metadata.file_type().is_symlink() => failures.push(format!(
                "{} is a symbolic link; refusing to delete it",
                dir.display()
            )),
            Ok(metadata) if !metadata.is_dir() => failures.push(format!(
                "{} is not a directory; refusing to delete it",
                dir.display()
            )),
            Ok(_) => {
                if let Err(error) = fs::remove_dir_all(&dir) {
                    failures.push(format!("{}: {error}", dir.display()));
                }
            }
        }
    }
    (!failures.is_empty()).then_some(CleanupWarning { failures })
}

fn write_new_and_rename(temp: &Path, destination: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, destination)
}

fn refuse_symlink(path: &Path) -> Result<(), VisualEnvelopeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(VisualEnvelopeError::Invalid(format!(
                "{} is a symbolic link; refusing to operate through it",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_run_id(id: &str) -> Result<(), VisualEnvelopeError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || id == "."
        || id == ".."
    {
        return Err(VisualEnvelopeError::Invalid(format!(
            "run id {id:?} is not a safe artifact-directory name"
        )));
    }
    Ok(())
}

fn is_portable_workspace_relative(value: &str, allow_root_dot: bool) -> bool {
    if value.is_empty()
        || value.contains('\\')
        || value.starts_with('/')
        || value.chars().any(|character| {
            character.is_control() || matches!(character, ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return false;
    }
    if allow_root_dot && value == "." {
        return true;
    }
    value
        .split('/')
        .all(|component| !component.is_empty() && component != "." && component != "..")
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn require_non_empty(field: &str, value: &str) -> Result<(), VisualEnvelopeError> {
    if value.trim().is_empty() {
        Err(VisualEnvelopeError::Invalid(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_rfc3339_utc(value: &str) -> Result<(), VisualEnvelopeError> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 20
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(10) == Some(&b'T')
        && bytes.get(13) == Some(&b':')
        && bytes.get(16) == Some(&b':')
        && bytes.get(19) == Some(&b'Z')
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        });
    if valid && timestamp_components_are_real(bytes) {
        Ok(())
    } else {
        Err(VisualEnvelopeError::Invalid(format!(
            "completedAt {value:?} is not an RFC3339 UTC timestamp"
        )))
    }
}

fn timestamp_components_are_real(bytes: &[u8]) -> bool {
    fn number(bytes: &[u8]) -> u32 {
        bytes
            .iter()
            .fold(0, |value, byte| value * 10 + u32::from(byte - b'0'))
    }
    let year = number(&bytes[0..4]);
    let month = number(&bytes[5..7]);
    let day = number(&bytes[8..10]);
    let hour = number(&bytes[11..13]);
    let minute = number(&bytes[14..16]);
    let second = number(&bytes[17..19]);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year != 0 && (1..=days).contains(&day) && hour < 24 && minute < 60 && second < 60
}

fn rfc3339_utc(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

// Howard Hinnant's civil-from-days algorithm, with day zero at 1970-01-01.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_formats_as_rfc3339() {
        assert_eq!(rfc3339_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    }
}
