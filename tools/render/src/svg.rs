//! `ply.yaml` -> SVG, the §7.1 visual grammar table made literal.
//!
//! This is a spec-validation tool: its only job is to prove every declarative
//! construct in the grammar has a drawable form. Layout is a simple
//! deterministic block stack (no layout library, no font-metrics library —
//! character widths are a fixed-width monospace estimate) so that the same
//! input always produces byte-identical output.

use crate::layout;
use indexmap::IndexMap;
use ply_check::{Diagnostic, Target as FindingTarget, run_checks};
use ply_kernel::{Evidence, NodeKind, VerdictNode, aggregate};
use ply_model::{
    Check, Component, Deny, Document, Edge, EdgeKind, FnClaim, InheritedChecks,
    component_default_checks, effective_checks, parse_check, parse_deny, parse_edge,
};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

// ---- layout constants -----------------------------------------------------

const PAD: f64 = 10.0;
const GAP: f64 = 12.0;
const NAME_CHAR_W: f64 = 8.0;
const SUB_CHAR_W: f64 = 6.2;
const CHIP_CHAR_W: f64 = 7.4;
const LINE_H: f64 = 16.0;
const HEADER_H: f64 = LINE_H * 2.0 + 6.0;
const BADGE_H: f64 = 20.0;
const BADGE_CHAR_W: f64 = 6.5;
const BADGE_PAD: f64 = 8.0;
const BADGE_GAP: f64 = 6.0;
const CHIP_H: f64 = 24.0;
const PIN_R: f64 = 9.0;
const SHIELD_W: f64 = 16.0;
const FRAME_PAD: f64 = 24.0;
const FRAME_TITLE_H: f64 = 30.0;
const MIN_BOX_W: f64 = 150.0;
/// §7.1: a collapsed box draws as a stack — one card edge offset behind it,
/// the "pile of cards" instinct for folded content. The 5px overhang lives in
/// the layout gaps (NODESEP 50, RANKSEP 70, FRAME_PAD 24), so the reported box
/// size stays the main box and edge attachment is unaffected.
const STACK_OFFSET: f64 = 5.0;

// ---- §7.1 "contract clauses" — the contract mark ---------------------------
//
// A small solid square at the fn chip's left edge, drawn inside the chip's
// existing `PAD` margin so it adds new geometry (its own few pixels) without
// shifting anything else already positioned by `cursor_x`.
const CONTRACT_MARK_W: f64 = 3.0;

// ---- findings (§7.1 "finding (tool-computed, not declared)") --------------
//
// `ply-check`'s document-local diagnostics, drawn on top of whatever the
// declared grammar already put on the canvas. A finding never changes the
// layout or markup of an item that has none of its own (see
// `FindingCtx::mark_and_collect`), so a clean document renders byte-for-byte
// as it always has.
const FINDING_BADGE_H: f64 = 14.0;
const FINDING_BADGE_CHAR_W: f64 = 6.5;
const FINDING_BADGE_PAD: f64 = 4.0;

// ---- top-level component graph layout -------------------------------------
//
// §7.1 amendment (vetting 002 render-pass findings): top-level components
// are no longer a single stacked column. They are laid out in ranked rows
// (see `layout.rs`), edges between the same pair get parallel lanes, and
// edge labels are reserved space beside the line rather than centered on
// it. Concepts pinched from archi-techture's dagre layout, not its code
// (`layout.rs` doc comment has the detail).

/// Vertical gap between ranks. Wider than archi-techture's 52: our boxes are
/// taller (multi-line headers, badge rows, fn chips) and a flow-edge label
/// must fit beside the line without touching either rank.
const RANKSEP: f64 = 70.0;
/// Horizontal gap between components sharing a rank. Wider than
/// archi-techture's 32 for the same reason (our boxes are wider too).
const NODESEP: f64 = 50.0;
/// Perpendicular separation between parallel edges connecting the same pair
/// of components (vetting 002 finding 4: call + flow, or opposite
/// directions, must not coincide).
const LANE_GAP: f64 = 16.0;
/// Fraction along an edge (from 0 = start, 1 = arrowhead) where its label
/// sits. Off-center so the label never approaches the arrowhead end.
const LABEL_T: f64 = 0.38;
/// Extra perpendicular offset that pushes a flow-edge label beside its line
/// rather than on top of it.
const LABEL_SIDE_GAP: f64 = 15.0;
/// Radius of a deny rule's `*` ("any component") pseudo-node.
const ANY_R: f64 = 14.0;
/// Clear space kept between an any-node, the line it anchors, and whatever
/// sits next to it (the frame edge on one side, a real component box or an
/// `except` label on the other).
const ANY_GAP: f64 = 16.0;

/// Every `class` this renderer emits must have a rule here. SVG's initial
/// paint is `fill: black; stroke: none`, so an unstyled shape is a solid black
/// box — a missing rule is invisible output, not a cosmetic slip.
/// `tests/render.rs::every_painted_element_resolves_a_style_rule` enforces it.
pub const STYLE: &str = "\
.workspace-frame{fill:#fbfbfd;stroke:#c8ccd4}\
.workspace-title{fill:#6b7280}\
.component-box{stroke:#3b4252;stroke-width:1.5}\
.hollow-box{stroke-dasharray:6 4}\
.collapsed-stack{stroke:#3b4252;stroke-width:1.5}\
.pure-seal{fill:none;stroke:#3b4252}\
.component-name{fill:#1f2430;font-weight:bold}\
.component-anchor{fill:#6b7280;font-size:10px}\
.component-owns{fill:#6b7280;font-size:10px;font-style:italic}\
.ceiling-unclaimed{fill:#fff}\
.ceiling-tested{fill:#eaf6ec}\
.ceiling-fuzzed{fill:#cdeed3}\
.ceiling-bounded{fill:#a3e0b3}\
.ceiling-proved{fill:#78d194}\
.contract-mark{fill:#1f2430}\
.cap-badge rect{fill:#fdecec;stroke:#c9534f}\
.cap-badge text{fill:#8f2f2c;font-size:10px}\
.profile-tag rect{fill:#eef2fb;stroke:#5570a8}\
.profile-tag text{fill:#334b78;font-size:10px}\
.fn-chip-box{fill:#f6f7f9;stroke:#9aa2b1}\
.fn-name{fill:#1f2430}\
.fn-checks{fill:#2f6f4f;font-size:11px}\
.fn-check-with{fill:#6b7280;font-size:10px}\
.fn-shield{fill:none;stroke:#9a7a1f;font-size:13px}\
.unresolved-pin circle,.registry-pin circle{fill:#fff6d8;stroke:#b08900}\
.pin-label{fill:#7a5c00;font-size:10px;text-anchor:middle}\
.edge-line{fill:none;stroke:#3b4252;stroke-width:1.5}\
.edge-label{fill:#3b4252;font-size:10px;text-anchor:middle}\
.deny-line{fill:none;stroke:#c9534f;stroke-width:1.5}\
.deny-bar{stroke:#c9534f;stroke-width:3}\
.deny-except{fill:#8f2f2c;font-size:10px;text-anchor:middle}\
.any-node circle{fill:#eceef2;stroke:#9aa2b1}\
.any-label{fill:#4b5563;text-anchor:middle}\
#arrow path{fill:#3b4252}\
";

/// Rules for the §7.1 "finding" classes, kept separate from `STYLE` and
/// appended to the embedded `<style>` only on a document that actually has
/// at least one finding (`render_svg`'s `style` local). A clean document —
/// every vetting fixture, the golden snapshot — must render byte-identical
/// to before this feature existed; folding these rules permanently into
/// `STYLE` would grow *every* document's stylesheet text regardless of
/// whether it uses any of these classes, which is exactly the kind of leak
/// CLAUDE.md's golden-review rule exists to catch.
pub const FINDING_STYLE: &str = "\
.fn-chip-box-finding{fill:#fdecec;stroke:#c9534f;stroke-width:2.5}\
.component-box-finding{stroke:#c9534f;stroke-width:3}\
.edge-line-finding{fill:none;stroke:#c9534f;stroke-width:3}\
.deny-line-finding{fill:none;stroke:#c9534f;stroke-width:3}\
.unresolved-pin-finding circle,.registry-pin-finding circle{fill:#fdecec;stroke:#c9534f;stroke-width:2.5}\
.pin-label-finding{fill:#8f2f2c;font-size:10px;text-anchor:middle}\
.finding-badge rect{fill:#c9534f;stroke:#8f2f2c}\
.finding-badge text{fill:#fff;font-size:9px;font-weight:bold;text-anchor:middle}\
.finding-count{fill:#c9534f;font-size:11px;font-weight:bold}\
";

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

fn text_w(s: &str, char_w: f64) -> f64 {
    (s.chars().count() as f64) * char_w
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One glyph token per check, in declaration order: `test`->T, `fuzz(n)`->Fn,
/// `bounded(k)`->Bk, `prove`->P, `mutate`->M. Unparseable strings are shown
/// verbatim so a malformed check is visible rather than silently dropped.
fn checks_glyph_row(checks: &[String]) -> String {
    checks
        .iter()
        .map(|c| match parse_check(c) {
            Ok(Check::Test) => "T".to_string(),
            Ok(Check::Fuzz(n)) => format!("F{n}"),
            Ok(Check::Bounded(k)) => format!("B{k}"),
            Ok(Check::Prove) => "P".to_string(),
            Ok(Check::Mutate) => "M".to_string(),
            Err(_) => c.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The glyph row spelled out for the hover tooltip. `<title>` is native SVG:
/// no script, no legend clutter on the canvas.
fn check_prose(c: &str) -> String {
    match parse_check(c) {
        Ok(Check::Test) => {
            "test — runs the declared examples plus generated inputs, checking the contract on each"
                .into()
        }
        Ok(Check::Fuzz(n)) => {
            format!(
                "fuzz({n}) — runs the function on {n} random inputs, checking the contract on each"
            )
        }
        Ok(Check::Bounded(k)) => {
            format!(
                "bounded({k}) — proves the contract for every input, unrolling loops at most {k} times"
            )
        }
        Ok(Check::Prove) => "prove — proves the contract for all inputs, with no bound".into(),
        Ok(Check::Mutate) => {
            "mutate — plants small deliberate bugs; the test/fuzz checks must catch every one, \
             or the contract is flagged weak"
                .into()
        }
        Err(e) => format!("{c} — unparseable: {e}"),
    }
}

/// §5.6: the wording explaining a fn-level unresolved marker, used both on
/// the pin glyph itself and in the fn-chip's aggregated tooltip.
fn unresolved_fn_pin_prose(id: u64, note: &str) -> String {
    format!(
        "#{id} marks an unresolved decision — a question this function still owes an \
         answer: {note}. Until it is resolved, this function's checks cap at `test` (§5.6)"
    )
}

fn title(text: &str) -> String {
    format!("<title>{}</title>", esc(text))
}

/// Every distinct location a `ply-check` diagnostic can attach to, indexed
/// for O(1) lookup while walking the same document a second time to draw
/// it. Keyed by findings-vector index rather than by reference, so a single
/// `RefCell<Vec<bool>>` can track which diagnostics got attached to a real
/// drawn item without fighting the borrow checker across the recursive
/// render functions below.
#[derive(Default)]
struct FindingsIndex {
    by_fn: HashMap<(String, String), Vec<usize>>,
    by_component: HashMap<String, Vec<usize>>,
    by_edge: HashMap<usize, Vec<usize>>,
    by_deny: HashMap<usize, Vec<usize>>,
    by_unresolved: HashMap<u64, Vec<usize>>,
}

fn build_findings_index(findings: &[Diagnostic]) -> FindingsIndex {
    let mut idx = FindingsIndex::default();
    for (i, d) in findings.iter().enumerate() {
        match &d.target {
            FindingTarget::Fn {
                component_path,
                fn_name,
            } => idx
                .by_fn
                .entry((component_path.clone(), fn_name.clone()))
                .or_default()
                .push(i),
            FindingTarget::Component(path) => {
                idx.by_component.entry(path.clone()).or_default().push(i)
            }
            FindingTarget::EdgeIndex(e) => idx.by_edge.entry(*e).or_default().push(i),
            FindingTarget::DenyIndex(d) => idx.by_deny.entry(*d).or_default().push(i),
            FindingTarget::UnresolvedId(id) => idx.by_unresolved.entry(*id).or_default().push(i),
            // §7.1: no drawable item — stays out of every index, so it can
            // never be marked attached and always lands in the workspace-
            // title fallback count (`unattached_count`).
            FindingTarget::Document => {}
        }
    }
    idx
}

/// Threaded through every render function that might draw an item a finding
/// attaches to. `attached` starts all-`false`; each successful lookup marks
/// its diagnostics attached (idempotently — a duplicate `UnresolvedId` may
/// be looked up twice, once per pin, and that must not double-count).
/// Whatever is still `false` once rendering finishes had no drawable item
/// and becomes the workspace-title fallback count.
struct FindingCtx<'a> {
    findings: &'a [Diagnostic],
    index: FindingsIndex,
    attached: RefCell<Vec<bool>>,
}

impl<'a> FindingCtx<'a> {
    fn new(findings: &'a [Diagnostic]) -> Self {
        let index = build_findings_index(findings);
        let attached = RefCell::new(vec![false; findings.len()]);
        FindingCtx {
            findings,
            index,
            attached,
        }
    }

    fn mark_and_collect(&self, idxs: Option<&Vec<usize>>) -> Vec<&'a Diagnostic> {
        let Some(idxs) = idxs else {
            return Vec::new();
        };
        let mut attached = self.attached.borrow_mut();
        for &i in idxs {
            attached[i] = true;
        }
        idxs.iter().map(|&i| &self.findings[i]).collect()
    }

    fn fn_findings(&self, component_path: &str, fn_name: &str) -> Vec<&'a Diagnostic> {
        let key = (component_path.to_string(), fn_name.to_string());
        self.mark_and_collect(self.index.by_fn.get(&key))
    }
    fn component_findings(&self, component_path: &str) -> Vec<&'a Diagnostic> {
        self.mark_and_collect(self.index.by_component.get(component_path))
    }
    fn edge_findings(&self, edge_index: usize) -> Vec<&'a Diagnostic> {
        self.mark_and_collect(self.index.by_edge.get(&edge_index))
    }
    fn deny_findings(&self, deny_index: usize) -> Vec<&'a Diagnostic> {
        self.mark_and_collect(self.index.by_deny.get(&deny_index))
    }
    fn unresolved_findings(&self, id: u64) -> Vec<&'a Diagnostic> {
        self.mark_and_collect(self.index.by_unresolved.get(&id))
    }

    /// Diagnostics that never matched a drawn item — the workspace-title
    /// fallback (§7.1: "a finding with no drawable item attaches a red
    /// count next to the workspace title").
    fn unattached_count(&self) -> usize {
        self.attached.borrow().iter().filter(|a| !**a).count()
    }
}

