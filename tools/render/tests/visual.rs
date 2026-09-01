use std::collections::BTreeMap;

use ply_core::diag::is_absence;
use ply_core::model::parse_document;
use ply_core::visual::{ElementEvidence, SourceLocation, VisualDiagnostic, VisualElement};
use ply_render::svg::{
    RenderOptions, render_svg, render_svg_with_evidence, render_svg_with_evidence_and_options,
};

fn element(
    id: &str,
    kind: &str,
    label: &str,
    parent_id: Option<&str>,
    verdict: &str,
) -> VisualElement {
    VisualElement {
        id: id.into(),
        kind: kind.into(),
        label: label.into(),
        parent_id: parent_id.map(str::to_owned),
        declaration: None,
        evidence: ElementEvidence {
            verdict: verdict.into(),
            statuses: vec![],
            reused: false,
            engine: None,
            seed: None,
            cases: None,
        },
        source: None,
        diagnostic_ids: vec![],
    }
}

fn has_semantic_parent(node: roxmltree::Node<'_, '_>, expected: &str) -> bool {
    node.ancestors()
        .find(|ancestor| {
            ancestor.tag_name().name() == "g" && ancestor.attribute("class") == Some("component")
        })
        .and_then(|component| component.attribute("data-name"))
        == Some(expected)
}

#[test]
fn completed_evidence_is_attached_to_actual_workspace_component_and_function_groups() {
    let doc = parse_document(
        "ply: 1\ncomponents:\n  alpha:\n    anchor: app::alpha\n    fns:\n      run: {}\n  beta:\n    anchor: app::beta\n    fns:\n      run: {}\n",
    )
    .unwrap();
    let mut elements = BTreeMap::from([
        (
            "workspace-id".into(),
            element("workspace-id", "workspace", "workspace", None, "violation"),
        ),
        (
            "alpha-id".into(),
            element(
                "alpha-id",
                "component",
                "alpha",
                Some("workspace-id"),
                "bounded(2)",
            ),
        ),
        (
            "beta-id".into(),
            element(
                "beta-id",
                "component",
                "beta",
                Some("workspace-id"),
                "violation",
            ),
        ),
        (
            "alpha-run-id".into(),
            element("alpha-run-id", "fn", "run", Some("alpha-id"), "bounded(4)"),
        ),
        (
            "beta-run-id".into(),
            element("beta-run-id", "fn", "run", Some("beta-id"), "violation"),
        ),
    ]);
    let hostile = r#"src/<hostile>&".rs"#;
    let alpha = elements.get_mut("alpha-run-id").unwrap();
    alpha.evidence.statuses = vec!["conditional".into(), "partial-history".into()];
    alpha.evidence.reused = true;
    alpha.evidence.engine = Some("kani<&>".into());
    alpha.evidence.seed = Some("seed<&>".into());
    alpha.evidence.cases = Some(42);
    alpha.source = Some(SourceLocation::point(hostile, 12, 7));
    alpha.diagnostic_ids = vec!["diag-alpha".into()];
    let diagnostics = vec![VisualDiagnostic {
        id: "diag-alpha".into(),
        code: "E<script>".into(),
        severity: "error".into(),
        message: "bad </title><script>alert(1)</script> & worse".into(),
        element_id: Some("alpha-run-id".into()),
        source: Some(SourceLocation::point(hostile, 12, 7)),
    }];

    let svg = render_svg_with_evidence(&doc, &elements, &diagnostics).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();

    let workspace = xml
        .descendants()
        .find(|node| node.attribute("data-element-id") == Some("workspace-id"))
        .expect("workspace frame must have an interactive group");
    assert!(workspace.descendants().any(|node| {
        node.tag_name().name() == "rect" && node.attribute("class") == Some("workspace-frame")
    }));

    for (name, id) in [("alpha", "alpha-id"), ("beta", "beta-id")] {
        let component = xml
            .descendants()
            .find(|node| {
                node.tag_name().name() == "g"
                    && node.attribute("class") == Some("component")
                    && node.attribute("data-name") == Some(name)
            })
            .unwrap();
        assert_eq!(component.attribute("data-element-id"), Some(id));
    }

    let functions = xml
        .descendants()
        .filter(|node| {
            node.tag_name().name() == "g"
                && node.attribute("class") == Some("fn-chip")
                && node.attribute("data-fn") == Some("run")
        })
        .collect::<Vec<_>>();
    let alpha_run = functions
        .iter()
        .find(|node| has_semantic_parent(**node, "alpha"))
        .unwrap();
    let beta_run = functions
        .iter()
        .find(|node| has_semantic_parent(**node, "beta"))
        .unwrap();
    assert_eq!(alpha_run.attribute("data-element-id"), Some("alpha-run-id"));
    assert_eq!(beta_run.attribute("data-element-id"), Some("beta-run-id"));

    let alpha_tip = alpha_run
        .children()
        .find(|node| node.tag_name().name() == "title")
        .unwrap()
        .text()
        .unwrap();
    assert!(
        alpha_tip.starts_with("run\n"),
        "declared tooltip was lost: {alpha_tip}"
    );
    assert!(alpha_tip.contains("verdict: bounded(4)"));
    assert!(alpha_tip.contains("statuses: conditional, partial-history"));
    assert!(alpha_tip.contains("reused: yes"));
    assert!(alpha_tip.contains("engine: kani<&>"));
    assert!(alpha_tip.contains("seed: seed<&>"));
    assert!(alpha_tip.contains("cases: 42"));
    assert!(alpha_tip.contains(&format!("source: {hostile}:12:7-12:7")));
    assert!(alpha_tip.contains("error E<script>: bad </title><script>alert(1)</script> & worse"));

    let beta_tip = beta_run
        .children()
        .find(|node| node.tag_name().name() == "title")
        .unwrap()
        .text()
        .unwrap();
    assert!(beta_tip.contains("verdict: violation"));
    assert!(!beta_tip.contains("bounded(4)"));
    assert!(!beta_tip.contains("E<script>"));

    assert!(!svg.contains("ply-evidence-overlay"));
    assert!(!svg.contains("<script"));
    assert!(!svg.contains("</title><script"));
    assert!(svg.contains("&lt;script&gt;"));
    assert!(svg.contains("src/&lt;hostile&gt;&amp;&quot;.rs"));
}

