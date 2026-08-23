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
    Check, Component, Deny, Document, Edge, EdgeKind, FnClaim, parse_check, parse_deny, parse_edge,
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

/// The strongest check kind one fn declares (The-Ply-Spec.md §7.1: `test` ->
/// tested, `fuzz(n)` -> fuzzed, `bounded(k)` -> bounded, `prove` -> proved).
/// `mutate` strengthens nothing on its own (it only ever rides alongside a
/// `test`/`fuzz` entry, D12) and an unparseable string names no real kind, so
/// both are skipped rather than treated as evidence. No checks at all (or
/// only skipped ones) -> `Unclaimed`.
fn fn_declared_ceiling(fc: &FnClaim) -> Evidence {
    let mut best: Option<Evidence> = None;
    for c in &fc.checks {
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
fn component_verdict_node(comp: &Component) -> VerdictNode {
    let mut children: Vec<VerdictNode> = comp
        .fns
        .values()
        .map(|fc| VerdictNode {
            kind: NodeKind::Claimable(fn_declared_ceiling(fc)),
            statuses: BTreeSet::new(),
            conditional: None,
            children: Vec::new(),
        })
        .collect();
    children.extend(comp.components.values().map(component_verdict_node));
    VerdictNode {
        kind: NodeKind::Container,
        statuses: BTreeSet::new(),
        conditional: None,
        children,
    }
}

fn component_ceiling(comp: &Component) -> Evidence {
    aggregate(&component_verdict_node(comp)).evidence
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

fn render_fn_chip(name: &str, fc: &FnClaim, component_path: &str, ctx: &FindingCtx) -> FnChip {
    let glyphs = checks_glyph_row(&fc.checks);
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
    for c in &fc.checks {
        tip.push(check_prose(c));
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
    if fc.checks.is_empty() {
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

fn render_component(
    name: &str,
    qualified: &str,
    comp: &Component,
    profiles: &IndexMap<String, Vec<String>>,
    ctx: &FindingCtx,
) -> ComponentBox {
    let findings = ctx.component_findings(qualified);
    // §7.1 "declared ceiling": the strongest verdict this component's own
    // declared checks could earn, folded worst-of over every fn in its
    // subtree by the real kernel `aggregate` (see `component_ceiling`).
    let ceiling = component_ceiling(comp);
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
                render_component(cname, &child_qualified, c, profiles, ctx),
            )
        })
        .collect();

    let chips: Vec<(String, FnChip)> = comp
        .fns
        .iter()
        .map(|(fname, fc)| (fname.clone(), render_fn_chip(fname, fc, qualified, ctx)))
        .collect();

    let content_w = [
        name_w,
        anchor_w,
        owns_w,
        badges_row_w + profile_w,
        MIN_BOX_W - PAD * 2.0,
    ]
    .into_iter()
    .fold(0.0_f64, f64::max)
    .max(children.iter().map(|(_, c)| c.width).fold(0.0, f64::max))
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

    // fn claims are not edge/deny endpoints, so unlike nested components,
    // their names are not recorded in `positions`.
    for (_, chip) in chips {
        body.push_str(&wrap_translate(&chip.svg, PAD, y));
        y += chip.height + GAP;
    }

    let box_h = y + PAD;

    let mut tip = finding_tooltip_lines(&findings);
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

pub fn render_svg(doc: &Document) -> Result<String, RenderError> {
    // §7.1 "finding (tool-computed, not declared)": run `ply-check`'s
    // document-local rules up front, then thread `ctx` through every render
    // function below so it can mark red whatever a finding attaches to and
    // tally whatever it doesn't (`ctx.unattached_count()`, consulted once
    // rendering finishes). A clean document (`findings` empty) makes every
    // lookup below return nothing, so nothing about its output changes.
    let findings = run_checks(doc);
    let ctx = FindingCtx::new(&findings);

    let top: Vec<(String, ComponentBox)> = doc
        .components
        .iter()
        .map(|(name, c)| {
            (
                name.clone(),
                render_component(name, name, c, &doc.profiles, &ctx),
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
        let label_w = if d.except.is_empty() {
            0.0
        } else {
            text_w(&format!("except {}", d.except.join(", ")), SUB_CHAR_W)
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
        resolved_edges.push(ResolvedEdge {
            from_q,
            to_q,
            from_rect,
            to_rect,
            edge,
            edge_index,
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
            let clear = LABEL_SIDE_GAP + text_w(ty_label, SUB_CHAR_W) / 2.0;
            let push = if lane < 0.0 { -clear } else { clear };
            (bx + px * push, by + py * push)
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
    for (i, (orig_index, d)) in parsed_denies.iter().enumerate() {
        render_deny(i, *orig_index, d, &deny_layout, &ctx, &mut deny_svg)?;
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
    let height = frame_h;

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
        let p = (any_x_from, to_rect.map(|r| r.cy()).unwrap_or(fallback_y));
        any_nodes.push_str(&any_node_svg(p.0, p.1));
        p
    } else {
        let r = from_rect.expect("non-wildcard, unresolved case already returned above");
        (r.cx(), r.cy())
    };

    let to_pt = if deny.to == "*" {
        let p = (any_x_to, from_rect.map(|r| r.cy()).unwrap_or(fallback_y));
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

    let mx = (fx + tx) / 2.0;
    let my = (fy + ty) / 2.0;
    // Perpendicular bar across the line midpoint: the "denied" mark.
    let (dx, dy) = (tx - fx, ty - fy);
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

    out.push_str(&format!(
        "<g class=\"deny-rule\">{tip_html}<path class=\"{line_class}\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" /><line class=\"deny-bar\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" />{badge_svg}",
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
        let (lx, ly) = if deny.from == "*" {
            (from_pt.0 + ANY_R + ANY_GAP + half_w, from_pt.1 - 6.0)
        } else if deny.to == "*" {
            (to_pt.0 - ANY_R - ANY_GAP - half_w, to_pt.1 - 6.0)
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
