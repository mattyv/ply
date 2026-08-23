use ply_render::model::{Check, parse_document};

#[test]
fn parses_minimal_document() {
    let doc = parse_document("ply: 1\n").expect("minimal doc should parse");
    assert_eq!(doc.ply, 1);
    assert!(doc.components.is_empty());
    assert!(doc.edges.is_empty());
    assert!(doc.deny.is_empty());
    assert!(doc.profiles.is_empty());
    assert!(doc.unresolved.is_empty());
}

#[test]
fn parses_disruptor_fixture() {
    let yaml = std::fs::read_to_string("../../vetting/001-spsc-disruptor.ply.yaml").unwrap();
    let doc = parse_document(&yaml).expect("disruptor fixture should parse");

    assert_eq!(doc.ply, 1);
    let ring = doc.components.get("ring").expect("ring component");
    assert_eq!(ring.anchor, "disruptor::spsc");
    assert_eq!(ring.uses, vec!["unsafe".to_string()]);
    assert_eq!(ring.owns, vec!["disruptor::spsc::Spsc".to_string()]);
    assert_eq!(ring.profile.as_deref(), Some("hot_path"));

    let slot = ring.fns.get("slot").expect("slot fn");
    assert_eq!(
        slot.checks,
        vec![
            "bounded(2)".to_string(),
            "mutate".to_string(),
            "fuzz(4096)".to_string()
        ]
    );

    let try_push = ring.fns.get("Spsc::try_push").expect("Spsc::try_push fn");
    assert_eq!(
        try_push.check_with.get("T").map(String::as_str),
        Some("u64")
    );
    assert_eq!(try_push.trusted.len(), 1);
    assert_eq!(
        try_push.trusted[0].claim,
        "SPSC cross-thread safety (happens-before between cursors)"
    );
    assert_eq!(try_push.examples.len(), 1);

    let profile = doc.profiles.get("hot_path").expect("hot_path profile");
    assert_eq!(
        profile,
        &vec!["no_panics".to_string(), "exhaustive_match".to_string()]
    );

    // sanity: every check string in the fixture parses under the micro-syntax
    for fn_claim in ring.fns.values() {
        for c in &fn_claim.checks {
            assert!(matches!(
                ply_render::model::parse_check(c).unwrap(),
                Check::Test | Check::Fuzz(_) | Check::Bounded(_) | Check::Prove | Check::Mutate
            ));
        }
    }
}

#[test]
fn unknown_field_is_rejected() {
    let yaml = r#"
ply: 1
components:
  pricing:
    anchor: pricing
    totally_unknown_field: true
"#;
    let err = parse_document(yaml).expect_err("unknown field must be rejected");
    assert!(
        err.to_string().to_lowercase().contains("unknown field")
            || err.to_string().contains("totally_unknown_field"),
        "error should mention the unknown field, got: {err}"
    );
}