/// The old renderer found its shapes by re-parsing its own rendered SVG:
/// walking `<g>` tags, reading their `data-name`/`data-fn` attributes back
/// as text, and joining nested ones into a dotted path with `.` — then
/// matched that path against `element.label`, which for every element (see
/// `collect_elements` in `visual/mod.rs`) is always the bare local name, not
/// the dotted path. A top-level component's bare name and its dotted path
/// are the same string, so the mismatch never showed up in a document with
/// no nesting — every existing fixture here is exactly that shape. Nest one
/// component inside another, give the outer one a name that needs XML
/// escaping, and put a same-named inner component under two different
/// outers: every one of those four nested elements is a shape the old
/// matcher genuinely drew but could never find a label match for, so it
/// silently left them all unattached.
#[test]
fn nested_components_with_a_shared_local_name_and_an_escaped_label_all_attach() {
    let doc = parse_document(
        r#"ply: 1
components:
  "Q&A":
    anchor: app::qa
    components:
      inner:
        anchor: app::qa::inner
        fns:
          run: {}
  beta:
    anchor: app::beta
    components:
      inner:
        anchor: app::beta::inner
        fns:
          run: {}
"#,
    )
    .unwrap();
    let elements = BTreeMap::from([
        (
            "workspace-id".into(),
            element("workspace-id", "workspace", "workspace", None, "bounded(2)"),
        ),
        (
            "qa-id".into(),
            element(
                "qa-id",
                "component",
                "Q&A",
                Some("workspace-id"),
                "bounded(2)",
            ),
        ),
        (
            "qa-inner-id".into(),
            element(
                "qa-inner-id",
                "component",
                "inner",
                Some("qa-id"),
                "bounded(2)",
            ),
        ),
        (
            "qa-inner-run-id".into(),
            element(
                "qa-inner-run-id",
                "fn",
                "run",
                Some("qa-inner-id"),
                "bounded(4)",
            ),
        ),
        (
            "beta-id".into(),
            element(
                "beta-id",
                "component",
                "beta",
                Some("workspace-id"),
                "bounded(2)",
            ),
        ),
        (
            "beta-inner-id".into(),
            element(
                "beta-inner-id",
                "component",
                "inner",
                Some("beta-id"),
                "bounded(2)",
            ),
        ),
        (
            "beta-inner-run-id".into(),
            element(
                "beta-inner-run-id",
                "fn",
                "run",
                Some("beta-inner-id"),
                "violation",
            ),
        ),
    ]);

    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();

    // Every element drawn corresponds to a real shape in this document, so
    // every one of them must carry its own id — the invariant, not a
    // spot-check of the two the old code happened to get right.
    for id in elements.keys() {
        assert!(
            svg.contains(&format!("data-element-id=\"{id}\"")),
            "{id} never attached a data-element-id: {svg}"
        );
    }
}

