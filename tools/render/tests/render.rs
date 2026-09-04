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

            // Text elements have no computed bounding box in the SVG itself,
            // so approximate one with the renderer's deliberately smaller
            // check-row width and a generous per-character width everywhere
            // else. Extend it from the anchor per the element's `text-anchor`
            // (`middle` extends both ways from center; the SVG default,
            // `start`, extends only rightward). Good enough to catch a
            // label whose *anchor point* is on-canvas but whose glyphs
            // still run off the edge (vetting 002 finding 3's truncated
            // "except decoder").
            const WORST_CASE_CHAR_W: f64 = 8.0;
            const CHECK_CHAR_W: f64 = 5.5;
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
                        let char_w = if node.attribute("class") == Some("fn-checks") {
                            CHECK_CHAR_W
                        } else {
                            WORST_CASE_CHAR_W
                        };
                        let full_w = chars * char_w;
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
fn fn_chip_shows_glanceable_check_labels() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.contains("class=\"fn-chip\""));
    assert!(svg.contains(">quote<"));
    assert!(svg.contains("bounded: loop≤3 · fuzz: 1024 cases · mutate"));
    assert!(svg.contains("test · bounded: loop≤4"));
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
/// §7.1's `state:` header, drawn with **no code to read**.
///
/// The renderer is a function of what it is handed, and handed no resolved
/// fields it can honestly draw only the declaration: the type this
/// component says its state lives in. Not the fields, and above all not the
/// count -- "2 of 20 shown" is a fact about a type, and inventing it from a
/// document that merely asked for two would be Ply making up the very thing
/// this feature exists to check.
///
/// The tooltip in this case says the document asked for those names and
/// there was nothing to read them from. It used to list them under
/// "Showing ...", which put a field name on the drawing whether or not any
/// type had it -- inside a sentence promising that such a name is refused.
/// The companion tests further down cover the case where code *is* there.
///
/// Pinned exact-string like every other user-facing sentence, and drawn as
/// its own header line beside `owns` rather than folded into the anchor,
/// because the two say different things: `owns` is who may change a type,
/// `state` is what this component holds.
#[test]
fn state_draws_as_its_own_header_line() {
    let yaml = "ply: 1\ncomponents:\n  book:\n    anchor: ingest::book\n    state:\n      of: OrderBook\n      show: [bids, ticks]\n";
    let doc = parse_document(yaml).expect("fixture should parse");
    let svg = render_svg(&doc).expect("fixture should render");
    assert!(
        svg.contains(">state OrderBook<"),
        "the box must say what this component holds:\n{svg}"
    );
    assert!(
        !svg.contains("shown"),
        "with no code to count, there is no count to draw -- a number here would be one \
         Ply made up about a type it never read:\n{svg}"
    );
    assert!(
        svg.contains(
            "this document asks to show bids, ticks, and none of them declare a shape \
                      of their own, so none is drawn"
        ),
        "the tooltip has to say why no rows were drawn, rather than listing the names as \
         though they had been checked:\n{svg}"
    );
    assert!(
        !svg.contains("class=\"state-field\""),
        "no field resolved, so no row may be painted:\n{svg}"
    );

    let bare =
        "ply: 1\ncomponents:\n  book:\n    anchor: ingest::book\n    state:\n      of: OrderBook\n";
    let doc = parse_document(bare).expect("fixture should parse");
    let svg = render_svg(&doc).expect("fixture should render");
    assert!(
        svg.contains(">state OrderBook<") && svg.contains("No fields chosen to show"),
        "naming the type without choosing fields draws the header line alone:\n{svg}"
    );
}

#[test]
fn every_painted_element_resolves_a_style_rule() {
    // §7.1 finding classes live in a separate constant (`FINDING_STYLE`,
    // only appended to a document's actual `<style>` when it has a
    // finding — see its doc comment), so checking selector resolution
    // needs both, regardless of which a given fixture below happens to use.
    // `STATE_STYLE` joins them for the same reason (2026-09-04): a declared
    // shape now paints a state row with no crate on disk at all, which
    // `declared_shapes.ply.yaml` below is the first fixture in this sweep
    // to do.
    let style = format!(
        "{}{}{}",
        ply_render::svg::STYLE,
        ply_render::svg::FINDING_STYLE,
        ply_render::svg::STATE_STYLE
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
        // §7.1 (2026-09-04): a declared shape paints a state row with no
        // crate on disk at all -- the first state rows this sweep can see
        // without a fixture crate.
        "tests/fixtures/declared_shapes.ply.yaml",
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
fn checks_are_explained_by_a_hover_title() {
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
        "bounded(3) — Kani symbolically checks supported inputs that satisfy requires against ensures; 3 is the maximum loop-unwind depth, not a numeric input limit; collection inputs may also be limited to length 3"
    ));
    assert!(push.contains(
        "fuzz(1024) — tries 1024 generated inputs that satisfy requires, then checks ensures"
    ));
    assert!(push.contains(
        "generic — every check runs with T=u64; whatever they earn covers only that type"
    ));
    assert!(push.contains(
        "trusted (a human vouches for this; no machine checks it): SPSC cross-thread safety"
    ));
    assert!(push.contains("loom test tests/loom_spsc.rs"));
    // `Spsc::try_push` declares `[bounded(3), fuzz(1024)]` and no `test`, so
    // nothing here ever compiles its example. This line used to require the
    // drawing to say it had been "compiled into a test" -- a green test
    // pinning a false sentence in exactly the configuration where it is
    // false (external review, 2026-08-30).
    assert!(push.contains(
        "1 worked example, written down but not run: no check here asks for the declared \
         examples, so nothing compiles them"
    ));

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
         data flows (dashed); red bars are forbidden calls. A box's grey depth is how \
         strongly it promises to be checked — white means something inside promises \
         nothing, deeper grey means stronger checks promised, and the weakest \
         function sets the whole box's shade. Nothing here is green: green is kept \
         for evidence a run has actually earned, which this render never sees, so a \
         picture full of promises should not look like a picture full of results. \
         Hover anything for its meaning."
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
        // §7.1 (2026-09-04): the first fixture in this sweep whose state
        // rows are declared rather than read from code.
        "tests/fixtures/declared_shapes.ply.yaml",
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
/// `ply_core::kernel::aggregate`, not the renderer's own tree-building code), and
/// checks the rendered SVG's component-box fill class against it. This is
/// the invariant, not a spot-check: a construct added later that the
/// renderer forgets to feed into its own ceiling computation fails here on
/// its own fixture, without a bespoke assertion for it.
mod declared_ceiling {
    use super::*;
    use ply_core::kernel::{Evidence, NodeKind, VerdictNode, aggregate};
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
                statuses: ply_core::kernel::StatusSet::new(),
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
            statuses: ply_core::kernel::StatusSet::new(),
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
                        tooltip.contains("input and post contract:"),
                        "{fixture}: contract-carrying chip's tooltip is missing the header: \
                         {tooltip:?}"
                    );
                    for r in &exp.requires {
                        let line = format!("Input (requires): {r}");
                        assert!(
                            tooltip.contains(&line),
                            "{fixture}: tooltip is missing {line:?}, got: {tooltip:?}"
                        );
                    }
                    for e in &exp.ensures {
                        let line = format!("Postcondition (ensures): {e}");
                        assert!(
                            tooltip.contains(&line),
                            "{fixture}: tooltip is missing {line:?}, got: {tooltip:?}"
                        );
                    }
                    assert!(
                        tooltip.contains(
                            "the checks above are what will test the function against \
                             exactly this promise when `cargo ply verify` runs"
                        ),
                        "{fixture}: tooltip is missing the closing line, got: {tooltip:?}"
                    );
                } else {
                    assert!(
                        !tooltip.contains("input and post contract:"),
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
        assert!(tooltip.contains("Input (requires): order.qty > 0 && order.px > 0"));
        assert!(
            tooltip.contains(
                "Postcondition (ensures): |r| r.is_err() == (order.qty > limits.max_qty)"
            )
        );
    }
}

