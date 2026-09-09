use ply_core::check::run_checks;
use ply_core::model::parse_document;

#[test]
fn acceptance_component_and_package_relative_paths_are_validated() {
    let document = parse_document(
        r#"
ply: 1
components:
  mapping:
    anchor: app::mapping
acceptance:
  wrong_component:
    requirement: maps records
    component: missing
    entry: app::mapping::map_response
    test: { package: app, target: venue_response, name: maps }
    inputs: [tests/fixtures/input.json]
    expected: [tests/fixtures/expected.json]
    required: true
  escaping_path:
    requirement: maps records
    component: mapping
    entry: app::mapping::map_response
    test: { package: app, target: venue_response, name: maps }
    inputs: [../private/input.json]
    expected: [tests/fixtures/expected.json]
    required: true
"#,
    )
    .unwrap();

    let diagnostics = run_checks(&document);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0212")
            .count(),
        2,
        "both a missing component and a path escaping the package are configuration errors: {diagnostics:#?}"
    );
}