/// §7.1: "its tooltip leads with the finding: `FINDING E0203: <message>`".
/// One line per diagnostic, in the order `ply-check` reported them.
fn finding_tooltip_lines(findings: &[&Diagnostic]) -> Vec<String> {
    findings
        .iter()
        .map(|d| format!("FINDING {}: {}", d.code, d.message))
        .collect()
}

/// §7.1: "a small red badge with the diagnostic code". One code if there's
/// only one finding; `CODE+N` for the rest, rather than a badge that grows
/// without bound.
fn finding_badge_label(findings: &[&Diagnostic]) -> String {
    match findings {
        [] => String::new(),
        [only] => only.code.to_string(),
        [first, rest @ ..] => format!("{}+{}", first.code, rest.len()),
    }
}

fn finding_badge_width(findings: &[&Diagnostic]) -> f64 {
    if findings.is_empty() {
        0.0
    } else {
        FINDING_BADGE_PAD * 2.0 + text_w(&finding_badge_label(findings), FINDING_BADGE_CHAR_W)
    }
}

/// Draws the badge at local origin `(x, y)`, top-left corner. Never wraps
/// its own `<title>` — it always sits inside an item's `<g>` that already
/// carries the finding-led tooltip (`finding_tooltip_lines`), so hovering
/// the badge itself still resolves one.
fn render_finding_badge(x: f64, y: f64, findings: &[&Diagnostic]) -> String {
    if findings.is_empty() {
        return String::new();
    }
    let label = finding_badge_label(findings);
    let w = finding_badge_width(findings);
    format!(
        "<g class=\"finding-badge\"><rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{FINDING_BADGE_H:.1}\" rx=\"2\" /><text x=\"{tx:.1}\" y=\"{ty:.1}\">{label}</text></g>",
        tx = x + w / 2.0,
        ty = y + FINDING_BADGE_H - 4.0,
        label = esc(&label)
    )
}

/// Plain-language gloss for a named profile rule (§5.3): the tag alone
/// (`no_panics`, `exhaustive_match`, ...) means nothing to a reader who
/// hasn't read the spec. A rule this renderer doesn't recognize is shown
/// verbatim, with no invented meaning.
fn profile_rule_gloss(rule: &str) -> String {
    let meaning = match rule {
        "no_panics" => Some("functions here must never panic (crash on purpose)"),
        "exhaustive_match" => Some("every match must handle all cases explicitly"),
        _ => None,
    };
    match meaning {
        Some(m) => format!("{rule} ({m})"),
        None => rule.to_string(),
    }
}

fn profile_rules_prose(rules: &[String]) -> String {
    rules
        .iter()
        .map(|r| profile_rule_gloss(r))
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_with_note(check_with: &IndexMap<String, String>) -> Option<String> {
    if check_with.is_empty() {
        return None;
    }
    Some(
        check_with
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

// ---- §7.1 "declared ceiling" -----------------------------------------------
//
// A promise, not proof: the strongest verdict a component's *declared*
// checks could earn if they all passed, computed the same way a real
// verdict tree would be (worst-of over fns, folded by the kernel's own
// container rule) but from declared check *kinds* rather than run results.

/// The strongest check kind one fn's *effective* checks declare
/// (The-Ply-Spec.md §7.1: `test` -> tested, `fuzz(n)` -> fuzzed, `bounded(k)`
/// -> bounded, `prove` -> proved). `mutate` strengthens nothing on its own
/// (it only ever rides alongside a `test`/`fuzz` entry, D12) and an
/// unparseable string names no real kind, so both are skipped rather than
/// treated as evidence. No checks at all (own or inherited), or only skipped
/// ones, -> `Unclaimed`.
fn fn_declared_ceiling(checks: &[String]) -> Evidence {
    let mut best: Option<Evidence> = None;
    for c in checks {
        let kind = match parse_check(c) {
            Ok(Check::Test) => Some(Evidence::Tested),
            Ok(Check::Fuzz(_)) => Some(Evidence::Fuzzed),
            Ok(Check::Bounded(_)) => Some(Evidence::Bounded),
            Ok(Check::Prove) => Some(Evidence::Proved),
            Ok(Check::Mutate) | Err(_) => None,
        };
        if let Some(k) = kind {
            best = Some(best.map_or(k, |b: Evidence| b.max(k)));
        }
    }
    best.unwrap_or(Evidence::Unclaimed)
}

/// Builds this component's subtree as real `ply_kernel::VerdictNode`s: its
/// own fns as `Claimable` leaves (never `Violation` — a declared ceiling is
/// never earned evidence, so that rung is unreachable here), its nested
/// components recursively as `Container`s. Handed to the kernel's own
/// `aggregate` — never re-folded by hand — so the worst-of rule this draws
/// is the exact one The-Ply-Spec.md §7 pins and `tools/kernel` checks
/// exhaustively.
///
/// `inherited` is the §5.1 checks default `comp` itself inherited from
/// further up (`None` at the document root, or wherever no ancestor ever
/// declared one) — each fn's ceiling is computed from its *effective* list
/// (`ply_model::effective_checks`), and each nested component inherits
/// `comp`'s own default in turn (`ply_model::component_default_checks`).
fn component_verdict_node<'a>(
    name: &'a str,
    comp: &'a Component,
    inherited: Option<InheritedChecks<'a>>,
) -> VerdictNode {
    let this_default = component_default_checks(name, comp, inherited);
    let mut children: Vec<VerdictNode> = comp
        .fns
        .values()
        .map(|fc| VerdictNode {
            kind: NodeKind::Claimable(fn_declared_ceiling(effective_checks(fc, this_default))),
            statuses: BTreeSet::new(),
            conditional: None,
            children: Vec::new(),
        })
        .collect();
    children.extend(
        comp.components
            .iter()
            .map(|(cname, c)| component_verdict_node(cname, c, this_default)),
    );
    VerdictNode {
        kind: NodeKind::Container,
        statuses: BTreeSet::new(),
        conditional: None,
        children,
    }
}

fn component_ceiling(name: &str, comp: &Component, inherited: Option<InheritedChecks>) -> Evidence {
    aggregate(&component_verdict_node(name, comp, inherited)).evidence
}

// ---- §7.1 collapse / expand (`--depth N`, `--focus`, `--collapse`) --------

/// `ply-render`'s CLI options for collapsing (The-Ply-Spec.md §7.1, mirroring
/// `tree --depth`/`--focus`). The default (every field empty/`None`) must
/// render exactly as this renderer always has — every existing caller of
/// [`render_svg`] gets this by construction, since it is the option value
/// `render_svg` passes through.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderOptions {
    /// Components nested `depth` or more levels deep (top-level = level 1)
    /// collapse into one box, folding everything beneath them. `None` means
    /// unlimited — nothing ever collapses.
    pub depth: Option<usize>,
    /// A dotted or bare component name (§5.1a rule 6 ambiguity rules apply
    /// to the bare form). Its whole subtree renders fully expanded
    /// regardless of `depth`; every component outside its ancestor chain
    /// collapses at the point it diverges from that chain.
    pub focus: Option<String>,
    /// Dotted or bare component names (§5.1a rule 6 applies to each) that
    /// collapse regardless of `depth`/`focus`. The inverse selection bias to
    /// `focus`: everything *not* named here renders exactly as the default
    /// (fully expanded) would.
    pub collapse: Vec<String>,
}

/// Where a component (named by its qualified path) sits relative to the
/// `--focus` target's own qualified path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathRelation {
    /// This is the focus target itself.
    Focus,
    /// This is an ancestor of the focus target — on the path a reader must
    /// walk down to reach it, so it stays expanded (its own box drawn
    /// normally) even though most of its children collapse.
    Ancestor,
    /// This is inside the focus target's own subtree.
    Descendant,
    /// Neither: outside the focus path entirely.
    Unrelated,
}

/// Dot-boundary-aware prefix comparison — `"ingest"` is an ancestor of
/// `"ingest.book"`, but `"ingest_other"` is not (a plain `starts_with` would
/// wrongly say otherwise).
fn path_relation(qualified: &str, focus: &str) -> PathRelation {
    if qualified == focus {
        return PathRelation::Focus;
    }
    if let Some(rest) = focus.strip_prefix(qualified)
        && rest.starts_with('.')
    {
        return PathRelation::Ancestor;
    }
    if let Some(rest) = qualified.strip_prefix(focus)
        && rest.starts_with('.')
    {
        return PathRelation::Descendant;
    }
    PathRelation::Unrelated
}

/// Resolved once per render from [`RenderOptions`], then consulted at every
/// component boundary during the recursive walk.
struct CollapseCtx<'a> {
    depth: Option<usize>,
    /// The `--focus` argument, already resolved to its unambiguous fully
    /// qualified path (§5.1a rule 6) — never the raw, possibly-bare CLI
    /// token.
    focus: Option<&'a str>,
    /// The `--collapse` arguments, each already resolved the same way.
    /// Checked first and unconditionally: naming a component here always
    /// folds it, regardless of `depth`/`focus`.
    explicit: &'a [String],
}

impl<'a> CollapseCtx<'a> {
    /// §7.1: "depth 1 = only top-level boxes, their interiors folded" — a
    /// component at nesting level `level` (top-level = 1) collapses once
    /// `level >= depth`; levels below that stay expanded, showing their own
    /// children (which may in turn collapse one level further down). With
    /// `--focus`, the rule inverts: everything on the focus target's own
    /// ancestor/self/descendant chain always expands, and anything that
    /// diverges from that chain collapses immediately, regardless of level —
    /// "collapses to depth 1" read as "depth 1 relative to itself": the
    /// divergent branch's own top is exactly as far as a viewer is meant to
    /// see without asking for it by name. `--collapse` overrides both: an
    /// explicitly named component always folds, and names nothing else, so
    /// used alone (no `--depth`/`--focus`) it folds exactly what it names
    /// and leaves everything else exactly as the fully-expanded default.
    fn should_collapse(&self, qualified: &str, level: usize) -> bool {
        if self.explicit.iter().any(|p| p == qualified) {
            return true;
        }
        if let Some(focus) = self.focus {
            !matches!(
                path_relation(qualified, focus),
                PathRelation::Focus | PathRelation::Ancestor | PathRelation::Descendant
            )
        } else if let Some(depth) = self.depth {
            level >= depth
        } else {
            false
        }
    }
}

/// Every component's fully qualified dotted path in `doc`, depth-first,
/// declaration order — independent of rendering/collapsing, used to resolve
/// `--focus` and to redirect a collapsed-away edge endpoint to whichever
/// ancestor box is actually drawn.
fn all_qualified_paths(doc: &Document) -> Vec<String> {
    fn walk(qualified: &str, comp: &Component, out: &mut Vec<String>) {
        for (cname, child) in &comp.components {
            let q = format!("{qualified}.{cname}");
            out.push(q.clone());
            walk(&q, child, out);
        }
    }
    let mut out = Vec::new();
    for (name, comp) in &doc.components {
        out.push(name.clone());
        walk(name, comp, &mut out);
    }
    out
}