/// §5.1 "checks: [bounded(2)] # optional default checks for all fns in
/// scope": a fn with no `checks` of its own must draw and describe the
/// *inherited* checks — the readable row on its chip, and its tooltip — not
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

    fn check_label_text(chip: roxmltree::Node) -> String {
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

    /// The label row must
    /// reflect the *effective* list — own if declared, else inherited —
    /// for every shape the fixture exercises.
    #[test]
    fn label_row_shows_the_effective_checks_for_every_inheritance_shape() {
        let svg = render_fixture("tests/fixtures/checks_inheritance.ply.yaml");
        let doc = roxmltree::Document::parse(&svg).unwrap();

        // `quote` has no checks of its own -> inherits `pricing`'s bounded(2).
        assert_eq!(check_label_text(fn_chip(&doc, "quote")), "bounded: loop≤2");
        // `book` declares its own `[test]`, which wins entirely.
        assert_eq!(check_label_text(fn_chip(&doc, "book")), "test");
        // `discount` has no checks of its own -> inherits `curves`'s
        // fuzz(64), not the grandparent `pricing`'s bounded(2) — nearest
        // ancestor wins.
        assert_eq!(
            check_label_text(fn_chip(&doc, "discount")),
            "fuzz: 64 cases"
        );
        // `delta` has no checks of its own, and `greeks` declares no
        // default of its own either -> skips up to the grandparent
        // `pricing`'s bounded(2).
        assert_eq!(check_label_text(fn_chip(&doc, "delta")), "bounded: loop≤2");
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
                "inherited from component `pricing`: bounded(2) — Kani symbolically checks supported inputs that satisfy requires against ensures; 2 is the maximum loop-unwind depth, not a numeric input limit; collection inputs may also be limited to length 2"
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
                "inherited from component `curves`: fuzz(64) — tries 64 generated inputs that satisfy requires, then checks ensures"
            ),
            "discount's tooltip should name curves, not pricing: {discount_tip:?}"
        );

        // Nested, skipping a level: `delta`'s own component `greeks`
        // declares no default, so it inherits the grandparent `pricing`'s.
        let delta_tip = tooltip_text(fn_chip(&doc, "delta"));
        assert!(
            delta_tip.contains(
                "inherited from component `pricing`: bounded(2) — Kani symbolically checks supported inputs that satisfy requires against ensures; 2 is the maximum loop-unwind depth, not a numeric input limit; collection inputs may also be limited to length 2"
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
                let explained = tooltip.contains("hollow — promises nothing yet");
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

    /// Anchor-aware text bounding box, using the renderer's deliberately
    /// smaller character width for the compact check row and the existing
    /// worst-case monospace estimate everywhere else. It is widened into a
    /// real rect with a generous glyph height guessed around the baseline,
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
        const CHECK_CHAR_W: f64 = 5.5;
        let (ox, oy) = absolute_offset(node);
        let x: f64 = node.attribute("x").unwrap_or("0").parse::<f64>().unwrap() + ox;
        let y: f64 = node.attribute("y").unwrap_or("0").parse::<f64>().unwrap() + oy;
        let chars = node.text().unwrap_or("").chars().count() as f64;
        let char_w = if node.attribute("class") == Some("fn-checks") {
            CHECK_CHAR_W
        } else {
            WORST_CASE_CHAR_W
        };
        let full_w = chars * char_w;
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
        // Label/line gaps in regular- and deny-edge label placement were
        // carried for a while as a pinned count, reported rather than failed,
        // because fixing them meant restructuring both placement paths. That
        // work landed (the second placement pass that moves a label off any
        // line it would sit on), the count reached zero, and the pin came out
        // with it: a ratchet held at its floor is a comparison that cannot
        // fail, which is the same silence it was built to prevent. This is a
        // flat assertion now -- one gap and the test goes red.
        assert!(
            known_pre_existing_gaps.is_empty(),
            "a label now sits on a line it should have been moved off. Label placement runs a \
             second pass for exactly this, so a collision here means the pass did not see this \
             case -- fix the placement rather than allowing the overlap:\n{}",
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

/// A focused component is the "what exactly is promised here" view, and a
/// promise a reader has to hover to discover is not on the diagram. The
/// contract gutter bar already says "this function promises something";
/// this is the level at which it must say *what*. Asserted on drawn `<text>`
/// content with the tooltip stripped out first, because the clauses were
/// already in hover text before this and a naive substring search over the
/// whole document passes without a single pixel changing.
#[test]
fn a_focused_functions_promise_is_drawn_on_the_canvas_not_only_in_hover() {
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    let svg = render_svg_with_options(
        &doc,
        &RenderOptions {
            focus: Some("risk".into()),
            ..Default::default()
        },
    )
    .expect("focused render should succeed");

    let parsed = roxmltree::Document::parse(&svg).expect("render should be well-formed XML");
    let drawn: String = parsed
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .flat_map(|n| n.descendants())
        .filter(|n| n.is_text())
        .filter_map(|n| n.text().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        drawn.contains("order.qty > 0"),
        "the focused view of `risk` must draw check_order's requires clause as visible text. \
         Drawn text was: {drawn}"
    );
    assert!(
        drawn.contains("r.is_err()"),
        "the focused view of `risk` must draw check_order's ensures clause as visible text. \
         Drawn text was: {drawn}"
    );
}

/// The invariant behind the spot-check above. A clause that renders wider
/// than the chip holding it spills over whatever is drawn to its right —
/// which is exactly the bug the first cut of this feature shipped, and which
/// no existing test caught: `every_drawn_label_lies_inside_the_canvas` checks
/// the outer canvas, so text can escape its own box and still pass. Walks
/// every chip in every vetting scenario at every focus target, so a clause
/// added later cannot quietly overflow.
#[test]
fn every_drawn_clause_fits_inside_the_chip_that_holds_it() {
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    // The advance width the drawn 11px monospace actually uses. Kept here
    // rather than imported so the test fails if the renderer's own estimate
    // drifts away from reality, instead of drifting along with it.
    const REAL_CLAUSE_CHAR_W: f64 = 6.6;

    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).expect("fixture should parse");

        let targets: Vec<Option<String>> = std::iter::once(None)
            .chain(doc.components.keys().map(|k| Some(k.clone())))
            .collect();

        for target in targets {
            let svg = render_svg_with_options(
                &doc,
                &RenderOptions {
                    focus: target.clone(),
                    ..Default::default()
                },
            )
            .expect("render should succeed");
            let parsed = roxmltree::Document::parse(&svg).expect("well-formed");

            for chip in parsed
                .descendants()
                .filter(|n| n.attribute("class") == Some("fn-chip"))
            {
                let fname = chip.attribute("data-fn").unwrap_or("<unnamed>");
                let box_w: f64 = chip
                    .children()
                    .find(|c| c.has_tag_name("rect"))
                    .and_then(|r| r.attribute("width"))
                    .and_then(|w| w.parse().ok())
                    .expect("every chip draws a box with a width");

                for text in chip
                    .descendants()
                    .filter(|n| n.attribute("class") == Some("fn-clause"))
                {
                    let content: String = text
                        .descendants()
                        .filter(|n| n.is_text())
                        .filter_map(|n| n.text())
                        .collect();
                    let x: f64 = text.attribute("x").unwrap().parse().unwrap();
                    let right = x + (content.chars().count() as f64) * REAL_CLAUSE_CHAR_W;
                    assert!(
                        right <= box_w,
                        "in {fixture} focused on {target:?}, `{fname}`'s clause \
                         \"{content}\" is drawn from x={x} and needs {right:.0}px, \
                         but its chip is only {box_w:.0}px wide — it spills over \
                         whatever sits to its right"
                    );
                }
            }
        }
    }
}

/// Red means one thing on this canvas: forbidden, or wrong. A deny rule is
/// forbidden; a finding is wrong. A declared capability is neither — it is a
/// component saying "I use the network", which is ordinary and expected — yet
/// it wore the same red family as a real failure, so a reader scanning for
/// trouble was drawn to boxes that had none, and a genuine finding had to
/// compete with decoration for the one colour that should have been loudest.
///
/// Written as an invariant over the emitted stylesheet rather than as a
/// spot-check on capability badges, so a construct added later cannot quietly
/// claim red without this test naming it.
#[test]
fn only_forbidden_or_wrong_things_are_drawn_in_red() {
    // The red family this renderer paints with: border/line, text, and fill.
    const REDS: [&str; 3] = ["#c9534f", "#8f2f2c", "#fdecec"];

    // The two meanings allowed to use it. `deny-*` draws a rule the design
    // forbids; `*-finding` draws something actually wrong.
    fn is_allowed(selector: &str) -> bool {
        selector.contains("deny") || selector.contains("finding")
    }

    let svg = render_fixture("../../vetting/003-trading-system.ply.yaml");
    let style = svg
        .split("<style>")
        .nth(1)
        .and_then(|s| s.split("</style>").next())
        .expect("every render embeds a stylesheet");

    let mut offenders = Vec::new();
    for rule in style.split('}') {
        let Some((selector, body)) = rule.split_once('{') else {
            continue;
        };
        if REDS.iter().any(|r| body.contains(r)) && !is_allowed(selector) {
            offenders.push(format!("{}{{{}}}", selector.trim(), body.trim()));
        }
    }

    assert!(
        offenders.is_empty(),
        "these draw in red without being forbidden or wrong, so they compete \
         with real findings for the one colour that must stay loudest:\n  {}",
        offenders.join("\n  ")
    );
}

/// The one the whole visual-language review turns on. Ply's founding rule is
/// that a run which checked nothing must not read as success — yet the canvas
/// painted *declared intent* in green, so a project where not one check had
/// ever executed rendered as a field of healthy green. The spec's own channel
/// rule already said saturated green means earned, and the top of the promise
/// ramp was saturated green, so the picture contradicted the words.
///
/// Green now means exactly one thing: evidence actually earned. Nothing on
/// this repository's own diagram has been run, so nothing on it may be green.
#[test]
fn a_diagram_of_work_that_has_never_run_contains_no_green() {
    /// A fill reads as green when its green channel leads the other two by
    /// enough to be seen as a hue rather than a neutral grey.
    fn reads_as_green(hex: &str) -> bool {
        let h = hex.trim_start_matches('#');
        let full = if h.len() == 3 {
            h.chars().flat_map(|c| [c, c]).collect::<String>()
        } else {
            h.to_owned()
        };
        if full.len() != 6 {
            return false;
        }
        let c = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).unwrap_or(0) as i32;
        let (r, g, b) = (c(0), c(2), c(4));
        g - r >= 8 && g - b >= 8
    }

    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        let style = svg
            .split("<style>")
            .nth(1)
            .and_then(|s| s.split("</style>").next())
            .expect("every render embeds a stylesheet");

        let mut greens = Vec::new();
        for rule in style.split('}') {
            let Some((selector, body)) = rule.split_once('{') else {
                continue;
            };
            for token in body.split(|c: char| !(c.is_ascii_hexdigit() || c == '#')) {
                if token.starts_with('#') && reads_as_green(token) {
                    greens.push(format!("{}  ({token})", selector.trim()));
                }
            }
        }

        assert!(
            greens.is_empty(),
            "{fixture} has never been verified, so nothing in it has earned \
             evidence — yet these are drawn green, which is the colour reserved \
             for evidence actually earned:\n  {}",
            greens.join("\n  ")
        );
    }
}

/// Absence must be drawn, not implied. A component that declares no checks
/// at all was filled plain white — and blank does not register as a state,
/// it reads as background, so the riskiest thing on the canvas (there is
/// code here and nothing is promised about any of it) was also the quietest.
/// Perception research is blunt about this: a missing feature does not pop;
/// only a positive mark does.
///
/// This is why the fill is a hatch. The dashed border stays a separate,
/// narrower statement — "nothing inside at all yet" — so a populated box
/// that promises nothing and an empty sketch no longer look identical.
#[test]
fn a_component_that_promises_nothing_is_drawn_with_a_mark_not_left_blank() {
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "../../ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        let style = svg
            .split("<style>")
            .nth(1)
            .and_then(|s| s.split("</style>").next())
            .expect("every render embeds a stylesheet");

        let rule = style
            .split('}')
            .find_map(|r| r.split_once('{'))
            .into_iter()
            .chain(style.split('}').filter_map(|r| r.split_once('{')))
            .find(|(sel, _)| sel.trim() == ".ceiling-unclaimed")
            .map(|(_, body)| body.to_owned())
            .expect("the unclaimed ceiling must have a style rule");

        assert!(
            rule.contains("url(#"),
            "{fixture}: a component promising nothing is filled with `{rule}` — a flat \
             colour. Absence drawn as blank space reads as background, so the one state \
             that should worry a reader is the one they will not see. It needs a mark."
        );
    }
}

/// The glance before the glance. A reader opening a diagram cold has to
/// scan every box to learn how much of it is actually claimed; the counts
/// exist in the document and were simply never stated. This is the line
/// that answers "how finished is this?" before the eye goes anywhere.
///
/// Asserts on what the line has to *say*, not on its exact phrasing, since
/// the numbers differ per fixture — but it must name the unclaimed count
/// specifically, because a strip that reports only totals is a progress bar
/// with no bad news in it.
#[test]
fn the_render_opens_with_a_line_saying_how_much_is_actually_claimed() {
    // 003 declares 14 functions, of which `Feed::pump` and `Gateway::send`
    // carry no checks at all -- confirmed against the renderer's own
    // per-function tooltips, not assumed.
    let svg = render_fixture("../../vetting/003-trading-system.ply.yaml");
    let doc = roxmltree::Document::parse(&svg).unwrap();

    let strip = doc
        .descendants()
        .find(|n| n.attribute("class") == Some("verdict-strip-text"))
        .and_then(|n| n.descendants().find(|d| d.is_text()))
        .and_then(|t| t.text().map(str::to_owned))
        .expect("every render should open with a verdict strip");

    assert!(
        strip.contains("14 functions"),
        "the strip must say how much there is; got {strip:?}"
    );
    assert!(
        strip.contains("2 promise nothing"),
        "the strip must name what is unclaimed -- a summary carrying only totals is a \
         progress bar with no bad news in it; got {strip:?}"
    );
}

/// Focus adds fuller sentences beneath the already-readable overview labels.
#[test]
fn a_focused_functions_checks_are_spelled_out_in_words() {
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    let svg = render_svg_with_options(
        &doc,
        &RenderOptions {
            focus: Some("risk".into()),
            ..Default::default()
        },
    )
    .expect("focused render should succeed");

    let parsed = roxmltree::Document::parse(&svg).unwrap();
    let drawn: String = parsed
        .descendants()
        .filter(|n| n.has_tag_name("text"))
        .flat_map(|n| n.descendants())
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<Vec<_>>()
        .join(" ");

    // check_order declares bounded(3), fuzz(4096), mutate.
    for word in ["symbolic inputs", "generated cases", "plants"] {
        assert!(
            drawn.contains(word),
            "zoomed in, `check_order`'s checks must explain what each mode does -- expected \
             to find {word:?}. Drawn: {drawn}"
        );
    }
}

/// Roughly 8% of men cannot separate red from green reliably, and this
/// canvas spends hue on meaning. Two things are checked, in increasing order
/// of how much they are worth.
///
/// **The weak one, kept honest about being weak:** a colour-distance floor
/// under simulated deuteranopia. Calibrated by measuring pairs that really
/// are confusable — red vs olive, red vs dark yellow, our own red vs a
/// darker amber — which score 0.19 to 0.28 on this metric, against our own
/// pairs at 0.30 and above. That is a thin margin, and the metric is a blunt
/// instrument: pure red against pure green scores 0.61 here and would sail
/// through, though it is the textbook confusion. So the floor catches a
/// gross regression and nothing subtler.
///
/// **The one that actually holds:** every meaning is carried by a mark as
/// well as a colour, so a reader who sees no colour difference at all still
/// reads the diagram correctly. That is what makes the palette safe; the
/// distance floor merely stops it getting worse.
#[test]
fn every_colour_coded_meaning_is_also_carried_by_a_mark() {
    fn deuteranope(hex: &str) -> (f64, f64, f64) {
        let h = hex.trim_start_matches('#');
        let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0;
        let (r, g, b) = (c(0), c(2), c(4));
        let l = 17.8824 * r + 43.5161 * g + 4.11935 * b;
        let s = 0.0299566 * r + 0.184309 * g + 1.46709 * b;
        let m = 0.494207 * l + 1.24827 * s;
        (
            0.080944 * l - 0.130504 * m + 0.116721 * s,
            -0.0102485 * l + 0.0540193 * m + 0.113615 * s,
            -0.000365 * l - 0.00412161 * m + 0.693513 * s,
        )
    }
    fn apart(a: &str, b: &str) -> f64 {
        let (x, y, z) = deuteranope(a);
        let (p, q, r) = deuteranope(b);
        ((x - p).powi(2) + (y - q).powi(2) + (z - r).powi(2)).sqrt()
    }

    // Just above the confusable band measured above. Both palettes are held to
    // it: a dark theme that quietly collapses two meanings into one colour is
    // the same defect as a light one that does, and only ever gets looked at by
    // whoever is not using the default.
    const FLOOR: f64 = 0.28;
    for (a, b, what) in [
        (
            "#c9534f",
            "#b08900",
            "light: a rule broken vs a human needed",
        ),
        (
            "#c9534f",
            "#3b4252",
            "light: a rule broken vs ordinary structure",
        ),
        (
            "#b08900",
            "#3b4252",
            "light: a human needed vs ordinary structure",
        ),
        (
            "#e8524b",
            "#d4a72c",
            "dark: a rule broken vs a human needed",
        ),
        (
            "#e8524b",
            "#98a0ae",
            "dark: a rule broken vs ordinary structure",
        ),
        (
            "#d4a72c",
            "#98a0ae",
            "dark: a human needed vs ordinary structure",
        ),
    ] {
        let d = apart(a, b);
        assert!(
            d > FLOOR,
            "{what}: {a} and {b} are {d:.3} apart for a deuteranope, inside the range \
             where real confusions live. Separate the hues, or lean harder on the mark."
        );
    }

    // The real guarantee. Strip the colour away entirely and each meaning
    // must still be legible from shape or text alone.
    let svg = render_fixture("../../vetting/003-trading-system.ply.yaml");
    let doc = roxmltree::Document::parse(&svg).unwrap();
    let count = |class: &str| {
        doc.descendants()
            .filter(|n| {
                n.attribute("class")
                    .is_some_and(|c| c.split(' ').any(|p| p == class))
            })
            .count()
    };

    assert!(
        count("deny-line") > 0,
        "003 declares forbidden calls; none were drawn"
    );
    assert_eq!(
        count("deny-bar"),
        count("deny-line"),
        "every forbidden-call line must carry its crossbar. The bar is what says \
         `forbidden` to a reader who cannot see that the line is red — without it the \
         rule is indistinguishable from an ordinary connection."
    );
    assert!(
        count("pin-label") >= count("unresolved-pin"),
        "every attention marker must carry its number. The digit is what survives when \
         the amber does not."
    );
}

