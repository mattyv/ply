//! `ply.yaml` -> SVG, the §7.1 visual grammar table made literal.
//!
//! This is a spec-validation tool: its only job is to prove every declarative
//! construct in the grammar has a drawable form. Layout is a simple
//! deterministic block stack (no layout library, no font-metrics library —
//! character widths are a fixed-width monospace estimate) so that the same
//! input always produces byte-identical output.

use crate::model::{
    parse_check, parse_deny, parse_edge, Check, Component, Deny, Document, Edge, EdgeKind,
    FnClaim,
};
use indexmap::IndexMap;

// ---- layout constants -----------------------------------------------------

const PAD: f64 = 10.0;
const GAP: f64 = 12.0;
const NAME_CHAR_W: f64 = 8.0;
const SUB_CHAR_W: f64 = 6.2;
const CHIP_CHAR_W: f64 = 7.0;
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

fn render_fn_chip(name: &str, fc: &FnClaim) -> FnChip {
    let glyphs = checks_glyph_row(&fc.checks);
    let note = check_with_note(&fc.check_with);
    let has_shield = !fc.trusted.is_empty();

    let mut cursor_x = PAD;
    let mut inner = String::new();
    let text_y = CHIP_H / 2.0 + 4.0;

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
        inner.push_str(&format!(
            "<text class=\"fn-shield\" x=\"{cursor_x:.1}\" y=\"{text_y:.1}\">\u{26C9}</text>",
        ));
        cursor_x += SHIELD_W + BADGE_GAP;
    }

    for p in &fc.unresolved {
        let label = format!("#{}", p.id);
        let cx = cursor_x + PIN_R;
        inner.push_str(&format!(
            "<g class=\"unresolved-pin\"><circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{PIN_R:.1}\" /><text class=\"pin-label\" x=\"{cx:.1}\" y=\"{text_y:.1}\">{label}</text></g>",
            cy = CHIP_H / 2.0,
            label = esc(&label)
        ));
        cursor_x += text_w(&label, CHIP_CHAR_W) + PIN_R * 2.0 + BADGE_GAP;
    }

    let width = cursor_x + PAD - BADGE_GAP;
    let svg = format!(
        "<g class=\"fn-chip\" data-fn=\"{}\"><rect class=\"fn-chip-box\" x=\"0\" y=\"0\" width=\"{width:.1}\" height=\"{CHIP_H:.1}\" rx=\"4\" />{inner}</g>",
        esc(name)
    );

    FnChip { width, height: CHIP_H, svg }
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

