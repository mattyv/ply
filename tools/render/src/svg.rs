//! `ply.yaml` -> SVG, the §7.1 visual grammar table made literal.
//!
//! This is a spec-validation tool: its only job is to prove every declarative
//! construct in the grammar has a drawable form. Layout is a simple
//! deterministic block stack (no layout library, no font-metrics library —
//! character widths are a fixed-width monospace estimate) so that the same
//! input always produces byte-identical output.

use ply_model::{
    parse_check, parse_deny, parse_edge, Check, Component, Deny, Document, Edge, EdgeKind,
    FnClaim,
};
use indexmap::IndexMap;

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

/// Every `class` this renderer emits must have a rule here. SVG's initial
/// paint is `fill: black; stroke: none`, so an unstyled shape is a solid black
/// box — a missing rule is invisible output, not a cosmetic slip.
/// `tests/render.rs::every_painted_element_resolves_a_style_rule` enforces it.
pub const STYLE: &str = "\
.workspace-frame{fill:#fbfbfd;stroke:#c8ccd4}\
.workspace-title{fill:#6b7280}\
.component-box{fill:#fff;stroke:#3b4252;stroke-width:1.5}\
.pure-seal{fill:none;stroke:#3b4252}\
.component-name{fill:#1f2430;font-weight:bold}\
.component-anchor{fill:#6b7280;font-size:10px}\
.component-owns{fill:#6b7280;font-size:10px;font-style:italic}\
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
        Ok(Check::Test) => "test — generated example and contract cases".into(),
        Ok(Check::Fuzz(n)) => format!("fuzz({n}) — {n} randomised property-test cases"),
        Ok(Check::Bounded(k)) => {
            format!("bounded({k}) — model-checked exhaustively to depth {k}")
        }
        Ok(Check::Prove) => "prove — unbounded proof".into(),
        Ok(Check::Mutate) => "mutate — mutants must be killed by the check suite".into(),
        Err(e) => format!("{c} — unparseable: {e}"),
    }
}