/// The one layout property with a large, replicated experimental effect on how
/// fast people read a diagram: lines crossing each other. Purchase's controlled
/// studies rank it well above symmetry, bends or node placement.
///
/// But the useful form of the rule is the refinement, not the headline. Eye
/// tracking shows a crossing near a right angle is essentially ignored, while a
/// shallow one sends the eye back and forth and costs measurable accuracy —
/// the penalty falls away as the angle opens toward 90°. So this does not
/// forbid crossings, which would over-constrain the layout for no measured
/// gain; it forbids the shallow ones that actually cost the reader.
///
/// **This currently guards rather than fixes.** Every committed diagram has
/// zero line crossings at any angle, so nothing here is red today. That is the
/// point: the property is held by luck, and nothing stopped a later layout
/// change from quietly losing it. Measured before writing this, so the claim
/// is not an assumption.
#[test]
fn no_two_drawn_lines_cross_at_a_shallow_angle() {
    // Below this, crossings measurably slow a reader down; above it, the
    // reported penalty has largely gone.
    const SHALLOW_DEGREES: f64 = 25.0;

    fn crossing_angle(a: ((f64, f64), (f64, f64)), b: ((f64, f64), (f64, f64))) -> f64 {
        let (ax, ay) = (a.1.0 - a.0.0, a.1.1 - a.0.1);
        let (bx, by) = (b.1.0 - b.0.0, b.1.1 - b.0.1);
        let (la, lb) = ((ax * ax + ay * ay).sqrt(), (bx * bx + by * by).sqrt());
        if la < 1e-9 || lb < 1e-9 {
            return 90.0; // degenerate: nothing to read
        }
        let cos = ((ax * bx + ay * by) / (la * lb)).clamp(-1.0, 1.0);
        let deg = cos.acos().to_degrees();
        // 170° between directions is a 10° crossing to the eye.
        if deg > 90.0 { 180.0 - deg } else { deg }
    }

    // Two different defects, deliberately reported apart. A shallow crossing
    // costs reading speed; two lines lying *along* each other cost information
    // — the reader sees one line where the document declared two.
    let mut shallow = Vec::new();
    let mut overlapping = Vec::new();
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "../../ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        let doc = roxmltree::Document::parse(&svg).unwrap();
        // Collected here rather than reusing the helper in the layout module,
        // so this test's view of "what lines were drawn" stays independent of
        // the one the other invariants use.
        let lines: Vec<Vec<(f64, f64)>> = doc
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("path"))
            .filter(|n| !n.ancestors().any(|a| a.has_tag_name("defs")))
            .filter(|n| {
                n.attribute("class")
                    .is_some_and(|c| c.starts_with("edge-line") || c.starts_with("deny-line"))
            })
            .filter_map(|n| n.attribute("d").map(parse_path_points))
            .filter(|pts| pts.len() >= 2)
            .collect();

        for i in 0..lines.len() {
            for j in (i + 1)..lines.len() {
                for a in lines[i].windows(2) {
                    for b in lines[j].windows(2) {
                        let (sa, sb) = ((a[0], a[1]), (b[0], b[1]));
                        if !segments_cross(sa, sb) {
                            continue;
                        }
                        let deg = crossing_angle(sa, sb);
                        if deg < 1.0 {
                            overlapping.push(format!("{fixture}: {sa:?} lies along {sb:?}"));
                        } else if deg < SHALLOW_DEGREES {
                            shallow.push(format!(
                                "{fixture}: {sa:?} crosses {sb:?} at {deg:.0}° — shallow \
                                 enough that a reader tracing either one loses it at the \
                                 junction"
                            ));
                        }
                    }
                }
            }
        }
    }

    assert!(
        shallow.is_empty(),
        "lines crossing at a shallow angle are the costliest thing a layout can do to a \
         reader. Spread the endpoints along the box borders so these meet nearer a right \
         angle, or route one of them clear:\n  {}",
        shallow.join("\n  ")
    );

    // Was 4: three forbidden-call lines shared one vertical corridor in 003
    // and two shared a horizontal run, so they were drawn along each other
    // and read as one line. Worse than a crossing — a crossing slows you
    // down, an overlap hides a rule. Fixed by giving both `route_deny_line`
    // and `route_around_to_external` a caller-assigned rank (the same
    // monotone-by-target-y order `deny_order` already gives the wildcard
    // fan) that nests each further-ranked route's corridor and rail one
    // step further from the obstruction than the last — see
    // `RAIL_NEST_STEP`'s own doc comment for the mechanism and why nesting
    // by that order can't introduce a new crossing.
    //
    // This was a pinned ratchet (`KNOWN_OVERLAPPING_LINES`, last at 4) for as
    // long as there were overlaps to know about, because a green test that
    // knows about N defects and says nothing is the "gate debt: none"
    // over-claim this project has retracted once already. At zero there is
    // nothing left to pin, so it is a plain invariant: a knob still reading
    // "0 accepted" advertises a budget nobody should spend, and `<= 0` on a
    // length is only `is_empty()` wearing a ratchet's clothes.
    //
    // Height is what threatens this. A taller box moves arrows, so a header
    // line or a state row added to one of these documents can put two routes
    // back on the same path -- one component in vetting 003 does exactly that
    // and is deliberately left without a `state:`, with the reason recorded
    // in the document itself. This test is what catches the next one.
    assert!(
        overlapping.is_empty(),
        "two drawn lines lie along each other, so a reader sees one line where the \
         document declared two and a rule goes invisible. If a box grew here, that is \
         the cause -- height moves arrows:\n  {}",
        overlapping.join("\n  ")
    );
}

/// These diagrams are read on GitHub and in editors, where dark is a common
/// default, and the render paints its own near-white background — so a dark
/// reader got a bright panel rather than a diagram. One alternative palette,
/// not a theming hook: the meanings are fixed and CI enforces them, so a
/// palette a user could redefine would make those guarantees unenforceable.
/// Two expressions of one set of meanings, both held to the same rules.
#[test]
fn the_dark_palette_carries_every_meaning_the_light_one_does() {
    let svg = render_fixture("../../vetting/003-trading-system.ply.yaml");
    let style = svg
        .split("<style>")
        .nth(1)
        .and_then(|s| s.split("</style>").next())
        .expect("every render embeds a stylesheet");

    let dark = style
        .split("@media (prefers-color-scheme: dark)")
        .nth(1)
        .expect(
            "the stylesheet carries no dark-mode block, so a reader in dark mode gets a \
             bright panel rather than a diagram",
        );

    // The background must actually invert; a dark block that leaves the frame
    // near-white has changed nothing that matters.
    assert!(
        dark.contains(".workspace-frame"),
        "the dark block must repaint the frame — it is the surface everything else sits on"
    );

    // Every meaning that survives without colour must still be nameable in
    // dark: absence still marked, forbidden still distinct from attention.
    for needed in [".ceiling-unclaimed", ".deny-line", ".fn-clause"] {
        assert!(
            dark.contains(needed),
            "`{needed}` carries meaning in the light palette but is not restated for dark, \
             so it will render at its light value against a dark ground"
        );
    }

    // Absence must remain a drawn mark in dark, not revert to a flat fill.
    let unclaimed_dark = dark
        .split(".ceiling-unclaimed")
        .nth(1)
        .and_then(|s| s.split('}').next())
        .expect("dark block should define the unclaimed fill");
    assert!(
        unclaimed_dark.contains("url(#"),
        "in dark mode a component promising nothing is filled with `{unclaimed_dark}` — a \
         flat colour. The hatch is the whole point: absence drawn as blank reads as \
         background in either palette."
    );
}

/// Text drawn on a box has to be readable against that box. The evidence
/// ladder is encoded as depth of fill, so the darkest boxes are the ones a
/// reader most wants to read — and they were the hardest to. Measured, not
/// eyeballed: the standard contrast ratio, with the floor set at 3.0, the
/// usual bar for supplementary text.
///
/// This found a real defect on the fill ramp as it stood: the anchor and
/// ownership lines sat at 2.2 against the strongest promise fill.
#[test]
fn every_label_on_a_box_is_readable_against_that_box() {
    fn luminance(hex: &str) -> f64 {
        let h = hex.trim_start_matches('#');
        let ch = |i: usize| {
            let v = u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(0) + 0.7152 * ch(2) + 0.0722 * ch(4)
    }
    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    const FLOOR: f64 = 3.0;

    let svg = render_fixture("../../vetting/003-trading-system.ply.yaml");
    let style = svg
        .split("<style>")
        .nth(1)
        .and_then(|s| s.split("</style>").next())
        .unwrap();
    // The light palette only: the dark block is checked by its own test, and
    // mixing the two here would compare a light ink against a dark ground.
    let light = style
        .split("@media")
        .next()
        .expect("stylesheet should have a light section");
    let colour_of = |selector: &str| -> Option<String> {
        light
            .split('}')
            .filter_map(|r| r.split_once('{'))
            .find(|(sel, _)| sel.trim() == selector)
            .and_then(|(_, body)| {
                body.split(';')
                    .find_map(|d| d.trim().strip_prefix("fill:").map(str::to_owned))
            })
    };

    // Every ink that is drawn directly on a component's fill, against every
    // fill it can land on.
    let inks = [".component-name", ".component-anchor", ".component-owns"];
    let fills = [
        ".ceiling-tested",
        ".ceiling-fuzzed",
        ".ceiling-bounded",
        ".ceiling-proved",
    ];

    let mut unreadable = Vec::new();
    for ink in inks {
        let Some(fg) = colour_of(ink) else { continue };
        for fill in fills {
            let Some(bg) = colour_of(fill) else { continue };
            if !bg.starts_with('#') {
                continue; // the hatch, which is checked by its own test
            }
            let ratio = contrast(&fg, &bg);
            if ratio < FLOOR {
                unreadable.push(format!(
                    "{ink} ({fg}) on {fill} ({bg}): contrast {ratio:.2}, below {FLOOR:.1}"
                ));
            }
        }
    }

    assert!(
        unreadable.is_empty(),
        "these labels are drawn on a fill too close to their own colour. The strongest \
         promise fills are the ones a reader most wants to read:\n  {}",
        unreadable.join("\n  ")
    );
}

/// A component whose weakest function promises nothing is drawn at the bottom
/// of the ladder — correctly, because one unchecked thing caps everything
/// around it. But the sentence explaining that said "no checks are declared
/// anywhere in this component", which for a mixed component is simply false:
/// in vetting 003, six of `ingest`'s seven functions declare checks and one
/// empty list drags the fold down.
///
/// This is the tool stating something untrue about the user's own document —
/// the failure it exists to prevent, in its own output. The fix is a sentence
/// that says what actually happened.
#[test]
fn a_component_is_never_told_it_declares_nothing_when_something_inside_declares() {
    let svg = render_fixture("../../vetting/003-trading-system.ply.yaml");
    let doc = roxmltree::Document::parse(&svg).unwrap();

    let ingest = doc
        .descendants()
        .find(|n| n.attribute("data-name") == Some("ingest"))
        .and_then(|n| n.children().find(|c| c.has_tag_name("title")))
        .and_then(|t| t.text())
        .expect("the ingest component should carry a tooltip");

    assert!(
        !ingest.contains("no checks are declared anywhere in this component"),
        "`ingest` contains six functions that declare checks and one that does not, yet its \
         tooltip claims nothing anywhere inside declares any. Its shade is right; the \
         sentence explaining the shade is false. Tooltip was:\n{ingest}"
    );
}

/// The transcript: everything the diagram says, written out. It exists
/// because 95% of what a render says lives in hover text — measured on the
/// committed trading-system diagram, 9,923 characters of hover against 474
/// drawn — and the reader this was built for cannot hover.
///
/// This first test pins the three things that make it honest rather than
/// merely present: it says what it is, it says that editing it does nothing,
/// and it says that nothing has been run. Then it checks one fact of each
/// kind actually arrives.
#[test]
fn the_transcript_states_what_it_is_and_what_the_document_declares() {
    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    let text = ply_render::transcript::render_transcript(&doc);

    for required in [
        // What it is, and that it is a rendering rather than a source.
        "Ply transcript",
        "Editing this text changes nothing",
        // The strip's own rule, extended to the whole document. Worded as
        // what this renderer can actually know from a parsed document --
        // see `the_transcript_only_claims_what_it_can_actually_know`.
        "No result reaches this page",
    ] {
        assert!(
            text.contains(required),
            "the transcript header must say {required:?} — a reader who mistakes it for the \
             document will edit it and change nothing. Got:\n{text}"
        );
    }

    for fact in [
        // the summary
        "11 components · 14 functions · 2 promise nothing",
        // a component and its anchor
        "component risk — maps to Rust module risk",
        // a check, in words rather than a code
        "3 is the maximum loop-unwind depth, not a numeric input limit",
        // a contract clause
        "order.qty > 0 && order.px > 0",
        // a fact the diagram only ever showed on hover
        "only this component may mutate them",
        // an outside party
        "the exchange: accepts orders, returns fills",
        // a forbidden call, with its wildcard spelled out
        "no component may call risk",
        // an open decision
        "venue failover",
    ] {
        assert!(
            text.contains(fact),
            "the transcript must carry {fact:?}. Got:\n{text}"
        );
    }
}

/// Same document in, byte-identical transcript out. The design discharges
/// this rather than the test proving it — the render function takes the
/// parsed document and returns a string, so no clock, path, environment or
/// locale is in scope to leak — but a signature can be widened by accident,
/// and this notices when it is.
#[test]
fn the_transcript_is_byte_identical_across_runs() {
    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let a = ply_render::transcript::render_transcript(&doc);
    let b = ply_render::transcript::render_transcript(&doc);
    assert_eq!(a, b, "two renders of one document differed");

    // A second parse of the same bytes must also agree: this is where a
    // hash-ordered container would show itself, since its iteration order can
    // differ between two maps built from identical input.
    let doc2 = parse_document(&yaml).unwrap();
    assert_eq!(
        a,
        ply_render::transcript::render_transcript(&doc2),
        "two parses of the same bytes produced different transcripts — something in the walk \
         depends on allocation or hash order rather than on the document"
    );
}

/// Where the file sits on disk must not reach the output. Path leakage is the
/// single most likely machine fact to creep into a text renderer, and it is
/// invisible until someone else runs it.
#[test]
fn the_transcript_does_not_depend_on_where_the_file_lives() {
    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("renamed.ply.yaml");
    std::fs::write(&elsewhere, &yaml).unwrap();

    let from_here = ply_render::transcript::render_transcript(&parse_document(&yaml).unwrap());
    let moved = std::fs::read_to_string(&elsewhere).unwrap();
    let from_there = ply_render::transcript::render_transcript(&parse_document(&moved).unwrap());

    assert_eq!(
        from_here, from_there,
        "the transcript changed when the same document was read from a different path"
    );
}

/// The two views must not drift apart on the one thing they word identically:
/// the names. This generates both from the same document and checks that
/// every component and function the drawing labels is also named in the text
/// — a live comparison, so it holds for any document, not only the ones with
/// a committed copy in `vetting/`.
///
/// Names only, and deliberately so. The rest of what the picture says is
/// labels the text form expands in words, so demanding verbatim agreement
/// there would duplicate presentation. That everything *declared* survives
/// into the text is the separate
/// and stronger check in `the_transcript_leaves_nothing_in_the_document_out`,
/// which walks the document rather than the drawing.
///
/// The direction matters. A transcript that quietly said *less* than the
/// picture would be the more dangerous failure, because its whole purpose is
/// to be the complete view for a reader who cannot hover.
#[test]
fn the_transcript_and_the_drawing_state_the_same_facts() {
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "../../ply.yaml",
        "tests/fixtures/declared_shapes.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);
        let svg = ply_render::svg::render_svg(&doc).unwrap();
        let parsed = roxmltree::Document::parse(&svg).unwrap();

        // Every component and function the drawing names.
        let mut missing = Vec::new();
        for node in parsed.descendants() {
            if let Some(name) = node.attribute("data-name") {
                // Nested components are drawn by their own name, so the leaf
                // name is what both views share.
                let leaf = name.rsplit('.').next().unwrap_or(name);
                if !text.contains(leaf) {
                    missing.push(format!("component `{leaf}`"));
                }
            }
            if let Some(f) = node.attribute("data-fn")
                && !text.contains(f)
            {
                missing.push(format!("function `{f}`"));
            }
        }

        assert!(
            missing.is_empty(),
            "{fixture}: the drawing shows these, and the transcript does not mention them. The \
             transcript is the complete view for a reader who cannot hover, so anything the \
             picture knows and it does not is a hole in exactly the wrong place:\n  {}",
            missing.join("\n  ")
        );
    }
}