#[test]
fn unmatched_or_ambiguous_elements_are_not_attached_to_another_shape() {
    let doc = parse_document(
        "ply: 1\ncomponents:\n  alpha:\n    anchor: app::alpha\n    fns:\n      run: {}\n",
    )
    .unwrap();
    let elements = BTreeMap::from([
        (
            "workspace-id".into(),
            element("workspace-id", "workspace", "workspace", None, "clean"),
        ),
        (
            "alpha-one".into(),
            element(
                "alpha-one",
                "component",
                "alpha",
                Some("workspace-id"),
                "clean",
            ),
        ),
        (
            "alpha-two".into(),
            element(
                "alpha-two",
                "component",
                "alpha",
                Some("workspace-id"),
                "violation",
            ),
        ),
        (
            "unknown".into(),
            element("unknown", "fn", "missing", Some("alpha-one"), "violation"),
        ),
    ]);

    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    assert!(!svg.contains("data-element-id=\"alpha-one\""));
    assert!(!svg.contains("data-element-id=\"alpha-two\""));
    assert!(!svg.contains("data-element-id=\"unknown\""));
}

#[test]
fn no_evidence_preserves_the_static_renderer_byte_for_byte() {
    let doc = parse_document("ply: 1\ncomponents: {}\n").unwrap();
    assert_eq!(
        render_svg_with_evidence(&doc, &BTreeMap::new(), &[]).unwrap(),
        render_svg(&doc).unwrap()
    );
}

// ---- The five display states (The-Ply-Spec.md's state model) -------------

/// One component `svc` (anchor `app::svc`) with one fn per name given.
fn single_component_doc(fn_names: &[&str]) -> ply_core::model::Document {
    let fns = fn_names
        .iter()
        .map(|n| format!("      {n}: {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    parse_document(&format!(
        "ply: 1\ncomponents:\n  svc:\n    anchor: app::svc\n    fns:\n{fns}\n"
    ))
    .unwrap()
}

/// A workspace + `svc` component + one `fn` element per `(name, verdict,
/// statuses)` triple, all correctly parented so every fn actually resolves
/// against `single_component_doc`'s `svc`.
fn evidence_for(fn_states: &[(&str, &str, &[&str])]) -> BTreeMap<String, VisualElement> {
    let mut elements = BTreeMap::from([
        (
            "workspace-id".to_string(),
            element("workspace-id", "workspace", "workspace", None, "clean"),
        ),
        (
            "svc-id".to_string(),
            element("svc-id", "component", "svc", Some("workspace-id"), "clean"),
        ),
    ]);
    for (name, verdict, statuses) in fn_states {
        let id = format!("{name}-id");
        let mut el = element(&id, "fn", name, Some("svc-id"), verdict);
        el.evidence.statuses = statuses.iter().map(|s| (*s).to_string()).collect();
        elements.insert(id, el);
    }
    elements
}

