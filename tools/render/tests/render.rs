use ply_render::model::parse_document;
use ply_render::svg::render_svg;

fn render_fixture(path: &str) -> String {
    let yaml = std::fs::read_to_string(path).unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    render_svg(&doc).expect("fixture should render")
}

/// Every point of this renderer's own `d="M x y L x2 y2 (L x3 y3)*"` path
/// format. Routed edges and deny lines are polylines (they elbow around
/// boxes), so a two-point assumption would be wrong.
fn parse_path_points(d: &str) -> Vec<(f64, f64)> {
    let nums: Vec<f64> = d
        .split_whitespace()
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    assert!(
        nums.len() >= 4 && nums.len().is_multiple_of(2),
        "expected an M..L.. path with an even count of at least 4 numbers, got {d:?}"
    );
    nums.chunks(2).map(|c| (c[0], c[1])).collect()
}

/// First and last point of a path — its two ends, whatever it does between.
fn parse_line_path(d: &str) -> ((f64, f64), (f64, f64)) {
    let pts = parse_path_points(d);
    (pts[0], pts[pts.len() - 1])
}

/// Do two line segments properly cross (or touch)? Standard orientation test.
fn segments_cross(a: ((f64, f64), (f64, f64)), b: ((f64, f64), (f64, f64))) -> bool {
    fn orient(p: (f64, f64), q: (f64, f64), r: (f64, f64)) -> f64 {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    }
    fn on_seg(p: (f64, f64), q: (f64, f64), r: (f64, f64)) -> bool {
        q.0 <= p.0.max(r.0) + 0.5
            && q.0 >= p.0.min(r.0) - 0.5
            && q.1 <= p.1.max(r.1) + 0.5
            && q.1 >= p.1.min(r.1) - 0.5
    }
    let (p1, q1) = a;
    let (p2, q2) = b;
    let (d1, d2, d3, d4) = (
        orient(p1, q1, p2),
        orient(p1, q1, q2),
        orient(p2, q2, p1),
        orient(p2, q2, q1),
    );
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) {
        return true;
    }
    (d1.abs() < 1e-9 && on_seg(p1, p2, q1))
        || (d2.abs() < 1e-9 && on_seg(p1, q2, q1))
        || (d3.abs() < 1e-9 && on_seg(p2, p1, q2))
        || (d4.abs() < 1e-9 && on_seg(p2, q1, q2))
}

/// Does a segment enter a rect? (Endpoint inside, or crossing an edge.)
fn segment_hits_rect(seg: ((f64, f64), (f64, f64)), (x, y, w, h): (f64, f64, f64, f64)) -> bool {
    let inside = |p: (f64, f64)| p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h;
    if inside(seg.0) || inside(seg.1) {
        return true;
    }
    let corners = [
        ((x, y), (x + w, y)),
        ((x + w, y), (x + w, y + h)),
        ((x + w, y + h), (x, y + h)),
        ((x, y + h), (x, y)),
    ];
    corners.iter().any(|e| segments_cross(seg, *e))
}

fn svg_dims(doc: &roxmltree::Document) -> (f64, f64) {
    let root = doc.root_element();
    let w: f64 = root.attribute("width").unwrap().parse().unwrap();
    let h: f64 = root.attribute("height").unwrap().parse().unwrap();
    (w, h)
}

/// Absolute bounding rect of a top-level `<g class="component" data-name=X>`:
/// the sum of every ancestor `translate(x,y)` plus its own `component-box`
/// rect (always drawn at local origin, per `render_component`).
fn absolute_component_rect(doc: &roxmltree::Document, name: &str) -> (f64, f64, f64, f64) {
    let node = doc
        .descendants()
        .find(|n| {
            n.tag_name().name() == "g"
                && n.attribute("class") == Some("component")
                && n.attribute("data-name") == Some(name)
        })
        .unwrap_or_else(|| panic!("no component named {name:?} found"));

    let mut x = 0.0;
    let mut y = 0.0;
    let mut cur = Some(node);
    while let Some(n) = cur {
        if let Some(t) = n.attribute("transform")
            && let Some(inner) = t
                .strip_prefix("translate(")
                .and_then(|s| s.strip_suffix(")"))
        {
            let parts: Vec<f64> = inner
                .split(',')
                .map(|p| p.trim().parse().unwrap())
                .collect();
            x += parts[0];
            y += parts[1];
        }
        cur = n.parent();
    }

    // The component-box rect now also carries a `ceiling-*` class alongside
    // `component-box`/`component-box-finding` (§7.1 "declared ceiling"), so
    // this can no longer be an exact-string match against the whole `class`
    // attribute — check membership in the space-separated token list instead.
    let rect = node
        .children()
        .find(|c| {
            c.tag_name().name() == "rect"
                && c.attribute("class").is_some_and(|cl| {
                    cl.split_whitespace()
                        .any(|t| t == "component-box" || t == "component-box-finding")
                })
        })
        .expect("component must have a component-box rect");
    let w: f64 = rect.attribute("width").unwrap().parse().unwrap();
    let h: f64 = rect.attribute("height").unwrap().parse().unwrap();
    (x, y, w, h)
}

/// Is `(px, py)` on the boundary of the rect `(x, y, w, h)`, within a small
/// float-rounding tolerance (coordinates are formatted to 1 decimal place)?
fn on_boundary(px: f64, py: f64, (x, y, w, h): (f64, f64, f64, f64), tol: f64) -> bool {
    let on_vertical = (px - x).abs() <= tol || (px - (x + w)).abs() <= tol;
    let on_horizontal = (py - y).abs() <= tol || (py - (y + h)).abs() <= tol;
    let within_x = px >= x - tol && px <= x + w + tol;
    let within_y = py >= y - tol && py <= y + h + tol;
    (on_vertical && within_y) || (on_horizontal && within_x)
}

/// §7.1 layout invariants, pinned against vetting 002's "Findings from the
/// render pass": call-edge stubs, labels on arrowheads, deny nodes clipped
/// off-canvas, and coincident parallel edges. Each was RED against the
/// pre-layout renderer before the fix (see vetting/002-ingest-pipeline.md).
mod layout_invariants {
    use super::*;
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    #[test]
    fn everything_renders_inside_the_canvas() {
        // Every fixture that renders, in its default form AND collapsed to
        // depth 1 — the collapsed form draws the stack cue (a card edge
        // offset +5,+5 behind the box), which is exactly the kind of
        // overhang this invariant exists to catch at the canvas edge.
        let fixtures = [
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "tests/fixtures/hollow.ply.yaml",
            "tests/fixtures/qualified_refs.ply.yaml",
            "tests/fixtures/deny_stress.ply.yaml",
        ];
        let variants: Vec<(String, String)> = fixtures
            .iter()
            .flat_map(|fixture| {
                let yaml = std::fs::read_to_string(fixture).unwrap();
                let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
                [
                    (
                        format!("{fixture} (default)"),
                        render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}")),
                    ),
                    (
                        format!("{fixture} (--depth 1)"),
                        render_svg_with_options(
                            &doc,
                            &RenderOptions {
                                depth: Some(1),
                                ..Default::default()
                            },
                        )
                        .unwrap_or_else(|e| panic!("{fixture} --depth 1: {e}")),
                    ),
                ]
            })
            .collect();
        for (fixture, svg) in &variants {
            let doc = roxmltree::Document::parse(svg).unwrap();
            let (w, h) = svg_dims(&doc);

            // Text elements have no computed bounding box in the SVG
            // itself, so approximate one: monospace at a generous per-char
            // width (8, the widest the renderer uses, for `component-name`)
            // extended from the anchor per the element's `text-anchor`
            // (`middle` extends both ways from center; the SVG default,
            // `start`, extends only rightward). Good enough to catch a
            // label whose *anchor point* is on-canvas but whose glyphs
            // still run off the edge (vetting 002 finding 3's truncated
            // "except decoder").
            const WORST_CASE_CHAR_W: f64 = 8.0;
            let style = ply_render::svg::STYLE;
            let is_middle_anchored = |class: Option<&str>| -> bool {
                let Some(c) = class else { return false };
                let needle = format!(".{c}{{");
                style
                    .find(&needle)
                    .and_then(|start| {
                        style[start..]
                            .find('}')
                            .map(|end| &style[start..start + end])
                    })
                    .is_some_and(|rule| rule.contains("text-anchor:middle"))
            };

            for node in doc.descendants().filter(|n| n.is_element()) {
                let tag = node.tag_name().name();
                // Coordinates in the SVG are local to their group: every
                // ancestor `translate(x,y)` shifts them. Without summing
                // those, anything nested inside a component <g> is checked
                // at its local origin and the invariant is blind to it.
                let (ox, oy) = {
                    let (mut ox, mut oy) = (0.0f64, 0.0f64);
                    let mut cur = node.parent();
                    while let Some(n) = cur {
                        if let Some(t) = n.attribute("transform")
                            && let Some(inner) = t
                                .strip_prefix("translate(")
                                .and_then(|s| s.strip_suffix(")"))
                        {
                            let parts: Vec<f64> = inner
                                .split(',')
                                .map(|p| p.trim().parse().unwrap())
                                .collect();
                            ox += parts[0];
                            oy += parts[1];
                        }
                        cur = n.parent();
                    }
                    (ox, oy)
                };
                let pts: Vec<(f64, f64)> = match tag {
                    "rect" => {
                        let x: f64 = node.attribute("x").unwrap_or("0").parse().unwrap();
                        let y: f64 = node.attribute("y").unwrap_or("0").parse().unwrap();
                        let rw: f64 = node.attribute("width").unwrap_or("0").parse().unwrap();
                        let rh: f64 = node.attribute("height").unwrap_or("0").parse().unwrap();
                        vec![(x, y), (x + rw, y + rh)]
                    }
                    "circle" => {
                        let cx: f64 = node.attribute("cx").unwrap().parse().unwrap();
                        let cy: f64 = node.attribute("cy").unwrap().parse().unwrap();
                        let r: f64 = node.attribute("r").unwrap().parse().unwrap();
                        vec![(cx - r, cy - r), (cx + r, cy + r)]
                    }
                    "line" => {
                        let x1: f64 = node.attribute("x1").unwrap().parse().unwrap();
                        let y1: f64 = node.attribute("y1").unwrap().parse().unwrap();
                        let x2: f64 = node.attribute("x2").unwrap().parse().unwrap();
                        let y2: f64 = node.attribute("y2").unwrap().parse().unwrap();
                        vec![(x1, y1), (x2, y2)]
                    }
                    "path" => {
                        if node.ancestors().any(|a| a.tag_name().name() == "defs") {
                            continue; // the arrowhead marker glyph, not real canvas geometry
                        }
                        let d = node.attribute("d").unwrap();
                        parse_path_points(d)
                    }
                    "text" => {
                        let x: f64 = node.attribute("x").unwrap_or("0").parse().unwrap();
                        let y: f64 = node.attribute("y").unwrap_or("0").parse().unwrap();
                        let chars = node.text().unwrap_or("").chars().count() as f64;
                        let full_w = chars * WORST_CASE_CHAR_W;
                        if is_middle_anchored(node.attribute("class")) {
                            vec![(x - full_w / 2.0, y), (x + full_w / 2.0, y)]
                        } else {
                            vec![(x, y), (x + full_w, y)]
                        }
                    }
                    _ => continue,
                };
                for (px, py) in pts {
                    let (px, py) = (px + ox, py + oy);
                    assert!(
                        px >= -0.5 && px <= w + 0.5 && py >= -0.5 && py <= h + 0.5,
                        "{fixture}: <{tag}> point ({px},{py}) outside canvas {w}x{h}"
                    );
                }
            }
        }
    }

    #[test]
    fn edge_labels_clear_the_arrowheads() {
        let svg = render_fixture("../../vetting/002-ingest-pipeline.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();

        let mut checked = 0;
        for flow in doc
            .descendants()
            .filter(|n| n.attribute("class") == Some("edge-flow"))
        {
            let path = flow
                .children()
                .find(|c| c.tag_name().name() == "path")
                .expect("edge-flow must have a line");
            let (_, arrow_tip) = parse_line_path(path.attribute("d").unwrap());

            let label = flow
                .children()
                .find(|c| {
                    c.tag_name().name() == "text" && c.attribute("class") == Some("edge-label")
                })
                .expect("edge-flow must have a label");
            let lx: f64 = label.attribute("x").unwrap().parse().unwrap();
            let ly: f64 = label.attribute("y").unwrap().parse().unwrap();

            let dist = ((lx - arrow_tip.0).powi(2) + (ly - arrow_tip.1).powi(2)).sqrt();
            assert!(
                dist >= 14.0,
                "flow label at ({lx},{ly}) sits on/near the arrowhead at {arrow_tip:?} (dist {dist})"
            );
            checked += 1;
        }
        assert!(checked > 0, "fixture has no flow edges to check");
    }

    #[test]
    fn every_declared_edge_is_visibly_drawn() {
        let svg = render_fixture("../../vetting/002-ingest-pipeline.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();

        // decoder -> ring, decoder -> book, feed -> ring: all top-level, all
        // named in vetting 002's edges list.
        let declared_call_pairs = [("feed", "ring"), ("decoder", "ring"), ("decoder", "book")];

        for (from, to) in declared_call_pairs {
            let from_rect = absolute_component_rect(&doc, from);
            let to_rect = absolute_component_rect(&doc, to);

            let line = doc
                .descendants()
                .filter(|n| n.attribute("class") == Some("edge-call"))
                .find_map(|g| {
                    let path = g.children().find(|c| c.tag_name().name() == "path")?;
                    let (a, b) = parse_line_path(path.attribute("d").unwrap());
                    let a_near_from = on_boundary(a.0, a.1, from_rect, 1.0);
                    let b_near_to = on_boundary(b.0, b.1, to_rect, 1.0);
                    (a_near_from && b_near_to).then_some((a, b))
                });

            let (a, b) = line.unwrap_or_else(|| {
                panic!("no call edge found connecting {from}'s and {to}'s box boundaries")
            });
            let len = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            assert!(
                len >= 30.0,
                "{from} -> {to} call edge is only {len} units long: {a:?} -> {b:?}"
            );
        }
    }

    #[test]
    fn parallel_edges_do_not_coincide() {
        let svg = render_fixture("../../vetting/002-ingest-pipeline.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();

        let mut seen_lines: Vec<((f64, f64), (f64, f64))> = Vec::new();
        for g in doc
            .descendants()
            .filter(|n| matches!(n.attribute("class"), Some("edge-call") | Some("edge-flow")))
        {
            let path = g
                .children()
                .find(|c| c.tag_name().name() == "path")
                .unwrap();
            let line = parse_line_path(path.attribute("d").unwrap());
            assert!(
                !seen_lines.contains(&line),
                "two edges render the exact same line {line:?} — parallel edges must use \
                 distinct lanes (vetting 002 finding 4)"
            );
            seen_lines.push(line);
        }
        // decoder<->ring (call + flow, opposite directions) and feed->ring /
        // decoder->book (call + flow, same direction) are exactly the
        // vetting-002 cases; make sure this test actually exercised them.
        assert!(
            seen_lines.len() >= 6,
            "expected at least 6 call/flow edges, saw {}",
            seen_lines.len()
        );
    }
}

#[test]
fn svg_root_element_is_well_formed_enough_to_open() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.starts_with("<svg"));
    assert!(svg.trim_end().ends_with("</svg>"));
    assert_eq!(svg.matches("<svg").count(), 1);
    // every opened group is closed
    assert_eq!(
        svg.matches("<g ").count() + svg.matches("<g>").count(),
        svg.matches("</g>").count()
    );

    let doc = roxmltree::Document::parse(&svg).expect("output must be well-formed XML");
    assert_eq!(doc.root_element().tag_name().name(), "svg");
}