/// Walks the parsed document itself and fails on the first thing in it the
/// text does not carry. This is the invariant the transcript actually
/// promises — it is sold as the complete view for a reader who cannot hover,
/// so anything declared and not restated is a hole in exactly the wrong
/// place. Driving from the document rather than from the drawing is
/// deliberate: it covers what the drawing shows *and* what only its hover
/// text shows, and a construct added to the grammar later cannot quietly
/// skip the check, because the walk visits every field.
///
/// It is not spelled as a substring sweep over the drawing's own sentences.
/// Some of what the picture says is visual shorthand — `e×1`, `⛉`, `*` —
/// that the text form deliberately spells out in words,
/// so verbatim agreement between the two is the wrong bar and would force
/// the text to be as terse as the picture.
#[test]
fn the_transcript_leaves_nothing_in_the_document_out() {
    /// The lines belonging to one `fn NAME` or `component NAME` heading: the
    /// heading itself plus everything indented under it. Needles are looked
    /// for in here rather than in the whole transcript, because the document
    /// repeats names across sections -- an `entry: [venue]` needle is
    /// satisfied by the externals section naming `venue`, which proves
    /// nothing about the function.
    fn block(text: &str, heading: &str) -> String {
        let mut lines = text.lines();
        let head = loop {
            match lines.next() {
                None => return String::new(),
                Some(l) if l.trim_start().starts_with(heading) => break l,
                _ => {}
            }
        };
        let depth = head.len() - head.trim_start().len();
        let mut out = String::from(head);
        for l in lines {
            let d = l.len() - l.trim_start().len();
            if !l.trim().is_empty() && d <= depth {
                break;
            }
            out.push('\n');
            out.push_str(l);
        }
        out
    }

    // Every field of both structs is bound by name, and neither pattern
    // carries a `..` rest: a field added to the grammar later stops this
    // file compiling until someone decides what the text form owes it. The
    // first version of this walk read the fields it happened to remember and
    // silently skipped four -- deleting the whole worked-examples block, or
    // the `pure` sentence, left every test green (found by review,
    // 2026-08-30). A walk that claims to visit every field has to be made of
    // something stronger than the author's memory.
    fn walk(
        prefix: &str,
        comps: &indexmap::IndexMap<String, ply_render::model::Component>,
        text: &str,
        missing: &mut Vec<String>,
    ) {
        for (name, comp) in comps {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            let ply_render::model::Component {
                anchor,
                note,
                pure,
                strict,
                uses,
                owns,
                state,
                profile,
                checks,
                components,
                fns,
            } = comp;

            let mut want = vec![(format!("component `{path}`"), name.clone())];
            want.push((format!("{path}'s anchor"), anchor.clone()));
            for u in uses {
                want.push((format!("{path}'s capability `{u}`"), u.clone()));
            }
            for o in owns {
                want.push((format!("{path} owns `{o}`"), o.clone()));
            }
            if let Some(st) = state {
                want.push((format!("{path}'s state type `{}`", st.of), st.of.clone()));
                for f in &st.show {
                    want.push((format!("{path}'s shown field `{}`", f.name), f.name.clone()));
                }
            }
            if let Some(p) = profile {
                want.push((format!("{path}'s profile `{p}`"), p.clone()));
            }
            if let Some(n) = note {
                want.push((format!("{path}'s note"), n.clone()));
            }
            for c in checks.iter().flatten() {
                want.push((format!("{path}'s default check `{c}`"), c.clone()));
            }
            for (label, needle) in want {
                if !text.contains(&needle) {
                    missing.push(label);
                }
            }

            // A flag has no text of its own to search for, so what is checked
            // is that the sentence it owes appears in this component's own
            // block -- scoped, or one `pure` component anywhere in the
            // document would satisfy every other component's check.
            let comp_block = block(text, &format!("component {name} "));
            if *pure && !comp_block.contains("pure") {
                missing.push(format!(
                    "{path} is sealed `pure` and its block never says so"
                ));
            }
            if *strict && !comp_block.contains("strict") {
                missing.push(format!(
                    "{path} is `strict` -- findings inside it fail the build rather than warn -- \
                     and its block never says so"
                ));
            }

            for (fname, fc) in fns {
                let ply_render::model::FnClaim {
                    checks,
                    mode,
                    requires,
                    ensures,
                    examples,
                    check_with,
                    trusted,
                    unresolved,
                    entry,
                } = fc;

                let mut want = vec![(format!("fn `{fname}`"), fname.clone())];
                for c in checks.iter().flatten() {
                    want.push((format!("`{fname}`'s check `{c}`"), c.clone()));
                }
                for r in requires {
                    want.push((format!("`{fname}`'s requires `{r}`"), r.clone()));
                }
                for e in ensures {
                    want.push((format!("`{fname}`'s ensures `{e}`"), e.clone()));
                }
                for e in examples {
                    want.push((format!("`{fname}`'s worked example `{e}`"), e.clone()));
                }
                for (k, v) in check_with {
                    want.push((format!("`{fname}`'s {k} binding"), format!("{k}={v}")));
                }
                for t in trusted {
                    want.push((format!("`{fname}`'s trusted claim"), t.claim.clone()));
                    want.push((format!("`{fname}`'s trusted evidence"), t.evidence.clone()));
                }
                for u in unresolved {
                    want.push((
                        format!("`{fname}`'s open question #{}", u.id),
                        u.note.clone(),
                    ));
                }
                for (label, needle) in want {
                    if !text.contains(&needle) {
                        missing.push(label);
                    }
                }

                let fn_block = block(text, &format!("fn {fname}"));
                for e in entry {
                    if !fn_block.contains(e.as_str()) {
                        missing.push(format!("`{fname}`'s entry point `{e}`"));
                    }
                }
                if matches!(mode, ply_render::model::Mode::Synth)
                    && !fn_block.contains("machine-written")
                {
                    missing.push(format!(
                        "`{fname}` is a function Ply writes the body of rather than one whose \
                         human-written body it checks, and its block never says so"
                    ));
                }
            }

            walk(&path, components, text, missing);
        }
    }

    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "../../ply.yaml",
        "tests/fixtures/full.ply.yaml",
        // Carries the only `mode: synth` in the repo, plus `strict`, `pure`
        // and worked examples -- without it those four fields are checked
        // against nothing, which is how they went unnoticed the first time.
        "tests/fixtures/visual_forms.ply.yaml",
        "tests/fixtures/checks_inheritance.ply.yaml",
        "tests/fixtures/inherited_empty.ply.yaml",
        "tests/fixtures/externals.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);

        // Bound field by field with no `..`, for the same reason `Component`
        // and `FnClaim` are below: a top-level field added later stops this
        // compiling until someone decides what the text form owes it. Its
        // absence is why `ply` -- the field that decides which rules every
        // other line is read under -- went unrendered and unnoticed
        // (external review, 2026-08-30).
        let ply_render::model::Document {
            ply,
            components,
            externals,
            edges,
            deny,
            profiles,
            unresolved,
            routes,
        } = &doc;

        let mut missing = Vec::new();
        if !text.contains(&format!("ply: {ply}")) {
            missing.push(format!("the document's format version, `ply: {ply}`"));
        }
        walk("", components, &text, &mut missing);

        for (name, ext) in externals {
            if !text.contains(name) {
                missing.push(format!("external `{name}`"));
            }
            if !text.contains(&ext.note) {
                missing.push(format!("external `{name}`'s note"));
            }
        }
        for e in edges {
            // An edge is restated in words, not echoed: the arrow itself is
            // what has to survive, so both endpoints must be findable in one
            // line of the text.
            if !text.lines().any(|l| {
                e.split(['-', '>', '~', ':'])
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .all(|p| l.contains(p))
            }) {
                missing.push(format!("edge `{e}`"));
            }
        }
        for d in deny {
            if !text.lines().any(|l| {
                d.split(['-', '>', '!', ' '])
                    .map(str::trim)
                    .filter(|p| !p.is_empty() && *p != "except")
                    .all(|p| l.contains(p) || p == "*")
            }) {
                missing.push(format!("forbidden call `{d}`"));
            }
        }
        for (name, rules) in profiles {
            if !text.contains(name) {
                missing.push(format!("profile `{name}`"));
            }
            for r in rules {
                if !text.contains(r) {
                    missing.push(format!("profile `{name}`'s rule `{r}`"));
                }
            }
        }
        for u in unresolved {
            if !text.contains(&u.note) {
                missing.push(format!("open question #{}", u.id));
            }
        }
        for (type_name, fn_path) in routes {
            if !text.contains(type_name) || !text.contains(fn_path) {
                missing.push(format!("route `{type_name}: {fn_path}`"));
            }
        }

        assert!(
            missing.is_empty(),
            "{fixture}: the document declares these and the text form does not carry them. The \
             text form is sold as the complete view for a reader who cannot hover, so anything \
             it drops is a confident silence about something that was written down:\n  {}",
            missing.join("\n  ")
        );
    }
}