/// Resolves a `--focus`/`--collapse` CLI argument to an unambiguous fully
/// qualified component path, applying the same §5.1a rule 6 semantics
/// `resolve()` uses for edge/deny endpoints: a dotted token must match
/// exactly; a bare token must name a unique leaf across the whole document.
/// `flag` (e.g. `"--focus"`) names the offending flag in any error, so two
/// different flags sharing this resolver still read as themselves.
fn resolve_component_ref(flag: &str, token: &str, doc: &Document) -> Result<String, RenderError> {
    let paths = all_qualified_paths(doc);
    if token.contains('.') {
        return paths
            .iter()
            .find(|p| p.as_str() == token)
            .cloned()
            .ok_or_else(|| {
                RenderError(format!(
                    "{flag} {token:?} does not match any component in this document"
                ))
            });
    }
    let matches: Vec<String> = paths
        .iter()
        .filter(|p| p.rsplit('.').next() == Some(token))
        .cloned()
        .collect();
    match matches.as_slice() {
        [] => Err(RenderError(format!(
            "{flag} {token:?} does not match any component in this document"
        ))),
        [only] => Ok(only.clone()),
        many => Err(RenderError(format!(
            "{flag} {token:?} is ambiguous: it could mean {} — write the dotted form (e.g. \
             {}) to say which",
            join_or(many),
            many[0]
        ))),
    }
}

/// Recursive `(components, fns)` counts over everything *beneath* `comp`
/// (not counting `comp` itself as a component) — §7.1's collapsed-box
/// contents line, `N components · M fns`.
fn count_subtree(comp: &Component) -> (usize, usize) {
    let mut components = comp.components.len();
    let mut fns = comp.fns.len();
    for child in comp.components.values() {
        let (c, f) = count_subtree(child);
        components += c;
        fns += f;
    }
    (components, fns)
}

/// Union of every capability `comp`'s subtree (including `comp` itself)
/// declares, deduplicated and in first-appearance order — §7.1: "a collapsed
/// box containing `net` still shows `net`." A `pure` node's own `uses`
/// contributes nothing, matching the same masking `render_component` already
/// applies to an expanded box's badge row.
fn union_badges_subtree(comp: &Component) -> Vec<String> {
    fn walk(c: &Component, out: &mut Vec<String>) {
        if !c.pure {
            for u in &c.uses {
                if !out.contains(u) {
                    out.push(u.clone());
                }
            }
        }
        for child in c.components.values() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(comp, &mut out);
    out
}

/// Every fn-level unresolved marker (§5.6) anywhere in `comp`'s subtree, as
/// `(id, note)` pairs — folded into one pin glyph on a collapsed box.
fn collect_unresolved_subtree(comp: &Component) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for fc in comp.fns.values() {
        for u in &fc.unresolved {
            out.push((u.id, u.note.clone()));
        }
    }
    for child in comp.components.values() {
        out.extend(collect_unresolved_subtree(child));
    }
    out
}

/// Every `ply-check` finding attached to `comp` itself or any fn/component
/// in its subtree, marking each attached (via `ctx`) so a folded finding
/// never falls through to the workspace-title fallback count.
fn collect_findings_subtree<'a>(
    qualified: &str,
    comp: &Component,
    ctx: &FindingCtx<'a>,
) -> Vec<&'a Diagnostic> {
    let mut out = ctx.component_findings(qualified);
    for fname in comp.fns.keys() {
        out.extend(ctx.fn_findings(qualified, fname));
    }
    for (cname, child) in &comp.components {
        out.extend(collect_findings_subtree(
            &format!("{qualified}.{cname}"),
            child,
            ctx,
        ));
    }
    out
}

/// The CSS class that fills a component box for a given ceiling. `Violation`
/// has no declared-ceiling meaning (see `fn_declared_ceiling`'s doc comment)
/// and falls back to `unclaimed` defensively rather than being unreachable.
pub fn ceiling_class(e: Evidence) -> &'static str {
    match e {
        Evidence::Violation | Evidence::Unclaimed => "ceiling-unclaimed",
        Evidence::Tested => "ceiling-tested",
        Evidence::Fuzzed => "ceiling-fuzzed",
        Evidence::Bounded => "ceiling-bounded",
        Evidence::Proved => "ceiling-proved",
    }
}

/// Plain-language gloss for a non-`unclaimed` ceiling level, worded to read
/// naturally after "declares checks up to ".
fn ceiling_level_prose(e: Evidence) -> &'static str {
    match e {
        Evidence::Tested => {
            "tested — checked once against the declared examples and generated inputs"
        }
        Evidence::Fuzzed => "fuzzed — checked against many random inputs",
        Evidence::Bounded => "bounded — proved for every input up to a loop bound",
        Evidence::Proved => "proved — proved for every input, with no bound",
        Evidence::Violation | Evidence::Unclaimed => {
            unreachable!("callers branch on unclaimed before reaching this")
        }
    }
}

/// The component tooltip's declared-ceiling line (§7.1: "its tooltip says
/// none of it has run"). `unclaimed` gets its own plain sentence rather than
/// the "declares checks up to unclaimed" template, which would read as a
/// contradiction (there is nothing to declare "up to").
fn ceiling_tooltip_line(e: Evidence) -> String {
    match e {
        Evidence::Violation | Evidence::Unclaimed => {
            "no checks are declared anywhere in this component — nothing here is verified yet \
             (unclaimed)"
                .to_string()
        }
        other => format!(
            "declares checks up to {} — the strongest verdict this could earn; none of it has \
             been run yet",
            ceiling_level_prose(other)
        ),
    }
}

// ---- geometry --------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    /// Point on the rectangle's border in the direction of `target`, used as
    /// a cheap arrow endpoint so edges visually touch the box rather than
    /// crossing into it or floating outside it.
    fn border_toward(&self, target: (f64, f64)) -> (f64, f64) {
        let (cx, cy) = (self.cx(), self.cy());
        let (dx, dy) = (target.0 - cx, target.1 - cy);
        if dx == 0.0 && dy == 0.0 {
            return (cx, cy);
        }
        let hw = self.w / 2.0;
        let hh = self.h / 2.0;
        let scale = if dx.abs() * hh > dy.abs() * hw {
            hw / dx.abs()
        } else {
            hh / dy.abs()
        };
        (cx + dx * scale, cy + dy * scale)
    }
}

/// Slides a point already on `rect`'s border by `offset`, staying on
/// whichever edge (top/bottom vs. left/right) it started on and clamped
/// short of that edge's corners. Used to give parallel edges (§7.1
/// amendment, vetting 002 finding 4) a lane separation that holds at the
/// full requested distance — offsetting `border_toward`'s *target* instead
/// would work, but its scale shrinks that offset by however much closer
/// the border is than the target, which for our tall inter-rank gaps meant
/// a requested 16-unit lane landing only ~6 units apart.
fn offset_along_border(rect: Rect, point: (f64, f64), offset: (f64, f64)) -> (f64, f64) {
    const CORNER_CLEARANCE: f64 = 1.0;
    let on_horizontal_edge =
        (point.1 - rect.y).abs() < 0.5 || (point.1 - (rect.y + rect.h)).abs() < 0.5;
    if on_horizontal_edge {
        let x = (point.0 + offset.0).clamp(
            rect.x + CORNER_CLEARANCE,
            rect.x + rect.w - CORNER_CLEARANCE,
        );
        (x, point.1)
    } else {
        let y = (point.1 + offset.1).clamp(
            rect.y + CORNER_CLEARANCE,
            rect.y + rect.h - CORNER_CLEARANCE,
        );
        (point.0, y)
    }
}

/// Wrap a self-contained SVG fragment (drawn from its own local origin) in a
/// `<g transform="translate(...)">` at its final position.
fn wrap_translate(inner: &str, x: f64, y: f64) -> String {
    format!("<g transform=\"translate({x:.1},{y:.1})\">{inner}</g>")
}

/// One fn claim rendered as a chip: its box, name, checks glyph row,
/// `check_with` note, trusted shield, and unresolved pins — all drawn from
/// local origin `(0,0)` so the caller can place it with `wrap_translate`.
struct FnChip {
    width: f64,
    height: f64,
    svg: String,
}

fn render_fn_chip(
    name: &str,
    fc: &FnClaim,
    component_path: &str,
    ctx: &FindingCtx,
    inherited: Option<InheritedChecks>,
) -> FnChip {
    // §5.1: the list that actually governs this fn — its own if it declared
    // one, else the nearest ancestor component's default (or nothing, if it
    // has neither). Every downstream read of "this fn's checks" — the
    // glyph row, the tooltip prose, the declared-ceiling fill computed
    // elsewhere — goes through this, not `fc.checks` directly.
    let effective = effective_checks(fc, inherited);
    // `inherited` only ever carries a *non-empty* list (`component_default_
    // checks` never constructs one otherwise), so this fn's own list being
    // empty is exactly when `effective` came from that ancestor default.
    let is_inherited = fc.checks.is_empty() && inherited.is_some();
    let glyphs = checks_glyph_row(effective);
    let note = check_with_note(&fc.check_with);
    let has_shield = !fc.trusted.is_empty();
    let has_contract = !fc.requires.is_empty() || !fc.ensures.is_empty();
    let findings = ctx.fn_findings(component_path, name);

    let mut cursor_x = PAD;
    let mut inner = String::new();
    let text_y = CHIP_H / 2.0 + 4.0;

    // §7.1 "contract clauses" (amended): a gutter bar — full chip height,
    // flush at the left edge. The original 6x6 square was too easy to miss.
    if has_contract {
        inner.push_str(&format!(
            "<rect class=\"contract-mark\" x=\"0\" y=\"0\" width=\"{CONTRACT_MARK_W:.1}\" height=\"{CHIP_H:.1}\" />"
        ));
    }

    inner.push_str(&format!(
        "<text class=\"fn-name\" x=\"{cursor_x:.1}\" y=\"{text_y:.1}\">{}</text>",
        esc(name)
    ));
    cursor_x += text_w(name, CHIP_CHAR_W) + BADGE_GAP;

    if !glyphs.is_empty() {
        inner.push_str(&format!(
            "<text class=\"fn-checks\" x=\"{cursor_x:.1}\" y=\"{text_y:.1}\">{}</text>",
            esc(&glyphs)
        ));
        cursor_x += text_w(&glyphs, CHIP_CHAR_W) + BADGE_GAP;
    }

    if let Some(n) = &note {
        inner.push_str(&format!(
            "<text class=\"fn-check-with\" x=\"{cursor_x:.1}\" y=\"{text_y:.1}\">{}</text>",
            esc(n)
        ));
        cursor_x += text_w(n, CHIP_CHAR_W) + BADGE_GAP;
    }

    if has_shield {
        let claims = fc
            .trusted
            .iter()
            .map(|t| format!("{} — evidence: {}", t.claim, t.evidence))
            .collect::<Vec<_>>()
            .join("\n");
        inner.push_str(&format!(
            "<g class=\"fn-shield\">{}<text x=\"{cursor_x:.1}\" y=\"{text_y:.1}\">\u{26C9}</text></g>",
            title(&format!(
                "a human vouches for the claims below; no machine checks them\n{claims}"
            ))
        ));
        cursor_x += SHIELD_W + BADGE_GAP;
    }

    for p in &fc.unresolved {
        let label = format!("#{}", p.id);
        let cx = cursor_x + PIN_R;
        let pin_findings = ctx.unresolved_findings(p.id);
        let (pin_class, label_class) = if pin_findings.is_empty() {
            ("unresolved-pin", "pin-label")
        } else {
            ("unresolved-pin-finding", "pin-label-finding")
        };
        let mut pin_tip = finding_tooltip_lines(&pin_findings);
        pin_tip.push(unresolved_fn_pin_prose(p.id, &p.note));
        inner.push_str(&format!(
            "<g class=\"{pin_class}\">{tip}<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{PIN_R:.1}\" /><text class=\"{label_class}\" x=\"{cx:.1}\" y=\"{text_y:.1}\">{label}</text></g>",
            cy = CHIP_H / 2.0,
            label = esc(&label),
            tip = title(&pin_tip.join("\n"))
        ));
        cursor_x += text_w(&label, CHIP_CHAR_W) + PIN_R * 2.0 + BADGE_GAP;
    }

    let badge_w = finding_badge_width(&findings);
    if !findings.is_empty() {
        cursor_x += badge_w + BADGE_GAP;
    }

    let mut tip = finding_tooltip_lines(&findings);
    tip.push(name.to_string());
    if is_inherited {
        // §5.1 "checks: [...] # optional default checks for all fns in
        // scope": this fn declares none of its own, so the tooltip must say
        // plainly where the check actually came from — a newbie reading
        // "bounded(2)" with no explanation would have no way to find the
        // component that promised it.
        let from = inherited
            .expect("is_inherited implies inherited.is_some()")
            .from_component;
        for c in effective {
            tip.push(format!(
                "inherited from component `{from}`: {}",
                check_prose(c)
            ));
        }
    } else {
        for c in effective {
            tip.push(check_prose(c));
        }
    }
    if has_contract {
        // §7.1 "contract clauses" / §7.2 the watermark: this is the mark
        // itself made legible — the promise standing at the watermark line,
        // spelled out verbatim rather than left for the reader to infer
        // from the mark's presence alone.
        tip.push("contract at the watermark:".to_string());
        for r in &fc.requires {
            tip.push(format!("requires: {r}"));
        }
        for e in &fc.ensures {
            tip.push(format!("ensures: {e}"));
        }
        tip.push("the checks above test the function against exactly this promise".to_string());
    }
    if let Some(n) = &note {
        tip.push(format!(
            "generic — every check ran with {n}; the evidence covers only that type"
        ));
    }
    for t in &fc.trusted {
        tip.push(format!(
            "trusted (a human vouches for this; no machine checks it): {} — evidence: {}",
            t.claim, t.evidence
        ));
    }
    if !fc.examples.is_empty() {
        tip.push(format!(
            "{} worked example(s), each compiled into a test",
            fc.examples.len()
        ));
    }
    for p in &fc.unresolved {
        tip.push(unresolved_fn_pin_prose(p.id, &p.note));
    }
    if effective.is_empty() {
        tip.push("no checks declared — nothing about this function is verified (unclaimed)".into());
    }

    let width = cursor_x + PAD - BADGE_GAP;
    let box_class = if findings.is_empty() {
        "fn-chip-box"
    } else {
        "fn-chip-box-finding"
    };
    let badge_svg = if findings.is_empty() {
        String::new()
    } else {
        render_finding_badge(
            width - PAD - badge_w,
            (CHIP_H - FINDING_BADGE_H) / 2.0,
            &findings,
        )
    };
    let svg = format!(
        "<g class=\"fn-chip\" data-fn=\"{}\">{}<rect class=\"{box_class}\" x=\"0\" y=\"0\" width=\"{width:.1}\" height=\"{CHIP_H:.1}\" rx=\"4\" />{inner}{badge_svg}</g>",
        esc(name),
        title(&tip.join("\n"))
    );

    FnChip {
        width,
        height: CHIP_H,
        svg,
    }
}