#[test]
fn component_renders_as_box_with_name_and_anchor_subtitle() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    // The component-box rect also carries a `ceiling-*` class (§7.1 "declared
    // ceiling"), so this checks token membership rather than an exact
    // `class="component-box"` substring.
    let doc = roxmltree::Document::parse(&svg).unwrap();
    assert!(doc.descendants().any(|n| {
        n.tag_name().name() == "rect"
            && n.attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|t| t == "component-box"))
    }));
    assert!(svg.contains(">parser<"));
    assert!(svg.contains(">app::parser<"));
}

#[test]
fn nested_component_renders_as_nested_box() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains(">curves<"));
    assert!(svg.contains(">app::pricing::curves<"));
}

#[test]
fn pure_component_gets_sealed_border_and_no_capability_badges() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"pure-seal\""));
    // pricing is pure and declares no `uses`, so it must carry no cap badges;
    // parser is not pure and uses fs+net, so those badges must exist elsewhere.
    assert!(svg.contains(">fs<"));
    assert!(svg.contains(">net<"));
    assert!(svg.contains(">db<"));
}

#[test]
fn profile_renders_as_tag_on_the_box() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"profile-tag\""));
    assert!(svg.contains(">hot_path<"));
}

#[test]
fn fn_chip_shows_checks_glyph_row() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"fn-chip\""));
    assert!(svg.contains(">quote<"));
    // bounded(3), fuzz(1024), mutate -> "B3 F1024 M"
    assert!(svg.contains("B3 F1024 M"));
    // test, bounded(4) -> "T B4"
    assert!(svg.contains("T B4"));
}

#[test]
fn check_with_is_noted_on_the_fn_chip() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("T=u64"));
}

#[test]
fn trusted_claim_renders_hollow_shield_badge() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"fn-shield\""));
    assert!(svg.contains('\u{26C9}')); // ⛉
}

#[test]
fn unresolved_fn_marker_renders_numbered_pin() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"unresolved-pin\""));
    assert!(svg.contains(">#147<"));
}

#[test]
fn top_level_registry_entry_pins_to_workspace_frame() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"registry-pin\""));
    assert!(svg.contains(">#151<"));
}

#[test]
fn call_edge_renders_solid_arrow_between_boxes() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"edge-call\""));
    assert!(svg.contains("marker-end"));
}

#[test]
fn flow_edge_renders_dashed_arrow_labeled_with_type() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"edge-flow\""));
    assert!(svg.contains("stroke-dasharray"));
    assert!(svg.contains(">pricing::Quote<"));
}

#[test]
fn deny_rule_renders_red_barred_arrow_with_any_pseudo_node() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"deny-rule\""));
    assert!(svg.contains("class=\"deny-bar\""));
    assert!(svg.contains("class=\"any-node\""));
    assert!(svg.contains(">*<"));
    assert!(svg.contains("except migrations"));
}

#[test]
fn each_deny_rule_draws_its_own_any_node_not_shared() {
    // §7.1 (amended): `*` has no shared identity across rules. Two
    // unrelated deny rules must each get their own pseudo-node rather than
    // being drawn as if fanning out from one shared "any" node.
    let yaml = r#"
ply: 1
components:
  risk:
    anchor: app::risk
  db_raw:
    anchor: app::db_raw
deny:
  - "* -> risk"
  - "* -> db_raw"
"#;
    let doc = parse_document(yaml).expect("doc should parse");
    let svg = render_svg(&doc).expect("doc should render");
    assert_eq!(
        svg.matches("class=\"any-node\"").count(),
        2,
        "each wildcard deny rule must draw its own any-node: {svg}"
    );
}

#[test]
fn unique_bare_component_name_resolves() {
    // §5.1a rule 6: a bare name that is unique across the whole merged
    // component tree resolves without qualification.
    let svg = render_fixture("tests/fixtures/qualified_refs.ply.yaml");
    // "sink -> alpha": both bare, both globally unique.
    assert_eq!(svg.matches("class=\"edge-call\"").count(), 3);
}

#[test]
fn qualified_dotted_path_resolves_the_exact_nested_component() {
    // §5.1a rule 6: `alpha` and `beta` each nest a component literally named
    // `shared` — the bare name `shared` is ambiguous, but the qualified
    // forms `alpha.shared` and `beta.shared` must resolve exactly and
    // independently (proven by both edges rendering, not erroring or
    // silently collapsing into one).
    let svg = render_fixture("tests/fixtures/qualified_refs.ply.yaml");
    assert_eq!(
        svg.matches("class=\"edge-call\"").count(),
        3,
        "alpha.shared -> sink, beta.shared -> sink, and sink -> alpha must all resolve: {svg}"
    );
}

#[test]
fn ambiguous_bare_reference_is_a_hard_error_naming_candidates() {
    // §5.1a rule 6: `shared` is ambiguous here (both `alpha.shared` and
    // `beta.shared` exist), so the bare reference must be a hard render
    // error naming every candidate, not a silent first-match guess.
    let yaml = std::fs::read_to_string("tests/fixtures/ambiguous_ref.ply.yaml").unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    let err = render_svg(&doc).expect_err("ambiguous bare reference must be rejected");
    // Exact plain-language wording, not just "it mentions the candidates" —
    // a newbie reading this on `ply-render`'s stderr gets no benefit from a
    // bare `matches X, Y` dump or a bare §-cite.
    assert_eq!(
        err.to_string(),
        "ambiguous component reference \"shared\": it could mean alpha.shared or beta.shared \
         — write the dotted form (e.g. alpha.shared) to say which"
    );
}

#[test]
fn all_component_boxes_are_unfilled_since_nothing_is_verified_yet() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    // §7.1: verdict is `unclaimed` for every node pre-verify -> unfilled boxes.
    assert!(!svg.contains("fill=\"red\""));
    assert!(!svg.contains("fill=\"green\""));
}

#[test]
fn rendering_is_byte_identical_across_runs() {
    let a = render_fixture("tests/fixtures/full.ply.yaml");
    let b = render_fixture("tests/fixtures/full.ply.yaml");
    assert_eq!(a, b);
}

#[test]
fn disruptor_fixture_golden_snapshot() {
    let svg = render_fixture("../../vetting/001-spsc-disruptor.ply.yaml");
    roxmltree::Document::parse(&svg).expect("disruptor svg must be well-formed XML");
    insta::assert_snapshot!(svg);
}

/// The bug this catches: the renderer emitted well-formed, correctly-classed
/// SVG with no stylesheet at all. SVG's initial paint is `fill: black`, so the
/// whole diagram rasterised as one solid black rectangle while every
/// structural assertion above still passed.
///
/// The invariant is per painted element, not per class: `<g>` wrappers paint
/// nothing, and some shapes are styled through an ancestor's descendant
/// selector (`.cap-badge rect`), so a shape passes if any class on its own
/// element or its ancestors resolves a rule.
#[test]
fn every_painted_element_resolves_a_style_rule() {
    // §7.1 finding classes live in a separate constant (`FINDING_STYLE`,
    // only appended to a document's actual `<style>` when it has a
    // finding — see its doc comment), so checking selector resolution
    // needs both, regardless of which a given fixture below happens to use.
    let style = format!(
        "{}{}",
        ply_render::svg::STYLE,
        ply_render::svg::FINDING_STYLE
    );
    let matches_selector = |class: &str, tag: &str| {
        style.contains(&format!(".{class}{{"))
            || style.contains(&format!(".{class},"))
            || style.contains(&format!(".{class} {tag}{{"))
            || style.contains(&format!(".{class} {tag},"))
    };

    let mut unstyled: Vec<String> = Vec::new();
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/qualified_refs.ply.yaml",
        // §7.1 gate-debt closure: `strict`, `mode: synth`, `examples`.
        "tests/fixtures/visual_forms.ply.yaml",
        // docs/plans/external-elements.md: external box, `~>` edges
        // touching one, and a derived `entry:` edge.
        "tests/fixtures/externals.ply.yaml",
        // §7.1 finding coverage: these fixtures each carry a real finding
        // that must resolve through `FINDING_STYLE`, not just parse clean.
        "../check/tests/fixtures/bad_check_syntax.ply.yaml",
        "../check/tests/fixtures/bad_path_form.ply.yaml",
        "../check/tests/fixtures/duplicate_unresolved_id.ply.yaml",
        "../../demos/fault3.ply.yaml",
        "tests/fixtures/strict_with_finding.ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        assert!(svg.contains("<style>"), "{fixture}: no stylesheet emitted");
        let doc = roxmltree::Document::parse(&svg).unwrap();
        for node in doc.descendants().filter(|n| n.is_element()) {
            let tag = node.tag_name().name();
            if !matches!(tag, "rect" | "circle" | "path" | "text" | "line") {
                continue;
            }
            // The arrowhead lives in <defs> and is styled through its marker id.
            if node.ancestors().any(|a| a.tag_name().name() == "defs") {
                continue;
            }
            // A class attribute may now carry more than one space-separated
            // token (the component-box rect stacks `component-box` with a
            // `ceiling-*` class, §7.1 "declared ceiling") — check each token
            // on its own rather than the whole attribute string at once.
            let resolved = node
                .ancestors()
                .filter_map(|a| a.attribute("class"))
                .flat_map(|c| c.split_whitespace())
                .any(|c| matches_selector(c, tag));
            if !resolved {
                unstyled.push(format!(
                    "{fixture}: <{tag}> class={:?}",
                    node.attribute("class")
                ));
            }
        }
    }
    assert!(
        unstyled.is_empty(),
        "painted elements with no style rule: {unstyled:?}"
    );
}

#[test]
fn owns_renders_as_a_header_line() {
    let svg = render_fixture("../../vetting/001-spsc-disruptor.ply.yaml");
    assert!(svg.contains("class=\"component-owns\""));
    assert!(svg.contains(">owns disruptor::spsc::Spsc<"));
}

#[test]
fn glyphs_are_explained_by_a_hover_title() {
    let svg = render_fixture("../../vetting/001-spsc-disruptor.ply.yaml");
    let doc = roxmltree::Document::parse(&svg).unwrap();
    let titles: Vec<String> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "title")
        .map(|n| n.text().unwrap_or("").to_string())
        .collect();

    // Coverage is asserted by `every_drawn_item_resolves_a_tooltip`; this test
    // checks the wording a reader actually needs.
    let push = titles
        .iter()
        .find(|t| t.starts_with("Spsc::try_push"))
        .unwrap();
    assert!(push.contains(
        "bounded(3) — proves the contract for every input, unrolling loops at most 3 times"
    ));
    assert!(push.contains(
        "fuzz(1024) — runs the function on 1024 random inputs, checking the contract on each"
    ));
    assert!(
        push.contains("generic — every check ran with T=u64; the evidence covers only that type")
    );
    assert!(push.contains(
        "trusted (a human vouches for this; no machine checks it): SPSC cross-thread safety"
    ));
    assert!(push.contains("loom test tests/loom_spsc.rs"));
    assert!(push.contains("1 worked example(s), each compiled into a test"));

    // The component tooltip expands its profile — the tag alone shows only a
    // name. Each rule name is glossed in plain language too: a newbie who
    // has never seen "no_panics" or "exhaustive_match" gets no benefit from
    // the bare name alone.
    let ring = titles
        .iter()
        .find(|t| t.starts_with("component ring"))
        .unwrap();
    assert!(ring.contains(
        "profile hot_path — a named bundle of extra rules this component must follow: \
         no_panics (functions here must never panic (crash on purpose)), exhaustive_match \
         (every match must handle all cases explicitly)"
    ));
    assert!(ring.contains("capabilities: unsafe"));
    assert!(ring.contains("owns disruptor::spsc::Spsc — only this component may mutate them"));

    // The profile-tag badge itself (hovered directly, not via the component
    // box) carries the same glossed wording.
    let profile_tag = titles
        .iter()
        .find(|t| t.starts_with("profile `hot_path`"))
        .unwrap();
    assert_eq!(
        profile_tag,
        "profile `hot_path` — a named bundle of extra rules this component must follow: \
         no_panics (functions here must never panic (crash on purpose)), exhaustive_match \
         (every match must handle all cases explicitly)"
    );

    // A pure component draws a double border; the tooltip must explain that
    // visual, not just assert the fact (vetting 002: "why does decoder have
    // a double line?").
    let svg = render_fixture("../../vetting/002-ingest-pipeline.ply.yaml");
    let doc = roxmltree::Document::parse(&svg).unwrap();
    let decoder = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "title")
        .filter_map(|n| n.text())
        .find(|t| t.starts_with("component decoder"))
        .unwrap();
    assert!(
        decoder.contains("double border"),
        "pure tooltip must explain the double border, got: {decoder}"
    );
    assert!(decoder.contains("no capabilities"));

    // Workspace-level unresolved marker (registry pin, #7 in this fixture):
    // plain-language wording for a reader who has never seen Ply before.
    let registry_pin = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "title")
        .filter_map(|n| n.text())
        .find(|t| t.starts_with("#7"))
        .unwrap();
    assert_eq!(
        registry_pin,
        "#7 marks an unresolved decision — a question the design still owes an answer: \
         backpressure policy when the ring is full: drop frame vs spin. It belongs to the \
         workspace as a whole, not to any function or component yet; Ply tracks it until \
         someone resolves it (§5.6)."
    );
}

/// The workspace frame is the first thing a newbie sees — hovering it must
/// explain the whole picture in plain language, not assume the reader has
/// already read The-Ply-Spec.md.
#[test]
fn workspace_frame_explains_the_whole_picture() {
    let svg = render_fixture("../../vetting/001-spsc-disruptor.ply.yaml");
    let doc = roxmltree::Document::parse(&svg).unwrap();
    let title = doc
        .descendants()
        .find(|n| n.attribute("class") == Some("workspace-frame"))
        .and_then(|n| n.children().find(|c| c.tag_name().name() == "title"))
        .and_then(|t| t.text())
        .expect("workspace-frame should carry a tooltip");
    assert_eq!(
        title,
        "This diagram is drawn from ply.yaml, the file describing this codebase's \
         architecture and verification claims. Each box is a component; chips are \
         functions with their declared checks; arrows are permitted calls (solid) and \
         data flows (dashed); red bars are forbidden calls. A box's green depth is the \
         strength of the checks it declares — white means something inside declares \
         none, deeper green means stronger checks, and the weakest function sets the \
         whole box's shade. It is a promise scale, not results: none of it has run \
         yet. Hover anything for its meaning."
    );
}