fn title(text: &str) -> String {
    format!("<title>{}</title>", esc(text))
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
        let claims = fc
            .trusted
            .iter()
            .map(|t| format!("{} — evidence: {}", t.claim, t.evidence))
            .collect::<Vec<_>>()
            .join("\n");
        inner.push_str(&format!(
            "<g class=\"fn-shield\">{}<text x=\"{cursor_x:.1}\" y=\"{text_y:.1}\">\u{26C9}</text></g>",
            title(&format!(
                "trusted claim — attested by a human, never machine-checked\n{claims}"
            ))
        ));
        cursor_x += SHIELD_W + BADGE_GAP;
    }

    for p in &fc.unresolved {
        let label = format!("#{}", p.id);
        let cx = cursor_x + PIN_R;
        inner.push_str(&format!(
            "<g class=\"unresolved-pin\">{tip}<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{PIN_R:.1}\" /><text class=\"pin-label\" x=\"{cx:.1}\" y=\"{text_y:.1}\">{label}</text></g>",
            cy = CHIP_H / 2.0,
            label = esc(&label),
            tip = title(&format!(
                "unresolved #{} — a decision still owed; caps this fn at check `test` (§5.6)\n{}",
                p.id, p.note
            ))
        ));
        cursor_x += text_w(&label, CHIP_CHAR_W) + PIN_R * 2.0 + BADGE_GAP;
    }

    let mut tip = vec![name.to_string()];
    for c in &fc.checks {
        tip.push(check_prose(c));
    }
    if let Some(n) = &note {
        tip.push(format!("checked at instantiation {n}"));
    }
    for t in &fc.trusted {
        tip.push(format!(
            "trusted (not machine-checked): {} — evidence: {}",
            t.claim, t.evidence
        ));
    }
    if !fc.examples.is_empty() {
        tip.push(format!("{} example(s)", fc.examples.len()));
    }
    for p in &fc.unresolved {
        tip.push(format!("unresolved #{}: {}", p.id, p.note));
    }
    if fc.checks.is_empty() {
        tip.push("no checks declared — unclaimed".into());
    }

    let width = cursor_x + PAD - BADGE_GAP;
    let svg = format!(
        "<g class=\"fn-chip\" data-fn=\"{}\">{}<rect class=\"fn-chip-box\" x=\"0\" y=\"0\" width=\"{width:.1}\" height=\"{CHIP_H:.1}\" rx=\"4\" />{inner}</g>",
        esc(name),
        title(&tip.join("\n"))
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

fn render_component(
    name: &str,
    comp: &Component,
    profiles: &IndexMap<String, Vec<String>>,
) -> ComponentBox {
    let name_w = text_w(name, NAME_CHAR_W);
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
        .map(|(cname, c)| (cname.clone(), render_component(cname, c, profiles)))
        .collect();

    let chips: Vec<(String, FnChip)> = comp
        .fns
        .iter()
        .map(|(fname, fc)| (fname.clone(), render_fn_chip(fname, fc)))
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
                    "capability `{b}` — declared by this component (§5.3). A component may only use capabilities it declares, and a `deny` rule can forbid it."
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
                    Some(rules) => format!("profile `{p}` = {}", rules.join(", ")),
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

    let mut tip = vec![format!("component {name} — anchored at {}", comp.anchor)];
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
        tip.push(format!("owns (sole mutator of): {}", comp.owns.join(", ")));
    }
    if let Some(p) = &comp.profile {
        tip.push(match profiles.get(p) {
            Some(rules) => format!("profile {p} = {}", rules.join(", ")),
            None => format!("profile {p} (not defined in this document)"),
        });
    }
    if comp.strict {
        tip.push("strict — item-tier architecture findings are errors".into());
    }

    let mut svg = format!(
        "<g class=\"component\" data-name=\"{}\">{}<rect class=\"component-box\" x=\"0\" y=\"0\" width=\"{box_w:.1}\" height=\"{box_h:.1}\" rx=\"6\" />",
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

    ComponentBox { width: box_w, height: box_h, svg, positions }
}

fn any_node_svg(x: f64, y: f64, rule: &str) -> String {
    format!(
        "<g class=\"any-node\">{tip}<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"14\" /><text class=\"any-label\" x=\"{x:.1}\" y=\"{:.1}\">*</text></g>",
        y + 4.0,
        tip = title(&format!(
            "* = any component — belongs to the rule `{rule}` alone. Wildcards have no shared identity, so two rules that both use `*` are unrelated."
        ))
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
/// nothing more).
fn resolve(
    token: &str,
    positions: &IndexMap<String, Rect>,
    leaf_index: &IndexMap<String, Vec<String>>,
) -> Result<Option<Rect>, RenderError> {
    if token.contains('.') {
        return Ok(positions.get(token).copied());
    }
    match leaf_index.get(token) {
        None => Ok(None),
        Some(paths) if paths.len() == 1 => Ok(positions.get(&paths[0]).copied()),
        Some(paths) => Err(RenderError(format!(
            "ambiguous component reference {token:?}: matches {} — use the dotted qualified form (§5.1a rule 6)",
            paths.join(", ")
        ))),
    }
}

pub fn render_svg(doc: &Document) -> Result<String, RenderError> {
    let top: Vec<(String, ComponentBox)> = doc
        .components
        .iter()
        .map(|(name, c)| (name.clone(), render_component(name, c, &doc.profiles)))
        .collect();

    let content_w = top.iter().map(|(_, c)| c.width).fold(0.0_f64, f64::max);
    let mut frame_y = FRAME_PAD + FRAME_TITLE_H;
    let mut body = String::new();
    // Absolute canvas position of every named component, keyed by its fully
    // qualified dotted path (`parent.child`; a top-level component's own
    // name, with no prefix, is its own qualified path). §5.1a rule 6: an
    // edge/deny endpoint may use the bare leaf name only when that name is
    // unique across the whole merged tree; `leaf_index` (built below) is
    // what makes that uniqueness check possible.
    let mut positions: IndexMap<String, Rect> = IndexMap::new();

    for (name, cbox) in top {
        let x = FRAME_PAD;
        body.push_str(&wrap_translate(&cbox.svg, x, frame_y));
        positions
            .entry(name.clone())
            .or_insert(Rect { x, y: frame_y, w: cbox.width, h: cbox.height });
        for (n, r) in cbox.positions {
            positions
                .entry(format!("{name}.{n}"))
                .or_insert(Rect { x: x + r.x, y: frame_y + r.y, w: r.w, h: r.h });
        }
        frame_y += cbox.height + GAP;
    }

    let mut leaf_index: IndexMap<String, Vec<String>> = IndexMap::new();
    for qualified in positions.keys() {
        let leaf = qualified.rsplit('.').next().unwrap_or(qualified);
        leaf_index.entry(leaf.to_string()).or_default().push(qualified.clone());
    }

    let frame_w = content_w + FRAME_PAD * 2.0;
    let frame_h = frame_y + FRAME_PAD;

    // §7.1 unresolved marker, registry case: entries with no code anchor
    // pin to the workspace frame itself rather than to any component.
    let mut registry_svg = String::new();
    if !doc.unresolved.is_empty() {
        let mut px = frame_w - FRAME_PAD - PIN_R;
        let py = FRAME_TITLE_H / 2.0 + 6.0;
        for entry in &doc.unresolved {
            let label = format!("#{}", entry.id);
            registry_svg.push_str(&format!(
                "<g class=\"registry-pin\">{tip}<circle cx=\"{px:.1}\" cy=\"{py:.1}\" r=\"{PIN_R:.1}\" /><text class=\"pin-label\" x=\"{px:.1}\" y=\"{:.1}\">{label}</text></g>",
                py + 4.0,
                label = esc(&label),
                tip = title(&format!(
                    "unresolved #{} — workspace-level, no code anchor yet (§5.6)\n{}",
                    entry.id, entry.note
                ))
            ));
            px -= PIN_R * 2.0 + 6.0 + text_w(&label, 6.0);
        }
    }

    let mut edges_svg = String::new();
    for e in &doc.edges {
        if let Ok(edge) = parse_edge(e) {
            render_edge(&edge, &positions, &leaf_index, &mut edges_svg)?;
        }
    }

    // Deny rules. §7.1 (amended): `*` has no shared identity, so each rule
    // that names it draws its own pseudo-node — never one shared node that
    // would visually imply unrelated rules are connected.
    let mut deny_svg = String::new();
    for (i, d) in doc.deny.iter().enumerate() {
        if let Ok(deny) = parse_deny(d) {
            render_deny(i, &deny, &positions, &leaf_index, frame_w, &mut deny_svg)?;
        }
    }

    let width = frame_w;
    let height = frame_h;

    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.1}\" height=\"{height:.1}\" \
         viewBox=\"0 0 {width:.1} {height:.1}\" font-family=\"monospace\" font-size=\"12\">\
         <style>{STYLE}</style>\
         <defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"8\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\">\
         <path d=\"M 0 0 L 10 5 L 0 10 z\" /></marker></defs>\
         <rect class=\"workspace-frame\" x=\"1\" y=\"1\" width=\"{frame_inner_w:.1}\" height=\"{frame_inner_h:.1}\" rx=\"8\" />\
         <text class=\"workspace-title\" x=\"{FRAME_PAD:.1}\" y=\"20\">ply.yaml</text>\
         {deny_svg}{registry_svg}{body}{edges_svg}\
         </svg>",
        frame_inner_w = width - 2.0,
        frame_inner_h = height - 2.0,
    ))
}