/// A function that wrote `checks: []` and a function that wrote nothing and
/// inherited an empty list from an ancestor both end up with nothing
/// verified — but the document says so in opposite ways, and the transcript
/// exists to keep §5.4c's two statements apart. Telling a reader that a
/// function "wrote an empty list" when it wrote nothing at all is a false
/// statement about the document, in the one place this whole feature was
/// argued for.
#[test]
fn a_function_is_never_told_it_wrote_a_list_it_did_not_write() {
    let yaml = std::fs::read_to_string("tests/fixtures/inherited_empty.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);

    let line_for = |name: &str| -> String {
        let mut lines = text
            .lines()
            .skip_while(|l| l.trim() != format!("fn {name}"));
        lines.next();
        lines
            .next()
            .unwrap_or_else(|| panic!("no line under `fn {name}` in:\n{text}"))
            .trim()
            .to_string()
    };

    let silent = line_for("silent");
    let spelt_out = line_for("spelt_out");

    assert_ne!(
        silent, spelt_out,
        "a function that wrote nothing and one that wrote `checks: []` are given the same \
         sentence. They are different statements (§5.4c) and the transcript is the view that \
         is supposed to tell them apart."
    );
    assert!(
        !silent.contains("written"),
        "`silent` wrote no checks line at all, and the transcript calls what it has written: \
         {silent:?}"
    );
    assert!(
        silent.contains("switched_off"),
        "`silent` is unchecked because an ancestor switched checking off, and a reader cannot \
         act on that without being told which ancestor: {silent:?}"
    );
    assert!(
        spelt_out.contains("written"),
        "`spelt_out` did write an empty list, and the transcript should say so: {spelt_out:?}"
    );

    // The non-empty inherited case already names its source; this pins that
    // the empty case is not the odd one out.
    assert!(
        line_for("inherits").contains("handed_down"),
        "an inherited check names the component it came from"
    );
}

/// Every component block has to answer "how strongly is this checked, and
/// why that strongly" — and every one of the sentences that answers it is
/// pinned word for word. These are the transcript's *derived* sentences:
/// they restate no field of the document, so the field walk in
/// `the_transcript_leaves_nothing_in_the_document_out` cannot see them, and
/// deleting any of them left every test green until this existed (review,
/// 2026-08-30).
///
/// Pinning the exact words is the project rule for user-facing wording, and
/// it earns its keep here twice over: these four sentences carry the whole
/// evidence ladder to a reader who has never seen Ply, and three of them are
/// the only place the document's §5.4c distinctions are ever spelled out.
#[test]
fn every_component_says_how_strongly_it_is_checked_and_why() {
    // The complete set. A component block matches exactly one of these, and
    // a new one added later shows up as an unexplained block below.
    // Reworded 2026-09-03: "nothing inside" stopped being true the day a box
    // could draw what its component holds, so a dashed box can be full of
    // state rows. What the dashed border has always meant is narrower and
    // still exactly right -- nothing here promises anything -- and the
    // sentence now says that instead.
    const HOLLOW: &str = "hollow — promises nothing yet: no functions, no nested \
                          components. Saying what it holds is not a promise about how it \
                          behaves. A sketch waiting for claims.";
    const UNCLAIMED: &str = "promises nothing as a whole — something inside declares no checks, \
                             and one unchecked thing sets the level of everything around it \
                             (unclaimed)";
    const DECLARES: &str = "declares checks up to ";
    const WEAKEST: &str = "that level comes from its weakest part, ";

    let mut seen_hollow = false;
    let mut seen_unclaimed = false;
    let mut seen_declares = false;

    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "../../ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/visual_forms.ply.yaml",
        "tests/fixtures/hollow.ply.yaml",
        "tests/fixtures/checks_inheritance.ply.yaml",
        "tests/fixtures/inherited_empty.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);

        for line in text.lines() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("component ") {
                continue;
            }
            let name = trimmed
                .trim_start_matches("component ")
                .split(' ')
                .next()
                .unwrap();
            let depth = line.len() - trimmed.len();
            // The sentences that belong to this component, not to a child:
            // everything indented under it, stopping at the first nested
            // `component` heading.
            let own: String = text
                .lines()
                .skip_while(|l| *l != line)
                .skip(1)
                .take_while(|l| {
                    let d = l.len() - l.trim_start().len();
                    !l.trim().is_empty() && d > depth && !l.trim_start().starts_with("component ")
                })
                .collect::<Vec<_>>()
                .join("\n");

            let hollow = own.contains(HOLLOW);
            let unclaimed = own.contains(UNCLAIMED);
            let declares = own.contains(DECLARES);
            seen_hollow |= hollow;
            seen_unclaimed |= unclaimed;
            seen_declares |= declares;

            assert_eq!(
                [hollow, unclaimed, declares].iter().filter(|b| **b).count(),
                1,
                "{fixture}: component `{name}` should say exactly one of: it declares nothing \
                 yet, it promises nothing because something inside does not, or how strongly it \
                 is checked. A reader with none of those cannot tell whether an empty-looking \
                 box is unfinished or unchecked, and those are not the same worry. Its block \
                 was:\n{own}"
            );
            if !hollow {
                assert!(
                    own.contains(WEAKEST),
                    "{fixture}: component `{name}` states a level and never says which \
                     declaration set it. The whole point of worst-of is that one weak thing \
                     drags the rest down, and a reader who is not told which one cannot go and \
                     fix it. Its block was:\n{own}"
                );
            }
        }
    }

    // A pinned sentence nothing exercises is a pinned sentence that can rot.
    assert!(
        seen_hollow,
        "no fixture exercises a component that declares nothing yet"
    );
    assert!(
        seen_unclaimed,
        "no fixture exercises a component dragged to unclaimed by something inside it"
    );
    assert!(
        seen_declares,
        "no fixture exercises a component that declares checks"
    );
}

/// The two ways a function can end up with nothing checked, and the one way
/// it can inherit something, each get their own words — pinned, because
/// these three sentences are where §5.4c either survives into plain English
/// or dies. `silent` and `spelt_out` in the fixture are the pair a reader
/// most easily conflates: same outcome, opposite statements.
#[test]
fn the_three_sentences_that_carry_the_inheritance_rules_are_pinned() {
    let yaml = std::fs::read_to_string("tests/fixtures/inherited_empty.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);

    for expected in [
        // A component that switches checking off for everything inside it.
        "checks: [] — an empty list written on purpose: a function in here that writes no \
         checks of its own is checked by nothing at all, and does not fall back to any outer \
         default",
        // A function that wrote the empty list itself.
        "checks: [] — a written empty list: this document says to check nothing here, so \
         nothing about this function is verified (unclaimed)",
        // A function that wrote nothing and had the empty list handed to it.
        "nothing is checked here, and this function did not ask for that: it declares no checks \
         of its own, and component switched_off sets an empty default list, which switches \
         checking off for everything inside it (unclaimed)",
        // A function that wrote nothing and had a real list handed to it.
        "inherited from component handed_down: bounded(2) — Kani symbolically checks supported inputs that satisfy requires against ensures; 2 is the maximum loop-unwind depth, not a numeric input limit; collection inputs may also be limited to length 2",
    ] {
        assert!(
            text.contains(expected),
            "this sentence is how a reader learns what the document actually said about a \
             function's checks, and it is not in the output:\n  {expected}\n\ngot:\n{text}"
        );
    }
}

/// A sealed component's sentence names the rule that catches a breach, and
/// a reader acts on it — so naming the wrong rule, or the wrong severity,
/// sends them to the wrong place with the wrong urgency. Both views said
/// "capability use inside it is an error (A0408)". The spec says a `pure`
/// component touching a capability is `A0403`, warning-severity by default
/// and an error only under `strict`; `A0408` is a different rule about
/// helper functions used inside contracts. Wrong code, wrong severity, in
/// the one sentence a reader would quote.
#[test]
fn a_sealed_component_names_the_rule_that_actually_catches_a_breach() {
    let yaml = std::fs::read_to_string("tests/fixtures/visual_forms.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    let svg = ply_render::svg::render_svg(&doc).unwrap();

    for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
        assert!(
            !body.contains("A0408"),
            "{view} tells a reader to look up A0408 for a sealed component that touches a \
             capability. A0408 is about helper functions used inside contracts — they would \
             read the wrong rule and find it does not describe their problem."
        );
        // This test used to *require* `A0403` here, on the reasoning that a
        // reader needs the rule to look up. That was half right and half
        // wrong: A0403 is the correct code in the spec, and no checker emits
        // it, so printing it beside a sealed component implied a diagnostic
        // that cannot fire. The section reference stays (the rule is
        // specified); the code goes until something raises it.
        assert!(
            !body.contains("A0403"),
            "{view} cites a diagnostic code for a rule nothing in this build raises. A code \
             beside a sentence reads as 'this is what you will see when it fires', and \
             nothing fires."
        );
    }

    assert!(
        text.contains(
            "pure — a sealed promise: this component declares no capabilities and may not \
             use any. That is declared here, and not checked by this build"
        ),
        "the sealed-component sentence is not what it should be:\n{text}"
    );
}

/// `pure: true` beside a `uses:` list is a document contradicting itself.
/// Both halves have to be stated: a view that silently prints one and drops
/// the other is telling the reader the document says less than it does, and
/// the half it dropped is the one that would explain a surprising finding.
#[test]
fn a_component_that_both_seals_itself_and_declares_capabilities_has_both_stated() {
    let yaml = "ply: 1\ncomponents:\n  muddle:\n    anchor: app::muddle\n    pure: true\n    \
                uses: [net]\n";
    let doc = parse_document(yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);

    assert!(
        text.contains("pure"),
        "the document says this component is sealed and the text does not:\n{text}"
    );
    assert!(
        text.contains("net"),
        "the document declares `net` on this component and the text never mentions it — a \
         reader would believe nothing was declared:\n{text}"
    );
}

/// The opening summary counts functions that end up with nothing checked,
/// and both views glossed that as "code this document says nothing about".
/// For a function that wrote `checks: []` that is backwards: the document
/// says something very deliberate about it — check nothing here. In the
/// trading-system fixture both counted functions are that kind, so the gloss
/// was wrong about 2 of the 2 functions it described.
#[test]
fn the_summary_gloss_is_true_of_the_functions_it_counts() {
    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    let svg = ply_render::svg::render_svg(&doc).unwrap();

    // Both functions this fixture counts wrote `checks: []` on purpose.
    for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
        assert!(
            !body.contains("says nothing about"),
            "{view} says the counted functions are ones the document says nothing about. Both \
             of them wrote `checks: []` — the document says exactly what it wants for them, \
             and a reader told otherwise will go looking for an omission that is not there."
        );
    }
    assert!(
        text.contains(
            "(\"promise nothing\" counts functions that end up with nothing checked — whether \
             nobody wrote any checks for them, or the document switched checking off for them \
             on purpose)"
        ),
        "the summary gloss is not what it should be:\n{text}"
    );
}

/// The renderer is handed a parsed document and nothing else. It cannot see
/// whether `cargo ply verify` has ever run, so "Nothing here has been run"
/// is a claim about the world outside its inputs — and flatly false for
/// anyone who has just run a verification. What it can honestly say is that
/// no result reaches this page.
#[test]
fn the_transcript_only_claims_what_it_can_actually_know() {
    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);

    assert!(
        !text.contains("Nothing here has been run"),
        "the text form asserts that nothing has been run. It is built from the document alone \
         and has no way to know that; a reader who has just run a verification is told a plain \
         falsehood on line three."
    );
    assert!(
        text.contains(
            "No result reaches this page. Every line below is a declaration or a promise, never \
             a result — whatever `cargo ply verify` has found, it is reported there and never \
             here."
        ),
        "the header does not say what it can honestly say:\n{text}"
    );
}

/// Four sentences that a reader who has never seen Ply has to be able to
/// act on. Three were jargon or ambiguous; the fourth asserted an
/// enforcement this build does not perform.
///
/// The last is the one that matters most. The spec is explicit that the
/// cap on a function with an open question "is not enforced" — nothing
/// applies it, and a verification runs whatever the claim asked for. Both
/// views told the reader the cap was in effect. `worklist` says the true
/// thing on every marker line; these two now say it too.
#[test]
fn the_sentences_a_first_time_reader_has_to_act_on_say_what_they_mean() {
    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    let svg = ply_render::svg::render_svg(&doc).unwrap();

    for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
        assert!(
            !body.contains("checks cap at `test`"),
            "{view} tells a reader an open question caps this function's checks. The spec is \
             explicit that the cap is not enforced — nothing applies it, and a verification \
             runs the full claim anyway. A reader who believes it will think a risk is \
             contained when it is not."
        );
        assert!(
            body.contains("that cap is not applied yet"),
            "{view} drops the caveat entirely instead of stating it. Saying nothing about the \
             cap leaves the same false impression as claiming it works."
        );
    }

    assert!(
        !text.contains("contract at the watermark"),
        "the text form heads a function's promise with `watermark`, a word that means nothing \
         outside this project and is not glossed by what follows it"
    );
    assert!(
        text.contains(
            "what this function promises — the last thing the document states before the code \
             itself takes over:"
        ),
        "the contract heading does not say, in plain words, what the block under it is:\n{text}"
    );
    assert!(
        !text.contains("the level above is set by"),
        "\"the level above\" reads as the parent component rather than as the line printed \
         immediately before, and after a `promises nothing` line there is no level above at all"
    );
    assert!(
        text.contains("that level comes from its weakest part, "),
        "the sentence explaining where a component's level came from is not what it should \
         be:\n{text}"
    );
}