/// The component tooltip lines that depend only on this node's own declared
/// fields — findings, the anchor line, pure/capabilities, owns, profile,
/// strict — shared verbatim by both an expanded box (`render_component`) and
/// a collapsed one (`render_collapsed_component`, §7.1: "the normal
/// component tooltip lines PLUS a plain [collapse] line ... and the ceiling
/// line"). Callers append their own ceiling line (and, for an expanded box
/// only, the hollow line) afterward — this deliberately stops short of both,
/// since a collapsed box inserts its own extra line between them.
fn component_tip_lines(
    name: &str,
    comp: &Component,
    profiles: &IndexMap<String, Vec<String>>,
    findings: &[&Diagnostic],
) -> Vec<String> {
    let mut tip = finding_tooltip_lines(findings);
    tip.push(format!(
        "component {name} — maps to Rust module {}",
        comp.anchor
    ));
    if comp.pure {
        tip.push(
            "pure — the double border is the seal: this component declares no capabilities \
             and may not use any; capability use inside it is an error (A0408)"
                .into(),
        );
    } else if !comp.uses.is_empty() {
        tip.push(format!("capabilities: {}", comp.uses.join(", ")));
    }
    if !comp.owns.is_empty() {
        tip.push(format!(
            "owns {} — only this component may mutate them",
            comp.owns.join(", ")
        ));
    }
    if let Some(p) = &comp.profile {
        tip.push(match profiles.get(p) {
            Some(rules) => format!(
                "profile {p} — a named bundle of extra rules this component must follow: {}",
                profile_rules_prose(rules)
            ),
            None => format!("profile {p} (not defined in this document)"),
        });
    }
    if comp.strict {
        tip.push(
            "strict — architecture findings inside this component fail the build \
             (errors, not warnings)"
                .into(),
        );
    }
    tip
}

/// One component rendered as a box (§7.1: nesting -> nested boxes), drawn
/// from local origin `(0,0)`.
struct ComponentBox {
    width: f64,
    height: f64,
    svg: String,
    /// leaf name -> rect, relative to this box's own local origin. Used by
    /// the caller to translate into absolute canvas coordinates for edges.
    positions: Vec<(String, Rect)>,
}

/// Everything the recursive component walk threads unchanged through every
/// level: capability profiles, the findings lookup, the collapse decisions,
/// and the document's edge list (consulted for a box's internal layout).
struct WalkCtx<'a> {
    profiles: &'a IndexMap<String, Vec<String>>,
    findings: &'a FindingCtx<'a>,
    collapse: &'a CollapseCtx<'a>,
    edges: &'a [String],
}

/// Picks between the two component renderers for one box: collapsed
/// (§7.1's "one solid-bordered box ... folded") if `collapse` says so and
/// there is actually something to fold, expanded (`render_component`)
/// otherwise. A hollow component (no fns, no nested components) never
/// collapses — "hollow means nothing inside; collapsed means plenty inside,
/// folded" are mutually exclusive states, and hollow wins when the subtree
/// really is empty.
fn render_component_dispatch<'a>(
    name: &'a str,
    qualified: &str,
    comp: &'a Component,
    walk: &WalkCtx,
    level: usize,
    inherited: Option<InheritedChecks<'a>>,
) -> ComponentBox {
    let is_hollow = comp.fns.is_empty() && comp.components.is_empty();
    if !is_hollow && walk.collapse.should_collapse(qualified, level) {
        render_collapsed_component(
            name,
            qualified,
            comp,
            walk.profiles,
            walk.findings,
            inherited,
        )
    } else {
        render_component(name, qualified, comp, walk, level, inherited)
    }
}

/// A collapsed component (The-Ply-Spec.md §7.1): one solid-bordered box
/// showing name, anchor, a `N components · M fns` contents line, and the
/// same worst-descendant ceiling fill an expanded box would earn (computed
/// over the *full* subtree, via `component_ceiling`, exactly as an expanded
/// box's fill already is). Three things never fold away: the subtree's
/// unioned capability badges, one pin glyph carrying the subtree's total
/// unresolved-marker count, and — via `collect_findings_subtree` — the
/// subtree's finding count. No fn chips, no nested boxes: everything below
/// this box is folded into those counts.
fn render_collapsed_component(
    name: &str,
    qualified: &str,
    comp: &Component,
    profiles: &IndexMap<String, Vec<String>>,
    ctx: &FindingCtx,
    inherited: Option<InheritedChecks>,
) -> ComponentBox {
    let findings = collect_findings_subtree(qualified, comp, ctx);
    let ceiling = component_ceiling(name, comp, inherited);
    let (n_components, n_fns) = count_subtree(comp);
    let contents_line = format!(
        "{n_components} component{} \u{b7} {n_fns} fn{}",
        if n_components == 1 { "" } else { "s" },
        if n_fns == 1 { "" } else { "s" },
    );
    let badges = union_badges_subtree(comp);
    let unresolved = collect_unresolved_subtree(comp);

    let finding_badge_w = finding_badge_width(&findings);
    let name_w = text_w(name, NAME_CHAR_W)
        + if findings.is_empty() {
            0.0
        } else {
            BADGE_GAP + finding_badge_w
        };
    let anchor_w = text_w(&comp.anchor, SUB_CHAR_W);
    let contents_w = text_w(&contents_line, SUB_CHAR_W);
    // Three header lines: name, anchor, contents — one more than the
    // baseline two-line header (`render_component`'s `owns_line` grows its
    // own header the same way, for the same reason).
    let header_h = HEADER_H + LINE_H;

    let badges_row_w: f64 = badges
        .iter()
        .map(|b| text_w(b, BADGE_CHAR_W) + BADGE_PAD * 2.0 + BADGE_GAP)
        .sum();
    let pin_label = unresolved.len().to_string();
    let pin_w = if unresolved.is_empty() {
        0.0
    } else {
        PIN_R * 2.0 + text_w(&pin_label, CHIP_CHAR_W) + BADGE_GAP
    };
    let footer_row_w = badges_row_w + pin_w;
    let footer_row_h = if footer_row_w > 0.0 {
        BADGE_H.max(PIN_R * 2.0) + GAP
    } else {
        0.0
    };

    let content_w = [
        name_w,
        anchor_w,
        contents_w,
        footer_row_w,
        MIN_BOX_W - PAD * 2.0,
    ]
    .into_iter()
    .fold(0.0_f64, f64::max);
    let box_w = content_w + PAD * 2.0;
    let box_h = PAD + header_h + footer_row_h + PAD;

    let mut body = String::new();
    let y = PAD + header_h;
    if footer_row_w > 0.0 {
        let mut bx = PAD;
        for b in &badges {
            let bw = text_w(b, BADGE_CHAR_W) + BADGE_PAD * 2.0;
            body.push_str(&format!(
                "<g class=\"cap-badge\">{tip}<rect x=\"{bx:.1}\" y=\"{y:.1}\" width=\"{bw:.1}\" height=\"{BADGE_H:.1}\" rx=\"3\" /><text x=\"{tx:.1}\" y=\"{ty:.1}\">{label}</text></g>",
                tx = bx + BADGE_PAD,
                ty = y + BADGE_H - 6.0,
                label = esc(b),
                tip = title(&format!(
                    "this component may use `{b}` somewhere in its folded subtree. A \
                     component may use only the capabilities it declares — using an \
                     undeclared one is an architecture finding (§5.3, A0404)."
                ))
            ));
            bx += bw + BADGE_GAP;
        }
        if !unresolved.is_empty() {
            let cx = bx + PIN_R;
            let cy = y + BADGE_H / 2.0;
            let mut pin_tip = vec![format!(
                "{} unresolved decision{} folded inside this collapsed component:",
                unresolved.len(),
                if unresolved.len() == 1 { "" } else { "s" }
            )];
            for (id, note) in &unresolved {
                pin_tip.push(format!("#{id} — {note}"));
            }
            body.push_str(&format!(
                "<g class=\"unresolved-pin\">{tip}<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{PIN_R:.1}\" /><text class=\"pin-label\" x=\"{cx:.1}\" y=\"{ty:.1}\">{label}</text></g>",
                ty = cy + 4.0,
                label = esc(&pin_label),
                tip = title(&pin_tip.join("\n"))
            ));
        }
    }

    let mut tip = component_tip_lines(name, comp, profiles, &findings);
    tip.push(format!(
        "collapsed — {n_components} component{} and {n_fns} function{} folded inside; render \
         with --depth or --focus <name> to expand",
        if n_components == 1 { "" } else { "s" },
        if n_fns == 1 { "" } else { "s" },
    ));
    tip.push(ceiling_tooltip_line(ceiling));

    let component_box_class = format!(
        "{} {}",
        if findings.is_empty() {
            "component-box"
        } else {
            "component-box-finding"
        },
        ceiling_class(ceiling)
    );
    let mut svg = format!(
        "<g class=\"component\" data-name=\"{}\">{}<rect class=\"collapsed-stack {ceiling}\" x=\"{STACK_OFFSET:.1}\" y=\"{STACK_OFFSET:.1}\" width=\"{box_w:.1}\" height=\"{box_h:.1}\" rx=\"6\" /><rect class=\"{component_box_class}\" x=\"0\" y=\"0\" width=\"{box_w:.1}\" height=\"{box_h:.1}\" rx=\"6\" />",
        esc(name),
        title(&tip.join("\n")),
        ceiling = ceiling_class(ceiling)
    );
    svg.push_str(&format!(
        "<text class=\"component-name\" x=\"{PAD:.1}\" y=\"{:.1}\">{}</text>",
        PAD + LINE_H - 2.0,
        esc(name)
    ));
    if !findings.is_empty() {
        svg.push_str(&render_finding_badge(
            box_w - PAD - finding_badge_w,
            PAD - 2.0,
            &findings,
        ));
    }
    svg.push_str(&format!(
        "<text class=\"component-anchor\" x=\"{PAD:.1}\" y=\"{:.1}\">{}</text>",
        PAD + LINE_H * 2.0 - 4.0,
        esc(&comp.anchor)
    ));
    svg.push_str(&format!(
        "<text class=\"component-anchor\" x=\"{PAD:.1}\" y=\"{:.1}\">{}</text>",
        PAD + LINE_H * 3.0 - 6.0,
        esc(&contents_line)
    ));
    svg.push_str(&body);
    svg.push_str("</g>");

    ComponentBox {
        width: box_w,
        height: box_h,
        svg,
        positions: Vec::new(),
    }
}