fn render_edge(
    edge: &Edge,
    positions: &IndexMap<String, Rect>,
    leaf_index: &IndexMap<String, Vec<String>>,
    out: &mut String,
) -> Result<(), RenderError> {
    let Some(from) = resolve(&edge.from, positions, leaf_index)? else {
        return Ok(());
    };
    let Some(to) = resolve(&edge.to, positions, leaf_index)? else {
        return Ok(());
    };
    let (fx, fy) = from.border_toward((to.cx(), to.cy()));
    let (tx, ty) = to.border_toward((from.cx(), from.cy()));
    match &edge.kind {
        EdgeKind::Call => {
            out.push_str(&format!(
                "<g class=\"edge-call\">{tip}<path class=\"edge-line\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" marker-end=\"url(#arrow)\" /></g>",
                tip = title(&format!(
                    "{} -> {} — permitted call: {} may call into {}",
                    edge.from, edge.to, edge.from, edge.to
                ))
            ));
        }
        EdgeKind::Flow(ty_label) => {
            let mx = (fx + tx) / 2.0;
            let my = (fy + ty) / 2.0;
            out.push_str(&format!(
                "<g class=\"edge-flow\">{tip}<path class=\"edge-line\" stroke-dasharray=\"6 4\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" marker-end=\"url(#arrow)\" /><text class=\"edge-label\" x=\"{mx:.1}\" y=\"{my:.1}\">{}</text></g>",
                esc(ty_label),
                tip = title(&format!(
                    "{} ~> {} : {ty_label} — declared data flow, carrying {ty_label}",
                    edge.from, edge.to
                ))
            ));
        }
    }
    Ok(())
}