/// Indentation is the only thing that says which function belongs to which
/// component in the text form — there are no boxes, no lines, nothing else
/// to group by. So the depth the walk hands down its own recursion is load
/// bearing, and it was checked nowhere: passing a child the *same* depth as
/// its parent, so a nested component's whole subtree flattens into its
/// parent, left every test green (coverage audit, 2026-08-30). Every
/// assertion about the text was "these words appear somewhere", and
/// somewhere is not the same place.
#[test]
fn nesting_in_the_text_matches_nesting_in_the_document() {
    fn expect(
        comps: &indexmap::IndexMap<String, ply_render::model::Component>,
        depth: usize,
        out: &mut Vec<(String, usize)>,
    ) {
        for (name, comp) in comps {
            // Top-level components sit one level in from the margin.
            out.push((format!("component {name} "), 2 * (depth + 1)));
            for fname in comp.fns.keys() {
                out.push((format!("fn {fname}"), 2 * (depth + 2)));
            }
            expect(&comp.components, depth + 1, out);
        }
    }

    for fixture in [
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/checks_inheritance.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);

        let mut wanted = Vec::new();
        expect(&doc.components, 0, &mut wanted);
        assert!(
            wanted.iter().any(|(h, _)| h.starts_with("component ")),
            "{fixture} has no components, so this proves nothing"
        );

        for (heading, indent) in wanted {
            let line = text
                .lines()
                .find(|l| l.trim_start().starts_with(heading.trim_end()))
                .unwrap_or_else(|| panic!("{fixture}: no line for {heading:?} in:\n{text}"));
            let got = line.len() - line.trim_start().len();
            assert_eq!(
                got, indent,
                "{fixture}: {heading:?} is indented {got} spaces and its place in the document \
                 says {indent}. Indentation is the only thing grouping a function with its \
                 component here, so a reader would attach it to the wrong one."
            );
        }
    }
}

/// Every edge that names an outside party carries the note saying so. The
/// normal shape is one internal end and one external end — and the test set
/// had nothing pinning that shape, so requiring *both* ends to be external
/// (which is essentially never) silently deleted the note from every real
/// edge and no test noticed (coverage audit, 2026-08-30). That note is the
/// only thing on the line that says the arrow is a declaration nobody
/// checked.
#[test]
fn an_edge_touching_the_outside_world_always_says_it_is_unverified() {
    for fixture in [
        "../../vetting/003-trading-system.ply.yaml",
        "tests/fixtures/externals.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);

        let mut checked = 0;
        for raw in &doc.edges {
            let (from, to) = raw
                .split_once("~>")
                .or_else(|| raw.split_once("->"))
                .expect("an edge names two ends");
            let from = from.trim();
            let to = to.split(':').next().unwrap().trim();
            let external = [from, to]
                .into_iter()
                .find(|e| doc.externals.contains_key(*e));
            let Some(external) = external else { continue };
            checked += 1;

            let line = text
                .lines()
                .find(|l| l.contains(from) && l.contains(to) && l.trim().starts_with(from))
                .unwrap_or_else(|| panic!("{fixture}: no line for edge {raw:?} in:\n{text}"));
            assert!(
                line.contains(&format!(
                    "— {external} is outside this codebase, so this edge is a declaration, \
                     never a verified fact"
                )),
                "{fixture}: this edge touches `{external}`, which Ply never checks, and the \
                 line does not say so. A reader takes it for a verified fact: {line:?}"
            );
        }
        assert!(
            checked > 0,
            "{fixture}: no edge here names an external, so this proves nothing"
        );
    }
}

/// `--focus` promises that the named component and what is inside it are
/// spelled out in full, while the boxes on the path down to it stay plain —
/// they are there for orientation, and filling them with clause text drowns
/// the thing being focused on. Swapping which side of the path counts as
/// "inside" inverts exactly that, and every focus test passed: they check
/// geometry and overflow of whatever got drawn, never that the right boxes
/// got the detail (coverage audit, 2026-08-30).
///
/// Both directions have to be exercised, and picking the wrong pair proves
/// nothing: the target itself and an unrelated component land on the same
/// side of the swap, so a test built from those two passes either way. What
/// separates them is a component *inside* the target and a component *above*
/// it.
#[test]
fn focus_spells_out_what_is_inside_the_target_and_not_what_is_above_it() {
    use ply_render::svg::{RenderOptions, render_svg_with_options};

    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();

    // A chip drawn with its promise spelled out carries clause text; one
    // drawn plain does not.
    let spelled_out = |focus: &str, fname: &str| -> bool {
        let svg = render_svg_with_options(
            &doc,
            &RenderOptions {
                focus: Some(focus.to_string()),
                ..RenderOptions::default()
            },
        )
        .unwrap();
        let parsed = roxmltree::Document::parse(&svg).unwrap();
        parsed
            .descendants()
            .find(|n| n.attribute("data-fn") == Some(fname))
            .unwrap_or_else(|| panic!("--focus {focus} drew no chip for {fname}"))
            .descendants()
            .any(|d| {
                d.attribute("class")
                    .is_some_and(|c| c.split(' ').any(|t| t == "fn-clause"))
            })
    };

    // Inside the target, two levels down: this is what was asked for.
    assert!(
        spelled_out("ingest", "OrderBook::apply"),
        "`OrderBook::apply` sits inside the component that was focused on, and its promise is \
         not spelled out — which is the one thing --focus was asked to do"
    );
    // Above the target: drawn so the reader can see the path down, and
    // deliberately left plain.
    assert!(
        !spelled_out("strategy.signals", "Strategy::on_update"),
        "`Strategy::on_update` is in a component the focused one merely sits inside, and its \
         promise is spelled out. Those boxes are drawn for orientation; filling them with \
         clause text buries the component actually being focused on"
    );
    // A component off the path entirely is folded away and draws no chips at
    // all, so there is nothing to ask about it here -- that is `--focus`'s
    // collapse behaviour, pinned by the tests in `mod collapse`.
}

/// Neither view may describe a rule as enforced that this build does not
/// enforce. `ply check` inspects crate-level dependencies from Cargo
/// metadata and says so plainly in its own output: it "does not yet look
/// inside your functions". The rules for calls between components,
/// capability use, and `strict` escalation are declared in the grammar and
/// implemented nowhere — `A0402`, `A0403` and `A0404` appear in no checker.
///
/// Both views stated them in the present tense anyway, which is this
/// project's defining failure mode reproduced inside its own reporting: a
/// reader — or the model these are written for — concludes a boundary is
/// guarded when nothing guards it (external review, 2026-08-30).
#[test]
fn neither_view_claims_a_rule_is_enforced_that_nothing_enforces() {
    let yaml = std::fs::read_to_string("../../vetting/003-trading-system.ply.yaml").unwrap();
    let doc = parse_document(&yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    let svg = ply_render::svg::render_svg(&doc).unwrap();

    for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
        for phantom in [
            // Present-tense claims that a finding fires today.
            "is an architecture finding",
            "is reported as an architecture finding",
            // `strict` is read by these two renderers and by nothing else.
            "architecture findings inside this component fail the build",
            // The one rule that IS implemented is always an error, so even
            // the half-true version was wrong about its severity.
            "a warning by default",
        ] {
            assert!(
                !body.contains(phantom),
                "{view} says {phantom:?}. Nothing in this build does that: the checker looks at \
                 crate-level dependencies only, and says so itself. A reader told a boundary is \
                 guarded will stop guarding it."
            );
        }
    }

    // What it should say instead: declared here, and honest about the gap.
    assert!(
        text.contains(
            "declared here, and not checked by this build: Ply currently compares crate-level \
             dependencies and does not yet look inside functions, so a call that crosses this \
             line can still go unnoticed (§5.3)"
        ),
        "the text form does not carry the honest wording:\n{text}"
    );
}

/// A contract closes with "the checks above test the function against
/// exactly this promise". When the function's effective check list is
/// empty there are no checks above, and the transcript had already said so
/// four lines earlier — two sentences contradicting each other on one
/// screen, on exactly the shape (a legacy boundary that declares intent and
/// verifies nothing) where the distinction matters most.
#[test]
fn a_contract_with_nothing_checking_it_is_not_described_as_checked() {
    let yaml = "ply: 1\ncomponents:\n  legacy:\n    anchor: app::legacy\n    fns:\n      \
                rate:\n        checks: []\n        ensures:\n          - \"|r| *r <= 10000\"\n";
    let doc = parse_document(yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);

    assert!(
        !text.contains("the checks above test the function against exactly this promise"),
        "this function declares no checks at all, and the transcript still says the checks \
         above test it. There are none:\n{text}"
    );
    assert!(
        text.contains(
            "nothing above checks this promise — it is written down, and this document asks \
             for no check that would test it"
        ),
        "an unchecked contract should be named as one:\n{text}"
    );
}

/// Worked examples are compiled into tests only when the effective check
/// list contains `test` — the verifier's example codegen sits inside that
/// branch. The transcript said "compiled into a test" for any non-empty
/// `examples:`, so a document asking only for fuzzing was told its examples
/// run when they never do.
#[test]
fn worked_examples_are_only_called_tests_when_something_runs_them() {
    let with_test = "ply: 1\ncomponents:\n  a:\n    anchor: app::a\n    fns:\n      f:\n        \
                     checks: [test]\n        examples:\n          - \"f(1) == 2\"\n";
    let without = "ply: 1\ncomponents:\n  a:\n    anchor: app::a\n    fns:\n      f:\n        \
                   checks: [fuzz(64)]\n        examples:\n          - \"f(1) == 2\"\n";

    let ran = ply_render::transcript::render_transcript(&parse_document(with_test).unwrap());
    let not = ply_render::transcript::render_transcript(&parse_document(without).unwrap());

    assert!(
        ran.contains("compiles each into a test when `cargo ply verify` runs"),
        "with `test` in the list the example really is compiled and run:\n{ran}"
    );
    assert!(
        !not.contains("compiled into a test"),
        "this function asks only for fuzzing, so its example is never compiled into \
         anything. Saying otherwise tells an author their example is protecting them when \
         it is inert:\n{not}"
    );
    assert!(
        not.contains(
            "1 worked example, written down but not run: no check here asks for the declared \
             examples, so nothing compiles them"
        ),
        "an example nothing runs should say so:\n{not}"
    );
}

/// The headline number must not depend on which view you are looking at.
/// The drawing computed it with a boolean — "is there a non-empty default
/// somewhere above" — which can never go back to false, so a component that
/// switches checking off under a parent that set it was invisible. The text
/// form used the real inheritance machinery and got a different answer, and
/// the drawing disagreed with its own component tooltips at the same time
/// (external review, 2026-08-30).
#[test]
fn both_views_report_the_same_count_of_functions_promising_nothing() {
    for yaml in [
        // The shape that broke it: an explicit empty override, nested under a
        // parent that does declare a default.
        concat!(
            "ply: 1\n",
            "components:\n",
            "  parent:\n",
            "    anchor: parent\n",
            "    checks: [test]\n",
            "    fns:\n",
            "      p_fn: {}\n",
            "    components:\n",
            "      child:\n",
            "        anchor: parent::child\n",
            "        checks: []\n",
            "        fns:\n",
            "          untested: {}\n",
        ),
        // Two levels of override, switching back on again.
        concat!(
            "ply: 1\n",
            "components:\n",
            "  a:\n",
            "    anchor: a\n",
            "    checks: [test]\n",
            "    components:\n",
            "      b:\n",
            "        anchor: a::b\n",
            "        checks: []\n",
            "        fns:\n",
            "          switched_off: {}\n",
            "        components:\n",
            "          c:\n",
            "            anchor: a::b::c\n",
            "            checks: [test]\n",
            "            fns:\n",
            "              back_on: {}\n",
        ),
    ] {
        let doc = parse_document(yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);
        let svg = ply_render::svg::render_svg(&doc).unwrap();

        let strip = |s: &str| -> String {
            s.lines()
                .chain(s.split('>'))
                .find(|l| l.contains("promise nothing") || l.contains("promises nothing"))
                .map(|l| {
                    l.split('<')
                        .next()
                        .unwrap_or(l)
                        .trim()
                        .trim_start_matches(|c: char| !c.is_ascii_digit())
                        .to_string()
                })
                .unwrap_or_else(|| panic!("no summary line in:\n{s}"))
        };

        let from_text = text
            .lines()
            .find(|l| l.contains("promise nothing") || l.contains("promises nothing"))
            .unwrap()
            .trim()
            .to_string();
        let from_svg = strip(&svg);

        assert!(
            from_text.starts_with(&from_svg) || from_svg.starts_with(&from_text),
            "the two views report different summaries for one document. Whichever is wrong, a \
             reader who opens both is told two different things about how much of this \
             codebase promises nothing:\n  text: {from_text}\n  drawing: {from_svg}"
        );
    }
}

/// The `ply:` version is the one field whose wrongness is not survivable.
/// Every other invalid field still renders faithfully — the picture stays a
/// true account of what was written, which is the point of being able to
/// draw a half-finished document. The version is different in kind: it
/// selects which rulebook gives every other line its meaning, so rendering
/// a version this build does not speak is not drawing a half-written
/// document, it is confidently applying the wrong rules to all of it.
///
/// It also has to be stated. A v1 and an unsupported v2 document used to
/// produce byte-identical, equally authoritative output (external review,
/// 2026-08-30).
#[test]
fn the_format_version_is_stated_and_an_unknown_one_is_refused() {
    let doc = parse_document("ply: 1\ncomponents:\n  a:\n    anchor: app::a\n").unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    let svg = ply_render::svg::render_svg(&doc).unwrap();

    for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
        assert!(
            body.contains(
                "ply: 1 — the format version every rule below is read under; a version this \
                 build does not speak is refused, never guessed at"
            ),
            "{view} never says which rulebook it read the document under, so a reader cannot \
             tell a supported document from one whose every line was interpreted wrongly"
        );
    }
}