/// Edges between two of `comp`'s own *direct* children, written (per the
/// grammar's scoping rule — vetting 003's "Scoping gap" observation) in
/// full dotted form at the document's top level (`qualified.child_a ->
/// qualified.child_b`). Anything else — a grandchild reference, a
/// cross-container edge, an edge naming this component itself — is not
/// this component's concern.
fn internal_child_edges(
    qualified: &str,
    comp: &Component,
    edges: &[String],
) -> Vec<(String, String)> {
    let prefix = format!("{qualified}.");
    let mut out = Vec::new();
    for e in edges {
        let Ok(edge) = parse_edge(e) else { continue };
        let (Some(from_leaf), Some(to_leaf)) = (
            edge.from.strip_prefix(&prefix),
            edge.to.strip_prefix(&prefix),
        ) else {
            continue;
        };
        if from_leaf.contains('.') || to_leaf.contains('.') {
            continue; // a grandchild, not a direct child of `comp`
        }
        if comp.components.contains_key(from_leaf) && comp.components.contains_key(to_leaf) {
            out.push((from_leaf.to_string(), to_leaf.to_string()));
        }
    }
    out
}

fn render_component<'a>(
    name: &'a str,
    qualified: &str,
    comp: &'a Component,
    walk: &WalkCtx,
    level: usize,
    inherited: Option<InheritedChecks<'a>>,
) -> ComponentBox {
    let WalkCtx {
        profiles,
        findings: ctx,
        edges,
        ..
    } = *walk;
    let findings = ctx.component_findings(qualified);
    // §7.1 "declared ceiling": the strongest verdict this component's own
    // declared checks could earn, folded worst-of over every fn in its
    // subtree by the real kernel `aggregate` (see `component_ceiling`).
    let ceiling = component_ceiling(name, comp, inherited);
    // §5.1: what this component's own fns (and any nested component that
    // declares no default of its own) inherit — threaded to the fn chips
    // and nested boxes below.
    let this_default = component_default_checks(name, comp, inherited);
    let finding_badge_w = finding_badge_width(&findings);
    let name_w = text_w(name, NAME_CHAR_W)
        + if findings.is_empty() {
            0.0
        } else {
            BADGE_GAP + finding_badge_w
        };
    let anchor_w = text_w(&comp.anchor, SUB_CHAR_W);
    // §7.1: `owns` is a third header line, `owns T, U` — the types this
    // component is the sole mutator of.
    let owns_line = (!comp.owns.is_empty()).then(|| format!("owns {}", comp.owns.join(", ")));
    let owns_w = owns_line.as_deref().map_or(0.0, |s| text_w(s, SUB_CHAR_W));
    let header_h = HEADER_H + if owns_line.is_some() { LINE_H } else { 0.0 };

    // §7.1: `pure` is a sealed border with no capability badges.
    let badges: &[String] = if comp.pure { &[] } else { &comp.uses };
    let badges_row_w: f64 = badges
        .iter()
        .map(|b| text_w(b, BADGE_CHAR_W) + BADGE_PAD * 2.0 + BADGE_GAP)
        .sum();
    let profile_w = comp
        .profile
        .as_ref()
        .map(|p| text_w(p, BADGE_CHAR_W) + BADGE_PAD * 2.0)
        .unwrap_or(0.0);
    let badge_row_h = if badges.is_empty() && comp.profile.is_none() {
        0.0
    } else {
        BADGE_H + GAP
    };

    let children: Vec<(String, ComponentBox)> = comp
        .components
        .iter()
        .map(|(cname, c)| {
            let child_qualified = format!("{qualified}.{cname}");
            (
                cname.clone(),
                render_component_dispatch(
                    cname,
                    &child_qualified,
                    c,
                    walk,
                    level + 1,
                    this_default,
                ),
            )
        })
        .collect();

    let chips: Vec<(String, FnChip)> = comp
        .fns
        .iter()
        .map(|(fname, fc)| {
            (
                fname.clone(),
                render_fn_chip(fname, fc, qualified, ctx, this_default),
            )
        })
        .collect();

    // §7.1 amendment (vetting 003 finding 1): when this component's own
    // children call/flow to each other, lay them out the same ranked way
    // the top-level graph is (see `layout.rs`) — same generous `RANKSEP`
    // gap, so a flow-edge label between two stacked children has the same
    // slack top-level labels already get, instead of colliding with
    // whichever child sits `GAP` (12px) below. A container with no
    // internal edges between its children keeps the original plain
    // vertical stack — nothing changes for the (overwhelming) common case.
    let internal_edges = internal_child_edges(qualified, comp, edges);
    let child_layout = (children.len() >= 2 && !internal_edges.is_empty()).then(|| {
        let names: Vec<String> = children.iter().map(|(n, _)| n.clone()).collect();
        let sizes: IndexMap<String, (f64, f64)> = children
            .iter()
            .map(|(n, c)| (n.clone(), (c.width, c.height)))
            .collect();
        layout::layered_layout(&names, &internal_edges, &sizes, RANKSEP, NODESEP)
    });

    let content_w = [
        name_w,
        anchor_w,
        owns_w,
        badges_row_w + profile_w,
        MIN_BOX_W - PAD * 2.0,
    ]
    .into_iter()
    .fold(0.0_f64, f64::max)
    .max(match &child_layout {
        Some(l) => l.content_w,
        None => children.iter().map(|(_, c)| c.width).fold(0.0, f64::max),
    })
    .max(chips.iter().map(|(_, c)| c.width).fold(0.0, f64::max));

    let box_w = content_w + PAD * 2.0;

    let mut y = PAD + header_h;
    let mut body = String::new();
    let mut positions: Vec<(String, Rect)> = Vec::new();

    if badge_row_h > 0.0 {
        let mut bx = PAD;
        for b in badges {
            let bw = text_w(b, BADGE_CHAR_W) + BADGE_PAD * 2.0;
            body.push_str(&format!(
                "<g class=\"cap-badge\">{tip}<rect x=\"{bx:.1}\" y=\"{y:.1}\" width=\"{bw:.1}\" height=\"{BADGE_H:.1}\" rx=\"3\" /><text x=\"{tx:.1}\" y=\"{ty:.1}\">{label}</text></g>",
                tx = bx + BADGE_PAD,
                ty = y + BADGE_H - 6.0,
                label = esc(b),
                tip = title(&format!(
                    "this component may use `{b}`. A component may use only the capabilities \
                     it declares — using an undeclared one is an architecture finding \
                     (§5.3, A0404)."
                ))
            ));
            bx += bw + BADGE_GAP;
        }
        if let Some(p) = &comp.profile {
            let pw = text_w(p, BADGE_CHAR_W) + BADGE_PAD * 2.0;
            body.push_str(&format!(
                "<g class=\"profile-tag\">{tip}<rect x=\"{bx:.1}\" y=\"{y:.1}\" width=\"{pw:.1}\" height=\"{BADGE_H:.1}\" rx=\"3\" /><text x=\"{tx:.1}\" y=\"{ty:.1}\">{label}</text></g>",
                tx = bx + BADGE_PAD,
                ty = y + BADGE_H - 6.0,
                label = esc(p),
                tip = title(&match profiles.get(p) {
                    Some(rules) => format!(
                        "profile `{p}` — a named bundle of extra rules this component must \
                         follow: {}",
                        profile_rules_prose(rules)
                    ),
                    None => format!("profile `{p}` — not defined in this document"),
                })
            ));
        }
        y += badge_row_h;
    }

    match &child_layout {
        Some(layered) => {
            for (cname, cbox) in &children {
                let (rel_x, rel_y) = layered.positions[cname];
                let (cx, cy) = (PAD + rel_x, y + rel_y);
                body.push_str(&wrap_translate(&cbox.svg, cx, cy));
                positions.push((
                    cname.clone(),
                    Rect {
                        x: cx,
                        y: cy,
                        w: cbox.width,
                        h: cbox.height,
                    },
                ));
                for (n, r) in &cbox.positions {
                    positions.push((
                        n.clone(),
                        Rect {
                            x: cx + r.x,
                            y: cy + r.y,
                            w: r.w,
                            h: r.h,
                        },
                    ));
                }
            }
            y += layered.content_h + GAP;
        }
        None => {
            for (cname, cbox) in children {
                body.push_str(&wrap_translate(&cbox.svg, PAD, y));
                positions.push((
                    cname.clone(),
                    Rect {
                        x: PAD,
                        y,
                        w: cbox.width,
                        h: cbox.height,
                    },
                ));
                for (n, r) in cbox.positions {
                    positions.push((
                        n,
                        Rect {
                            x: PAD + r.x,
                            y: y + r.y,
                            w: r.w,
                            h: r.h,
                        },
                    ));
                }
                y += cbox.height + GAP;
            }
        }
    }

    // fn claims are not edge/deny endpoints, so unlike nested components,
    // their names are not recorded in `positions`.
    for (_, chip) in chips {
        body.push_str(&wrap_translate(&chip.svg, PAD, y));
        y += chip.height + GAP;
    }

    let box_h = y + PAD;

    let mut tip = component_tip_lines(name, comp, profiles, &findings);
    tip.push(ceiling_tooltip_line(ceiling));

    // §7.1 "hollow component": derived from absence — nothing declared
    // inside means a dashed sketch outline, the opposite claim to a
    // collapsed box (plenty inside, folded away), which stays solid.
    let is_hollow = comp.fns.is_empty() && comp.components.is_empty();
    if is_hollow {
        tip.push(
            "hollow — declares nothing inside yet: no functions, no nested components. \
             Nothing to zoom into; a sketch waiting for claims."
                .into(),
        );
    }

    let component_box_class = format!(
        "{} {}{}",
        if findings.is_empty() {
            "component-box"
        } else {
            "component-box-finding"
        },
        ceiling_class(ceiling),
        if is_hollow { " hollow-box" } else { "" }
    );
    let mut svg = format!(
        "<g class=\"component\" data-name=\"{}\">{}<rect class=\"{component_box_class}\" x=\"0\" y=\"0\" width=\"{box_w:.1}\" height=\"{box_h:.1}\" rx=\"6\" />",
        esc(name),
        title(&tip.join("\n"))
    );
    if comp.pure {
        // §7.1: `pure` renders as a sealed (double) border, badge-free.
        svg.push_str(&format!(
            "<rect class=\"pure-seal\" x=\"4\" y=\"4\" width=\"{:.1}\" height=\"{:.1}\" rx=\"4\" />",
            box_w - 8.0,
            box_h - 8.0
        ));
    }
    svg.push_str(&format!(
        "<text class=\"component-name\" x=\"{PAD:.1}\" y=\"{:.1}\">{}</text>",
        PAD + LINE_H - 2.0,
        esc(name)
    ));
    if !findings.is_empty() {
        svg.push_str(&render_finding_badge(
            box_w - PAD - finding_badge_w,
            PAD - 2.0,
            &findings,
        ));
    }
    svg.push_str(&format!(
        "<text class=\"component-anchor\" x=\"{PAD:.1}\" y=\"{:.1}\">{}</text>",
        PAD + LINE_H * 2.0 - 4.0,
        esc(&comp.anchor)
    ));
    if let Some(line) = &owns_line {
        svg.push_str(&format!(
            "<text class=\"component-owns\" x=\"{PAD:.1}\" y=\"{:.1}\">{}</text>",
            PAD + LINE_H * 3.0 - 6.0,
            esc(line)
        ));
    }
    svg.push_str(&body);
    svg.push_str("</g>");

    ComponentBox {
        width: box_w,
        height: box_h,
        svg,
        positions,
    }
}

fn any_node_svg(x: f64, y: f64) -> String {
    format!(
        "<g class=\"any-node\">{tip}<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"{ANY_R:.1}\" /><text class=\"any-label\" x=\"{x:.1}\" y=\"{:.1}\">*</text></g>",
        y + 4.0,
        tip = title("`*` stands for every component")
    )
}

/// A `ply.yaml` document that failed to render — currently only raised for
/// an ambiguous bare component reference in an edge or deny string (§5.1a
/// rule 6; `E0206` in the full tool). This renderer refuses to guess.
#[derive(Debug)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RenderError {}

/// Resolves an edge/deny endpoint token to the component it names (§5.1a
/// rule 6): a token containing `.` is a fully qualified path (`parent.child`),
/// matched exactly against `positions`; a bare token (no `.`) resolves only
/// if its leaf name is unique across the whole merged component tree —
/// otherwise it is `E0206 ambiguous component reference`, naming every
/// qualified path it could mean. Returns `Ok(None)` for a token that matches
/// nothing at all (the caller treats that edge/deny as a no-op, consistent
/// with this renderer's philosophy of drawing exactly what resolves and
/// nothing more). The qualified name comes back alongside the rect so
/// callers can group edges by the component pair they actually connect
/// (parallel-lane offsetting) without re-deriving it.
fn resolve(
    token: &str,
    positions: &IndexMap<String, Rect>,
    leaf_index: &IndexMap<String, Vec<String>>,
) -> Result<Option<(String, Rect)>, RenderError> {
    if token.contains('.') {
        return Ok(positions.get(token).map(|r| (token.to_string(), *r)));
    }
    match leaf_index.get(token) {
        None => Ok(None),
        Some(paths) if paths.len() == 1 => {
            Ok(positions.get(&paths[0]).map(|r| (paths[0].clone(), *r)))
        }
        Some(paths) => Err(RenderError(format!(
            "ambiguous component reference {token:?}: it could mean {} — write the dotted \
             form (e.g. {}) to say which",
            join_or(paths),
            paths[0]
        ))),
    }
}

