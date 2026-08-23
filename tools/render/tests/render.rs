use ply_render::model::parse_document;
use ply_render::svg::render_svg;

fn render_fixture(path: &str) -> String {
    let yaml = std::fs::read_to_string(path).unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    render_svg(&doc).expect("fixture should render")
}

/// Parses this renderer's own `d="M x y L x2 y2"` path format (used by every
/// straight edge/deny line) back into its two endpoints.
fn parse_line_path(d: &str) -> ((f64, f64), (f64, f64)) {
    let nums: Vec<f64> = d
        .split_whitespace()
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    assert_eq!(
        nums.len(),
        4,
        "expected an M..L.. path with 4 numbers, got {d:?}"
    );
    ((nums[0], nums[1]), (nums[2], nums[3]))
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

    let rect = node
        .children()
        .find(|c| c.tag_name().name() == "rect" && c.attribute("class") == Some("component-box"))
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

    #[test]
    fn everything_renders_inside_the_canvas() {
        for fixture in [
            "../../vetting/002-ingest-pipeline.ply.yaml",
            "../../vetting/001-spsc-disruptor.ply.yaml",
            "tests/fixtures/full.ply.yaml",
            "tests/fixtures/qualified_refs.ply.yaml",
        ] {
            let svg = render_fixture(fixture);
            let doc = roxmltree::Document::parse(&svg).unwrap();
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
                        let (a, b) = parse_line_path(d);
                        vec![a, b]
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
    assert!(svg.contains("class=\"component-box\""));
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
    let msg = err.to_string();
    assert!(
        msg.contains("shared"),
        "error should name the ambiguous token: {msg}"
    );
    assert!(
        msg.contains("alpha.shared"),
        "error should list this candidate: {msg}"
    );
    assert!(
        msg.contains("beta.shared"),
        "error should list this candidate: {msg}"
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
        // §7.1 finding coverage: these fixtures each carry a real finding
        // that must resolve through `FINDING_STYLE`, not just parse clean.
        "../check/tests/fixtures/bad_check_syntax.ply.yaml",
        "../check/tests/fixtures/bad_path_form.ply.yaml",
        "../check/tests/fixtures/duplicate_unresolved_id.ply.yaml",
        "../../demos/fault3.ply.yaml",
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
            let resolved = node
                .ancestors()
                .filter_map(|a| a.attribute("class"))
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

    // The component tooltip expands its profile — the tag alone shows only a name.
    let ring = titles
        .iter()
        .find(|t| t.starts_with("component ring"))
        .unwrap();
    assert!(ring.contains("profile hot_path = no_panics, exhaustive_match"));
    assert!(ring.contains("capabilities: unsafe"));
    assert!(ring.contains("owns disruptor::spsc::Spsc — only this component may mutate them"));

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

/// "Tooltips for all items": the invariant, not a spot-check. Every drawn item
/// — component, fn chip, badge, tag, shield, pin, arrow, deny bar, wildcard
/// node — must resolve a `<title>` on itself or an ancestor, so nothing in the
/// picture is unexplainable by hovering it.
#[test]
fn every_drawn_item_resolves_a_tooltip() {
    // Item-bearing groups: a class here means "this is a thing a reader can
    // point at", so it must be explained.
    const ITEM_CLASSES: &[&str] = &[
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
    ];

    let mut untitled: Vec<String> = Vec::new();
    for fixture in [
        "../../vetting/001-spsc-disruptor.ply.yaml",
        "tests/fixtures/full.ply.yaml",
        "tests/fixtures/qualified_refs.ply.yaml",
    ] {
        let svg = render_fixture(fixture);
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let mut seen: Vec<&str> = Vec::new();
        for node in doc.descendants().filter(|n| n.is_element()) {
            let Some(class) = node.attribute("class") else {
                continue;
            };
            if !ITEM_CLASSES.contains(&class) {
                continue;
            }
            seen.push(class);
            let titled = node
                .ancestors()
                .any(|a| a.children().any(|c| c.tag_name().name() == "title"));
            if !titled {
                untitled.push(format!("{fixture}: .{class}"));
            }
        }
        assert!(!seen.is_empty(), "{fixture}: no item classes found at all");
    }
    untitled.sort();
    untitled.dedup();
    assert!(
        untitled.is_empty(),
        "drawn items with no tooltip: {untitled:?}"
    );
}
