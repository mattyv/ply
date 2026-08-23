use ply_render::model::parse_document;
use ply_render::svg::render_svg;

fn render_fixture(path: &str) -> String {
    let yaml = std::fs::read_to_string(path).unwrap();
    let doc = parse_document(&yaml).expect("fixture should parse");
    render_svg(&doc).expect("fixture should render")
}

#[test]
fn svg_root_element_is_well_formed_enough_to_open() {
    let svg = render_fixture("tests/fixtures/full.ply.yaml");
    assert!(svg.starts_with("<svg"));
    assert!(svg.trim_end().ends_with("</svg>"));
    assert_eq!(svg.matches("<svg").count(), 1);
    // every opened group is closed
    assert_eq!(svg.matches("<g ").count() + svg.matches("<g>").count(), svg.matches("</g>").count());

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
    assert!(msg.contains("shared"), "error should name the ambiguous token: {msg}");
    assert!(msg.contains("alpha.shared"), "error should list this candidate: {msg}");
    assert!(msg.contains("beta.shared"), "error should list this candidate: {msg}");
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
    let style = ply_render::svg::STYLE;
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
                unstyled.push(format!("{fixture}: <{tag}> class={:?}", node.attribute("class")));
            }
        }
    }
    assert!(unstyled.is_empty(), "painted elements with no style rule: {unstyled:?}");
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
    let push = titles.iter().find(|t| t.starts_with("Spsc::try_push")).unwrap();
    assert!(push.contains("bounded(3) — model-checked exhaustively to depth 3"));
    assert!(push.contains("fuzz(1024) — 1024 randomised property-test cases"));
    assert!(push.contains("checked at instantiation T=u64"));
    assert!(push.contains("trusted (not machine-checked): SPSC cross-thread safety"));
    assert!(push.contains("loom test tests/loom_spsc.rs"));
    assert!(push.contains("1 example(s)"));

    // The component tooltip expands its profile — the tag alone shows only a name.
    let ring = titles.iter().find(|t| t.starts_with("component ring")).unwrap();
    assert!(ring.contains("profile hot_path = no_panics, exhaustive_match"));
    assert!(ring.contains("capabilities: unsafe"));
    assert!(ring.contains("owns (sole mutator of): disruptor::spsc::Spsc"));

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
            let Some(class) = node.attribute("class") else { continue };
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
    assert!(untitled.is_empty(), "drawn items with no tooltip: {untitled:?}");
}
