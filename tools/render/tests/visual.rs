use std::collections::BTreeMap;

use ply_core::model::parse_document;
use ply_core::visual::{ElementEvidence, SourceLocation, VisualDiagnostic, VisualElement};
use ply_render::svg::{render_svg, render_svg_with_evidence};

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