/// "Tooltips for all items": the invariant, not a spot-check. Every drawn item
/// — the workspace frame, component, fn chip, badge, tag, shield, pin, arrow,
/// deny bar, wildcard node — must resolve a `<title>` on itself or an
/// ancestor, so nothing in the picture is unexplainable by hovering it.
#[test]
fn every_drawn_item_resolves_a_tooltip() {
    // This walked a hand-maintained list of classes, which meant its name
    // was an overclaim: of the 35 classes the renderer actually emits, the
    // list named 14. A construct added later was explained only if someone
    // remembered to add it here, and nothing failed if they did not --
    // which is the same shape of silent absence this project treats as a
    // defect everywhere else (2026-08-28, prompted by a smoke test
    // reporting that the check badges carry no tooltip; they do, but
    // checking that claim showed the test could not have told us either
    // way).
    //
    // Inverted: every class the renderer emits must resolve a tooltip, on
    // itself or on an ancestor -- which is what a reader hovering it gets.
    // Anything that genuinely cannot carry one has to be named in
    // DECORATION below, so a new construct fails this test until someone
    // decides which it is, rather than passing by omission.
    const DECORATION: &[&str] = &[];

    let mut untitled: Vec<String> = Vec::new();
    let mut classes_seen = 0usize;
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/qualified_refs.ply.yaml",
        "tests/fixtures/visual_forms.ply.yaml",
        "tests/fixtures/externals.ply.yaml",
        "tests/fixtures/deny_stress.ply.yaml",
        "tests/fixtures/hollow.ply.yaml",
        "tests/fixtures/strict_with_finding.ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let mut seen_here = 0usize;
        for node in doc.descendants().filter(|n| n.is_element()) {
            let Some(class) = node.attribute("class") else {
                continue;
            };
            if DECORATION.contains(&class) {
                continue;
            }
            seen_here += 1;
            let titled = node
                .ancestors()
                .any(|a| a.children().any(|c| c.tag_name().name() == "title"));
            if !titled {
                untitled.push(format!("{fixture}: .{class}"));
            }
        }
        assert!(seen_here > 0, "{fixture}: nothing drawn carried a class");
        classes_seen += seen_here;
    }
    assert!(
        classes_seen > 100,
        "these fixtures are meant to exercise the whole grammar; only {classes_seen} \
         classed elements were drawn, so this test is checking far less than it looks"
    );
    untitled.sort();
    untitled.dedup();
    assert!(
        untitled.is_empty(),
        "drawn items a reader can point at, with nothing to tell them what they are: \
         {untitled:?}"
    );
}

/// §7.1 "declared ceiling": walks every fixture below, recomputes each
/// component's ceiling with an oracle built independently in this test (real
/// `ply_kernel::aggregate`, not the renderer's own tree-building code), and
/// checks the rendered SVG's component-box fill class against it. This is
/// the invariant, not a spot-check: a construct added later that the
/// renderer forgets to feed into its own ceiling computation fails here on
/// its own fixture, without a bespoke assertion for it.
mod declared_ceiling {
    use super::*;
    use ply_kernel::{Evidence, NodeKind, VerdictNode, aggregate};
    use ply_render::model::{
        Check, Component, FnClaim, InheritedChecks, component_default_checks, effective_checks,
        parse_check,
    };
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    /// The strongest check *kind* one fn's *effective* checks list declares
    /// (The-Ply-Spec.md §7.1: `test` -> tested, `fuzz` -> fuzzed, `bounded`
    /// -> bounded, `prove` -> proved; `mutate` and an unparseable string
    /// contribute nothing). Deliberately re-derived here from `parse_check`,
    /// not imported from the renderer, so this is a real second opinion
    /// rather than the same code asserting against itself — the effective
    /// list itself comes from `ply_core::model::effective_checks`, the single
    /// shared resolution of §5.1's component-default inheritance rule that
    /// both `ply-check` and this renderer must agree with (re-deriving that
    /// resolution independently here would just be a second copy of the
    /// same rule to keep in sync).
    fn fn_ceiling(fc: &FnClaim, inherited: Option<InheritedChecks>) -> Evidence {
        let mut best: Option<Evidence> = None;
        for c in effective_checks(fc, inherited).unwrap_or(&[]) {
            let kind = match parse_check(c) {
                Ok(Check::Test) => Some(Evidence::Tested),
                Ok(Check::Fuzz(_)) => Some(Evidence::Fuzzed),
                Ok(Check::Bounded(_)) => Some(Evidence::Bounded),
                Ok(Check::Prove) => Some(Evidence::Proved),
                Ok(Check::Mutate) | Err(_) => None,
            };
            if let Some(k) = kind {
                best = Some(best.map_or(k, |b: Evidence| if k > b { k } else { b }));
            }
        }
        best.unwrap_or(Evidence::Unclaimed)
    }