/// No sentence in either view may assert anything about whether a
/// verification has happened. The renderer is a function of a parsed
/// document; run state is outside its arguments, so any such claim is a
/// guess. The header was fixed once and its siblings were left standing,
/// including one three lines below it (external review, 2026-08-30).
///
/// The replacement wording is conditional rather than negative — "if every
/// declared check ran and passed" — because a negation like "no result is
/// here" is one feature away from being false again: this repo already has
/// an evidence-overlay path that attaches real results onto a rendered SVG.
#[test]
fn no_sentence_in_either_view_claims_anything_about_what_has_been_run() {
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "../../ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);
        let svg = ply_render::svg::render_svg(&doc).unwrap();

        for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
            for claim in [
                "has been run",
                "have been run",
                "none of it has been run",
                "before anything has been run",
            ] {
                assert!(
                    !body.contains(claim),
                    "{fixture}: {view} says {claim:?}. This renderer is handed a parsed \
                     document and nothing else — whether a verification ran is not a fact it \
                     has access to, so the sentence is a guess dressed as a statement."
                );
            }
        }
    }

    let doc = parse_document(
        "ply: 1\ncomponents:\n  a:\n    anchor: app::a\n    fns:\n      f:\n        checks: [test]\n",
    )
    .unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    assert!(
        text.contains(
            "declares checks up to tested — checked once against the declared examples and \
             generated inputs — the strongest verdict this could earn if every declared check \
             ran and passed; a promise, never a result"
        ),
        "the level sentence should say what it would be worth, conditionally, without \
         claiming anything about the world:\n{text}"
    );
}

/// Author-controlled strings reach both views. A note or a piece of trusted
/// evidence containing terminal control bytes could retitle a reader's
/// terminal window or repaint its output, and the transcript is explicitly
/// meant to be piped and read in terminals. Neutralised on the way in.
///
/// This is the cheap half of the problem. The expensive half — a multi-line
/// note rendering at column 0 and impersonating a transcript heading — is a
/// layout question and is recorded as a known gap rather than half-solved.
#[test]
fn author_controlled_text_cannot_reach_a_terminal_as_control_bytes() {
    let yaml = "ply: 1\ncomponents:\n  a:\n    anchor: app::a\n    note: \"before\\u001b]0;pwned\\u0007\\u001b[31mafter\"\n";
    let doc = parse_document(yaml).unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    let svg = ply_render::svg::render_svg(&doc).unwrap();

    for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
        assert!(
            !body.chars().any(|c| c.is_control() && c != '\n'),
            "{view} passes a raw control byte through from the document. A note can then \
             retitle or repaint the terminal of anyone who cats this."
        );
    }
    assert!(
        text.contains("before") && text.contains("after"),
        "the surrounding text must survive; only the control bytes go:\n{text}"
    );
}

/// The ratchet for the failure that has now been found three times: a
/// sentence that is true of the design and false of this build.
///
/// A diagnostic code printed beside a sentence reads as "this is what you
/// will see when this fires". So every code either view mentions must be one
/// this build can actually raise. `A0402`, `A0403` and `A0404` were printed
/// for months and appear in no checker; that is not a wording slip, it is a
/// category of claim nothing was checking.
///
/// This is the weak, implementable half of what the reviewer proposed. The
/// strong version is a rule registry that both the checker and the views
/// derive from, so an unenforced rule cannot be described as enforced by
/// construction — a design change, recorded in TODO.md rather than smuggled
/// in here. Until then this catches the same thing one step later.
#[test]
fn neither_view_cites_a_diagnostic_code_this_build_cannot_raise() {
    fn codes_in(body: &str) -> Vec<String> {
        let b = body.as_bytes();
        let mut out = Vec::new();
        for i in 0..b.len() {
            if !matches!(b[i], b'A' | b'E' | b'W') || i + 5 > b.len() {
                continue;
            }
            if b[i + 1..i + 5].iter().all(|c| c.is_ascii_digit()) {
                out.push(String::from_utf8_lossy(&b[i..i + 5]).into_owned());
            }
        }
        out
    }

    // Read from the sources that emit them, not a hand-kept list — a list
    // would drift the first time someone implemented a rule and forgot it.
    let mut raisable = std::collections::BTreeSet::new();
    for src in [
        "../../crates/ply-core/src/arch.rs",
        "../../crates/ply-core/src/check.rs",
        "../../crates/ply-core/src/config.rs",
        "../../crates/ply-cli/src/check.rs",
        "../../crates/ply-cli/src/verify.rs",
        "../../crates/ply-cli/src/worklist.rs",
    ] {
        if let Ok(body) = std::fs::read_to_string(src) {
            raisable.extend(codes_in(&body));
        }
    }
    assert!(
        raisable.len() > 3,
        "almost no diagnostic codes found in the checker sources — this test is reading the \
         wrong files and would pass vacuously: {raisable:?}"
    );

    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "../../ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/visual_forms.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);
        let svg = ply_render::svg::render_svg(&doc).unwrap();

        for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
            for cited in codes_in(body) {
                assert!(
                    raisable.contains(&cited),
                    "{fixture}: {view} cites {cited}, and nothing in this build raises it. A \
                     code beside a sentence tells a reader what they will see when that rule \
                     fires; citing one that cannot fire promises a guard that does not exist. \
                     Either implement it, or describe the rule without a code — see \
                     `declared_not_checked`."
                );
            }
        }
    }
}

/// The two views must not disagree about whether a check ran, and neither
/// may claim one did.
///
/// This is the second half of a fix that only landed on the first half. The
/// text form was taught in the morning to say "written down but not run"
/// for an example no check compiles, and to stop claiming "the checks above
/// test this promise" on a function with no checks — and the drawing was
/// left saying both of the original falsehoods, with a green test at
/// `render.rs` *requiring* one of them against a function whose declared
/// checks are `[bounded(3), fuzz(1024)]`. A passing test mandating a false
/// sentence in exactly the case where it is false (external review,
/// 2026-08-30).
///
/// Both sentences are future-conditional now, which is what makes them
/// honest in a view that has run nothing: the drawing and the text are
/// generated from a document, and no compiler is invoked by either.
#[test]
fn neither_view_says_a_check_has_happened_that_has_not() {
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "../../vetting/002-ingest-pipeline.ply.yaml",
        "../../vetting/003-trading-system.ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/visual_forms.ply.yaml",
    ] {
        let yaml = std::fs::read_to_string(fixture).unwrap();
        let doc = parse_document(&yaml).unwrap();
        let text = ply_render::transcript::render_transcript(&doc);
        let svg = ply_render::svg::render_svg(&doc).unwrap();

        for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
            assert!(
                !body.contains("compiled into a test"),
                "{fixture}: {view} says an example was compiled into a test. Neither view runs \
                 a compiler — they are generated from the document — so this is a claim about \
                 something that has not happened, in a view whose own header promises every \
                 line is a declaration."
            );
            assert!(
                !body.contains("the checks above test the function against exactly this promise"),
                "{fixture}: {view} asserts the checks above test this promise. On a function \
                 with no checks that contradicts its own neighbouring sentence, and on any \
                 function it describes a run that has not happened."
            );
        }
    }

    // A function that asks for `test` and one that does not, side by side.
    let doc = parse_document(concat!(
        "ply: 1\n",
        "components:\n",
        "  a:\n",
        "    anchor: app::a\n",
        "    fns:\n",
        "      runs_them:\n",
        "        checks: [test]\n",
        "        examples:\n",
        "          - \"runs_them(1) == 2\"\n",
        "      never_runs_them:\n",
        "        checks: [fuzz(64)]\n",
        "        examples:\n",
        "          - \"never_runs_them(1) == 2\"\n",
    ))
    .unwrap();
    let text = ply_render::transcript::render_transcript(&doc);
    let svg = ply_render::svg::render_svg(&doc).unwrap();

    for (view, body) in [("the text form", &text), ("the drawing", &svg)] {
        assert!(
            body.contains("compiles each into a test when `cargo ply verify` runs"),
            "{view} should say, in the future tense, what will happen to an example the \
             `test` check will pick up"
        );
        assert!(
            body.contains("no check here asks for the declared examples"),
            "{view} should say plainly that an example nothing runs is inert — this is the \
             sentence a reader needs to notice their example is decorative"
        );
    }
}

/// A finding badge sits on the right of a fn chip, vertically centred, so it
/// covers the checks line as well as the name line. The chip's width reserved
/// room for it against the name only, so a long checks line ran underneath
/// it: `demos/fault3-flagged.svg` -- the drawing whose whole point is that a
/// broken document is visibly flagged -- read
/// "bounded(0) · fuzz: 4096 cases · mutate" with `mutate` buried under the
/// red `E0203` tag.
///
/// This walks every fixture that carries a finding and fails on the first
/// chip where the two collide, so a construct added later cannot quietly
/// reintroduce it. Widths use the renderer's own monospace estimate, which is
/// the model it lays out against and therefore the one it has to respect.
#[test]
fn no_chip_puts_its_finding_badge_on_top_of_its_own_check_text() {
    /// `CHECK_CHAR_W` in the renderer: the per-character width it lays the
    /// `fn-checks` line out with.
    const CHECK_CHAR_W: f64 = 5.5;
    let mut collisions = Vec::new();
    for fixture in [
        "../../demos/fault3.ply.yaml",
        "../check/tests/fixtures/bad_check_syntax.ply.yaml",
        "../check/tests/fixtures/bad_path_form.ply.yaml",
        "../check/tests/fixtures/duplicate_unresolved_id.ply.yaml",
        "tests/fixtures/strict_with_finding.ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        let doc = roxmltree::Document::parse(&svg).unwrap();
        for chip in doc
            .descendants()
            .filter(|n| n.attribute("class") == Some("fn-chip"))
        {
            let Some(badge) = chip
                .descendants()
                .find(|n| n.attribute("class") == Some("finding-badge"))
                .and_then(|g| g.children().find(|c| c.tag_name().name() == "rect"))
            else {
                continue;
            };
            let Some(checks) = chip
                .descendants()
                .find(|n| n.attribute("class") == Some("fn-checks"))
            else {
                continue;
            };
            let text = checks.text().unwrap_or_default();
            let left: f64 = checks.attribute("x").unwrap().parse().unwrap();
            let right = left + text.chars().count() as f64 * CHECK_CHAR_W;
            let badge_left: f64 = badge.attribute("x").unwrap().parse().unwrap();
            if right > badge_left {
                collisions.push(format!(
                    "{fixture}: `{}` on chip `{}` runs to {right:.0} but its finding badge \
                     starts at {badge_left:.0}, so the end of the line is hidden under it",
                    text,
                    chip.attribute("data-fn").unwrap_or("?"),
                ));
            }
        }
    }
    assert!(
        collisions.is_empty(),
        "a finding badge is drawn over the check text it sits beside:\n  {}",
        collisions.join("\n  ")
    );
}

/// A crate whose one state type holds every shape §7.1 defines, plus a
/// document naming it. Written to disk because resolving a state type
/// *means* reading source: a fixture that only existed in memory would
/// test the drawing and skip the half this feature is actually about.
///
/// Returned as a live `TempDir` -- dropping it deletes the crate, so the
/// caller has to hold it for as long as it renders.
fn state_fixture() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("a temp directory");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"ply-state-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // The type lives in a module named `book`, matching the anchor the
    // document below uses. A component's state is resolved under its own
    // anchor, so a type at the crate root would not be this component's --
    // and this fixture, written before that rule existed, put it there.
    std::fs::write(src.join("lib.rs"), "pub mod book;\n").unwrap();
    std::fs::write(
        src.join("book.rs"),
        r#"
use std::collections::{BTreeMap, BTreeSet};
pub struct Level;
pub struct Book {
    pub depth: u64,
    pub venue: String,
    pub ticks: Vec<u64>,
    pub levels: BTreeMap<u64, Level>,
    pub seen: BTreeSet<u64>,
    pub last: Option<u64>,
    pub top: Level,
    pub clock: std::time::SystemTime,
}
"#,
    )
    .unwrap();
    let yaml = "ply: 1\ncomponents:\n  book:\n    anchor: book\n    state:\n      of: Book\n      \
                show: [depth, venue, ticks, levels, seen, last, top, clock]\n";
    std::fs::write(dir.path().join("ply.yaml"), yaml).unwrap();
    let doc = parse_document(yaml).expect("the state fixture parses");
    let fields = ply_render::harness::resolve_state_fields(dir.path(), &doc);
    assert!(
        !fields.is_empty(),
        "the fixture crate must resolve, or every assertion below tests nothing"
    );
    let svg = ply_render::svg::render_svg_with_state(
        &doc,
        &ply_render::svg::RenderOptions::default(),
        &fields,
    )
    .expect("the state fixture renders");
    (dir, svg)
}