/// Groups edges/deny rules by the unordered pair of components they connect,
/// so parallel edges between the same pair (vetting 002 finding 4: call +
/// flow, or opposite directions) can be assigned distinct lanes.
fn pair_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// `idx`-th of `total` parallel lanes, centered on the centerline (so 2
/// lanes sit at `-gap/2`/`+gap/2`, 3 at `-gap`/`0`/`+gap`, etc).
fn lane_offset(idx: usize, total: usize, gap: f64) -> f64 {
    (idx as f64 - (total as f64 - 1.0) / 2.0) * gap
}

/// Worst-case (middle-anchored, `NAME_CHAR_W`-wide) bounding box for a
/// flow-edge label centered at `pos`, checked against every real component
/// box for overlap — used to pick whichever side of the line a label's
/// perpendicular push actually lands clear on.
fn label_clashes_with_any_box(
    pos: (f64, f64),
    text: &str,
    positions: &IndexMap<String, Rect>,
) -> bool {
    let half_w = text_w(text, NAME_CHAR_W) / 2.0;
    let (lx0, lx1) = (pos.0 - half_w, pos.0 + half_w);
    let (ly0, ly1) = (pos.1 - 11.0, pos.1 + 3.0);
    positions
        .values()
        .any(|r| lx0 < r.x + r.w && lx1 > r.x && ly0 < r.y + r.h && ly1 > r.y)
}

/// The stable entry point every existing caller uses: fully expanded,
/// unchanged since before The-Ply-Spec.md §7.1's collapse/expand feature
/// existed. A thin wrapper over [`render_svg_with_options`] with the default
/// options (`depth: None, focus: None`), which that function treats as
/// "never collapse anything" — so this is byte-for-byte what it always was.
pub fn render_svg(doc: &Document) -> Result<String, RenderError> {
    render_svg_with_options(doc, &RenderOptions::default())
}

/// `render_svg`, plus The-Ply-Spec.md §7.1's `--depth`/`--focus`/`--collapse`
/// collapsing.
pub fn render_svg_with_options(
    doc: &Document,
    options: &RenderOptions,
) -> Result<String, RenderError> {
    // §7.1 "finding (tool-computed, not declared)": run `ply-check`'s
    // document-local rules up front, then thread `ctx` through every render
    // function below so it can mark red whatever a finding attaches to and
    // tally whatever it doesn't (`ctx.unattached_count()`, consulted once
    // rendering finishes). A clean document (`findings` empty) makes every
    // lookup below return nothing, so nothing about its output changes.
    let findings = run_checks(doc);
    let ctx = FindingCtx::new(&findings);

    // §7.1 collapse/expand: resolved once, consulted at every component
    // boundary below. `collapsing_active` is false exactly when `options` is
    // the default — every branch gated on it is therefore dead code on the
    // default path, which is what keeps `render_svg` byte-for-byte unchanged.
    let focus_qualified: Option<String> = match &options.focus {
        Some(f) => Some(resolve_component_ref("--focus", f, doc)?),
        None => None,
    };
    let mut explicit_collapse: Vec<String> = Vec::new();
    for token in &options.collapse {
        explicit_collapse.push(resolve_component_ref("--collapse", token, doc)?);
    }
    let collapse = CollapseCtx {
        depth: options.depth,
        focus: focus_qualified.as_deref(),
        explicit: &explicit_collapse,
    };
    let collapsing_active =
        options.depth.is_some() || options.focus.is_some() || !options.collapse.is_empty();

    let top: Vec<(String, ComponentBox)> = doc
        .components
        .iter()
        .map(|(name, c)| {
            (
                name.clone(),
                render_component_dispatch(
                    name,
                    name,
                    c,
                    &WalkCtx {
                        profiles: &doc.profiles,
                        findings: &ctx,
                        collapse: &collapse,
                        edges: &doc.edges,
                    },
                    1,
                    None,
                ),
            )
        })
        .collect();

    // ---- qualified-name resolution, position-independent --------------
    // `resolve()` only needs a positions map's *keys* (for the dotted-path
    // branch) and a leaf index built from those same keys (for the bare-name
    // branch) to settle §5.1a rule 6 ambiguity; the Rect values themselves
    // are unused until Phase 2 below, so a placeholder positions map lets us
    // resolve edges once, early, for ranking — and again later against real
    // coordinates, guaranteed to agree since both are the same lookup over
    // the same keys.
    let mut prelim_positions: IndexMap<String, Rect> = IndexMap::new();
    for (name, cbox) in &top {
        prelim_positions.entry(name.clone()).or_insert(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        for (n, _) in &cbox.positions {
            prelim_positions
                .entry(format!("{name}.{n}"))
                .or_insert(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                });
        }
    }
    let prelim_leaf_index = leaf_index_of(&prelim_positions);

    // ---- layered top-level layout (finding 1: stub call edges) ---------
    // Every edge endpoint maps to the top-level ancestor of the component it
    // resolves to — a nested component moves with its parent box, so only
    // top-level placement needs ranking (see layout.rs).
    let top_names: Vec<String> = top.iter().map(|(n, _)| n.clone()).collect();
    let mut rank_edges: Vec<(String, String)> = Vec::new();
    for e in &doc.edges {
        let Ok(edge) = parse_edge(e) else { continue };
        let Some((from_q, _)) = resolve(&edge.from, &prelim_positions, &prelim_leaf_index)? else {
            continue;
        };
        let Some((to_q, _)) = resolve(&edge.to, &prelim_positions, &prelim_leaf_index)? else {
            continue;
        };
        let top_of = |q: &str| q.split('.').next().unwrap_or(q).to_string();
        rank_edges.push((top_of(&from_q), top_of(&to_q)));
    }
    let sizes: IndexMap<String, (f64, f64)> = top
        .iter()
        .map(|(n, c)| (n.clone(), (c.width, c.height)))
        .collect();
    let layered = layout::layered_layout(&top_names, &rank_edges, &sizes, RANKSEP, NODESEP);

    // ---- deny-driven margins (finding 3: clipped deny nodes) -----------
    // A `*` pseudo-node needs a reserved band clear of both the frame edge
    // and the nearest real box, wide enough for itself, a gap on each side,
    // and its own `except` label — sized from the label's actual text
    // rather than a guessed constant, so the reservation is always enough.
    // Kept alongside each deny's position in `doc.deny` (not the position in
    // this filtered vec) so a finding on an unparseable deny string — which
    // never makes it into this vec at all — still resolves to the right
    // `DenyIndex`, and so a parseable-but-flagged one attaches to the deny
    // it actually is rather than whichever survived the filter next to it.
    let parsed_denies: Vec<(usize, Deny)> = doc
        .deny
        .iter()
        .enumerate()
        .filter_map(|(i, d)| parse_deny(d).ok().map(|pd| (i, pd)))
        .collect();
    let mut extra_left = 0.0_f64;
    let mut extra_right = 0.0_f64;
    for (_, d) in &parsed_denies {
        // Reserved with the *worst-case* character width (`NAME_CHAR_W`,
        // the widest this renderer uses), not the narrower width the label
        // is actually drawn at (`SUB_CHAR_W`) — the same over-provisioning
        // `everything_renders_inside_the_canvas` already assumes when
        // checking the canvas boundary. Reserving exactly the drawn width
        // left zero clearance between the label and the first real box
        // (vetting 003 finding 4's "same clearance family").
        let label_w = if d.except.is_empty() {
            0.0
        } else {
            text_w(&format!("except {}", d.except.join(", ")), NAME_CHAR_W)
        };
        let needed = ANY_R * 2.0 + ANY_GAP * 2.0 + label_w;
        if d.from == "*" {
            extra_left = extra_left.max(needed);
        }
        if d.to == "*" {
            extra_right = extra_right.max(needed);
        }
    }

    // ---- absolute placement ---------------------------------------------
    let content_top = FRAME_PAD + FRAME_TITLE_H;
    let content_left = FRAME_PAD + extra_left;
    let mut body = String::new();
    // Absolute canvas position of every named component, keyed by its fully
    // qualified dotted path (`parent.child`; a top-level component's own
    // name, with no prefix, is its own qualified path). §5.1a rule 6: an
    // edge/deny endpoint may use the bare leaf name only when that name is
    // unique across the whole merged tree; `leaf_index` (built below) is
    // what makes that uniqueness check possible.
    let mut positions: IndexMap<String, Rect> = IndexMap::new();
    for (name, cbox) in &top {
        let (rel_x, rel_y) = layered.positions[name];
        let x = content_left + rel_x;
        let y = content_top + rel_y;
        body.push_str(&wrap_translate(&cbox.svg, x, y));
        positions.entry(name.clone()).or_insert(Rect {
            x,
            y,
            w: cbox.width,
            h: cbox.height,
        });
        for (n, r) in &cbox.positions {
            positions.entry(format!("{name}.{n}")).or_insert(Rect {
                x: x + r.x,
                y: y + r.y,
                w: r.w,
                h: r.h,
            });
        }
    }

    // §7.1: "an edge whose endpoint is inside a collapsed component
    // reattaches to the collapsed box itself." A qualified path with no
    // entry above (folded away because an ancestor collapsed) redirects to
    // the nearest ancestor that *does* have one — its box is what actually
    // gets drawn. `effective` remembers that redirect target so edge
    // resolution below can tell "two different endpoints" from "two
    // endpoints that now land on the same box" (needed for the dedup and
    // self-loop rules a few lines down); `positions` gets the same rect
    // under the folded-away key too, so `resolve()` itself needs no change.
    let mut effective: HashMap<String, String> = HashMap::new();
    if collapsing_active {
        for p in all_qualified_paths(doc) {
            if positions.contains_key(&p) {
                effective.insert(p.clone(), p);
                continue;
            }
            let mut cur = p.as_str();
            while let Some(idx) = cur.rfind('.') {
                cur = &cur[..idx];
                if let Some(&rect) = positions.get(cur) {
                    positions.insert(p.clone(), rect);
                    effective.insert(p.clone(), cur.to_string());
                    break;
                }
            }
        }
    }
    let leaf_index = leaf_index_of(&positions);

    let frame_w = content_left + layered.content_w + extra_right + FRAME_PAD;
    let frame_h = content_top + layered.content_h + FRAME_PAD;
    // Fixed positions inside the reserved deny margins (always on-canvas
    // and clear of every real box, by construction of `extra_left`/`extra_right`).
    let any_x_from = FRAME_PAD + ANY_R + ANY_GAP;
    let any_x_to = content_left + layered.content_w + ANY_GAP + ANY_R;

    // §7.1 unresolved marker, registry case: entries with no code anchor
    // pin to the workspace frame itself rather than to any component.
    let mut registry_svg = String::new();
    if !doc.unresolved.is_empty() {
        let mut px = frame_w - FRAME_PAD - PIN_R;
        let py = FRAME_TITLE_H / 2.0 + 6.0;
        for entry in &doc.unresolved {
            let label = format!("#{}", entry.id);
            let pin_findings = ctx.unresolved_findings(entry.id);
            let (pin_class, label_class) = if pin_findings.is_empty() {
                ("registry-pin", "pin-label")
            } else {
                ("registry-pin-finding", "pin-label-finding")
            };
            let mut pin_tip = finding_tooltip_lines(&pin_findings);
            pin_tip.push(format!(
                "#{} marks an unresolved decision — a question the design still owes an \
                 answer: {}. It belongs to the workspace as a whole, not to any function or \
                 component yet; Ply tracks it until someone resolves it (§5.6).",
                entry.id, entry.note
            ));
            registry_svg.push_str(&format!(
                "<g class=\"{pin_class}\">{tip}<circle cx=\"{px:.1}\" cy=\"{py:.1}\" r=\"{PIN_R:.1}\" /><text class=\"{label_class}\" x=\"{px:.1}\" y=\"{:.1}\">{label}</text></g>",
                py + 4.0,
                label = esc(&label),
                tip = title(&pin_tip.join("\n"))
            ));
            px -= PIN_R * 2.0 + 6.0 + text_w(&label, 6.0);
        }
    }

    // ---- edges, grouped into parallel lanes (findings 2 and 4) ----------
    struct ResolvedEdge {
        from_q: String,
        to_q: String,
        from_rect: Rect,
        to_rect: Rect,
        edge: Edge,
        // Position in `doc.edges`, not in this filtered vec — what a
        // finding's `EdgeIndex` target actually names.
        edge_index: usize,
    }
    let mut resolved_edges: Vec<ResolvedEdge> = Vec::new();
    for (edge_index, e) in doc.edges.iter().enumerate() {
        let Ok(edge) = parse_edge(e) else { continue };
        let Some((from_q, from_rect)) = resolve(&edge.from, &positions, &leaf_index)? else {
            continue;
        };
        let Some((to_q, to_rect)) = resolve(&edge.to, &positions, &leaf_index)? else {
            continue;
        };
        // §7.1 reattachment: redirect each endpoint to the box that's
        // actually drawn (itself, unless it folded into a collapsed
        // ancestor). A no-op when nothing is collapsing (`effective` is
        // empty), so this never touches the default rendering path.
        let from_q = effective.get(&from_q).cloned().unwrap_or(from_q);
        let to_q = effective.get(&to_q).cloned().unwrap_or(to_q);
        // Both ends now land on the same box: the edge is entirely internal
        // to a collapsed component, and the grammar has no visual form for
        // "a box calls itself" — drop it rather than draw a degenerate
        // zero-length arrow.
        if collapsing_active && from_q == to_q {
            continue;
        }
        resolved_edges.push(ResolvedEdge {
            from_q,
            to_q,
            from_rect,
            to_rect,
            edge,
            edge_index,
        });
    }
    // §7.1: "two reattached edges that become duplicates (same from/to/kind)
    // draw once." Only reachable once collapsing is active — declaring two
    // edges into different descendants of the same collapsed component,
    // which redirect to an identical (from, to, kind) triple. Kept in
    // declaration order; `EdgeKind` doesn't derive `Hash`, so a `Vec` scan
    // stands in for a set (edge counts here are always small).
    if collapsing_active {
        let mut seen: Vec<(String, String, EdgeKind)> = Vec::new();
        resolved_edges.retain(|re| {
            let key = (re.from_q.clone(), re.to_q.clone(), re.edge.kind.clone());
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        });
    }
    let mut pair_total: IndexMap<(String, String), usize> = IndexMap::new();
    for re in &resolved_edges {
        *pair_total
            .entry(pair_key(&re.from_q, &re.to_q))
            .or_insert(0) += 1;
    }
    let mut pair_seen: IndexMap<(String, String), usize> = IndexMap::new();
    let mut edges_svg = String::new();
    for re in &resolved_edges {
        let key = pair_key(&re.from_q, &re.to_q);
        let total = pair_total[&key];
        let idx_slot = pair_seen.entry(key.clone()).or_insert(0);
        let idx = *idx_slot;
        *idx_slot += 1;
        let lane = lane_offset(idx, total, LANE_GAP);

        // The perpendicular axis is computed from the *canonical* pair
        // order (`key`), never from this edge's own from/to direction: two
        // edges pointing opposite ways between the same pair must still
        // land in distinct, non-cancelling lanes (vetting 002 finding 4's
        // `decoder -> ring` vs `ring ~> decoder`).
        let (rect_a, rect_b) = if re.from_q == key.0 {
            (re.from_rect, re.to_rect)
        } else {
            (re.to_rect, re.from_rect)
        };
        let (dx, dy) = (rect_b.cx() - rect_a.cx(), rect_b.cy() - rect_a.cy());
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let (px, py) = (-dy / len, dx / len);
        let offset = (px * lane, py * lane);

        // The natural (un-offset) border point first, then the lane offset
        // applied directly to it — not to the target `border_toward` aims
        // at. Offsetting the target instead would work for a square box,
        // but `border_toward`'s scale shrinks any target-side offset in
        // proportion to how much taller the gap to the next rank is than
        // the box (exactly our case: `ranksep` is wide, so it landed lanes
        // only ~6 units apart for a requested 16 — indistinguishable at a
        // glance, the parallel-edges bug in a different guise).
        let (fx, fy) = offset_along_border(
            re.from_rect,
            re.from_rect
                .border_toward((re.to_rect.cx(), re.to_rect.cy())),
            offset,
        );
        let (tx, ty) = offset_along_border(
            re.to_rect,
            re.to_rect
                .border_toward((re.from_rect.cx(), re.from_rect.cy())),
            offset,
        );

        // The label sits `LABEL_T` of the way along the (already offset)
        // line — never at the midpoint, so it stays clear of the arrowhead
        // — then pushed out past the line by its own half-width plus a
        // clearance gap, in the same perpendicular direction its lane
        // already leans (finding 2: labels must never overlap an
        // arrowhead, a line end, or a box). Sizing the push from the
        // label's own text matters: a fixed push clears the line at the
        // label's *center* but a `text-anchor:middle` label still reaches
        // back over the line by half its own width otherwise.
        let label_pos = if let EdgeKind::Flow(ty_label) = &re.edge.kind {
            let bx = fx + (tx - fx) * LABEL_T;
            let by = fy + (ty - fy) * LABEL_T;
            // Reserved with the worst-case character width (`NAME_CHAR_W`),
            // not the narrower width the label is actually drawn at
            // (`SUB_CHAR_W`) — same reasoning as the deny-except margin fix
            // above: a push sized from the exact drawn width leaves zero
            // real clearance.
            let clear = LABEL_SIDE_GAP + text_w(ty_label, NAME_CHAR_W) / 2.0;
            let sign = if lane < 0.0 { -1.0 } else { 1.0 };
            let at = |mult: f64, s: f64| (bx + px * clear * mult * s, by + py * clear * mult * s);
            // The perpendicular push that clears the *line* can still land
            // the label on top of an unrelated box that happens to sit on
            // that side (vetting 003's "same clearance family" as finding
            // 1 — this time between two top-level ranks rather than inside
            // a container, and between a short line and a short collapsed
            // box rather than a tall one). Escalate: try the natural side
            // first, then the mirrored side, at growing multiples of the
            // base clearance, and take the first that lands clear of
            // every real box; if none do within the budget, keep the
            // original (lane-consistent, unscaled) placement rather than
            // drifting arbitrarily far.
            [1.0, 1.5, 2.0, 2.5, 3.0]
                .into_iter()
                .flat_map(|mult| [(mult, sign), (mult, -sign)])
                .map(|(mult, s)| at(mult, s))
                .find(|&pos| !label_clashes_with_any_box(pos, ty_label, &positions))
                .unwrap_or_else(|| at(1.0, sign))
        } else {
            (0.0, 0.0) // unused: EdgeKind::Call never renders a label
        };

        let findings = ctx.edge_findings(re.edge_index);
        render_edge(
            &re.edge,
            (fx, fy),
            (tx, ty),
            label_pos,
            &findings,
            &mut edges_svg,
        );
    }

    // Deny rules. §7.1 (amended): `*` has no shared identity, so each rule
    // that names it draws its own pseudo-node — never one shared node that
    // would visually imply unrelated rules are connected.
    let mut deny_svg = String::new();
    let deny_layout = DenyLayout {
        positions: &positions,
        leaf_index: &leaf_index,
        any_x_from,
        any_x_to,
    };
    // Every wildcard any-node placed so far, one list per margin column
    // (finding 3: two rules anchoring `*` in the same column must not land
    // at the same, or too close a, y — `place_clear` pushes a conflicting
    // one down until it clears every prior node in its own column).
    let mut any_columns = AnyColumns::default();
    // §7.1 channel discipline: two deny lines that cross are unreadable.
    // A wildcard node's height is assigned in the order of the target it
    // points at (monotone), so the fan of deny lines never self-intersects
    // — then stacked apart where targets are too close together.
    let deny_order: Vec<usize> = {
        let mut keyed: Vec<(usize, f64)> = parsed_denies
            .iter()
            .enumerate()
            .map(|(i, (_, d))| {
                let other = if d.from == "*" { &d.to } else { &d.from };
                let y = resolve(other, deny_layout.positions, deny_layout.leaf_index)
                    .ok()
                    .flatten()
                    .map(|(_, r)| r.cy())
                    .unwrap_or(f64::MAX);
                (i, y)
            })
            .collect();
        keyed.sort_by(|a, b| a.1.total_cmp(&b.1));
        keyed.into_iter().map(|(i, _)| i).collect()
    };
    for i in deny_order {
        let (orig_index, d) = &parsed_denies[i];
        render_deny(
            i,
            *orig_index,
            d,
            &deny_layout,
            &ctx,
            &mut any_columns,
            &mut deny_svg,
        )?;
    }

    // §7.1: "a finding with no drawable item attaches a red count next to
    // the workspace title." Checked last, once every render call above has
    // had its chance to mark a diagnostic attached — a clean document
    // (`findings` empty) always has `unattached_count() == 0`, so this is a
    // no-op and the title is untouched.
    let unattached = ctx.unattached_count();
    let (title_extra, title_min_w) = if unattached > 0 {
        let count_text = format!(
            "{unattached} finding{} — run ply-check",
            if unattached == 1 { "" } else { "s" }
        );
        let title_x = FRAME_PAD + text_w("ply.yaml", CHIP_CHAR_W) + BADGE_GAP;
        let text = format!(
            "<text class=\"finding-count\" x=\"{title_x:.1}\" y=\"20\">{}</text>",
            esc(&count_text)
        );
        (text, title_x + text_w(&count_text, CHIP_CHAR_W) + FRAME_PAD)
    } else {
        (String::new(), 0.0)
    };

    let width = frame_w.max(title_min_w);
    // A tall stack of wildcard any-nodes (several `*` rules anchoring in
    // one margin column, `place_clear` pushing each below the last) can
    // run past the box layout's bottom edge; the canvas and frame grow so
    // no node ever draws off-canvas.
    let deny_bottom = any_columns
        .from
        .iter()
        .chain(any_columns.to.iter())
        .fold(f64::MIN, |a, &y| a.max(y))
        + ANY_R
        + FRAME_PAD;
    let height = frame_h.max(deny_bottom);

    // A clean document (no findings at all) gets exactly `STYLE`, unchanged
    // — see `FINDING_STYLE`'s doc comment for why this is conditional
    // rather than always-appended.
    let style: std::borrow::Cow<str> = if findings.is_empty() {
        std::borrow::Cow::Borrowed(STYLE)
    } else {
        std::borrow::Cow::Owned(format!("{STYLE}{FINDING_STYLE}"))
    };

    // §7.1 / newbie bar: the frame is the first thing anyone sees, so its
    // tooltip explains the whole picture rather than assuming the reader
    // has already read The-Ply-Spec.md.
    let workspace_tip = title(
        "This diagram is drawn from ply.yaml, the file describing this codebase's \
         architecture and verification claims. Each box is a component; chips are \
         functions with their declared checks; arrows are permitted calls (solid) and \
         data flows (dashed); red bars are forbidden calls. A box's green depth is the \
         strength of the checks it declares — white means something inside declares \
         none, deeper green means stronger checks, and the weakest function sets the \
         whole box's shade. It is a promise scale, not results: none of it has run \
         yet. Hover anything for its meaning.",
    );

    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.1}\" height=\"{height:.1}\" \
         viewBox=\"0 0 {width:.1} {height:.1}\" font-family=\"monospace\" font-size=\"12\">\
         <style>{style}</style>\
         <defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"8\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\">\
         <path d=\"M 0 0 L 10 5 L 0 10 z\" /></marker></defs>\
         <rect class=\"workspace-frame\" x=\"1\" y=\"1\" width=\"{frame_inner_w:.1}\" height=\"{frame_inner_h:.1}\" rx=\"8\">{workspace_tip}</rect>\
         <text class=\"workspace-title\" x=\"{FRAME_PAD:.1}\" y=\"20\">ply.yaml</text>\
         {title_extra}\
         {deny_svg}{registry_svg}{body}{edges_svg}\
         </svg>",
        frame_inner_w = width - 2.0,
        frame_inner_h = height - 2.0,
    ))
}