fn render_component(name: &str, comp: &Component) -> ComponentBox {
    let name_w = text_w(name, NAME_CHAR_W);
    let anchor_w = text_w(&comp.anchor, SUB_CHAR_W);

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
        .map(|(cname, c)| (cname.clone(), render_component(cname, c)))
        .collect();

    let chips: Vec<(String, FnChip)> = comp
        .fns
        .iter()
        .map(|(fname, fc)| (fname.clone(), render_fn_chip(fname, fc)))
        .collect();

    let content_w = [
        name_w,
        anchor_w,
        badges_row_w + profile_w,
        MIN_BOX_W - PAD * 2.0,
    ]
    .into_iter()
    .fold(0.0_f64, f64::max)
    .max(children.iter().map(|(_, c)| c.width).fold(0.0, f64::max))
    .max(chips.iter().map(|(_, c)| c.width).fold(0.0, f64::max));

    let box_w = content_w + PAD * 2.0;

    let mut y = PAD + HEADER_H;
    let mut body = String::new();
    let mut positions: Vec<(String, Rect)> = Vec::new();

    if badge_row_h > 0.0 {
        let mut bx = PAD;
        for b in badges {
            let bw = text_w(b, BADGE_CHAR_W) + BADGE_PAD * 2.0;
            body.push_str(&format!(
                "<g class=\"cap-badge\"><rect x=\"{bx:.1}\" y=\"{y:.1}\" width=\"{bw:.1}\" height=\"{BADGE_H:.1}\" rx=\"3\" /><text x=\"{tx:.1}\" y=\"{ty:.1}\">{label}</text></g>",
                tx = bx + BADGE_PAD,
                ty = y + BADGE_H - 6.0,
                label = esc(b)
            ));
            bx += bw + BADGE_GAP;
        }
        if let Some(p) = &comp.profile {
            let pw = text_w(p, BADGE_CHAR_W) + BADGE_PAD * 2.0;
            body.push_str(&format!(
                "<g class=\"profile-tag\"><rect x=\"{bx:.1}\" y=\"{y:.1}\" width=\"{pw:.1}\" height=\"{BADGE_H:.1}\" rx=\"3\" /><text x=\"{tx:.1}\" y=\"{ty:.1}\">{label}</text></g>",
                tx = bx + BADGE_PAD,
                ty = y + BADGE_H - 6.0,
                label = esc(p)
            ));
        }
        y += badge_row_h;
    }

    for (cname, cbox) in children {
        body.push_str(&wrap_translate(&cbox.svg, PAD, y));
        positions.push((
            cname.clone(),
            Rect { x: PAD, y, w: cbox.width, h: cbox.height },
        ));
        for (n, r) in cbox.positions {
            positions.push((n, Rect { x: PAD + r.x, y: y + r.y, w: r.w, h: r.h }));
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

    let mut svg = format!(
        "<g class=\"component\" data-name=\"{}\"><rect class=\"component-box\" x=\"0\" y=\"0\" width=\"{box_w:.1}\" height=\"{box_h:.1}\" rx=\"6\" />",
        esc(name)
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
    svg.push_str(&format!(
        "<text class=\"component-anchor\" x=\"{PAD:.1}\" y=\"{:.1}\">{}</text>",
        PAD + LINE_H * 2.0 - 4.0,
        esc(&comp.anchor)
    ));
    svg.push_str(&body);
    svg.push_str("</g>");

    ComponentBox { width: box_w, height: box_h, svg, positions }
}

fn any_node_svg(x: f64, y: f64) -> String {
    format!(
        "<g class=\"any-node\"><circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"14\" /><text class=\"any-label\" x=\"{x:.1}\" y=\"{:.1}\">*</text></g>",
        y + 4.0
    )
}

pub fn render_svg(doc: &Document) -> String {
    let top: Vec<(String, ComponentBox)> = doc
        .components
        .iter()
        .map(|(name, c)| (name.clone(), render_component(name, c)))
        .collect();

    let content_w = top.iter().map(|(_, c)| c.width).fold(0.0_f64, f64::max);
    let mut frame_y = FRAME_PAD + FRAME_TITLE_H;
    let mut body = String::new();
    // Absolute canvas position of every named component (leaf name -> rect),
    // used to anchor edges and deny rules. Component names are unique across
    // the whole merged document except for same-named siblings under
    // different parents (§5.1); the first one encountered wins here — see
    // the render report for why that ambiguity is a spec-feedback item, not
    // a bug in this renderer.
    let mut positions: IndexMap<String, Rect> = IndexMap::new();

    for (name, cbox) in top {
        let x = FRAME_PAD;
        body.push_str(&wrap_translate(&cbox.svg, x, frame_y));
        positions
            .entry(name.clone())
            .or_insert(Rect { x, y: frame_y, w: cbox.width, h: cbox.height });
        for (n, r) in cbox.positions {
            positions
                .entry(n)
                .or_insert(Rect { x: x + r.x, y: frame_y + r.y, w: r.w, h: r.h });
        }
        frame_y += cbox.height + GAP;
    }

    let frame_w = content_w + FRAME_PAD * 2.0;
    let mut frame_h = frame_y + FRAME_PAD;

    // §7.1 unresolved marker, registry case: entries with no code anchor
    // pin to the workspace frame itself rather than to any component.
    let mut registry_svg = String::new();
    if !doc.unresolved.is_empty() {
        let mut px = frame_w - FRAME_PAD - PIN_R;
        let py = FRAME_TITLE_H / 2.0 + 6.0;
        for entry in &doc.unresolved {
            let label = format!("#{}", entry.id);
            registry_svg.push_str(&format!(
                "<g class=\"registry-pin\"><circle cx=\"{px:.1}\" cy=\"{py:.1}\" r=\"{PIN_R:.1}\" /><text class=\"pin-label\" x=\"{px:.1}\" y=\"{:.1}\">{label}</text></g>",
                py + 4.0,
                label = esc(&label)
            ));
            px -= PIN_R * 2.0 + 6.0 + text_w(&label, 6.0);
        }
    }

    let mut edges_svg = String::new();
    for e in &doc.edges {
        if let Ok(edge) = parse_edge(e) {
            render_edge(&edge, &positions, &mut edges_svg);
        }
    }

    // Deny rules. §7.1 leaves open how to draw the `*` wildcard pattern;
    // this renderer's choice is a single shared "any" pseudo-node per
    // document (a small circle labeled `*`), reused by every deny rule that
    // names it, rather than one per rule — see the render report.
    let mut deny_svg = String::new();
    let any_pos = (FRAME_PAD + 14.0, FRAME_TITLE_H / 2.0 + 6.0);
    let mut any_used = false;
    for d in &doc.deny {
        if let Ok(deny) = parse_deny(d) {
            any_used |= render_deny(&deny, &positions, any_pos, &mut deny_svg);
        }
    }
    if any_used {
        deny_svg.push_str(&any_node_svg(any_pos.0, any_pos.1));
        frame_h = frame_h.max(any_pos.1 + FRAME_PAD);
    }

    let width = frame_w;
    let height = frame_h;

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.1}\" height=\"{height:.1}\" \
         viewBox=\"0 0 {width:.1} {height:.1}\" font-family=\"monospace\" font-size=\"12\">\
         <defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"8\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\">\
         <path d=\"M 0 0 L 10 5 L 0 10 z\" /></marker></defs>\
         <rect class=\"workspace-frame\" x=\"1\" y=\"1\" width=\"{frame_inner_w:.1}\" height=\"{frame_inner_h:.1}\" rx=\"8\" />\
         <text class=\"workspace-title\" x=\"{FRAME_PAD:.1}\" y=\"20\">ply.yaml</text>\
         {deny_svg}{registry_svg}{body}{edges_svg}\
         </svg>",
        frame_inner_w = width - 2.0,
        frame_inner_h = height - 2.0,
    )
}

fn render_edge(edge: &Edge, positions: &IndexMap<String, Rect>, out: &mut String) {
    let (Some(&from), Some(&to)) = (positions.get(&edge.from), positions.get(&edge.to)) else {
        return;
    };
    let (fx, fy) = from.border_toward((to.cx(), to.cy()));
    let (tx, ty) = to.border_toward((from.cx(), from.cy()));
    match &edge.kind {
        EdgeKind::Call => {
            out.push_str(&format!(
                "<g class=\"edge-call\"><path class=\"edge-line\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" marker-end=\"url(#arrow)\" /></g>",
            ));
        }
        EdgeKind::Flow(ty_label) => {
            let mx = (fx + tx) / 2.0;
            let my = (fy + ty) / 2.0;
            out.push_str(&format!(
                "<g class=\"edge-flow\"><path class=\"edge-line\" stroke-dasharray=\"6 4\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" marker-end=\"url(#arrow)\" /><text class=\"edge-label\" x=\"{mx:.1}\" y=\"{my:.1}\">{}</text></g>",
                esc(ty_label)
            ));
        }
    }
}

/// Renders one deny rule; returns true if it referenced the shared `*`
/// any-node (so the caller draws that pseudo-node once, after the loop).
fn render_deny(
    deny: &Deny,
    positions: &IndexMap<String, Rect>,
    any_pos: (f64, f64),
    out: &mut String,
) -> bool {
    let from_rect = (deny.from != "*").then(|| positions.get(&deny.from)).flatten().copied();
    let to_rect = (deny.to != "*").then(|| positions.get(&deny.to)).flatten().copied();
    let uses_any = deny.from == "*" || deny.to == "*";

    let from_pt = from_rect.map(|r| (r.cx(), r.cy())).unwrap_or(any_pos);
    let to_pt = to_rect.map(|r| (r.cx(), r.cy())).unwrap_or(any_pos);
    if from_pt == to_pt {
        return uses_any;
    }

    let (fx, fy) = from_rect.map_or(from_pt, |r| r.border_toward(to_pt));
    let (tx, ty) = to_rect.map_or(to_pt, |r| r.border_toward(from_pt));

    let mx = (fx + tx) / 2.0;
    let my = (fy + ty) / 2.0;
    // Perpendicular bar across the line midpoint: the "denied" mark.
    let (dx, dy) = (tx - fx, ty - fy);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (px, py) = (-dy / len * 8.0, dx / len * 8.0);

    out.push_str(&format!(
        "<g class=\"deny-rule\"><path class=\"deny-line\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" /><line class=\"deny-bar\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" />",
        mx - px, my - py, mx + px, my + py
    ));
    if !deny.except.is_empty() {
        out.push_str(&format!(
            "<text class=\"deny-except\" x=\"{mx:.1}\" y=\"{:.1}\">except {}</text>",
            my + 14.0,
            esc(&deny.except.join(", "))
        ));
    }
    out.push_str("</g>");

    uses_any
}