/// The class attribute of the `<rect>` a given fn's chip draws its state
/// with — the first `rect` child of that fn's `<g class="fn-chip">`.
fn fn_chip_rect_class(doc: &roxmltree::Document, fn_name: &str) -> String {
    doc.descendants()
        .find(|n| {
            n.tag_name().name() == "g"
                && n.attribute("class") == Some("fn-chip")
                && n.attribute("data-fn") == Some(fn_name)
        })
        .and_then(|g| g.children().find(|c| c.tag_name().name() == "rect"))
        .and_then(|r| r.attribute("class"))
        .unwrap_or_else(|| panic!("fn {fn_name:?} drew no chip"))
        .to_string()
}

/// The body of one CSS rule (`selector{body}`) out of a rendered SVG's
/// embedded `<style>`, e.g. `style_rule(&svg, ".fn-chip-box-stale")`.
fn style_rule(svg: &str, selector: &str) -> String {
    let style = svg
        .split("<style>")
        .nth(1)
        .and_then(|s| s.split("</style>").next())
        .expect("every render embeds a stylesheet");
    style
        .split('}')
        .filter_map(|r| r.split_once('{'))
        .find(|(sel, _)| sel.trim() == selector)
        .map(|(_, body)| body.to_owned())
        .unwrap_or_else(|| panic!("no style rule for {selector}"))
}

/// The text content of the `.fn-evidence-mark` this fn's chip drew, if any.
fn fn_chip_mark_text(doc: &roxmltree::Document, fn_name: &str) -> Option<String> {
    doc.descendants()
        .find(|n| {
            n.tag_name().name() == "g"
                && n.attribute("class") == Some("fn-chip")
                && n.attribute("data-fn") == Some(fn_name)
        })
        .and_then(|g| {
            g.children().find(|c| {
                c.attribute("class")
                    .is_some_and(|cl| cl.split(' ').any(|p| p == "fn-evidence-mark"))
            })
        })
        .and_then(|t| t.text())
        .map(str::to_owned)
}