/// Renders one deny rule. §7.1 (amended): `*` has no shared identity, so a
/// rule that names it draws its own pseudo-node, anchored near whichever
/// side did resolve to a real component (or staggered by rule index, as a
/// last resort, if neither side resolved).
fn render_deny(
    index: usize,
    deny: &Deny,
    positions: &IndexMap<String, Rect>,
    leaf_index: &IndexMap<String, Vec<String>>,
    frame_w: f64,
    out: &mut String,
) -> Result<(), RenderError> {
    let from_rect = if deny.from == "*" {
        None
    } else {
        match resolve(&deny.from, positions, leaf_index)? {
            Some(r) => Some(r),
            None => return Ok(()), // unresolvable, non-wildcard pattern: draw nothing
        }
    };
    let to_rect = if deny.to == "*" {
        None
    } else {
        match resolve(&deny.to, positions, leaf_index)? {
            Some(r) => Some(r),
            None => return Ok(()),
        }
    };

    let fallback_y = FRAME_TITLE_H / 2.0 + 6.0 + index as f64 * 34.0;
    let rule_text = format!("{} -> {}", deny.from, deny.to);
    let mut any_nodes = String::new();

    let from_pt = if deny.from == "*" {
        let p = to_rect.map(|r| (14.0, r.cy())).unwrap_or((14.0, fallback_y));
        any_nodes.push_str(&any_node_svg(p.0, p.1, &rule_text));
        p
    } else {
        let r = from_rect.expect("non-wildcard, unresolved case already returned above");
        (r.cx(), r.cy())
    };

    let to_pt = if deny.to == "*" {
        let p = from_rect
            .map(|r| (frame_w - 14.0, r.cy()))
            .unwrap_or((frame_w - 14.0, fallback_y));
        any_nodes.push_str(&any_node_svg(p.0, p.1, &rule_text));
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

    out.push_str(&format!(
        "<g class=\"deny-rule\">{tip}<path class=\"deny-line\" d=\"M {fx:.1} {fy:.1} L {tx:.1} {ty:.1}\" /><line class=\"deny-bar\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" />",
        mx - px, my - py, mx + px, my + py,
        tip = title(&{
            let mut t = format!(
                "denied: {} -> {} — this call is an architecture violation",
                deny.from, deny.to
            );
            if !deny.except.is_empty() {
                t.push_str(&format!("\nexcept: {}", deny.except.join(", ")));
            }
            t
        })
    ));
    if !deny.except.is_empty() {
        out.push_str(&format!(
            "<text class=\"deny-except\" x=\"{mx:.1}\" y=\"{:.1}\">except {}</text>",
            my + 14.0,
            esc(&deny.except.join(", "))
        ));
    }
    out.push_str("</g>");

    Ok(())
}