/// Builds a leaf-name -> qualified-paths index from a positions-shaped map's
/// keys (§5.1a rule 6): `resolve()`'s bare-name branch needs this to detect
/// ambiguity.
fn leaf_index_of(positions: &IndexMap<String, Rect>) -> IndexMap<String, Vec<String>> {
    let mut leaf_index: IndexMap<String, Vec<String>> = IndexMap::new();
    for qualified in positions.keys() {
        let leaf = qualified.rsplit('.').next().unwrap_or(qualified);
        leaf_index
            .entry(leaf.to_string())
            .or_default()
            .push(qualified.clone());
    }
    leaf_index
}

fn render_edge(
    edge: &Edge,
    (fx, fy): (f64, f64),
    (tx, ty): (f64, f64),
    label_pos: (f64, f64),
    findings: &[&Diagnostic],
    out: &mut String,
) {
    let line_class = if findings.is_empty() {
        "edge-line"
    } else {
        "edge-line-finding"
    };
    let badge_svg = if findings.is_empty() {
        String::new()
    } else {
        let mx = (fx + tx) / 2.0;
        let my = (fy + ty) / 2.0;
        render_finding_badge(mx + 6.0, my - FINDING_BADGE_H - 2.0, findings)
    };
    match &edge.kind {
        EdgeKind::Call => {
            let mut tip = finding_tooltip_lines(findings);
            tip.push(format!(
                "{a} -> {b} — {a} may call {b}. An undeclared cross-component call is \
                 flagged as an architecture finding — a warning by default, an error if \
                 the calling component is `strict` (§5.3, A0402).",
                a = edge.from,
                b = edge.to
            ));
            out.push_str(&format!(
                "<g class=\"edge-call\">{tip}<path class=\"{line_class}\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" marker-end=\"url(#arrow)\" />{badge_svg}</g>",
                tip = title(&tip.join("\n"))
            ));
        }
        EdgeKind::Flow(ty_label) => {
            let (lx, ly) = label_pos;
            let mut tip = finding_tooltip_lines(findings);
            tip.push(format!(
                "{ty_label} data flows from {} to {} — declared for the picture; \
                 nothing checks flows in v1",
                edge.from, edge.to
            ));
            out.push_str(&format!(
                "<g class=\"edge-flow\">{tip_html}<path class=\"{line_class}\" stroke-dasharray=\"6 4\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" marker-end=\"url(#arrow)\" /><text class=\"edge-label\" x=\"{lx:.1}\" y=\"{ly:.1}\">{}</text>{badge_svg}</g>",
                esc(ty_label),
                tip_html = title(&tip.join("\n"))
            ));
        }
    }
}