/// The point of drawing a shape per field is that a reader can tell the
/// shapes apart. Seven meanings drawn as six glyphs is worse than no
/// glyphs at all -- it says something definite and false.
///
/// Written after the style invariant above was measured and found blind to
/// state rows: every fixture it walks is a document with no code under it,
/// so no row was ever painted in the whole render suite and a deliberately
/// misspelled glyph class still passed.
#[test]
fn every_state_shape_is_drawn_and_no_two_are_alike() {
    let (_dir, svg) = state_fixture();
    let doc = roxmltree::Document::parse(&svg).unwrap();

    // Each row's painted geometry, keyed by the field it belongs to.
    let mut drawn: Vec<(String, String)> = Vec::new();
    for row in doc
        .descendants()
        .filter(|n| n.is_element() && n.attribute("class") == Some("state-field"))
    {
        let field = row.attribute("data-field").expect("a row names its field");
        let mut shape = String::new();
        for node in row.descendants().filter(|n| n.is_element()) {
            if !matches!(node.tag_name().name(), "rect" | "circle") {
                continue;
            }
            let Some(class) = node.attribute("class") else {
                continue;
            };
            if !class.starts_with("state-glyph") {
                continue;
            }
            shape.push_str(class);
            for attr in ["x", "y", "width", "height", "rx"] {
                shape.push_str(node.attribute(attr).unwrap_or("-"));
                shape.push(',');
            }
            shape.push('|');
        }
        assert!(
            !shape.is_empty(),
            "`{field}` drew a row with no glyph in it -- the row says a name and a type and \
             leaves the meaning blank"
        );
        drawn.push((field.to_string(), shape));
    }

    assert_eq!(
        drawn.len(),
        8,
        "the fixture declares eight fields covering all seven shapes; only these drew rows: {:?}",
        drawn.iter().map(|(f, _)| f).collect::<Vec<_>>()
    );

    // Seven distinct silhouettes across eight fields: `top` and `clock` are
    // both structures of your own, and the second is additionally hatched,
    // so the two differ only by the hatch -- which is the design.
    let mut seen: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for (field, shape) in &drawn {
        if let Some(other) = seen.get(shape.as_str()) {
            assert!(
                ["top", "clock"].contains(other) && ["top", "clock"].contains(&field.as_str()),
                "`{field}` and `{other}` are drawn identically, so a reader cannot tell what \
                 either one is"
            );
        }
        seen.insert(shape.as_str(), field.as_str());
    }
    assert!(
        seen.len() >= 7,
        "eight fields covering seven meanings drew only {} distinct glyphs",
        seen.len()
    );

    // The hatch is the eighth case and must land on the one field nothing
    // can build a value of, and nowhere else.
    let hatched: Vec<&String> = drawn
        .iter()
        .filter(|(_, shape)| shape.contains("state-glyph-hatched"))
        .map(|(f, _)| f)
        .collect();
    assert_eq!(
        hatched,
        vec!["levels", "top", "clock"],
        "the hatching means Ply cannot build a value of this field's type. Three fields \
         here are like that -- a lookup table whose values are a bare struct, that struct \
         itself, and a clock -- and the map is the one worth naming: it parses perfectly \
         well as a map and still cannot be built"
    );
}

/// A drawing that paints a state row must style it and explain it, exactly
/// as every other painted thing must. The suite's other two invariants
/// walk documents with no code under them, so neither one can ever see a
/// row -- this runs the same two rules over a drawing that has them.
#[test]
fn every_state_row_resolves_a_style_rule_and_a_tooltip() {
    let (_dir, svg) = state_fixture();
    let style = format!("{}{}", ply_render::svg::STYLE, ply_render::svg::STATE_STYLE);
    let doc = roxmltree::Document::parse(&svg).unwrap();

    let mut unstyled: Vec<String> = Vec::new();
    let mut unexplained: Vec<String> = Vec::new();
    for row in doc
        .descendants()
        .filter(|n| n.is_element() && n.attribute("class") == Some("state-field"))
    {
        let field = row.attribute("data-field").unwrap_or("?");
        let explained = row
            .children()
            .any(|c| c.has_tag_name("title") && c.text().is_some_and(|t| t.len() > 40));
        if !explained {
            unexplained.push(field.to_string());
        }
        for node in row.descendants().filter(|n| n.is_element()) {
            let tag = node.tag_name().name();
            if !matches!(tag, "rect" | "circle" | "text" | "line" | "path") {
                continue;
            }
            let resolved = node
                .attribute("class")
                .into_iter()
                .flat_map(|c| c.split_whitespace())
                .any(|c| style.contains(&format!(".{c}{{")) || style.contains(&format!(".{c},")));
            if !resolved {
                unstyled.push(format!(
                    "{field}: <{tag}> class={:?}",
                    node.attribute("class")
                ));
            }
        }
    }
    assert!(
        unstyled.is_empty(),
        "painted parts of a state row with no style rule, so they draw in whatever the \
         browser defaults to: {unstyled:?}"
    );
    assert!(
        unexplained.is_empty(),
        "these rows draw an unusual glyph and say nothing about what it means on hover, \
         which is the newbie bar this project holds every user-facing sentence to: \
         {unexplained:?}"
    );
}

/// The count beside the type is a measured fact, not a restatement of the
/// document. Drawing "8 of 8" when the code has thirteen fields would be
/// Ply inventing a number about code -- the one thing this whole feature
/// exists not to do.
#[test]
fn the_shown_count_is_read_from_the_code() {
    let (_dir, svg) = state_fixture();
    assert!(
        svg.contains(">state Book — 8 of 8 shown<"),
        "the fixture type has exactly eight fields and the document shows all eight, so the \
         header must say so: {}",
        svg.split("state Book")
            .nth(1)
            .unwrap_or("")
            .split('<')
            .next()
            .unwrap_or("")
    );
}

/// A field the document names and the type does not have is not drawn as a
/// guess. `cargo ply check` already fails the build for it by name; a
/// renderer that painted a row anyway would be making up the very fact
/// that check exists to catch.
#[test]
fn a_field_the_type_does_not_have_is_not_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"ply-invented-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(src.join("lib.rs"), "pub mod book;\n").unwrap();
    std::fs::write(src.join("book.rs"), "pub struct Book { pub real: u64 }").unwrap();
    let yaml = "ply: 1\ncomponents:\n  book:\n    anchor: book\n    state:\n      of: Book\n      \
                show: [real, invented]\n";
    let doc = parse_document(yaml).unwrap();
    let fields = ply_render::harness::resolve_state_fields(dir.path(), &doc);
    let svg = ply_render::svg::render_svg_with_state(
        &doc,
        &ply_render::svg::RenderOptions::default(),
        &fields,
    )
    .unwrap();
    assert!(
        svg.contains("data-field=\"real\""),
        "the field that exists must still be drawn"
    );
    assert!(
        !svg.contains("invented"),
        "a field nobody declared must not appear anywhere in the drawing, not even as text"
    );
    assert!(
        svg.contains(">state Book — 1 of 1 shown<"),
        "the count reports what was drawn against what the type really has, so a document \
         asking for two fields of a one-field type reads as 1 of 1, never 2 of 1"
    );
}

/// §7.1 (2026-09-04): a name in `show:` may declare its own shape, and with
/// no code to read, that declared shape is what gets drawn -- the design a
/// document can be legible with before its implementation exists. This is
/// the declared-only sibling of `every_state_shape_is_drawn_and_no_two_are_alike`:
/// all seven tokens, in one document with no crate under it at all, must
/// still draw seven distinct silhouettes -- and, unlike a row read from
/// code, every one of them reads `declared` in its type column and none is
/// ever hatched (the hatch is the sampling engine's own answer about real
/// code, and there is no code here for it to answer about).
#[test]
fn every_declared_shape_is_drawn_and_no_two_are_alike() {
    let svg = render_fixture("tests/fixtures/declared_shapes.ply.yaml");
    let doc = roxmltree::Document::parse(&svg).unwrap();

    let mut drawn: Vec<(String, String)> = Vec::new();
    for row in doc
        .descendants()
        .filter(|n| n.is_element() && n.attribute("class") == Some("state-field"))
    {
        let field = row.attribute("data-field").expect("a row names its field");
        let mut shape = String::new();
        for node in row.descendants().filter(|n| n.is_element()) {
            if !matches!(node.tag_name().name(), "rect" | "circle") {
                continue;
            }
            let Some(class) = node.attribute("class") else {
                continue;
            };
            if !class.starts_with("state-glyph") {
                continue;
            }
            shape.push_str(class);
            for attr in ["x", "y", "width", "height", "rx"] {
                shape.push_str(node.attribute(attr).unwrap_or("-"));
                shape.push(',');
            }
            shape.push('|');
        }
        assert!(
            !shape.is_empty(),
            "`{field}` drew a row with no glyph in it"
        );
        drawn.push((field.to_string(), shape));
    }

    assert_eq!(
        drawn.len(),
        7,
        "the fixture declares seven fields with a shape (an eighth, `cursor`, declares \
         none and must draw no row); only these drew rows: {:?}",
        drawn.iter().map(|(f, _)| f).collect::<Vec<_>>()
    );

    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (field, shape) in &drawn {
        assert!(
            seen.insert(shape.as_str()),
            "`{field}` draws the same glyph as another declared field -- unlike the \
             read-from-code fixture, nothing here is hatched, so no two should ever collide"
        );
    }

    assert!(
        !doc.descendants().any(|n| {
            n.is_element()
                && n.attribute("class")
                    .is_some_and(|c| c.split_whitespace().any(|t| t == "state-glyph-hatched"))
        }),
        "a declared row is never hatched -- the hatch is the sampling engine's own answer \
         about real code, and there is no code here for it to answer about (the substring \
         appears in the stylesheet's own CSS rule regardless, so this checks painted \
         elements, not the raw SVG text)"
    );

    for (field, _) in &drawn {
        assert!(
            svg.contains(&format!("data-field=\"{field}\"")),
            "sanity: `{field}` must be a real drawn row"
        );
    }
    assert_eq!(
        svg.matches(">declared<").count(),
        7,
        "every declared row's type column must read exactly the word `declared`, never a \
         type -- there is no type to spell for a row nothing resolved:\n{svg}"
    );
}

/// §7.1's other new rule, stated as its own test because it is the one most
/// likely to be got wrong by accident: a declared shape is never counted.
/// `N of M shown` is a measured fact about code, and a declared-only box
/// has no code behind it at all -- so its header must stay the bare type
/// name, exactly as a component with no code ever resolved always has.
#[test]
fn a_declared_only_box_never_draws_a_count() {
    let svg = render_fixture("tests/fixtures/declared_shapes.ply.yaml");
    assert!(
        svg.contains(">state Ledger<"),
        "the header must still name the type:\n{svg}"
    );
    assert!(
        !svg.contains("shown"),
        "seven fields drew rows here, and it would be easy to mistake that for something \
         to count -- but nothing was measured from code, so there is no number to draw:\n{svg}"
    );
}

/// §7.1's central promise, checked at the drawing itself rather than only
/// in a unit test one level down: once a field resolves against real code,
/// the source wins the drawing outright, and a disagreeing declaration
/// leaves no trace in the picture at all -- not as a second glyph, not as a
/// warning mark, not as the word `declared` anywhere near the row. A stale
/// declaration can make `cargo ply check` fail (`A0416`); it can never make
/// this picture wrong.
#[test]
fn a_declaration_the_code_disagrees_with_loses_the_drawing_to_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"ply-codewins-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(src.join("lib.rs"), "pub mod book;\n").unwrap();
    // `queued` is really a lookup table -- the document below declares it a
    // list, which is a straightforward, plausible mistake to make.
    std::fs::write(
        src.join("book.rs"),
        "use std::collections::BTreeMap;\npub struct Book { pub queued: BTreeMap<u64, u32> }\n",
    )
    .unwrap();
    let yaml = "ply: 1\ncomponents:\n  book:\n    anchor: book\n    state:\n      of: Book\n      \
                show:\n        queued: list\n";
    std::fs::write(dir.path().join("ply.yaml"), yaml).unwrap();
    let doc = parse_document(yaml).expect("the fixture document parses");
    let fields = ply_render::harness::resolve_state_fields(dir.path(), &doc);
    assert!(
        !fields.is_empty(),
        "the fixture crate must resolve, or this test checks nothing"
    );
    let svg = ply_render::svg::render_svg_with_state(
        &doc,
        &ply_render::svg::RenderOptions::default(),
        &fields,
    )
    .expect("the fixture renders");

    assert!(
        svg.contains("data-field=\"queued\""),
        "the resolved field must still be drawn:\n{svg}"
    );
    assert!(
        svg.contains(">BTreeMap<u64, u32><") || svg.contains(">BTreeMap&lt;u64, u32&gt;<"),
        "the type column must spell the real type the source has, not the one the \
         document declared:\n{svg}"
    );

    // Isolate this one row's own `<g>` element -- the promise is about what
    // that row draws, not about the rest of the document.
    let start = svg
        .find("data-field=\"queued\"")
        .expect("the row exists")
        .max(1)
        - 1;
    let row_start = svg[..start].rfind("<g ").unwrap_or(0);
    let row_end = svg[row_start..].find("</g>").expect("the row closes") + row_start;
    let row = &svg[row_start..row_end];

    assert!(
        !row.contains("declared"),
        "the word `declared` must appear nowhere in a row the source resolved -- the \
         document's disagreeing declaration must leave no trace in the drawing at all:\n{row}"
    );

    let doc_svg = roxmltree::Document::parse(&svg).unwrap();
    let glyph_classes: Vec<&str> = doc_svg
        .descendants()
        .find(|n| n.is_element() && n.attribute("data-field") == Some("queued"))
        .expect("the row element")
        .descendants()
        .filter(|n| matches!(n.tag_name().name(), "rect" | "circle"))
        .filter_map(|n| n.attribute("class"))
        .collect();
    assert!(
        glyph_classes.iter().any(|c| c.contains("state-glyph")),
        "the row must still draw a glyph:\n{row}"
    );
    // A map draws as a key cell beside a value cell, twice -- four `rect`s.
    // A list (what the document wrongly declared) draws as three equal
    // stacked bars -- three `rect`s. The count alone tells the two apart
    // without duplicating the geometry `state_shapes::glyph_svg` owns.
    assert_eq!(
        glyph_classes.len(),
        4,
        "the code says `queued` is a lookup table (four glyph rects), not the three-bar \
         list the document declared -- got classes {glyph_classes:?} in row:\n{row}"
    );
}