    fn verdict_node<'a>(
        name: &'a str,
        comp: &'a Component,
        inherited: Option<InheritedChecks<'a>>,
    ) -> VerdictNode {
        let this_default = component_default_checks(name, comp, inherited);
        let mut children: Vec<VerdictNode> = comp
            .fns
            .values()
            .map(|fc| VerdictNode {
                kind: NodeKind::Claimable(fn_ceiling(fc, this_default)),
                statuses: ply_kernel::StatusSet::new(),
                conditional: None,
                children: Vec::new(),
            })
            .collect();
        children.extend(
            comp.components
                .iter()
                .map(|(cname, c)| verdict_node(cname, c, this_default)),
        );
        VerdictNode {
            kind: NodeKind::Container,
            statuses: ply_kernel::StatusSet::new(),
            conditional: None,
            children,
        }
    }

    /// This component's own ceiling, then each nested component's,
    /// recursively — the same preorder the renderer draws component boxes
    /// in (a box is emitted before its own nested children's boxes).
    fn expected_ceilings<'a>(
        name: &'a str,
        comp: &'a Component,
        inherited: Option<InheritedChecks<'a>>,
    ) -> Vec<Evidence> {
        let mut out = vec![aggregate(&verdict_node(name, comp, inherited)).evidence];
        let this_default = component_default_checks(name, comp, inherited);
        for (cname, c) in &comp.components {
            out.extend(expected_ceilings(cname, c, this_default));
        }
        out
    }

    /// The `ceiling-*` class actually painted on each component-box rect, in
    /// the SVG's document order (which is the same preorder as
    /// `expected_ceilings` — see that fn's doc comment).
    fn rendered_ceiling_classes(svg: &str) -> Vec<String> {
        let doc = roxmltree::Document::parse(svg).unwrap();
        doc.descendants()
            .filter(|n| n.tag_name().name() == "g" && n.attribute("class") == Some("component"))
            .map(|g| {
                let rect = g
                    .children()
                    .find(|c| {
                        c.tag_name().name() == "rect"
                            && c.attribute("class").is_some_and(|cl| {
                                cl.split_whitespace()
                                    .any(|t| t == "component-box" || t == "component-box-finding")
                            })
                    })
                    .expect("every component must draw a component-box rect");
                rect.attribute("class")
                    .unwrap()
                    .split_whitespace()
                    .find(|t| t.starts_with("ceiling-"))
                    .unwrap_or_else(|| panic!("component-box rect has no ceiling-* class: {:?}", g))
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn every_component_fill_matches_the_kernel_ceiling() {
        // The three vetting scenarios plus the render crate's own fixtures
        // that render successfully (`ambiguous_ref.ply.yaml` is designed to
        // be a render *error*, so it has no fill to check).
        for fixture in [
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "tests/fixtures/qualified_refs.ply.yaml",
            // §5.1 checks inheritance: `pricing` declares a component-level
            // default that some fns inherit, some override, and one nested
            // component re-shadows — this is what pins the ceiling fill (and
            // the depth-1 collapsed fill below) to the real inherited
            // evidence rather than silently reading `unclaimed`.
            "tests/fixtures/checks_inheritance.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));

            let mut expected: Vec<Evidence> = Vec::new();
            for (name, c) in &doc.components {
                expected.extend(expected_ceilings(name, c, None));
            }
            let expected_classes: Vec<String> = expected
                .iter()
                .map(|e| ply_render::svg::ceiling_class(*e).to_string())
                .collect();
            let rendered = rendered_ceiling_classes(&svg);

            assert_eq!(
                expected_classes, rendered,
                "{fixture}: rendered ceiling-* classes (in document order) don't match the \
                 kernel-recomputed ceilings"
            );

            // Collapsed to depth 1, only the top-level boxes remain — and
            // each must wear the ceiling the kernel computes for its FULL
            // subtree (a collapsed box folds its contents, not its
            // verdict). `rendered_ceiling_classes` reads one class per
            // drawn component box, so the expectation is simply the
            // top-level components' subtree ceilings, in document order.
            let depth1_svg = render_svg_with_options(
                &doc,
                &RenderOptions {
                    depth: Some(1),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{fixture} --depth 1: {e}"));
            let expected_depth1: Vec<String> = doc
                .components
                .iter()
                .map(|(name, c)| {
                    ply_render::svg::ceiling_class(aggregate(&verdict_node(name, c, None)).evidence)
                        .to_string()
                })
                .collect();
            assert_eq!(
                expected_depth1,
                rendered_ceiling_classes(&depth1_svg),
                "{fixture} --depth 1: collapsed boxes' fills don't match the kernel's \
                 subtree ceilings"
            );
        }
    }

    /// Names the levels vetting 003 must show, so a future refactor that
    /// keeps the invariant test above green by accident (e.g. everything
    /// collapsing to the same ceiling) still gets caught: risk's declared
    /// ceiling is `bounded` (from `check_order`'s bounded/fuzz/mutate list)
    /// and must read strictly stronger than strategy's `tested` (dragged
    /// down by `Strategy::on_update`'s plain `test`), and `ingest` must read
    /// `unclaimed` because `Feed::pump` declares no checks at all.
    #[test]
    fn trading_system_ceilings_have_the_expected_relative_depth() {
        let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
        let doc = parse_document(&yaml).unwrap();
        let ceiling_of =
            |name: &str| aggregate(&verdict_node(name, &doc.components[name], None)).evidence;
        assert_eq!(ceiling_of("ingest"), Evidence::Unclaimed);
        assert_eq!(ceiling_of("strategy"), Evidence::Tested);
        assert_eq!(ceiling_of("risk"), Evidence::Bounded);
        assert!(ceiling_of("risk") > ceiling_of("strategy"));
    }
}

/// §7.1 "contract clauses": any fn claim with a non-empty `requires` or
/// `ensures` must draw the contract mark and list its clauses verbatim in
/// its tooltip. The invariant, not a spot-check on `check_order` alone —
/// though `check_order` (vetting 003, risk component) is exactly the
/// fixture that first exercises it (its `requires`/`ensures` were added
/// alongside this feature).
mod contract_mark {
    use super::*;

    #[derive(Debug, Clone)]
    struct ExpectedFn {
        requires: Vec<String>,
        ensures: Vec<String>,
    }

    /// Fn claims in the same order the renderer draws their chips: a
    /// component's *nested* components' fns first (recursively), then its
    /// own — see `render_component`'s body-building loop (children before
    /// chips).
    fn walk_fn_claims(comp: &ply_render::model::Component, out: &mut Vec<ExpectedFn>) {
        for c in comp.components.values() {
            walk_fn_claims(c, out);
        }
        for fc in comp.fns.values() {
            out.push(ExpectedFn {
                requires: fc.requires.clone(),
                ensures: fc.ensures.clone(),
            });
        }
    }

    fn rendered_fn_chip_marks_and_tooltips(svg: &str) -> Vec<(bool, String)> {
        let doc = roxmltree::Document::parse(svg).unwrap();
        doc.descendants()
            .filter(|n| n.tag_name().name() == "g" && n.attribute("class") == Some("fn-chip"))
            .map(|g| {
                let has_mark = g
                    .children()
                    .any(|c| c.attribute("class") == Some("contract-mark"));
                let tooltip = g
                    .children()
                    .find(|c| c.tag_name().name() == "title")
                    .and_then(|t| t.text())
                    .unwrap_or_default()
                    .to_string();
                (has_mark, tooltip)
            })
            .collect()
    }

    #[test]
    fn every_contract_carrying_chip_shows_the_mark_and_clauses() {
        let mut any_contract_seen = false;
        for fixture in [
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "tests/fixtures/qualified_refs.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));

            let mut expected: Vec<ExpectedFn> = Vec::new();
            for c in doc.components.values() {
                walk_fn_claims(c, &mut expected);
            }
            let rendered = rendered_fn_chip_marks_and_tooltips(&svg);
            assert_eq!(
                expected.len(),
                rendered.len(),
                "{fixture}: fn claim count doesn't match rendered fn-chip count"
            );

            for (exp, (has_mark, tooltip)) in expected.iter().zip(rendered.iter()) {
                let has_contract = !exp.requires.is_empty() || !exp.ensures.is_empty();
                assert_eq!(
                    *has_mark, has_contract,
                    "{fixture}: contract-mark presence ({has_mark}) doesn't match whether the \
                     fn declares requires/ensures ({has_contract}); tooltip: {tooltip:?}"
                );
                if has_contract {
                    any_contract_seen = true;
                    assert!(
                        tooltip.contains("contract at the watermark:"),
                        "{fixture}: contract-carrying chip's tooltip is missing the header: \
                         {tooltip:?}"
                    );
                    for r in &exp.requires {
                        let line = format!("requires: {r}");
                        assert!(
                            tooltip.contains(&line),
                            "{fixture}: tooltip is missing {line:?}, got: {tooltip:?}"
                        );
                    }
                    for e in &exp.ensures {
                        let line = format!("ensures: {e}");
                        assert!(
                            tooltip.contains(&line),
                            "{fixture}: tooltip is missing {line:?}, got: {tooltip:?}"
                        );
                    }
                    assert!(
                        tooltip.contains(
                            "the checks above test the function against exactly this promise"
                        ),
                        "{fixture}: tooltip is missing the closing line, got: {tooltip:?}"
                    );
                } else {
                    assert!(
                        !tooltip.contains("contract at the watermark:"),
                        "{fixture}: a chip with no requires/ensures must not gain contract \
                         wording, got: {tooltip:?}"
                    );
                }
            }
        }
        assert!(
            any_contract_seen,
            "no fixture exercised a contract-carrying fn — this test would pass vacuously"
        );
    }

    /// Vetting 003's `risk.check_order` is the fixture this feature was
    /// written for — named explicitly so a regression there is never lost
    /// in the generic sweep above.
    #[test]
    fn check_order_shows_the_contract_mark() {
        let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
        let doc = parse_document(&yaml).unwrap();
        let svg = render_svg(&doc).unwrap();
        let doc_xml = roxmltree::Document::parse(&svg).unwrap();
        let chip = doc_xml
            .descendants()
            .find(|n| {
                n.tag_name().name() == "g"
                    && n.attribute("class") == Some("fn-chip")
                    && n.attribute("data-fn") == Some("check_order")
            })
            .expect("check_order chip must exist");
        assert!(
            chip.children()
                .any(|c| c.attribute("class") == Some("contract-mark")),
            "check_order must draw the contract mark"
        );
        let tooltip = chip
            .children()
            .find(|c| c.tag_name().name() == "title")
            .and_then(|t| t.text())
            .unwrap();
        assert!(tooltip.contains("requires: order.qty > 0 && order.px > 0"));
        assert!(tooltip.contains("ensures: |r| r.is_err() == (order.qty > limits.max_qty)"));
    }
}

/// §5.1 "checks: [bounded(2)] # optional default checks for all fns in
/// scope": a fn with no `checks` of its own must draw and describe the
/// *inherited* checks — the glyph row on its chip, and its tooltip — not
/// render as if it declared nothing. `tests/fixtures/checks_inheritance.
/// ply.yaml` exercises every shape the spec names: a direct default use, a
/// fn-level override, a nested component with its own default, and a
/// nested component without one (skipping to the grandparent's).
mod checks_inheritance {
    use super::*;

    fn fn_chip<'a>(doc: &'a roxmltree::Document, name: &str) -> roxmltree::Node<'a, 'a> {
        doc.descendants()
            .find(|n| {
                n.tag_name().name() == "g"
                    && n.attribute("class") == Some("fn-chip")
                    && n.attribute("data-fn") == Some(name)
            })
            .unwrap_or_else(|| panic!("no fn-chip named {name:?} found"))
    }

    fn glyph_text(chip: roxmltree::Node) -> String {
        chip.children()
            .find(|c| c.attribute("class") == Some("fn-checks"))
            .and_then(|t| t.text())
            .unwrap_or_default()
            .to_string()
    }

    fn tooltip_text(chip: roxmltree::Node) -> String {
        chip.children()
            .find(|c| c.tag_name().name() == "title")
            .and_then(|t| t.text())
            .unwrap_or_default()
            .to_string()
    }

    /// The glyph row (§7.1 "checks list -> glyph row on the fn chip") must
    /// reflect the *effective* list — own if declared, else inherited —
    /// for every shape the fixture exercises.
    #[test]
    fn glyph_row_shows_the_effective_checks_for_every_inheritance_shape() {
        let svg = render_fixture("tests/fixtures/checks_inheritance.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();

        // `quote` has no checks of its own -> inherits `pricing`'s bounded(2).
        assert_eq!(glyph_text(fn_chip(&doc, "quote")), "B2");
        // `book` declares its own `[test]`, which wins entirely.
        assert_eq!(glyph_text(fn_chip(&doc, "book")), "T");
        // `discount` has no checks of its own -> inherits `curves`'s
        // fuzz(64), not the grandparent `pricing`'s bounded(2) — nearest
        // ancestor wins.
        assert_eq!(glyph_text(fn_chip(&doc, "discount")), "F64");
        // `delta` has no checks of its own, and `greeks` declares no
        // default of its own either -> skips up to the grandparent
        // `pricing`'s bounded(2).
        assert_eq!(glyph_text(fn_chip(&doc, "delta")), "B2");
    }

    /// The tooltip must make the inheritance visible to a newbie: which
    /// component the check came from, and what the check itself means —
    /// exact wording, since CLAUDE.md pins user-facing sentences like code.
    #[test]
    fn tooltip_names_the_inherited_component_and_the_check() {
        let svg = render_fixture("tests/fixtures/checks_inheritance.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();

        let quote_tip = tooltip_text(fn_chip(&doc, "quote"));
        assert!(
            quote_tip.contains(
                "inherited from component `pricing`: bounded(2) — proves the contract for \
                 every input, unrolling loops at most 2 times"
            ),
            "quote's tooltip should name the inherited check and its source: {quote_tip:?}"
        );
        // An inherited fn is not "unclaimed" — the fallback line must not
        // appear once inheritance actually supplies a check.
        assert!(
            !quote_tip.contains("no checks declared"),
            "quote inherits a check, so it must not read as unclaimed: {quote_tip:?}"
        );

        // Nested: `discount` inherits from its own parent `curves`, not the
        // grandparent `pricing` — the tooltip must name the nearer one.
        let discount_tip = tooltip_text(fn_chip(&doc, "discount"));
        assert!(
            discount_tip.contains(
                "inherited from component `curves`: fuzz(64) — runs the function on 64 random \
                 inputs, checking the contract on each"
            ),
            "discount's tooltip should name curves, not pricing: {discount_tip:?}"
        );

        // Nested, skipping a level: `delta`'s own component `greeks`
        // declares no default, so it inherits the grandparent `pricing`'s.
        let delta_tip = tooltip_text(fn_chip(&doc, "delta"));
        assert!(
            delta_tip.contains(
                "inherited from component `pricing`: bounded(2) — proves the contract for \
                 every input, unrolling loops at most 2 times"
            ),
            "delta's tooltip should skip greeks (no default) up to pricing: {delta_tip:?}"
        );

        // `book` declares its own checks — no inheritance line at all.
        let book_tip = tooltip_text(fn_chip(&doc, "book"));
        assert!(
            !book_tip.contains("inherited from component"),
            "book declares its own checks and must not claim to inherit anything: {book_tip:?}"
        );
    }
}

mod hollow_and_gutter {
    use super::*;

    /// §7.1 "contract clauses" (amended): the mark is a gutter bar — full
    /// chip height, flush at the chip's left edge — because the original
    /// 6x6 square was too easy to miss at a glance.
    #[test]
    fn contract_mark_is_a_full_height_gutter_bar() {
        let svg = render_fixture("../../vetting/003-trading-system.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let mark = doc
            .descendants()
            .find(|n| n.attribute("class") == Some("contract-mark"))
            .expect("003's check_order declares clauses, so a mark must exist");
        let h: f64 = mark.attribute("height").unwrap().parse().unwrap();
        let x: f64 = mark.attribute("x").unwrap().parse().unwrap();
        let w: f64 = mark.attribute("width").unwrap().parse().unwrap();
        assert_eq!(h, 24.0, "gutter bar must span the full chip height");
        assert_eq!(x, 0.0, "gutter bar sits flush at the chip's left edge");
        assert!(w <= 4.0, "a gutter bar is thin, got width {w}");
    }

    /// §7.1 "hollow component": nothing declared inside — no fns, no nested
    /// components — draws dashed (a sketch outline, nothing to zoom into)
    /// and says so in the tooltip. The invariant runs both directions over
    /// every fixture: hollow iff dashed-and-explained.
    #[test]
    fn every_hollow_component_is_dashed_and_says_so() {
        for fixture in [
            "tests/fixtures/hollow.ply.yaml",
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "tests/fixtures/qualified_refs.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let sdoc = roxmltree::Document::parse(&svg).unwrap();

            fn walk(
                comp: &ply_render::model::Component,
                name: &str,
                out: &mut Vec<(String, bool)>,
            ) {
                out.push((
                    name.to_string(),
                    comp.fns.is_empty() && comp.components.is_empty(),
                ));
                for (n, c) in &comp.components {
                    walk(c, n, out);
                }
            }
            let mut expected = Vec::new();
            for (n, c) in &doc.components {
                walk(c, n, &mut expected);
            }

            for (name, is_hollow) in expected {
                let g = sdoc
                    .descendants()
                    .find(|n| {
                        n.attribute("class")
                            .is_some_and(|c| c.split(' ').any(|t| t == "component"))
                            && n.attribute("data-name") == Some(name.as_str())
                    })
                    .unwrap_or_else(|| panic!("{fixture}: no component group named {name:?}"));
                let box_classes = g
                    .children()
                    .find(|c| {
                        c.attribute("class")
                            .is_some_and(|cl| cl.split(' ').any(|t| t == "component-box"))
                    })
                    .and_then(|r| r.attribute("class"))
                    .unwrap_or_default()
                    .to_string();
                let dashed = box_classes.split(' ').any(|t| t == "hollow-box");
                let tooltip = g
                    .children()
                    .find(|c| c.tag_name().name() == "title")
                    .and_then(|t| t.text())
                    .unwrap_or_default()
                    .to_string();
                let explained = tooltip.contains("hollow — declares nothing inside yet");
                assert_eq!(
                    dashed, is_hollow,
                    "{fixture}: component {name:?} hollow={is_hollow} but dashed={dashed}"
                );
                assert_eq!(
                    explained, is_hollow,
                    "{fixture}: component {name:?} hollow={is_hollow} but tooltip explanation \
                     present={explained}: {tooltip:?}"
                );
            }
        }
    }
}

/// §7.1 collapse/expand: `ply-render --depth N` / `--focus <component>`.
mod collapse {
    use super::*;
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    fn component_node<'a>(
        doc: &'a roxmltree::Document<'a>,
        name: &str,
    ) -> Option<roxmltree::Node<'a, 'a>> {
        doc.descendants().find(|n| {
            n.tag_name().name() == "g"
                && n.attribute("class") == Some("component")
                && n.attribute("data-name") == Some(name)
        })
    }

    /// The regression guard: with neither flag, output must stay exactly
    /// what it always was. The committed vetting SVGs already are that
    /// "always was" (verified byte-identical to the current renderer before
    /// this feature existed) — read here, never written.
    #[test]
    fn default_output_is_unchanged_without_flags() {
        for (yaml_path, svg_path) in [
            (
                "../../vetting/001-spsc-disruptor.ply.yaml",
                "../../vetting/001-spsc-disruptor.svg",
            ),
            (
                "../../vetting/002-ingest-pipeline.ply.yaml",
                "../../vetting/002-ingest-pipeline.svg",
            ),
            (
                "../../vetting/003-trading-system.ply.yaml",
                "../../vetting/003-trading-system-full.svg",
            ),
        ] {
            let svg = render_fixture(yaml_path);
            let expected = std::fs::read_to_string(svg_path).unwrap();
            assert_eq!(
                svg, expected,
                "{yaml_path}: default (no flags) output must stay byte-identical to the \
                 committed vetting SVG"
            );
        }
    }

    /// §7.1: "A collapsed component is one solid-bordered box ... showing
    /// its name, anchor, a contents line (`N components · M fns`), and its
    /// worst-descendant ceiling/verdict fill. Three things never fold away:
    /// capability badges ..., the unresolved-pin count, and the finding
    /// count." Checked against vetting 003's `ingest` at `--depth 1`.
    #[test]
    fn collapsed_box_shows_counts_caps_pins_and_subtree_ceiling() {
        let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
        let doc = parse_document(&yaml).unwrap();
        let svg = render_svg_with_options(
            &doc,
            &RenderOptions {
                depth: Some(1),
                focus: None,
                ..Default::default()
            },
        )
        .unwrap();
        let xml = roxmltree::Document::parse(&svg).unwrap();

        // ingest is one box: none of its nested components draw their own.
        for leaf in ["feed", "ring", "decoder", "book"] {
            assert!(
                component_node(&xml, leaf).is_none(),
                "{leaf} must not be drawn as its own box at --depth 1"
            );
        }

        let ingest = component_node(&xml, "ingest").expect("ingest box must exist");
        let box_rect = ingest
            .children()
            .find(|c| {
                c.attribute("class")
                    .is_some_and(|cl| cl.split_whitespace().any(|t| t == "component-box"))
            })
            .expect("ingest must have a component-box rect");
        let classes: Vec<&str> = box_rect
            .attribute("class")
            .unwrap()
            .split_whitespace()
            .collect();
        assert!(
            classes.contains(&"ceiling-unclaimed"),
            "ingest's worst descendant (Feed::pump, no checks) must drag its ceiling to \
             unclaimed, got {classes:?}"
        );

        let contents = ingest
            .children()
            .filter(|c| c.tag_name().name() == "text")
            .filter_map(|t| t.text())
            .find(|t| t.contains("component") && t.contains("fn"))
            .expect("ingest must draw a contents line");
        assert_eq!(
            contents, "4 components · 7 fns",
            "ingest's recursive counts (feed+ring+decoder+book, and their fns) are wrong"
        );

        let badge_labels: Vec<&str> = ingest
            .descendants()
            .filter(|n| n.attribute("class") == Some("cap-badge"))
            .filter_map(|g| g.children().find(|c| c.tag_name().name() == "text"))
            .filter_map(|t| t.text())
            .collect();
        assert!(
            badge_labels.contains(&"net"),
            "ingest's collapsed badges must include feed's `net` (union of the subtree), got \
             {badge_labels:?}"
        );
        assert!(
            badge_labels.contains(&"unsafe"),
            "ingest's collapsed badges must include ring's `unsafe` (union of the subtree), got \
             {badge_labels:?}"
        );

        // the workspace registry pin (#8) is unaffected by any component
        // collapsing — it belongs to the workspace, not to a component.
        let registry_pin = xml
            .descendants()
            .filter(|n| n.tag_name().name() == "title")
            .filter_map(|n| n.text())
            .find(|t| t.starts_with("#8"));
        assert!(
            registry_pin.is_some(),
            "the #8 registry pin must still render at --depth 1"
        );
    }

    /// §7.1: "an edge whose endpoint is inside a collapsed component
    /// reattaches to the collapsed box itself." Checked against vetting
    /// 003's `strategy -> ingest.book` at `--depth 1`.
    #[test]
    fn edges_reattach_to_collapsed_ancestors() {
        let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
        let doc = parse_document(&yaml).unwrap();
        let svg = render_svg_with_options(
            &doc,
            &RenderOptions {
                depth: Some(1),
                focus: None,
                ..Default::default()
            },
        )
        .unwrap();
        let xml = roxmltree::Document::parse(&svg).unwrap();

        let strategy_rect = absolute_component_rect(&xml, "strategy");
        let ingest_rect = absolute_component_rect(&xml, "ingest");

        let mut found = false;
        for node in xml
            .descendants()
            .filter(|n| n.attribute("class") == Some("edge-call"))
        {
            let path = node
                .children()
                .find(|c| c.tag_name().name() == "path")
                .expect("edge-call must draw a path");
            let (from, to) = parse_line_path(path.attribute("d").unwrap());
            if on_boundary(from.0, from.1, strategy_rect, 1.0)
                && on_boundary(to.0, to.1, ingest_rect, 1.0)
            {
                found = true;
            }
        }
        assert!(
            found,
            "no edge-call runs from strategy's box boundary to ingest's box boundary — the \
             strategy -> ingest.book edge must reattach to the collapsed ingest box"
        );
    }

    /// §7.1: `--collapse <component>` (repeatable, dotted paths allowed)
    /// folds exactly the named component(s); everything else renders fully
    /// expanded — the opposite selection bias to `--depth`/`--focus`.
    #[test]
    fn collapse_flag_folds_only_the_named_component() {
        let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
        let doc = parse_document(&yaml).unwrap();
        let svg = render_svg_with_options(
            &doc,
            &RenderOptions {
                depth: None,
                focus: None,
                collapse: vec!["ingest".to_string()],
            },
        )
        .unwrap();
        let xml = roxmltree::Document::parse(&svg).unwrap();

        // ingest folds...
        for leaf in ["feed", "ring", "decoder", "book"] {
            assert!(
                component_node(&xml, leaf).is_none(),
                "{leaf} must fold away under --collapse ingest"
            );
        }
        let ingest = component_node(&xml, "ingest").expect("ingest box must exist");
        let contents = ingest
            .children()
            .filter(|c| c.tag_name().name() == "text")
            .filter_map(|t| t.text())
            .find(|t| t.contains("component") && t.contains("fn"))
            .expect("ingest must draw a contents line");
        // feed(1) + ring(2) + decoder(1) + book(3) fns, 4 nested components.
        assert_eq!(contents, "4 components · 7 fns");
        let badge_labels: Vec<&str> = ingest
            .descendants()
            .filter(|n| n.attribute("class") == Some("cap-badge"))
            .filter_map(|g| g.children().find(|c| c.tag_name().name() == "text"))
            .filter_map(|t| t.text())
            .collect();
        assert!(badge_labels.contains(&"net"));
        assert!(badge_labels.contains(&"unsafe"));

        // ...but everything else stays fully expanded, exactly as default.
        for name in ["strategy", "signals", "risk", "oms", "gateway", "pnl"] {
            assert!(
                component_node(&xml, name).is_some(),
                "{name} must still render its own box at --collapse ingest"
            );
        }
        let momentum_chip = xml.descendants().find(|n| {
            n.tag_name().name() == "g"
                && n.attribute("class") == Some("fn-chip")
                && n.attribute("data-fn") == Some("momentum")
        });
        assert!(
            momentum_chip.is_some(),
            "strategy.signals::momentum must still draw its fn chip — strategy is not collapsed"
        );
    }

    /// §7.1: "`--focus <component>` ... the focused component renders fully
    /// expanded, everything else collapses to depth 1."
    #[test]
    fn focus_expands_its_target_and_collapses_the_rest() {
        let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
        let doc = parse_document(&yaml).unwrap();
        let svg = render_svg_with_options(
            &doc,
            &RenderOptions {
                depth: None,
                focus: Some("ingest".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let xml = roxmltree::Document::parse(&svg).unwrap();

        // the focus target's whole subtree is fully expanded.
        for leaf in ["feed", "ring", "decoder", "book"] {
            assert!(
                component_node(&xml, leaf).is_some(),
                "{leaf} must still be drawn at its own box when its ancestor is the --focus \
                 target"
            );
        }

        // everything else collapses.
        assert!(
            component_node(&xml, "signals").is_none(),
            "signals must collapse away — it is outside the --focus target"
        );
    }

    /// §7.1 invariant coverage at depth: the two whole-document invariants
    /// (every painted element resolves a style rule; every drawn item
    /// resolves a tooltip) must still hold once collapsing is in play — a
    /// construct that only shows up in the collapsed form must not slip
    /// through unstyled or untitled.
    #[test]
    fn invariants_hold_at_depth_1() {
        let style = format!(
            "{}{}",
            ply_render::svg::STYLE,
            ply_render::svg::FINDING_STYLE
        );
        let matches_selector = |class: &str, tag: &str| {
            style.contains(&format!(".{class}{{"))
                || style.contains(&format!(".{class},"))
                || style.contains(&format!(".{class} {tag}{{"))
                || style.contains(&format!(".{class} {tag},"))
        };
        const ITEM_CLASSES: &[&str] = &[
            "workspace-frame",
            "component",
            "fn-chip",
            "cap-badge",
            "profile-tag",
            "fn-shield",
            "unresolved-pin",
            "registry-pin",
            "edge-call",
            "edge-flow",
            "deny-rule",
            "any-node",
            "external",
            "edge-entry",
        ];

        let mut unstyled: Vec<String> = Vec::new();
        let mut untitled: Vec<String> = Vec::new();
        for fixture in [
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "tests/fixtures/qualified_refs.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg_with_options(
                &doc,
                &RenderOptions {
                    depth: Some(1),
                    focus: None,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let xml = roxmltree::Document::parse(&svg).unwrap();

            for node in xml.descendants().filter(|n| n.is_element()) {
                let tag = node.tag_name().name();
                if matches!(tag, "rect" | "circle" | "path" | "text" | "line") {
                    if node.ancestors().any(|a| a.tag_name().name() == "defs") {
                        continue;
                    }
                    let resolved = node
                        .ancestors()
                        .filter_map(|a| a.attribute("class"))
                        .flat_map(|c| c.split_whitespace())
                        .any(|c| matches_selector(c, tag));
                    if !resolved {
                        unstyled.push(format!(
                            "{fixture}: <{tag}> class={:?}",
                            node.attribute("class")
                        ));
                    }
                }
                let Some(class) = node.attribute("class") else {
                    continue;
                };
                if !ITEM_CLASSES.contains(&class) {
                    continue;
                }
                let titled = node
                    .ancestors()
                    .any(|a| a.children().any(|c| c.tag_name().name() == "title"));
                if !titled {
                    untitled.push(format!("{fixture}: .{class}"));
                }
            }
        }
        assert!(
            unstyled.is_empty(),
            "painted elements with no style rule at --depth 1: {unstyled:?}"
        );
        untitled.sort();
        untitled.dedup();
        assert!(
            untitled.is_empty(),
            "drawn items with no tooltip at --depth 1: {untitled:?}"
        );
    }
}

/// §7.1 "collision-freedom inside containers" (vetting 003's render-pass
/// findings 1, 3, 4): every symptom reported there — intra-container edge
/// labels sitting on neighboring boxes, same-rank deny rules overlapping
/// each other, a deny bar striking its own `except` label — is really the
/// same missing property: nothing drawn should intersect a box it isn't
/// inside. This walks the real rendered output (every fixture, default and
/// `--depth 1`) and fails on the first offender, so a construct added later
/// that skips this property is caught here rather than needing a bespoke
/// spot-check of its own.
mod no_overlap {
    use super::*;
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    type Rectf = (f64, f64, f64, f64); // (x, y, w, h)

    /// Sums every ancestor `translate(x,y)` above `node` — the same
    /// accumulation `absolute_component_rect` does for box rects. Needed for
    /// any element read by its raw `x`/`y` attributes (text) that may be
    /// nested several component/chip `<g transform>`s deep, where those
    /// attributes are local to the innermost group, not absolute canvas
    /// coordinates.
    fn absolute_offset(node: roxmltree::Node) -> (f64, f64) {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut cur = node.parent();
        while let Some(n) = cur {
            if let Some(t) = n.attribute("transform")
                && let Some(inner) = t
                    .strip_prefix("translate(")
                    .and_then(|s| s.strip_suffix(")"))
            {
                let parts: Vec<f64> = inner
                    .split(',')
                    .map(|p| p.trim().parse().unwrap())
                    .collect();
                x += parts[0];
                y += parts[1];
            }
            cur = n.parent();
        }
        (x, y)
    }

    /// Anchor-aware text bounding box, the same worst-case monospace
    /// estimate `everything_renders_inside_the_canvas` uses, widened into a
    /// real rect with a generous glyph height guessed around the baseline
    /// (big enough for any font-size this renderer uses, 9-13px), and
    /// converted to absolute canvas coordinates via `absolute_offset` —
    /// most text (component names, fn chips, badges) is nested several
    /// `<g transform>`s deep, so its raw `x`/`y` attributes alone are not
    /// its real position.
    /// Returns the label's bounding box plus its raw anchor point (the
    /// exact `x`/`y` the SVG places it at, absolute-adjusted) — the anchor
    /// is what `check_point_item` uses to decide whether the label
    /// legitimately belongs inside a given box (see that function's doc
    /// comment): a label whose own anchor sits deep inside a box belongs
    /// there; one whose anchor sits outside a box it merely bleeds *into*
    /// (via this worst-case width estimate) does not.
    fn text_bbox(node: roxmltree::Node, style: &str) -> (Rectf, (f64, f64)) {
        const WORST_CASE_CHAR_W: f64 = 8.0;
        let (ox, oy) = absolute_offset(node);
        let x: f64 = node.attribute("x").unwrap_or("0").parse::<f64>().unwrap() + ox;
        let y: f64 = node.attribute("y").unwrap_or("0").parse::<f64>().unwrap() + oy;
        let chars = node.text().unwrap_or("").chars().count() as f64;
        let full_w = chars * WORST_CASE_CHAR_W;
        let is_middle = node.attribute("class").is_some_and(|c| {
            let needle = format!(".{c}{{");
            style
                .find(&needle)
                .and_then(|start| {
                    style[start..]
                        .find('}')
                        .map(|end| &style[start..start + end])
                })
                .is_some_and(|rule| rule.contains("text-anchor:middle"))
        });
        let (x0, x1) = if is_middle {
            (x - full_w / 2.0, x + full_w / 2.0)
        } else {
            (x, x + full_w)
        };
        ((x0, y - 11.0, (x1 - x0).max(0.1), 14.0), (x, y))
    }

    /// Parses this renderer's `d="M x y L x y L x y ..."` path format (a
    /// straight 2-point line, or — for a deny line routed around an
    /// obstruction — a longer polyline) into its ordered points.
    fn parse_path_points(d: &str) -> Vec<(f64, f64)> {
        let nums: Vec<f64> = d
            .split_whitespace()
            .filter_map(|t| t.parse::<f64>().ok())
            .collect();
        nums.chunks(2).map(|c| (c[0], c[1])).collect()
    }

    /// Is `p` inside (or on the boundary of, within `eps`) rect `r`? Used to
    /// decide whether a box is one a line's endpoint is legitimately
    /// attached to (or nested inside — a nested component's own box rect
    /// sits geometrically inside every ancestor container's rect too, by
    /// construction, so this single check permits both without needing to
    /// know the containment chain explicitly).
    fn box_contains_point(r: Rectf, p: (f64, f64), eps: f64) -> bool {
        p.0 >= r.0 - eps && p.0 <= r.0 + r.2 + eps && p.1 >= r.1 - eps && p.1 <= r.1 + r.3 + eps
    }

    /// Bounding box of every point in a (possibly multi-point, routed)
    /// line, padded by `pad` on every side.
    fn line_bbox(points: &[(f64, f64)], pad: f64) -> Rectf {
        let x0 = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - pad;
        let y0 = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - pad;
        let x1 = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) + pad;
        let y1 = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) + pad;
        (x0, y0, (x1 - x0).max(0.1), (y1 - y0).max(0.1))
    }

    fn rects_overlap(a: Rectf, b: Rectf, eps: f64) -> bool {
        a.0 + eps < b.0 + b.2
            && a.0 + a.2 > b.0 + eps
            && a.1 + eps < b.1 + b.3
            && a.1 + a.3 > b.1 + eps
    }

    /// Standard slab (Liang-Barsky-style) segment-vs-rect clip, run against
    /// `rect` shrunk inward by `shrink` on every side. A line whose only
    /// contact with `rect` is a point exactly on its true boundary — the
    /// normal case for an edge/deny endpoint legitimately touching the box
    /// it terminates on — lands just *outside* the shrunk rect, so this
    /// reports no intersection for that case; a line that actually cuts
    /// across the box's interior is still caught.
    fn segment_crosses_interior(p0: (f64, f64), p1: (f64, f64), rect: Rectf, shrink: f64) -> bool {
        let (x, y, w, h) = rect;
        let (xmin, xmax) = (x + shrink, x + w - shrink);
        let (ymin, ymax) = (y + shrink, y + h - shrink);
        if xmin >= xmax || ymin >= ymax {
            return false; // box too small to have a meaningful interior
        }
        let mut t_enter = 0.0_f64;
        let mut t_exit = 1.0_f64;
        for &(p, d, lo, hi) in &[
            (p0.0, p1.0 - p0.0, xmin, xmax),
            (p0.1, p1.1 - p0.1, ymin, ymax),
        ] {
            if d.abs() < 1e-9 {
                if p < lo || p > hi {
                    return false;
                }
            } else {
                let (ta, tb) = ((lo - p) / d, (hi - p) / d);
                let (ta, tb) = if ta < tb { (ta, tb) } else { (tb, ta) };
                t_enter = t_enter.max(ta);
                t_exit = t_exit.min(tb);
                if t_enter > t_exit {
                    return false;
                }
            }
        }
        t_enter < t_exit - 1e-9
    }

    /// Every `<g class="component">`'s own g-node (for ancestor/containment
    /// checks below) paired with its absolute on-canvas rect.
    fn all_component_boxes<'a>(
        doc: &'a roxmltree::Document<'a>,
    ) -> Vec<(roxmltree::Node<'a, 'a>, Rectf)> {
        let mut out = Vec::new();
        for g in doc
            .descendants()
            .filter(|n| n.tag_name().name() == "g" && n.attribute("class") == Some("component"))
        {
            let mut x = 0.0;
            let mut y = 0.0;
            let mut cur = Some(g);
            while let Some(n) = cur {
                if let Some(t) = n.attribute("transform")
                    && let Some(inner) = t
                        .strip_prefix("translate(")
                        .and_then(|s| s.strip_suffix(")"))
                {
                    let parts: Vec<f64> = inner
                        .split(',')
                        .map(|p| p.trim().parse().unwrap())
                        .collect();
                    x += parts[0];
                    y += parts[1];
                }
                cur = n.parent();
            }
            let Some(rect) = g.children().find(|c| {
                c.tag_name().name() == "rect"
                    && c.attribute("class").is_some_and(|cl| {
                        cl.split_whitespace()
                            .any(|t| t == "component-box" || t == "component-box-finding")
                    })
            }) else {
                continue;
            };
            let w: f64 = rect.attribute("width").unwrap().parse().unwrap();
            let h: f64 = rect.attribute("height").unwrap().parse().unwrap();
            out.push((g, (x, y, w, h)));
        }
        out
    }

    /// A text label or a wildcard node: excluded from a box's crossing
    /// check when that box's rect contains the element's own anchor point
    /// (the text's `x`/`y`, or the wildcard node's center) — the same
    /// "contains the real attachment point" test `check_line_item` uses
    /// for lines, and for the same reason: a component-name/fn-chip/badge
    /// label is nested (and hence anchored) inside its own component's box
    /// and every ancestor container's box by construction, and — since
    /// vetting 003 finding 1's fix — a flow-edge label between two of a
    /// container's own children is legitimately anchored inside that
    /// container's box too, even though neither is DOM-nested inside it. A
    /// genuinely misplaced label's anchor sits *outside* every box it only
    /// bleeds into via this worst-case width estimate, so this still
    /// catches it.
    fn check_point_item(
        anchor: (f64, f64),
        bbox: Rectf,
        boxes: &[(roxmltree::Node, Rectf)],
        fixture: &str,
        label: &str,
        what: &str,
        violations: &mut Vec<String>,
    ) {
        for (g, box_rect) in boxes {
            if box_contains_point(*box_rect, anchor, 1.0) {
                continue;
            }
            if rects_overlap(bbox, *box_rect, 0.5) {
                violations.push(format!(
                    "{fixture} ({label}): {what} {bbox:?} intersects {:?}'s box {box_rect:?}",
                    g.attribute("data-name")
                ));
            }
        }
    }

    /// A line (edge/deny path, deny bar): excluded from a box's crossing
    /// check when that box contains (not just touches — a nested
    /// component's own rect sits fully inside every ancestor container's
    /// rect too, by construction) either of the line's two real endpoints.
    /// Lines are drawn at document-root level, never DOM-nested inside a
    /// component `<g>`, so ancestry can't do this job the way it does for
    /// interior text — only geometry can. Every intermediate segment of a
    /// multi-point (routed) path is still checked against the interior of
    /// every box this exclusion doesn't cover.
    fn check_line_item(
        points: &[(f64, f64)],
        boxes: &[(roxmltree::Node, Rectf)],
        fixture: &str,
        label: &str,
        what: &str,
        violations: &mut Vec<String>,
    ) {
        let (p_first, p_last) = (points[0], points[points.len() - 1]);
        for (g, box_rect) in boxes {
            if box_contains_point(*box_rect, p_first, 1.0)
                || box_contains_point(*box_rect, p_last, 1.0)
            {
                continue;
            }
            for seg in points.windows(2) {
                if segment_crosses_interior(seg[0], seg[1], *box_rect, 2.0) {
                    violations.push(format!(
                        "{fixture} ({label}): {what} {points:?} crosses {:?}'s box {box_rect:?}",
                        g.attribute("data-name")
                    ));
                    break;
                }
            }
        }
    }

    /// An edge label struck by its *own* line — the narrower property
    /// (vetting 003's coordinator review, second round): a first attempt at
    /// this checked every label against every drawn line in the document
    /// and immediately produced false positives on already-reviewed,
    /// unrelated designs -- a wildcard `*` any-node's own "*" glyph is
    /// legitimately touched by the deny line that starts there (that is
    /// the point: the line visibly emanates from the node it names), and a
    /// deny rule's `except` label sits deliberately close to its own
    /// routed line inside the reserved margin. Both are real, reviewed
    /// design choices this invariant has no business flagging. `edge-label`
    /// text (the payload-type label on a `~>`/derived `entry:` edge) has no
    /// such legitimate reason to touch a line at all -- it is meant to sit
    /// clearly *beside* its own path, never on it -- so the check is
    /// scoped to exactly that pairing: each `edge-label` against the
    /// points of the `<path>` sharing its own immediate `<g>` (the two are
    /// always drawn as siblings -- see `render_edge`/`render_external_edge`),
    /// not the general "any label vs. any line" sweep the coordinator's
    /// own fallback anticipated could be needed.
    /// Every drawn line's points, gathered in one pass — the same shapes
    /// `check_fixture`'s own line-matching arms already recognize: any
    /// edge's path (call, flow, or the derived `entry:` edge all share the
    /// `edge-line`/`edge-line-finding` class on the `<path>` itself,
    /// regardless of their wrapping `<g>`'s class), a deny line, or a deny
    /// bar.
    fn all_line_segments(doc: &roxmltree::Document) -> Vec<Vec<(f64, f64)>> {
        let mut out = Vec::new();
        for node in doc.descendants().filter(|n| n.is_element()) {
            if node.ancestors().any(|a| a.tag_name().name() == "defs") {
                continue; // the arrowhead marker glyph, not real canvas geometry
            }
            match (node.tag_name().name(), node.attribute("class")) {
                ("line", Some("deny-bar")) => {
                    let x1: f64 = node.attribute("x1").unwrap().parse().unwrap();
                    let y1: f64 = node.attribute("y1").unwrap().parse().unwrap();
                    let x2: f64 = node.attribute("x2").unwrap().parse().unwrap();
                    let y2: f64 = node.attribute("y2").unwrap().parse().unwrap();
                    out.push(vec![(x1, y1), (x2, y2)]);
                }
                (
                    "path",
                    Some("edge-line" | "edge-line-finding" | "deny-line" | "deny-line-finding"),
                ) => {
                    out.push(parse_path_points(node.attribute("d").unwrap()));
                }
                _ => {}
            }
        }
        out
    }

    /// An `edge-label` (the payload-type label on a `~>` edge, or the
    /// literal word on a derived `entry:` edge) struck by *any* drawn
    /// line — its own edge's included, but not only that: the real defect
    /// this was written for (vetting 003's coordinator review, second
    /// round) turned out to be `RawFrame`'s label struck by the
    /// *`entry:`* edge's line running close beside it, not by `RawFrame`'s
    /// own path — a same-edge-only check would have missed it entirely.
    /// Deliberately scoped to the `edge-label` class alone, not every
    /// drawn string: a first attempt checked *every* text node against
    /// every line and immediately produced real false positives on
    /// already-reviewed, unrelated designs — a wildcard `*` any-node's own
    /// glyph (class `any-label`) is legitimately touched by the deny line
    /// that starts there (the line is meant to visibly emanate from the
    /// node it names), and a deny rule's `except` text (class
    /// `deny-except`) sits deliberately close to its own routed line
    /// inside the reserved margin. Neither of those classes is
    /// `edge-label`, so scoping to that one class keeps the real property
    /// (an edge's payload/`entry` label must sit clear of every line) and
    /// drops the false positives, without needing the narrower
    /// "only its own path" fallback the coordinator's own message allowed
    /// for — that fallback was tried first and rejected here precisely
    /// because it would have missed the actual reported defect.
    fn edge_labels_struck_by_any_line(
        doc: &roxmltree::Document,
        style: &str,
        lines: &[Vec<(f64, f64)>],
        fixture: &str,
        label: &str,
        violations: &mut Vec<String>,
        known_pre_existing_gaps: &mut Vec<String>,
    ) {
        for text in doc
            .descendants()
            .filter(|n| n.tag_name().name() == "text" && n.attribute("class") == Some("edge-label"))
        {
            let (bbox, _) = text_bbox(text, style);
            // docs/external-elements-adoption.md, "the coverage gap, round
            // three": every `edge-label` this session's own work can draw
            // (an explicit `~>` touching an external, or a derived
            // `entry:` edge) carries this exact phrase in its enclosing
            // `<g>`'s tooltip (`external_flow_tooltip`/`entry_edge_
            // tooltip`, svg.rs) — a reliable, non-coordinate-based way to
            // tell "one of this session's own constructs" from "a regular,
            // pre-existing flow edge's label", since the two need
            // different verdicts here: a strike on the former is this
            // session's own defect to fix (and is fixed — this branch is
            // never reached for one); a strike on the latter is real but
            // predates this feature entirely (confirmed against the
            // pre-session commit, same coordinates) and needs a larger,
            // separate fix (a two-pass label-placement restructure for
            // *every* edge class, not just externals) this session did not
            // attempt — recorded, not silently swallowed.
            let is_external_construct = text
                .ancestors()
                .find_map(|a| a.children().find(|c| c.tag_name().name() == "title"))
                .and_then(|t| t.text().map(str::to_string))
                .is_some_and(|t| t.contains("outside this codebase"));
            for pts in lines {
                for seg in pts.windows(2) {
                    if segment_crosses_interior(seg[0], seg[1], bbox, 1.0) {
                        let msg = format!(
                            "{fixture} ({label}): edge label {bbox:?} is struck by a drawn line segment {seg:?}"
                        );
                        if is_external_construct {
                            violations.push(msg);
                        } else {
                            known_pre_existing_gaps.push(msg);
                        }
                        break;
                    }
                }
            }
        }
    }

    fn check_fixture(
        fixture: &str,
        label: &str,
        svg: &str,
        style: &str,
        violations: &mut Vec<String>,
        known_pre_existing_gaps: &mut Vec<String>,
    ) {
        let xml = roxmltree::Document::parse(svg).unwrap();
        let boxes = all_component_boxes(&xml);
        let lines = all_line_segments(&xml);
        edge_labels_struck_by_any_line(
            &xml,
            style,
            &lines,
            fixture,
            label,
            violations,
            known_pre_existing_gaps,
        );

        for node in xml.descendants().filter(|n| n.is_element()) {
            if node.ancestors().any(|a| a.tag_name().name() == "defs") {
                continue; // the arrowhead marker glyph, not real canvas geometry
            }
            match (node.tag_name().name(), node.attribute("class")) {
                ("text", _) => {
                    let (bbox, anchor) = text_bbox(node, style);
                    check_point_item(
                        anchor,
                        bbox,
                        &boxes,
                        fixture,
                        label,
                        "text label",
                        violations,
                    );
                }
                (_, Some("any-node")) => {
                    let circle = node.children().find(|c| c.tag_name().name() == "circle");
                    if let Some(circle) = circle {
                        let cx: f64 = circle.attribute("cx").unwrap().parse().unwrap();
                        let cy: f64 = circle.attribute("cy").unwrap().parse().unwrap();
                        let r: f64 = circle.attribute("r").unwrap().parse().unwrap();
                        check_point_item(
                            (cx, cy),
                            (cx - r, cy - r, r * 2.0, r * 2.0),
                            &boxes,
                            fixture,
                            label,
                            "wildcard node",
                            violations,
                        );
                    }
                }
                ("line", Some("deny-bar")) => {
                    let x1: f64 = node.attribute("x1").unwrap().parse().unwrap();
                    let y1: f64 = node.attribute("y1").unwrap().parse().unwrap();
                    let x2: f64 = node.attribute("x2").unwrap().parse().unwrap();
                    let y2: f64 = node.attribute("y2").unwrap().parse().unwrap();
                    check_line_item(
                        &[(x1, y1), (x2, y2)],
                        &boxes,
                        fixture,
                        label,
                        "deny bar",
                        violations,
                    );
                }
                (
                    "path",
                    Some("edge-line" | "edge-line-finding" | "deny-line" | "deny-line-finding"),
                ) => {
                    let points = parse_path_points(node.attribute("d").unwrap());
                    check_line_item(
                        &points,
                        &boxes,
                        fixture,
                        label,
                        "edge/deny line",
                        violations,
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn no_drawn_element_intersects_a_box_it_is_not_inside() {
        let style = format!(
            "{}{}",
            ply_render::svg::STYLE,
            ply_render::svg::FINDING_STYLE
        );
        let mut violations: Vec<String> = Vec::new();
        // docs/external-elements-adoption.md, "the coverage gap, round
        // three": real, pre-session, out-of-scope label/line collisions on
        // *regular* (non-external) edges — confirmed against the commit
        // that predates the external-elements feature entirely, same
        // coordinates. Reported, never silently dropped, but not a hard
        // failure here: fixing them needs a two-pass label-placement
        // restructure for every edge class (this session only built and
        // verified that pattern for external/`entry:` edges, the ones the
        // coordinator's review actually asked about), which this session
        // did not attempt against well-tested, delicate deny/lane code it
        // has no mandate to touch.
        let mut known_pre_existing_gaps: Vec<String> = Vec::new();
        for fixture in [
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "tests/fixtures/hollow.ply.yaml",
            "tests/fixtures/qualified_refs.ply.yaml",
            // ambiguous_ref.ply.yaml is designed to be a render *error*
            // (§5.1a rule 6), so it has no output to check — same
            // exclusion `declared_ceiling` makes, for the same reason.
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));

            let default_svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            check_fixture(
                fixture,
                "default",
                &default_svg,
                &style,
                &mut violations,
                &mut known_pre_existing_gaps,
            );

            let depth1_svg = render_svg_with_options(
                &doc,
                &RenderOptions {
                    depth: Some(1),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{fixture} --depth 1: {e}"));
            check_fixture(
                fixture,
                "--depth 1",
                &depth1_svg,
                &style,
                &mut violations,
                &mut known_pre_existing_gaps,
            );

            // §7.1 `--collapse <name>`: folding *one* top-level component
            // while every other stays fully expanded is its own distinct
            // layout (routing/obstruction geometry for an edge into the
            // collapsed box's siblings differs from both "default" and
            // "--depth 1", which folds *everything* at once) — and it is
            // exactly the configuration vetting 003's own canonical
            // committed SVG uses (`--collapse ingest`). Neither "default"
            // nor "--depth 1" alone exercises a single-component collapse,
            // so a routing bug specific to that shape was invisible to this
            // invariant until every top-level name got its own sweep here
            // (found: `venue ~> ingest.feed` routed straight through
            // `strategy.signals` under `--collapse ingest` specifically).
            for name in doc.components.keys() {
                let label = format!("--collapse {name}");
                let collapsed_svg = render_svg_with_options(
                    &doc,
                    &RenderOptions {
                        collapse: vec![name.clone()],
                        ..Default::default()
                    },
                )
                .unwrap_or_else(|e| panic!("{fixture} {label}: {e}"));
                check_fixture(
                    fixture,
                    &label,
                    &collapsed_svg,
                    &style,
                    &mut violations,
                    &mut known_pre_existing_gaps,
                );
            }
        }
        // Pre-existing label/line gaps (regular- and deny-edge label
        // placement, predating external elements) are reported rather than
        // failed -- fixing them means restructuring those two placement
        // paths, which is its own work item (docs/external-elements-adoption.md,
        // TODO.md). But the count is PINNED, not merely printed: a passing
        // test that knows about N violations and says nothing is the
        // "gate debt: none" over-claim this project already retracted once
        // (see The-Ply-Spec.md 7.1's own note), and an eprintln in a green
        // test is read by nobody. Pinning makes the debt a ratchet: it can
        // only be paid down, never silently grow. Lower this number when you
        // fix one; if it rises, you added one and the test says so.
        const KNOWN_LABEL_LINE_GAPS: usize = 13;
        if !known_pre_existing_gaps.is_empty() {
            eprintln!(
                "no_drawn_element_intersects_a_box_it_is_not_inside: {} known pre-existing \
                 label/line gap(s), predating the external-elements feature, reported not \
                 failed (docs/external-elements-adoption.md):\n{}",
                known_pre_existing_gaps.len(),
                known_pre_existing_gaps.join("\n")
            );
        }
        assert!(
            known_pre_existing_gaps.len() <= KNOWN_LABEL_LINE_GAPS,
            "label/line gaps grew from {} to {} -- this change added one. Fix the new \
             collision rather than raising the pin:\n{}",
            KNOWN_LABEL_LINE_GAPS,
            known_pre_existing_gaps.len(),
            known_pre_existing_gaps.join("\n")
        );
        assert!(
            violations.is_empty(),
            "drawn elements intersecting boxes they are not inside:\n{}",
            violations.join("\n")
        );
    }

    /// Stress for the wildcard-node stacking and ordering rules: several
    /// `*` denies whose resolved endpoints share one row (equal centre-y,
    /// the case the "monotone in target y" ordering cannot separate), both
    /// margin columns in use at once, and denies naming components that do
    /// not exist (which must draw nothing rather than panic or scribble).
    /// Asserts: every wildcard node inside the canvas (RED before the
    /// canvas learned to grow under a tall node stack), nodes pairwise
    /// clear of each other, and no two same-column rules' lines crossing.
    ///
    /// KNOWN GAP (found by this fixture, deliberately not asserted yet):
    /// a left-column rule's line can still cross a *right*-column rule's
    /// routed detour — `* -> beta` vs `alpha -> *` here — because the two
    /// margin columns assign their node heights independently and a detour
    /// picks its side with no knowledge of the other column's lines. §7.1
    /// channel discipline only promises each fan is crossing-free today.
    /// One wildcard node's drawn circle: (cx, cy, r).
    type Circle = (f64, f64, f64);
    /// One straight piece of a deny line's (possibly routed) path.
    type Seg = ((f64, f64), (f64, f64));

    #[test]
    fn stacked_wildcard_denies_never_cross_each_other() {
        let svg = render_fixture("tests/fixtures/deny_stress.ply.yaml");
        let xml = roxmltree::Document::parse(&svg).unwrap();
        let (w, h) = svg_dims(&xml);

        // Rules in document order, each with its wildcard node(s) and its
        // line segments — `render_deny` emits 1-2 sibling `any-node` <g>s
        // immediately before their `deny-rule` <g>.
        let mut rules: Vec<(Vec<Circle>, Vec<Seg>)> = Vec::new();
        let mut pending: Vec<Circle> = Vec::new();
        for child in xml.root_element().children().filter(|n| n.is_element()) {
            match child.attribute("class") {
                Some("any-node") => {
                    let c = child
                        .children()
                        .find(|c| c.tag_name().name() == "circle")
                        .unwrap();
                    pending.push((
                        c.attribute("cx").unwrap().parse().unwrap(),
                        c.attribute("cy").unwrap().parse().unwrap(),
                        c.attribute("r").unwrap().parse().unwrap(),
                    ));
                }
                Some("deny-rule") => {
                    let segs = child
                        .children()
                        .filter(|c| {
                            matches!(
                                c.attribute("class"),
                                Some("deny-line" | "deny-line-finding")
                            )
                        })
                        .flat_map(|c| {
                            let pts = parse_path_points(c.attribute("d").unwrap());
                            pts.windows(2).map(|s| (s[0], s[1])).collect::<Vec<_>>()
                        })
                        .collect();
                    rules.push((std::mem::take(&mut pending), segs));
                }
                _ => {}
            }
        }
        // 5 drawable wildcard rules — the two naming unknown components
        // (`* -> nowhere`, `ghost -> *`) draw nothing: no node, no line.
        assert_eq!(rules.len(), 5, "one deny-rule group per drawable rule");

        let nodes: Vec<Circle> = rules.iter().flat_map(|(n, _)| n.iter().copied()).collect();
        assert_eq!(nodes.len(), 5, "one any-node per drawable wildcard rule");
        for (cx, cy, r) in &nodes {
            assert!(
                cx - r >= 0.0 && cx + r <= w && cy - r >= 0.0 && cy + r <= h,
                "any-node at ({cx},{cy}) r={r} clips the {w}x{h} canvas"
            );
        }
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let (ax, ay, ar) = nodes[i];
                let (bx, by, br) = nodes[j];
                let d2 = (ax - bx).powi(2) + (ay - by).powi(2);
                assert!(
                    d2.sqrt() >= ar + br,
                    "any-nodes at ({ax},{ay}) and ({bx},{by}) overlap"
                );
            }
        }

        // Same-column fan discipline: rules whose wildcard node sits in the
        // same margin column must never cross each other's lines.
        let column =
            |rule: &(Vec<Circle>, Vec<Seg>)| rule.0.first().map(|(cx, _, _)| *cx < w / 2.0);
        let mut crossings = Vec::new();
        for i in 0..rules.len() {
            for j in (i + 1)..rules.len() {
                if column(&rules[i]) != column(&rules[j]) {
                    continue; // cross-column: the KNOWN GAP above
                }
                for a in &rules[i].1 {
                    for b in &rules[j].1 {
                        if segments_cross(*a, *b) {
                            crossings.push(format!(
                                "deny rule #{i} segment {a:?} crosses rule #{j} segment {b:?}"
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            crossings.is_empty(),
            "same-column deny lines cross each other:\n{}",
            crossings.join("\n")
        );
    }

    /// Vetting 003 finding 3: deny rules must not draw their
    /// lines/bars/labels/wildcard-nodes on top of *each other's* — checked
    /// as a pairwise bounding-box comparison across every pair of
    /// different deny rules' geometry (a rule's own bar crossing its own
    /// line is by design, so only cross-rule pairs count).
    #[test]
    fn deny_geometry_never_overlaps_another_deny_rules_geometry() {
        let mut violations: Vec<String> = Vec::new();
        // Only fixtures that actually declare more than zero deny rules;
        // 001-spsc-disruptor, hollow, and qualified_refs declare none.
        for fixture in [
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "tests/fixtures/full.ply.yaml",
        ] {
            let svg = render_fixture(fixture);
            let xml = roxmltree::Document::parse(&svg).unwrap();
            let style = format!(
                "{}{}",
                ply_render::svg::STYLE,
                ply_render::svg::FINDING_STYLE
            );

            // Groups in document order: 0-2 sibling `any-node` <g>s
            // immediately followed by the one `deny-rule` <g> they belong
            // to — exactly how `render_deny` emits them, one deny rule at
            // a time. Anything else at the root is irrelevant here.
            #[allow(clippy::type_complexity)]
            let mut groups: Vec<(Vec<Rectf>, Vec<((f64, f64), (f64, f64))>)> = Vec::new();
            let mut pending: Vec<Rectf> = Vec::new();
            for child in xml.root_element().children().filter(|n| n.is_element()) {
                match child.attribute("class") {
                    Some("any-node") => {
                        let circle = child
                            .children()
                            .find(|c| c.tag_name().name() == "circle")
                            .unwrap();
                        let cx: f64 = circle.attribute("cx").unwrap().parse().unwrap();
                        let cy: f64 = circle.attribute("cy").unwrap().parse().unwrap();
                        let r: f64 = circle.attribute("r").unwrap().parse().unwrap();
                        pending.push((cx - r, cy - r, r * 2.0, r * 2.0));
                    }
                    Some("deny-rule") => {
                        let mut items = std::mem::take(&mut pending);
                        let mut segs: Vec<((f64, f64), (f64, f64))> = Vec::new();
                        for c in child.children() {
                            match (c.tag_name().name(), c.attribute("class")) {
                                ("path", Some("deny-line" | "deny-line-finding")) => {
                                    // One bbox per straight segment, not one
                                    // spanning the whole (possibly routed,
                                    // multi-point) path — a detour that
                                    // steps around an unrelated box legally
                                    // sweeps through a lot of open canvas
                                    // that a whole-path bbox would count as
                                    // this rule's "geometry" even where
                                    // nothing is actually drawn.
                                    let points = parse_path_points(c.attribute("d").unwrap());
                                    for seg in points.windows(2) {
                                        segs.push((seg[0], seg[1]));
                                    }
                                }
                                ("line", Some("deny-bar")) => {
                                    let x1: f64 = c.attribute("x1").unwrap().parse().unwrap();
                                    let y1: f64 = c.attribute("y1").unwrap().parse().unwrap();
                                    let x2: f64 = c.attribute("x2").unwrap().parse().unwrap();
                                    let y2: f64 = c.attribute("y2").unwrap().parse().unwrap();
                                    items.push(line_bbox(&[(x1, y1), (x2, y2)], 2.0));
                                }
                                ("text", Some("deny-except")) => {
                                    items.push(text_bbox(c, &style).0);
                                }
                                _ => {}
                            }
                        }
                        groups.push((items, segs));
                    }
                    _ => {}
                }
            }
            assert!(
                !groups.is_empty(),
                "{fixture}: no deny rules found to check"
            );

            for i in 0..groups.len() {
                for j in (i + 1)..groups.len() {
                    for a in &groups[i].0 {
                        for b in &groups[j].0 {
                            if rects_overlap(*a, *b, 0.5) {
                                violations.push(format!(
                                    "{fixture}: deny rule #{i}'s geometry {a:?} overlaps deny \
                                     rule #{j}'s geometry {b:?}"
                                ));
                            }
                        }
                    }
                    // Lines are compared by whether they actually cross, not
                    // by bounding box: a diagonal segment's bbox is mostly
                    // empty canvas, and two lines with overlapping bboxes
                    // that never meet are perfectly readable.
                    for a in &groups[i].1 {
                        for b in &groups[j].1 {
                            if segments_cross(*a, *b) {
                                violations.push(format!(
                                    "{fixture}: deny rule #{i}'s line {a:?} crosses deny \
                                     rule #{j}'s line {b:?}"
                                ));
                            }
                        }
                    }
                    // A line running *through* another rule's label or node
                    // is a real collision, bbox is right for that.
                    for (segs, boxes) in
                        [(&groups[i].1, &groups[j].0), (&groups[j].1, &groups[i].0)]
                    {
                        for seg in segs {
                            for b in boxes {
                                if rects_overlap(line_bbox(&[seg.0, seg.1], 2.0), *b, 0.5)
                                    && segment_hits_rect(*seg, *b)
                                {
                                    violations.push(format!(
                                        "{fixture}: a deny line {seg:?} runs through another \
                                         deny rule's item {b:?}"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            violations.is_empty(),
            "deny rules draw overlapping geometry:\n{}",
            violations.join("\n")
        );
    }
}

/// §7.1 gate debt closed 2026-08-23: `strict` was assigned a visual form (a
/// solid ink triangle notch in the box's top-right corner) but the renderer
/// never drew it. The invariant runs both directions, like
/// `hollow_and_gutter::every_hollow_component_is_dashed_and_says_so`: a
/// component draws the notch iff it declares `strict: true`.
mod strict_notch {
    use super::*;
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    /// Keyed by leaf name, same convention `every_hollow_component_is_dashed_and_says_so`
    /// already uses — every fixture below has unique leaf names.
    fn walk_components(
        comp: &ply_render::model::Component,
        name: &str,
        out: &mut Vec<(String, bool)>,
    ) {
        out.push((name.to_string(), comp.strict));
        for (n, c) in &comp.components {
            walk_components(c, n, out);
        }
    }

    fn has_notch(g: roxmltree::Node) -> bool {
        g.children()
            .any(|c| c.attribute("class") == Some("strict-notch"))
    }

    #[test]
    fn every_strict_component_draws_the_notch_and_only_strict_components_do() {
        for fixture in [
            "tests/fixtures/visual_forms.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
            "../../demos/fault3.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let sdoc = roxmltree::Document::parse(&svg).unwrap();

            let mut expected = Vec::new();
            for (n, c) in &doc.components {
                walk_components(c, n, &mut expected);
            }

            for (name, strict) in expected {
                let g = sdoc
                    .descendants()
                    .find(|n| {
                        n.attribute("class")
                            .is_some_and(|c| c.split(' ').any(|t| t == "component"))
                            && n.attribute("data-name") == Some(name.as_str())
                    })
                    .unwrap_or_else(|| panic!("{fixture}: no component group named {name:?}"));
                assert_eq!(
                    has_notch(g),
                    strict,
                    "{fixture}: component {name:?} strict={strict} but notch-drawn={}",
                    has_notch(g)
                );
            }
        }
    }

    /// `strict` must compose with the finding-red border: a component that
    /// is both `strict` and the target of a real `ply-check` finding must
    /// draw both marks together, not one clobbering the other.
    #[test]
    fn strict_notch_composes_with_the_finding_red_border() {
        let svg = render_fixture("tests/fixtures/strict_with_finding.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let vault = doc
            .descendants()
            .find(|n| {
                n.attribute("class") == Some("component")
                    && n.attribute("data-name") == Some("vault")
            })
            .expect("vault component group must exist");
        let box_rect = vault
            .children()
            .find(|c| {
                c.attribute("class").is_some_and(|cl| {
                    cl.split_whitespace()
                        .any(|t| t == "component-box" || t == "component-box-finding")
                })
            })
            .expect("vault must have a component-box rect");
        assert!(
            box_rect
                .attribute("class")
                .unwrap()
                .split_whitespace()
                .any(|t| t == "component-box-finding"),
            "vault should carry the finding-red border (E0304 on its anchor)"
        );
        assert!(
            has_notch(vault),
            "vault is strict, so it must still draw the notch alongside the red border"
        );
    }

    /// `strict` must compose with the collapsed-stack card: a strict
    /// component that folds under `--depth` still draws its notch.
    #[test]
    fn strict_notch_still_draws_when_the_component_collapses() {
        let yaml = std::fs::read_to_string("../../demos/fault3.ply.yaml").unwrap();
        let doc = parse_document(&yaml).unwrap();
        let svg = render_svg_with_options(
            &doc,
            &RenderOptions {
                depth: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let sdoc = roxmltree::Document::parse(&svg).unwrap();
        let book = sdoc
            .descendants()
            .find(|n| {
                n.attribute("class") == Some("component")
                    && n.attribute("data-name") == Some("book")
            })
            .expect("book component group must exist");
        assert!(
            book.children().any(|c| c
                .attribute("class")
                .is_some_and(|cl| cl.split_whitespace().any(|t| t == "collapsed-stack"))),
            "book must be collapsed at --depth 1"
        );
        assert!(
            has_notch(book),
            "book is strict, so the collapsed box must still draw the notch"
        );
    }
}

/// §7.1 gate debt closed 2026-08-23: `mode: synth` and `examples` were
/// assigned visual forms but never drawn. Reuses `contract_mark`'s
/// declare-order walk (a component's nested components' fns first,
/// recursively, then its own) so expected and rendered line up by position.
mod new_visual_forms {
    use super::*;
    use ply_render::model::Mode;

    #[derive(Debug, Clone)]
    struct ExpectedFn {
        mode: Mode,
        examples: usize,
    }

    fn walk_fn_claims(comp: &ply_render::model::Component, out: &mut Vec<ExpectedFn>) {
        for c in comp.components.values() {
            walk_fn_claims(c, out);
        }
        for fc in comp.fns.values() {
            out.push(ExpectedFn {
                mode: fc.mode.clone(),
                examples: fc.examples.len(),
            });
        }
    }

    struct RenderedChip {
        box_class: String,
        examples_text: Option<String>,
        tooltip: String,
    }

    fn rendered_fn_chips(svg: &str) -> Vec<RenderedChip> {
        let doc = roxmltree::Document::parse(svg).unwrap();
        doc.descendants()
            .filter(|n| n.tag_name().name() == "g" && n.attribute("class") == Some("fn-chip"))
            .map(|g| {
                let box_class = g
                    .children()
                    .find(|c| c.tag_name().name() == "rect")
                    .and_then(|r| r.attribute("class"))
                    .unwrap_or_default()
                    .to_string();
                let examples_text = g
                    .children()
                    .find(|c| c.attribute("class") == Some("fn-examples"))
                    .and_then(|t| t.text())
                    .map(|s| s.to_string());
                let tooltip = g
                    .children()
                    .find(|c| c.tag_name().name() == "title")
                    .and_then(|t| t.text())
                    .unwrap_or_default()
                    .to_string();
                RenderedChip {
                    box_class,
                    examples_text,
                    tooltip,
                }
            })
            .collect()
    }

    #[test]
    fn every_synth_fn_gets_violet_fill_and_explains_why_in_the_tooltip() {
        let mut any_synth_seen = false;
        for fixture in [
            "tests/fixtures/visual_forms.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));

            let mut expected: Vec<ExpectedFn> = Vec::new();
            for c in doc.components.values() {
                walk_fn_claims(c, &mut expected);
            }
            let rendered = rendered_fn_chips(&svg);
            assert_eq!(
                expected.len(),
                rendered.len(),
                "{fixture}: fn claim count doesn't match rendered fn-chip count"
            );

            for (exp, chip) in expected.iter().zip(rendered.iter()) {
                let is_synth = exp.mode == Mode::Synth;
                let has_violet = chip
                    .box_class
                    .split_whitespace()
                    .any(|t| t == "fn-chip-box-synth");
                assert_eq!(
                    has_violet, is_synth,
                    "{fixture}: fn-chip-box-synth presence ({has_violet}) doesn't match \
                     mode: synth ({is_synth}); box_class: {:?}",
                    chip.box_class
                );
                if is_synth {
                    any_synth_seen = true;
                    assert!(
                        chip.tooltip.contains("machine-written"),
                        "{fixture}: a synth chip's tooltip must say machine-written, got: {:?}",
                        chip.tooltip
                    );
                    assert!(
                        chip.tooltip.contains(
                            "the body below the watermark is synthesized from the contract, \
                             with the checks holding the line"
                        ),
                        "{fixture}: a synth chip's tooltip must say exactly what the violet \
                         fill means, got: {:?}",
                        chip.tooltip
                    );
                } else {
                    assert!(
                        !chip.tooltip.contains("machine-written"),
                        "{fixture}: a non-synth chip must not gain authorship wording, got: {:?}",
                        chip.tooltip
                    );
                }
            }
        }
        assert!(
            any_synth_seen,
            "no fixture exercised mode: synth — this test would pass vacuously"
        );
    }

    #[test]
    fn every_example_bearing_chip_shows_the_e_times_n_token() {
        let mut any_examples_seen = false;
        for fixture in [
            "tests/fixtures/visual_forms.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));

            let mut expected: Vec<ExpectedFn> = Vec::new();
            for c in doc.components.values() {
                walk_fn_claims(c, &mut expected);
            }
            let rendered = rendered_fn_chips(&svg);
            assert_eq!(expected.len(), rendered.len());

            for (exp, chip) in expected.iter().zip(rendered.iter()) {
                if exp.examples > 0 {
                    any_examples_seen = true;
                    assert_eq!(
                        chip.examples_text,
                        Some(format!("e\u{d7}{}", exp.examples)),
                        "{fixture}: expected an e×{} token, got {:?}",
                        exp.examples,
                        chip.examples_text
                    );
                } else {
                    assert_eq!(
                        chip.examples_text, None,
                        "{fixture}: a chip with no examples must not draw the e×N token, got \
                         {:?}",
                        chip.examples_text
                    );
                }
            }
        }
        assert!(
            any_examples_seen,
            "no fixture exercised examples — this test would pass vacuously"
        );
    }
}

/// docs/plans/external-elements.md §4.2 / CLAUDE.md's render-invariant
/// family (same shape as `no_drawn_element_intersects_a_box_it_is_not_
/// inside`): the frame-crossing invariant the external-elements gate is
/// conditioned on. Three clauses, walked against the real rendered output,
/// not spot-checked:
///
/// 1. No external box intersects the workspace frame — externals draw
///    strictly *outside* it, never overlapping (§7.1's extended "inside the
///    frame = part of the system").
/// 2. Every deny `*` wildcard node stays *inside* the frame — a `*` means
///    "any component inside the workspace", and must never be mistaken for
///    an external by drawing in the same outside region.
/// 3. Every edge with an external endpoint crosses the frame border exactly
///    once — not zero (it would then read as entirely inside or outside),
///    not two-or-more (it would zigzag across the boundary it exists to
///    show).
///
/// Written RED FIRST: before `svg.rs` knew about `externals:`, `~>` edges
/// naming one were silently dropped (resolved to nothing, matching any
/// other unresolvable endpoint) and `entry:` had no rendering at all, so
/// `tests/fixtures/externals.ply.yaml` rendered with zero external boxes
/// and zero external-touching edges — the "any external box found" /
/// "any external edge found" guards below failed with exactly that message,
/// not a compile error, confirming the fixture and the *absence* of the
/// feature were the reason, not a typo in the test.
mod frame_boundary {
    use super::*;

    type Rectf = (f64, f64, f64, f64); // (x, y, w, h)

    fn rects_overlap(a: Rectf, b: Rectf, eps: f64) -> bool {
        a.0 + eps < b.0 + b.2
            && a.0 + a.2 > b.0 + eps
            && a.1 + eps < b.1 + b.3
            && a.1 + a.3 > b.1 + eps
    }

    fn point_in_rect(p: (f64, f64), r: Rectf, eps: f64) -> bool {
        p.0 >= r.0 - eps && p.0 <= r.0 + r.2 + eps && p.1 >= r.1 - eps && p.1 <= r.1 + r.3 + eps
    }

    fn absolute_offset(node: roxmltree::Node) -> (f64, f64) {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut cur = Some(node);
        while let Some(n) = cur {
            if let Some(t) = n.attribute("transform")
                && let Some(inner) = t
                    .strip_prefix("translate(")
                    .and_then(|s| s.strip_suffix(")"))
            {
                let parts: Vec<f64> = inner
                    .split(',')
                    .map(|p| p.trim().parse().unwrap())
                    .collect();
                x += parts[0];
                y += parts[1];
            }
            cur = n.parent();
        }
        (x, y)
    }

    /// The workspace frame's own rect, in absolute canvas coordinates (it is
    /// drawn at document-root level with no ancestor transform, but read via
    /// the same accumulation as everything else for robustness).
    fn frame_rect(doc: &roxmltree::Document) -> Rectf {
        let node = doc
            .descendants()
            .find(|n| n.attribute("class") == Some("workspace-frame"))
            .expect("workspace-frame rect must exist");
        let (ox, oy) = absolute_offset(node);
        let x: f64 = node.attribute("x").unwrap().parse().unwrap();
        let y: f64 = node.attribute("y").unwrap().parse().unwrap();
        let w: f64 = node.attribute("width").unwrap().parse().unwrap();
        let h: f64 = node.attribute("height").unwrap().parse().unwrap();
        (x + ox, y + oy, w, h)
    }

    /// Every external box's absolute rect — the `external-box` rect inside
    /// each `<g class="external">`.
    fn external_rects(doc: &roxmltree::Document) -> Vec<Rectf> {
        doc.descendants()
            .filter(|n| n.tag_name().name() == "g" && n.attribute("class") == Some("external"))
            .map(|g| {
                let rect = g
                    .children()
                    .find(|c| c.attribute("class") == Some("external-box"))
                    .expect("external group must have an external-box rect");
                let (ox, oy) = absolute_offset(g);
                let x: f64 = rect.attribute("x").unwrap_or("0").parse().unwrap();
                let y: f64 = rect.attribute("y").unwrap_or("0").parse().unwrap();
                let w: f64 = rect.attribute("width").unwrap().parse().unwrap();
                let h: f64 = rect.attribute("height").unwrap().parse().unwrap();
                (x + ox, y + oy, w, h)
            })
            .collect()
    }

    /// Every deny wildcard node's bounding box (from its `circle`), absolute.
    fn any_node_rects(doc: &roxmltree::Document) -> Vec<Rectf> {
        doc.descendants()
            .filter(|n| n.attribute("class") == Some("any-node"))
            .map(|g| {
                let circle = g
                    .children()
                    .find(|c| c.tag_name().name() == "circle")
                    .unwrap();
                let (ox, oy) = absolute_offset(g);
                let cx: f64 = circle.attribute("cx").unwrap().parse().unwrap();
                let cy: f64 = circle.attribute("cy").unwrap().parse().unwrap();
                let r: f64 = circle.attribute("r").unwrap().parse().unwrap();
                (cx + ox - r, cy + oy - r, r * 2.0, r * 2.0)
            })
            .collect()
    }

    /// Every edge/entry line whose path has at least one endpoint landing
    /// inside an external box — the "edge with an external endpoint" set
    /// clause 3 is about. Absolute path points.
    fn external_touching_edge_paths(
        doc: &roxmltree::Document,
        externals: &[Rectf],
    ) -> Vec<Vec<(f64, f64)>> {
        let mut out = Vec::new();
        for g in doc.descendants().filter(|n| {
            n.tag_name().name() == "g"
                && matches!(n.attribute("class"), Some("edge-flow") | Some("edge-entry"))
        }) {
            let path = g
                .children()
                .find(|c| c.tag_name().name() == "path")
                .expect("edge-flow/edge-entry group must have a path");
            let (ox, oy) = absolute_offset(g);
            let raw = parse_path_points(path.attribute("d").unwrap());
            let pts: Vec<(f64, f64)> = raw.iter().map(|p| (p.0 + ox, p.1 + oy)).collect();
            let first = pts[0];
            let last = pts[pts.len() - 1];
            if externals.iter().any(|r| point_in_rect(first, *r, 1.0))
                || externals.iter().any(|r| point_in_rect(last, *r, 1.0))
            {
                out.push(pts);
            }
        }
        out
    }

    fn count_border_crossings(points: &[(f64, f64)], (x, y, w, h): Rectf) -> usize {
        let border = [
            ((x, y), (x + w, y)),
            ((x + w, y), (x + w, y + h)),
            ((x + w, y + h), (x, y + h)),
            ((x, y + h), (x, y)),
        ];
        let mut count = 0;
        for seg in points.windows(2) {
            for edge in &border {
                if segments_cross((seg[0], seg[1]), *edge) {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn no_external_box_intersects_the_frame_deny_wildcards_stay_inside_and_external_edges_cross_once()
     {
        let mut any_external_seen = false;
        let mut any_edge_seen = false;
        let mut violations: Vec<String> = Vec::new();

        for fixture in [
            "tests/fixtures/externals.ply.yaml",
            "../../vetting/003-trading-system.ply.yaml",
        ] {
            let yaml = std::fs::read_to_string(fixture).unwrap();
            let doc = parse_document(&yaml).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let svg = render_svg(&doc).unwrap_or_else(|e| panic!("{fixture}: {e}"));
            let xml = roxmltree::Document::parse(&svg).unwrap();

            let frame = frame_rect(&xml);
            let externals = external_rects(&xml);
            if !externals.is_empty() {
                any_external_seen = true;
            }

            // Clause 1: no external box intersects the frame.
            for ext in &externals {
                if rects_overlap(*ext, frame, 0.5) {
                    violations.push(format!(
                        "{fixture}: external box {ext:?} intersects the frame {frame:?}"
                    ));
                }
            }

            // Clause 2: every deny `*` node stays inside the frame.
            for any in any_node_rects(&xml) {
                let (ax, ay, aw, ah) = any;
                let inside = ax >= frame.0 - 0.5
                    && ay >= frame.1 - 0.5
                    && ax + aw <= frame.0 + frame.2 + 0.5
                    && ay + ah <= frame.1 + frame.3 + 0.5;
                if !inside {
                    violations.push(format!(
                        "{fixture}: deny wildcard node {any:?} does not stay inside the frame \
                         {frame:?}"
                    ));
                }
            }

            // Clause 3: every edge with an external endpoint crosses the
            // frame border exactly once.
            for path in external_touching_edge_paths(&xml, &externals) {
                any_edge_seen = true;
                let crossings = count_border_crossings(&path, frame);
                if crossings != 1 {
                    violations.push(format!(
                        "{fixture}: external-touching edge {path:?} crosses the frame border \
                         {crossings} times, expected exactly 1"
                    ));
                }
            }
        }

        assert!(
            any_external_seen,
            "no fixture drew any external box — this test would pass vacuously"
        );
        assert!(
            any_edge_seen,
            "no fixture drew any external-touching edge — this test would pass vacuously"
        );
        assert!(
            violations.is_empty(),
            "frame-crossing invariant violated:\n{}",
            violations.join("\n")
        );
    }
}

/// Every drawn edge label lies inside the drawing.
///
/// Written after a smoke test on a real project found two flow labels
/// misplaced at `--depth 1`: one sitting up in the title band with no line
/// under it, and one at y=162 on a canvas 152 tall -- outside the image
/// entirely, so the reader is not told the flow's type at all. The label
/// placement escalates away from its line until it finds a spot clear of
/// every box, and between two boxes sitting side by side there is no such
/// spot, so the search ran to the end of its budget and off the page.
///
/// This is an invariant rather than a spot-check on that one document: it
/// walks whatever the renderer actually emitted and fails on the first
/// label outside the canvas, so a layout change that reintroduces the
/// problem somewhere else cannot pass.
#[test]
fn every_drawn_label_lies_inside_the_canvas() {
    let docs: Vec<(&str, &str)> = vec![
        ("side by side, folded", SIDE_BY_SIDE_FOLDED),
        (
            "vetting 002",
            include_str!("../../../vetting/002-ingest-pipeline.ply.yaml"),
        ),
        (
            "vetting 003",
            include_str!("../../../vetting/003-trading-system.ply.yaml"),
        ),
    ];
    for (name, yaml) in docs {
        let doc = ply_render::model::parse_document(yaml)
            .unwrap_or_else(|e| panic!("{name} must parse: {e}"));
        for depth in [None, Some(1), Some(2)] {
            let opts = ply_render::svg::RenderOptions {
                depth,
                focus: None,
                collapse: Vec::new(),
            };
            let svg = ply_render::svg::render_svg_with_options(&doc, &opts)
                .unwrap_or_else(|e| panic!("{name} must render: {e}"));
            let (w, h) = canvas_size(&svg);
            for (x, y, text) in text_elements(&svg) {
                assert!(
                    x >= 0.0 && x <= w && y >= 0.0 && y <= h,
                    "{name} at depth {depth:?}: the label {text:?} is drawn at ({x}, {y}), \
                     outside the {w}x{h} canvas -- a reader never sees it"
                );
            }
        }
    }
}

/// A document with two top-level components, each holding children, and
/// flows between those children. At `--depth 1` both ends of every flow
/// fold into their parents, which is the shape that stranded the labels.
const SIDE_BY_SIDE_FOLDED: &str = "\
ply: 1

components:
  scheduling:
    anchor: sched
    components:
      queue:
        anchor: sched::queue
      poller:
        anchor: sched::poller
  edge:
    anchor: edge
    components:
      mapper:
        anchor: edge::mapper
      sink:
        anchor: edge::sink

edges:
  - \"scheduling.queue ~> edge.mapper : MappedRecord\"
  - \"edge.sink ~> scheduling.poller : PollAttempt\"
";

fn canvas_size(svg: &str) -> (f64, f64) {
    let grab = |key: &str| -> f64 {
        let at = svg
            .find(&format!("{key}=\""))
            .expect("svg carries width/height");
        let rest = &svg[at + key.len() + 2..];
        let end = rest.find('"').expect("attribute is closed");
        rest[..end].parse().expect("a number")
    };
    (grab("width"), grab("height"))
}

/// Every `<text ...>` element's position and content.
fn text_elements(svg: &str) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    for chunk in svg.split("<text").skip(1) {
        let Some(close) = chunk.find('>') else {
            continue;
        };
        let attrs = &chunk[..close];
        let body = &chunk[close + 1..];
        let text = body.split('<').next().unwrap_or("").to_string();
        let num = |key: &str| -> Option<f64> {
            let at = attrs.find(&format!(" {key}=\""))?;
            let rest = &attrs[at + key.len() + 3..];
            let end = rest.find('"')?;
            rest[..end].parse().ok()
        };
        if let (Some(x), Some(y)) = (num("x"), num("y")) {
            out.push((x, y, text));
        }
    }
    out
}

/// The sentence explaining a box's colour must agree with the colour the
/// box is actually painted.
///
/// A tooltip that explains the shade is only worth having if it cannot
/// drift from the shade. The canvas already says "the weakest function
/// sets the whole box's shade"; each box now names which one, and this
/// checks the level it names is the level the box is filled with -- so a
/// change to either the aggregation or the wording that separates them
/// fails here rather than quietly misinforming a reader.
#[test]
fn the_sentence_explaining_a_box_colour_agrees_with_the_colour_drawn() {
    // The ceiling class a box carries, and the words a reader would have to
    // read in the tooltip for that class to be honest.
    const LEVEL_WORDS: &[(&str, &str)] = &[
        ("ceiling-unclaimed", "declares no checks at all"),
        ("ceiling-tested", "it declares tested"),
        ("ceiling-fuzzed", "it declares fuzzed"),
        ("ceiling-bounded", "it declares bounded"),
        ("ceiling-proved", "it declares proved"),
    ];

    let mut checked = 0usize;
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/visual_forms.ply.yaml",
        "tests/fixtures/checks_inheritance.ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        let doc = roxmltree::Document::parse(&svg).unwrap();
        for node in doc.descendants().filter(|n| n.is_element()) {
            if node.attribute("class") != Some("component") {
                continue;
            }
            let title = node
                .children()
                .find(|c| c.tag_name().name() == "title")
                .and_then(|t| t.text())
                .unwrap_or("");
            // A hollow component declares nothing anywhere inside; the
            // unclaimed sentence covers it and there is no weakest link to
            // name, so there is no colour sentence to check.
            let Some(why) = title.lines().find(|l| l.starts_with("this box is")) else {
                continue;
            };
            let box_class = node
                .descendants()
                .filter_map(|d| d.attribute("class"))
                .find(|c| c.contains("component-box"))
                .unwrap_or_else(|| panic!("{fixture}: a component with no box"));
            let level = LEVEL_WORDS
                .iter()
                .find(|(class, _)| box_class.contains(*class))
                .unwrap_or_else(|| panic!("{fixture}: unknown ceiling class in {box_class:?}"));
            assert!(
                why.contains(level.1),
                "{fixture}: this box is painted {} but its tooltip says {why:?} -- the words \
                 and the colour disagree, so one of them is lying to the reader",
                level.0
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 15,
        "only {checked} boxes carried a colour explanation; these fixtures should exercise \
         several levels, so this test is checking less than it looks"
    );
}