/// The parts of `render_svg`'s layout `render_deny` only reads, bundled so
/// adding the finding plumbing didn't push it over clippy's argument-count
/// lint. `any_x_from`/`any_x_to` are fixed x-positions inside the margins
/// `render_svg` reserved for exactly this (finding 3: deny nodes clipped
/// off-canvas).
#[derive(Clone, Copy)]
struct DenyLayout<'a> {
    positions: &'a IndexMap<String, Rect>,
    leaf_index: &'a IndexMap<String, Vec<String>>,
    any_x_from: f64,
    any_x_to: f64,
}

/// How far apart two wildcard any-nodes anchored in the *same* margin
/// column must be kept — vetting 003 finding 3: two unrelated deny rules
/// both naming `*` on the same side landed on the same spot otherwise.
/// Wide enough that the nodes (radius `ANY_R`) plus a real gap never touch.
const DENY_LANE_GAP: f64 = ANY_R * 2.0 + ANY_GAP;

/// The y positions already claimed by wildcard any-nodes, one list per
/// margin column (left = deny `from`, right = deny `to`), threaded through
/// every `render_deny` call so `place_clear` can stack a new node clear of
/// the ones already drawn in its column.
#[derive(Default)]
struct AnyColumns {
    from: Vec<f64>,
    to: Vec<f64>,
}

/// Pushes `natural` down (repeatedly, by `min_gap`) until it is at least
/// `min_gap` away from every y already in `occupied`, then records it there
/// — a simple, deterministic (declaration-order) way to stack same-column
/// wildcard nodes with clear vertical gaps instead of letting them land on
/// top of each other.
fn place_clear(natural: f64, occupied: &mut Vec<f64>, min_gap: f64) -> f64 {
    let mut y = natural;
    // Jump just past whichever already-placed y is closest to conflicting,
    // rather than a fixed `min_gap` stride — striding past one conflict can
    // still land inside another that was less than `min_gap` beyond it
    // (two rules `min_gap - 1` apart would otherwise need two strides to
    // clear the first), so keep resolving conflicts until none remain.
    while let Some(&o) = occupied.iter().find(|&&o| (o - y).abs() < min_gap) {
        y = o + min_gap;
    }
    occupied.push(y);
    y
}

/// A straight line unless it would cut through a top-level box neither of
/// its ends is attached to (vetting 003 finding 3: `* -> gateway` sits in
/// the same row as `risk`, so a direct line from the far margin to
/// `gateway` would pass straight through `risk`'s box). When it would, the
/// path steps around every such box instead: over to just outside its
/// combined x-span, up (or down, whichever is the shorter detour) clear of
/// its combined y-span, across, then straight on to `to` — still a
/// straight run before and after the detour, since nothing else occupies
/// that space (every box in the original line's path is already folded
/// into the one detour).
fn route_deny_line(
    from: (f64, f64),
    to: (f64, f64),
    exclude: &[Rect],
    top_level_positions: &IndexMap<String, Rect>,
) -> Vec<(f64, f64)> {
    const CLEARANCE: f64 = 12.0;
    let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
    let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
    let mut obstructions: Vec<Rect> = top_level_positions
        .values()
        .copied()
        .filter(|r| {
            !exclude.contains(r)
                && r.x < x1 - 1.0
                && r.x + r.w > x0 + 1.0
                && r.y < y1 + 1.0
                && r.y + r.h > y0 - 1.0
        })
        .collect();
    if obstructions.is_empty() {
        return vec![from, to];
    }
    obstructions.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
    let span_x0 = obstructions
        .iter()
        .map(|r| r.x)
        .fold(f64::INFINITY, f64::min)
        - CLEARANCE;
    let span_x1 = obstructions
        .iter()
        .map(|r| r.x + r.w)
        .fold(f64::NEG_INFINITY, f64::max)
        + CLEARANCE;
    let top = obstructions
        .iter()
        .map(|r| r.y)
        .fold(f64::INFINITY, f64::min)
        - CLEARANCE;
    let bottom = obstructions
        .iter()
        .map(|r| r.y + r.h)
        .fold(f64::NEG_INFINITY, f64::max)
        + CLEARANCE;
    let mid_y = (from.1 + to.1) / 2.0;
    let rail_y = if (mid_y - top).abs() <= (bottom - mid_y).abs() {
        top
    } else {
        bottom
    };
    let (enter_x, exit_x) = if from.0 <= to.0 {
        (span_x0, span_x1)
    } else {
        (span_x1, span_x0)
    };
    // Rise to the rail at `from`'s own y first, rather than cutting
    // diagonally there directly — `from` is usually a wildcard any-node
    // sharing its whole margin column with every *other* wildcard deny's
    // any-node/line/label (each at a different y via `place_clear`), and a
    // diagonal straight to the rail sweeps across that entire column,
    // right through neighbors a plain box-obstruction check never sees. A
    // horizontal run at the original y, stopping at `enter_x` (already
    // clear of every real box — the whole reason `enter_x` exists), avoids
    // that column before rising.
    vec![
        from,
        (enter_x, from.1),
        (enter_x, rail_y),
        (exit_x, rail_y),
        to,
    ]
}

/// The longest straight segment of a (possibly routed) path — where the
/// deny bar's "denied" tick is drawn, so it always lands on open canvas
/// rather than risking a short, obstruction-hugging segment right at
/// either end. For an unrouted 2-point line this is just that line, so the
/// bar's position is unchanged from before routing existed.
fn longest_segment(points: &[(f64, f64)]) -> ((f64, f64), (f64, f64)) {
    points
        .windows(2)
        .map(|w| (w[0], w[1]))
        .max_by(|a, b| {
            let len = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0).hypot(q.1 - p.1);
            len(a.0, a.1).partial_cmp(&len(b.0, b.1)).unwrap()
        })
        .expect("a path always has at least one segment")
}

/// Renders one deny rule. §7.1 (amended): `*` has no shared identity, so a
/// rule that names it draws its own pseudo-node, anchored near whichever
/// side did resolve to a real component (or staggered by rule index, as a
/// last resort, if neither side resolved).
fn render_deny(
    index: usize,
    orig_index: usize,
    deny: &Deny,
    layout: &DenyLayout,
    ctx: &FindingCtx,
    columns: &mut AnyColumns,
    out: &mut String,
) -> Result<(), RenderError> {
    let DenyLayout {
        positions,
        leaf_index,
        any_x_from,
        any_x_to,
    } = *layout;
    let from_rect = if deny.from == "*" {
        None
    } else {
        match resolve(&deny.from, positions, leaf_index)? {
            Some((_, r)) => Some(r),
            None => return Ok(()), // unresolvable, non-wildcard pattern: draw nothing
        }
    };
    let to_rect = if deny.to == "*" {
        None
    } else {
        match resolve(&deny.to, positions, leaf_index)? {
            Some((_, r)) => Some(r),
            None => return Ok(()),
        }
    };

    let fallback_y = FRAME_TITLE_H / 2.0 + 6.0 + index as f64 * 34.0;
    let mut any_nodes = String::new();

    let from_pt = if deny.from == "*" {
        let natural = to_rect.map(|r| r.cy()).unwrap_or(fallback_y);
        let y = place_clear(natural, &mut columns.from, DENY_LANE_GAP);
        let p = (any_x_from, y);
        any_nodes.push_str(&any_node_svg(p.0, p.1));
        p
    } else {
        let r = from_rect.expect("non-wildcard, unresolved case already returned above");
        (r.cx(), r.cy())
    };

    let to_pt = if deny.to == "*" {
        let natural = from_rect.map(|r| r.cy()).unwrap_or(fallback_y);
        let y = place_clear(natural, &mut columns.to, DENY_LANE_GAP);
        let p = (any_x_to, y);
        any_nodes.push_str(&any_node_svg(p.0, p.1));
        p
    } else {
        let r = to_rect.expect("non-wildcard, unresolved case already returned above");
        (r.cx(), r.cy())
    };

    out.push_str(&any_nodes);
    if from_pt == to_pt {
        return Ok(());
    }

    let (fx, fy) = from_rect.map_or(from_pt, |r| r.border_toward(to_pt));
    let (tx, ty) = to_rect.map_or(to_pt, |r| r.border_toward(from_pt));

    // Only a wildcard-vs-real-component deny (exactly one resolved rect)
    // can have an unrelated top-level box sitting between the far-margin
    // any-node and its target — the only shape finding 3 needs routing
    // for. A concrete-to-concrete deny keeps its original direct line.
    let exclude: Vec<Rect> = [from_rect, to_rect].into_iter().flatten().collect();
    let top_level_positions: IndexMap<String, Rect> = positions
        .iter()
        .filter(|(k, _)| !k.contains('.'))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    let route = if deny.from == "*" || deny.to == "*" {
        route_deny_line((fx, fy), (tx, ty), &exclude, &top_level_positions)
    } else {
        vec![(fx, fy), (tx, ty)]
    };

    let (bar_a, bar_b) = longest_segment(&route);
    let mx = (bar_a.0 + bar_b.0) / 2.0;
    let my = (bar_a.1 + bar_b.1) / 2.0;
    // Perpendicular bar across the longest segment's midpoint: the
    // "denied" mark.
    let (dx, dy) = (bar_b.0 - bar_a.0, bar_b.1 - bar_a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (px, py) = (-dy / len * 8.0, dx / len * 8.0);

    // Looked up here, past every early return above, so a finding on a
    // deny rule that never resolves to a drawn line (matching nothing, or
    // collapsing to a single point) is never marked attached — it falls
    // through to the workspace-title fallback instead.
    let findings = ctx.deny_findings(orig_index);
    let line_class = if findings.is_empty() {
        "deny-line"
    } else {
        "deny-line-finding"
    };
    let mut tip = finding_tooltip_lines(&findings);
    tip.push({
        let mut t = match (deny.from.as_str(), deny.to.as_str()) {
            ("*", "*") => "no component may call any component".to_string(),
            ("*", to) => format!("no component may call {to}"),
            (from, "*") => format!("{from} may not call any component"),
            (from, to) => format!("{from} may not call {to}"),
        };
        if !deny.except.is_empty() {
            t.push_str(&format!(" — except {}", deny.except.join(", ")));
        }
        t.push_str(" — such a call fails the build");
        t
    });
    let badge_svg = render_finding_badge(mx + 6.0, my - FINDING_BADGE_H - 10.0, &findings);

    let path_d = route
        .iter()
        .enumerate()
        .map(|(i, (x, y))| {
            if i == 0 {
                format!("M {x:.1} {y:.1}")
            } else {
                format!("L {x:.1} {y:.1}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    out.push_str(&format!(
        "<g class=\"deny-rule\">{tip_html}<path class=\"{line_class}\" d=\"{path_d}\" /><line class=\"deny-bar\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" />{badge_svg}",
        mx - px, my - py, mx + px, my + py,
        tip_html = title(&tip.join("\n"))
    ));
    if !deny.except.is_empty() {
        // Beside whichever side is the wildcard node — inside the margin
        // `render_svg` reserved for it — rather than at the line's
        // midpoint, which may crowd the real component box on the other
        // side (finding 3).
        let label = format!("except {}", deny.except.join(", "));
        let half_w = text_w(&label, SUB_CHAR_W) / 2.0;
        // Clear of both the any-node circle (radius `ANY_R`) and the
        // perpendicular deny-bar tick (which sits astride the line at
        // roughly the same height as a wildcard endpoint whenever the line
        // is near-horizontal — vetting 003 finding 4) rather than the
        // previous fixed 6px, which put the label inside both.
        const EXCEPT_LABEL_CLEARANCE: f64 = ANY_R + 10.0;
        let (lx, ly) = if deny.from == "*" {
            (
                from_pt.0 + ANY_R + ANY_GAP + half_w,
                from_pt.1 - EXCEPT_LABEL_CLEARANCE,
            )
        } else if deny.to == "*" {
            (
                to_pt.0 - ANY_R - ANY_GAP - half_w,
                to_pt.1 - EXCEPT_LABEL_CLEARANCE,
            )
        } else {
            (mx, my + 14.0)
        };
        out.push_str(&format!(
            "<text class=\"deny-except\" x=\"{lx:.1}\" y=\"{ly:.1}\">{}</text>",
            esc(&label)
        ));
    }
    out.push_str("</g>");

    Ok(())
}