#[test]
fn earned_fn_chip_is_backed_by_a_stored_result() {
    let doc = single_component_doc(&["run"]);
    let elements = evidence_for(&[("run", "tested", &[])]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();

    assert!(
        fn_chip_rect_class(&xml, "run")
            .split(' ')
            .any(|c| c == "fn-chip-box-earned"),
        "a fn with a stored `tested` verdict must draw as earned"
    );
    assert_eq!(
        fn_chip_mark_text(&xml, "run").as_deref(),
        Some("\u{2713}"),
        "earned must carry a non-colour mark too, for a reader who cannot see the fill colour"
    );
}

#[test]
fn violated_fn_chip_is_marked_broken_and_uses_the_forbidden_red_family() {
    let doc = single_component_doc(&["run"]);
    let elements = evidence_for(&[("run", "violation", &[])]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();

    assert!(
        fn_chip_rect_class(&xml, "run")
            .split(' ')
            .any(|c| c == "fn-chip-box-violated"),
        "a fn with a stored `violation` verdict must draw as violated"
    );
    assert_eq!(fn_chip_mark_text(&xml, "run").as_deref(), Some("\u{2717}"));
    assert!(
        svg.contains("fill:#c9534f") || svg.contains("stroke:#c9534f"),
        "violated is one of the two meanings allowed the forbidden-red family"
    );
}

#[test]
fn unanswered_fn_chip_is_distinct_and_never_red() {
    let doc = single_component_doc(&["run"]);
    let elements = evidence_for(&[("run", "timeout", &[])]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();

    let class = fn_chip_rect_class(&xml, "run");
    assert!(class.split(' ').any(|c| c == "fn-chip-box-unanswered"));
    assert_eq!(fn_chip_mark_text(&xml, "run").as_deref(), Some("?"));

    let rule = style_rule(&svg, ".fn-chip-box-unanswered");
    for red in ["#c9534f", "#8f2f2c", "#fdecec"] {
        assert!(
            !rule.contains(red),
            "unanswered must never be red — a run that could not decide is not the same as a \
             broken promise or a forbidden call"
        );
    }
}

#[test]
fn a_stale_status_overrides_the_verdict_it_sits_beside_and_is_never_red() {
    let doc = single_component_doc(&["run"]);
    // A `tested` verdict would ordinarily be `Earned` -- `stale` must win.
    let elements = evidence_for(&[("run", "tested", &["stale"])]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();

    let class = fn_chip_rect_class(&xml, "run");
    assert!(
        class.split(' ').any(|c| c == "fn-chip-box-stale"),
        "a `stale` status must draw as stale even though the verdict beside it is `tested`, \
         got class {class:?}"
    );
    assert!(!class.split(' ').any(|c| c == "fn-chip-box-earned"));
    assert_eq!(fn_chip_mark_text(&xml, "run").as_deref(), Some("\u{21bb}"));
    let rule = style_rule(&svg, ".fn-chip-box-stale");
    for red in ["#c9534f", "#8f2f2c", "#fdecec"] {
        assert!(
            !rule.contains(red),
            "stale must never be drawn in the forbidden-red family"
        );
    }
}

#[test]
fn earned_on_assumptions_keeps_the_earned_colour_and_only_adds_a_mark() {
    let doc = single_component_doc(&["plain", "conditional"]);
    let elements = evidence_for(&[
        ("plain", "tested", &[]),
        ("conditional", "bounded(2)", &["conditional"]),
    ]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();

    // The-Ply-Spec.md: "a marked variant of earned, not a sixth state --
    // show the string attached, do not invent a colour." Same class as
    // plain earned, on purpose.
    assert_eq!(
        fn_chip_rect_class(&xml, "plain"),
        fn_chip_rect_class(&xml, "conditional"),
        "earned-on-assumptions must not invent a new colour of its own"
    );
    let plain_mark = fn_chip_mark_text(&xml, "plain").unwrap();
    let conditional_mark = fn_chip_mark_text(&xml, "conditional").unwrap();
    assert_ne!(
        plain_mark, conditional_mark,
        "the assumption debt must still be visible as an attached mark"
    );
    assert!(conditional_mark.starts_with(&plain_mark));
}

/// The load-bearing invariant: walk every chip the renderer actually drew,
/// and for every one painted as earned, demand a stored result behind it —
/// an element in the evidence input whose own verdict is a real evidence
/// verdict, not an absence and not a violation. `is_absence` is reused
/// directly from `ply_core::diag` rather than re-derived, so this checks
/// against the project's own canonical vocabulary, not a second copy of it.
#[test]
fn no_chip_renders_as_earned_without_a_stored_result_behind_it() {
    let doc = single_component_doc(&["a", "b", "c", "d"]);
    let elements = evidence_for(&[
        ("a", "tested", &[]),
        ("b", "violation", &[]),
        ("c", "timeout", &[]),
        ("d", "unclaimed", &[]),
    ]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();

    let mut checked = 0;
    for g in xml
        .descendants()
        .filter(|n| n.tag_name().name() == "g" && n.attribute("class") == Some("fn-chip"))
    {
        let rect = g
            .children()
            .find(|c| c.tag_name().name() == "rect")
            .unwrap();
        let is_earned = rect
            .attribute("class")
            .unwrap()
            .split(' ')
            .any(|c| c == "fn-chip-box-earned");
        if !is_earned {
            continue;
        }
        checked += 1;
        let element_id = g.attribute("data-element-id").unwrap_or_else(|| {
            panic!(
                "earned chip {:?} carries no element id",
                g.attribute("data-fn")
            )
        });
        let backing = elements.get(element_id).unwrap_or_else(|| {
            panic!("earned chip's element id {element_id:?} is not in the evidence input")
        });
        assert!(
            backing.evidence.verdict != "violation" && !is_absence(&backing.evidence.verdict),
            "chip drawn as earned is backed by verdict {:?}, which is not real evidence",
            backing.evidence.verdict
        );
    }
    assert_eq!(
        checked, 1,
        "exactly one of the four fns here should have drawn as earned"
    );
}

#[test]
fn the_verdict_strip_states_results_alongside_promises_once_evidence_resolves() {
    let doc = single_component_doc(&["a", "b", "c"]);
    let elements = evidence_for(&[
        ("a", "tested", &[]),
        ("b", "fuzzed(64)", &[]),
        ("c", "violation", &[]),
    ]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();
    let strip = xml
        .descendants()
        .find(|n| n.attribute("class") == Some("verdict-strip-text"))
        .and_then(|n| n.descendants().find(|d| d.is_text()))
        .and_then(|t| t.text().map(str::to_owned))
        .unwrap();

    assert!(strip.contains("2 earned"), "strip was {strip:?}");
    assert!(strip.contains("1 broken"), "strip was {strip:?}");
}

#[test]
fn the_strip_states_no_results_when_evidence_settles_nothing() {
    let doc = single_component_doc(&["a"]);
    // Evidence is present and resolves against the document, but the one fn
    // in it settles as `Declared` (`unclaimed`) -- nothing for the strip to
    // report beyond what the document already promises.
    let elements = evidence_for(&[("a", "unclaimed", &[])]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let xml = roxmltree::Document::parse(&svg).unwrap();
    let strip = xml
        .descendants()
        .find(|n| n.attribute("class") == Some("verdict-strip-text"))
        .and_then(|n| n.descendants().find(|d| d.is_text()))
        .and_then(|t| t.text().map(str::to_owned))
        .unwrap();
    assert!(
        !strip.contains('\u{2014}'),
        "nothing was earned, broken, unanswered, or stale here -- the strip must not invent a \
         results clause, got {strip:?}"
    );
    assert!(
        svg.contains("this line never          reports results."),
        "the strip's own tooltip must not claim results it never actually attached"
    );
}

/// The second load-bearing invariant: a collapsed box's earned-over-promised
/// split must equal what the same evidence, over the same document, would
/// show chip-by-chip if it were expanded instead. Rendered both ways from
/// exactly one evidence view, so the two views cannot silently drift apart.
#[test]
fn a_collapsed_boxs_earned_split_equals_the_counts_folded_beneath_it() {
    let doc = single_component_doc(&["a", "b", "c"]);
    let elements = evidence_for(&[
        ("a", "tested", &[]),
        ("b", "violation", &[]),
        ("c", "timeout", &[]),
    ]);

    let expanded = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let expanded_xml = roxmltree::Document::parse(&expanded).unwrap();
    let earned_when_expanded = ["a", "b", "c"]
        .iter()
        .filter(|name| {
            fn_chip_rect_class(&expanded_xml, name)
                .split(' ')
                .any(|c| c == "fn-chip-box-earned")
        })
        .count();
    assert_eq!(earned_when_expanded, 1);

    let collapsed = render_svg_with_evidence_and_options(
        &doc,
        &elements,
        &[],
        &RenderOptions {
            depth: Some(1),
            focus: None,
            collapse: vec![],
        },
    )
    .unwrap();
    // The collapsed box must state the exact same count this evidence would
    // show chip-by-chip, never a proportion or an average over it.
    assert!(
        collapsed.contains(&format!("{earned_when_expanded} of 3 earned")),
        "collapsed box did not show the folded split; svg was:\n{collapsed}"
    );
    // And it must never claim more than that: an evidence-derived count
    // greater than what the document actually holds would be the "reads
    // 90% healthy" failure this feature exists to prevent.
    assert!(!collapsed.contains("2 of 3 earned"));
    assert!(!collapsed.contains("3 of 3 earned"));
}

#[test]
fn the_dark_palette_carries_every_new_evidence_state_too() {
    let doc = single_component_doc(&["a", "b", "c", "d"]);
    let elements = evidence_for(&[
        ("a", "tested", &[]),
        ("b", "violation", &[]),
        ("c", "timeout", &[]),
        ("d", "tested", &["stale"]),
    ]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let style = svg
        .split("<style>")
        .nth(1)
        .and_then(|s| s.split("</style>").next())
        .unwrap();
    let dark = style
        .split("@media (prefers-color-scheme: dark)")
        .nth(1)
        .expect("a render using the new evidence states must still carry a dark block");

    for needed in [
        ".fn-chip-box-earned",
        ".fn-chip-box-violated",
        ".fn-chip-box-unanswered",
        ".fn-chip-box-stale",
    ] {
        assert!(
            dark.contains(needed),
            "`{needed}` is painted in light but not restated for dark, so it renders at its \
             light value against a dark ground"
        );
    }
}

#[test]
fn every_evidence_painted_element_resolves_a_style_rule() {
    let doc = single_component_doc(&["a", "b", "c", "d"]);
    let elements = evidence_for(&[
        ("a", "tested", &[]),
        ("b", "violation", &[]),
        ("c", "timeout", &[]),
        ("d", "tested", &["stale"]),
    ]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let style = format!(
        "{}{}{}",
        ply_render::svg::STYLE,
        ply_render::svg::FINDING_STYLE,
        ply_render::svg::EVIDENCE_STYLE
    );
    let matches_selector = |class: &str, tag: &str| {
        style.contains(&format!(".{class}{{"))
            || style.contains(&format!(".{class},"))
            || style.contains(&format!(".{class} {tag}{{"))
            || style.contains(&format!(".{class} {tag},"))
    };

    let xml = roxmltree::Document::parse(&svg).unwrap();
    let mut unstyled = Vec::new();
    for node in xml.descendants().filter(|n| n.is_element()) {
        let tag = node.tag_name().name();
        if !matches!(tag, "rect" | "circle" | "path" | "text" | "line") {
            continue;
        }
        // The arrowhead marker and the unclaimed-hatch pattern live in
        // <defs> and are styled through their own id/pattern reference, not
        // a class (see `every_painted_element_resolves_a_style_rule` in
        // render.rs, which excludes the same thing for the same reason).
        if node.ancestors().any(|a| a.tag_name().name() == "defs") {
            continue;
        }
        let resolved = node
            .ancestors()
            .filter_map(|a| a.attribute("class"))
            .flat_map(|c| c.split_whitespace())
            .any(|c| matches_selector(c, tag));
        if !resolved {
            unstyled.push(format!("<{tag}> class={:?}", node.attribute("class")));
        }
    }
    assert!(
        unstyled.is_empty(),
        "painted elements with no style rule: {unstyled:?}"
    );
}

/// The evidence-state twin of `only_forbidden_or_wrong_things_are_drawn_in_red`
/// in render.rs (that test only ever sees fixtures rendered with no
/// evidence, so it can never actually exercise these classes): red stays
/// reserved for `violated` and for the document's own `deny`/`finding`
/// rules — never for `unanswered` or `stale`, which are not wrong, only
/// unresolved.
#[test]
fn only_forbidden_or_wrong_evidence_states_are_drawn_in_red() {
    const REDS: [&str; 3] = ["#c9534f", "#8f2f2c", "#fdecec"];
    fn is_allowed(selector: &str) -> bool {
        selector.contains("deny") || selector.contains("finding") || selector.contains("violated")
    }

    let doc = single_component_doc(&["a", "b", "c", "d"]);
    let elements = evidence_for(&[
        ("a", "tested", &[]),
        ("b", "violation", &[]),
        ("c", "timeout", &[]),
        ("d", "tested", &["stale"]),
    ]);
    let svg = render_svg_with_evidence(&doc, &elements, &[]).unwrap();
    let style = svg
        .split("<style>")
        .nth(1)
        .and_then(|s| s.split("</style>").next())
        .unwrap();

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
        "these draw in red without being forbidden or wrong, so they compete with a real \
         violation for the one colour that must stay loudest:\n  {}",
        offenders.join("\n  ")
    );
}
