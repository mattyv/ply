use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use ply_core::diag::{Diagnostic, Envelope, Node, Span};
use ply_core::model::parse_document;
use ply_core::visual::{
    DEFAULT_RETAINED_RUNS, ElementEvidence, RootIdentity, RunMetadata, RunOutcome, SourceLocation,
    ToolIdentity, VisualDiagnostic, VisualElement, VisualEnvelope, VisualEnvelopeError,
    VisualPublisher, build_visual_envelope_with_sources, completed_run_metadata, stable_element_id,
};
use tempfile::tempdir;

fn envelope(id: &str, completed_at: &str, outcome: RunOutcome) -> VisualEnvelope {
    let element_id = stable_element_id("fn", "billing::total");
    let workspace_id = stable_element_id("workspace", "workspace");
    VisualEnvelope {
        protocol_version: 1,
        run: RunMetadata {
            id: id.into(),
            completed_at: completed_at.into(),
            root: RootIdentity { path: ".".into() },
            tool: ToolIdentity {
                name: "cargo-ply".into(),
                version: "test-build".into(),
            },
            outcome,
        },
        svg: format!("<svg><g id=\"{element_id}\"/></svg>"),
        elements: BTreeMap::from([
            (
                workspace_id.clone(),
                VisualElement {
                    id: workspace_id.clone(),
                    kind: "workspace".into(),
                    label: "workspace".into(),
                    parent_id: None,
                    evidence: ElementEvidence {
                        verdict: "bounded(2)".into(),
                        statuses: vec![],
                        reused: false,
                        engine: None,
                        seed: None,
                        cases: None,
                    },
                    source: None,
                    diagnostic_ids: vec![],
                },
            ),
            (
                element_id.clone(),
                VisualElement {
                    id: element_id.clone(),
                    kind: "fn".into(),
                    label: "billing::total".into(),
                    parent_id: Some(workspace_id),
                    evidence: ElementEvidence {
                        verdict: "bounded(2)".into(),
                        statuses: vec!["conditional".into()],
                        reused: true,
                        engine: Some("kani".into()),
                        seed: None,
                        cases: None,
                    },
                    source: Some(SourceLocation::point("src/lib.rs", 12, 7)),
                    diagnostic_ids: vec!["diag-E1-0".into()],
                },
            ),
        ]),
        diagnostics: vec![VisualDiagnostic {
            id: "diag-E1-0".into(),
            code: "E1".into(),
            severity: "error".into(),
            message: "example".into(),
            element_id: Some(element_id),
            source: Some(SourceLocation::point("src/lib.rs", 12, 7)),
        }],
    }
}

#[test]
fn schema_is_camel_case_and_rejects_an_unknown_major_version() {
    let json = envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean).to_json_pretty();
    assert!(json.contains("\"protocolVersion\": 1"));
    assert!(json.contains("\"completedAt\""));
    assert!(json.contains("\"parentId\""));
    assert!(json.contains("\"diagnosticIds\""));
    assert!(json.contains("\"startLine\": 12"));
    assert!(json.contains("\"startColumn\": 7"));
    assert!(json.contains("\"endLine\": 12"));
    assert!(json.contains("\"endColumn\": 7"));
    assert!(!json.contains("protocol_version"));

    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["protocolVersion"] = 2.into();
    let error = VisualEnvelope::from_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
    assert!(matches!(error, VisualEnvelopeError::UnsupportedVersion(2)));
}

#[test]
fn stable_ids_and_exact_source_locations_survive_round_trip() {
    let id = stable_element_id("fn", "billing::total");
    assert_eq!(id, stable_element_id("fn", "billing::total"));
    assert_ne!(id, stable_element_id("component", "billing::total"));

    let parsed = VisualEnvelope::from_json(
        &envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Violation).to_json_pretty(),
    )
    .unwrap();
    assert_eq!(
        parsed.elements[&id].source.as_ref().unwrap().file,
        "src/lib.rs"
    );
    assert_eq!(parsed.elements[&id].source.as_ref().unwrap().start_line, 12);
    assert_eq!(
        parsed.elements[&id].source.as_ref().unwrap().start_column,
        7
    );
}

#[test]
fn publication_is_immutable_atomic_and_indexed_by_safe_relative_paths() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
    let publisher = VisualPublisher::new(dir.path());
    publisher
        .publish(
            &envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean),
            20,
        )
        .unwrap();

    let artifact = dir.path().join("target/ply/views/run-1/visual.json");
    assert!(artifact.is_file());
    let index = publisher.read_index().unwrap().unwrap();
    assert_eq!(index.current_run, "run-1");
    assert_eq!(index.runs[0].path, "views/run-1/visual.json");
    assert!(index.runs[0].path_is_safe());

    let before = fs::read(dir.path().join("target/ply/view.json")).unwrap();
    let error = publisher
        .publish(
            &envelope("run-1", "2026-08-28T10:12:12Z", RunOutcome::Violation),
            20,
        )
        .unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(
        fs::read(dir.path().join("target/ply/view.json")).unwrap(),
        before
    );
}

#[test]
fn index_write_failure_leaves_the_previous_index_and_no_new_run() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
    let publisher = VisualPublisher::new(dir.path());
    publisher
        .publish(
            &envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean),
            20,
        )
        .unwrap();
    let index_path = dir.path().join("target/ply/view.json");
    let before = fs::read(&index_path).unwrap();
    let blocked_temp = dir
        .path()
        .join(format!("target/ply/.view.json.{}.tmp", std::process::id()));
    fs::create_dir(&blocked_temp).unwrap();

    assert!(
        publisher
            .publish(
                &envelope("run-2", "2026-08-28T10:12:12Z", RunOutcome::Violation,),
                20
            )
            .is_err()
    );
    assert_eq!(fs::read(index_path).unwrap(), before);
    assert!(!dir.path().join("target/ply/views/run-2").exists());
}

#[test]
fn retention_never_deletes_the_current_run() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
    let publisher = VisualPublisher::new(dir.path());
    for n in 0..25 {
        publisher
            .publish(
                &envelope(
                    &format!("run-{n:02}"),
                    &format!("2026-08-28T10:{n:02}:00Z"),
                    RunOutcome::Clean,
                ),
                DEFAULT_RETAINED_RUNS,
            )
            .unwrap();
    }
    let index = publisher.read_index().unwrap().unwrap();
    assert_eq!(index.runs.len(), DEFAULT_RETAINED_RUNS);
    assert_eq!(index.current_run, "run-24");
    assert!(
        dir.path()
            .join("target/ply/views/run-24/visual.json")
            .is_file()
    );
    assert!(!dir.path().join("target/ply/views/run-00").exists());
}

#[test]
fn explicit_cleanup_uses_the_same_retention_rule() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
    let publisher = VisualPublisher::new(dir.path());
    for n in 0..4 {
        publisher
            .publish(
                &envelope(
                    &format!("run-{n}"),
                    &format!("2026-08-28T10:0{n}:00Z"),
                    RunOutcome::Clean,
                ),
                20,
            )
            .unwrap();
    }
    let cleanup = publisher.cleanup(2).unwrap();
    assert_eq!(cleanup.removed, 2);
    assert!(cleanup.warning.is_none());
    let index = publisher.read_index().unwrap().unwrap();
    assert_eq!(index.current_run, "run-3");
    assert_eq!(index.runs.len(), 2);
    assert!(
        dir.path()
            .join("target/ply/views/run-3/visual.json")
            .is_file()
    );
}

#[test]
fn concurrent_publishers_merge_under_one_cross_process_lock() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
    let root = Arc::new(dir.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(3));
    let handles = [
        ("run-a", "2026-08-28T10:11:12Z"),
        ("run-b", "2026-08-28T10:11:13Z"),
    ]
    .map(|(id, completed_at)| {
        let root = Arc::clone(&root);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            VisualPublisher::new(root.as_ref())
                .publish(&envelope(id, completed_at, RunOutcome::Clean), 20)
                .unwrap();
        })
    });
    barrier.wait();
    for handle in handles {
        handle.join().unwrap();
    }

    let index = VisualPublisher::new(root.as_ref())
        .read_index()
        .unwrap()
        .unwrap();
    assert_eq!(
        index
            .runs
            .iter()
            .map(|run| run.id.as_str())
            .collect::<Vec<_>>(),
        ["run-a", "run-b"]
    );
    assert!(matches!(index.current_run.as_str(), "run-a" | "run-b"));
}

#[cfg(unix)]
#[test]
fn cleanup_refuses_symlinked_publication_ancestors_before_reading_the_index() {
    use std::os::unix::fs::symlink;

    for link in ["target", "target/ply", "target/ply/views"] {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
        let outside = tempdir().unwrap();
        let link_path = dir.path().join(link);
        fs::create_dir_all(link_path.parent().unwrap()).unwrap();
        symlink(outside.path(), &link_path).unwrap();

        let error = VisualPublisher::new(dir.path()).cleanup(1).unwrap_err();
        assert!(
            error.to_string().contains("symbolic link"),
            "{link}: {error}"
        );
    }
}

#[cfg(unix)]
#[test]
fn publication_commits_the_index_and_warns_when_pruning_refuses_a_run_link() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
    let publisher = VisualPublisher::new(dir.path());
    publisher
        .publish(
            &envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean),
            20,
        )
        .unwrap();
    let old_run = dir.path().join("target/ply/views/run-1");
    fs::remove_dir_all(&old_run).unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("keep"), "untouched").unwrap();
    symlink(outside.path(), &old_run).unwrap();

    let publication = publisher
        .publish(
            &envelope("run-2", "2026-08-28T10:11:13Z", RunOutcome::Clean),
            1,
        )
        .unwrap();
    assert!(publication.warning.is_some());
    let index = publisher.read_index().unwrap().unwrap();
    assert_eq!(index.current_run, "run-2");
    assert_eq!(index.runs.len(), 1);
    assert_eq!(index.runs[0].id, "run-2");
    assert_eq!(
        fs::read_to_string(outside.path().join("keep")).unwrap(),
        "untouched"
    );
}

#[cfg(unix)]
#[test]
fn explicit_cleanup_reports_committed_index_and_incomplete_disk_pruning() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    fs::write(dir.path().join("ply.yaml"), "ply: 1\ncomponents: {}\n").unwrap();
    let publisher = VisualPublisher::new(dir.path());
    for (id, completed_at) in [
        ("run-1", "2026-08-28T10:11:12Z"),
        ("run-2", "2026-08-28T10:11:13Z"),
    ] {
        publisher
            .publish(&envelope(id, completed_at, RunOutcome::Clean), 20)
            .unwrap();
    }
    let old_run = dir.path().join("target/ply/views/run-1");
    fs::remove_dir_all(&old_run).unwrap();
    let outside = tempdir().unwrap();
    symlink(outside.path(), &old_run).unwrap();

    let cleanup = publisher.cleanup(1).unwrap();
    assert_eq!(cleanup.removed, 1);
    assert!(cleanup.warning.is_some());
    assert_eq!(publisher.read_index().unwrap().unwrap().runs.len(), 1);
}

#[test]
fn malformed_index_cannot_escape_the_ply_artifact_directory() {
    let json = r#"{
      "protocolVersion": 1,
      "currentRun": "run-1",
      "runs": [{
        "id": "run-1",
        "path": "../../ply.lock",
        "completedAt": "2026-08-28T10:11:12Z",
        "outcome": "clean"
      }]
    }"#;
    assert!(ply_core::visual::ViewIndex::from_json(json).is_err());

    let future = json
        .replace("\"protocolVersion\": 1", "\"protocolVersion\": 2")
        .replace("../../ply.lock", "views/run-1/visual.json");
    assert!(matches!(
        ply_core::visual::ViewIndex::from_json(&future),
        Err(VisualEnvelopeError::UnsupportedVersion(2))
    ));
}

#[test]
fn source_ranges_cannot_run_backwards_or_escape_the_workspace() {
    let mut backwards = SourceLocation::point("src/lib.rs", 12, 7);
    backwards.end_line = 11;
    assert!(backwards.validate().is_err());
    assert!(
        SourceLocation::point("../ply.lock", 0, 0)
            .validate()
            .is_err()
    );
    assert!(
        SourceLocation::point("/etc/passwd", 0, 0)
            .validate()
            .is_err()
    );
    assert!(
        SourceLocation::point(r"src\lib.rs", 0, 0)
            .validate()
            .is_err()
    );
}

#[test]
fn envelope_rejects_empty_required_values_and_non_portable_root_paths() {
    let valid = envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean);
    let cases = [
        ("svg", ""),
        ("run.root.path", ""),
        ("run.root.path", r"src\workspace"),
        ("run.root.path", "C:/workspace"),
        ("run.tool.name", ""),
        ("run.tool.version", ""),
        ("element.id", ""),
        ("element.kind", ""),
        ("element.label", ""),
        ("element.evidence.verdict", ""),
        ("diagnostic.id", ""),
        ("diagnostic.code", ""),
        ("diagnostic.severity", ""),
        ("diagnostic.message", ""),
    ];
    for (field, replacement) in cases {
        let mut candidate = valid.clone();
        let element_key = candidate.elements.keys().next().unwrap().clone();
        match field {
            "svg" => candidate.svg = replacement.into(),
            "run.root.path" => candidate.run.root.path = replacement.into(),
            "run.tool.name" => candidate.run.tool.name = replacement.into(),
            "run.tool.version" => candidate.run.tool.version = replacement.into(),
            "element.id" => {
                candidate.elements.get_mut(&element_key).unwrap().id = replacement.into()
            }
            "element.kind" => {
                candidate.elements.get_mut(&element_key).unwrap().kind = replacement.into()
            }
            "element.label" => {
                candidate.elements.get_mut(&element_key).unwrap().label = replacement.into()
            }
            "element.evidence.verdict" => {
                candidate
                    .elements
                    .get_mut(&element_key)
                    .unwrap()
                    .evidence
                    .verdict = replacement.into()
            }
            "diagnostic.id" => candidate.diagnostics[0].id = replacement.into(),
            "diagnostic.code" => candidate.diagnostics[0].code = replacement.into(),
            "diagnostic.severity" => candidate.diagnostics[0].severity = replacement.into(),
            "diagnostic.message" => candidate.diagnostics[0].message = replacement.into(),
            _ => unreachable!(),
        }
        assert!(
            candidate.validate().is_err(),
            "accepted empty/unsafe {field}"
        );
    }
}

#[test]
fn envelope_rejects_impossible_utc_timestamps() {
    for timestamp in [
        "2026-02-29T10:11:12Z",
        "0000-01-01T00:00:00Z",
        "2024-02-30T10:11:12Z",
        "2026-13-01T10:11:12Z",
        "2026-08-28T24:00:00Z",
        "2026-08-28T10:60:00Z",
        "2026-08-28T10:11:60Z",
    ] {
        assert!(
            envelope("run-1", timestamp, RunOutcome::Clean)
                .validate()
                .is_err(),
            "accepted {timestamp}"
        );
    }
    envelope("run-1", "2024-02-29T23:59:59Z", RunOutcome::Clean)
        .validate()
        .unwrap();
}

#[test]
fn generated_run_metadata_never_leaks_an_absolute_host_root() {
    let run = completed_run_metadata(
        std::path::Path::new("/private/build/checkout"),
        "test-build",
        RunOutcome::Clean,
    );
    assert_eq!(run.root.path, ".");
}

#[test]
fn envelope_rejects_unknown_top_level_run_and_source_fields() {
    let json = envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean).to_json_pretty();
    for pointer in ["top", "run", "source"] {
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        match pointer {
            "top" => value["unexpected"] = true.into(),
            "run" => value["run"]["unexpected"] = true.into(),
            "source" => {
                let element = value["elements"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .find(|v| v.get("source").is_some())
                    .unwrap();
                element["source"]["unexpected"] = true.into();
            }
            _ => unreachable!(),
        }
        assert!(
            VisualEnvelope::from_json(&serde_json::to_string(&value).unwrap()).is_err(),
            "accepted unknown {pointer} field"
        );
    }
}

#[test]
fn envelope_rejects_missing_required_collection_fields() {
    let json = envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean).to_json_pretty();
    for field in ["diagnostics", "statuses", "reused", "diagnosticIds"] {
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        match field {
            "diagnostics" => {
                value.as_object_mut().unwrap().remove("diagnostics");
            }
            "statuses" | "reused" => {
                let element = value["elements"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap();
                element["evidence"].as_object_mut().unwrap().remove(field);
            }
            "diagnosticIds" => {
                let element = value["elements"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap();
                element.as_object_mut().unwrap().remove(field);
            }
            _ => unreachable!(),
        }
        assert!(
            VisualEnvelope::from_json(&serde_json::to_string(&value).unwrap()).is_err(),
            "accepted missing required field {field}"
        );
    }
}

#[test]
fn evidence_payload_remains_forward_extensible() {
    let mut value: serde_json::Value = serde_json::from_str(
        &envelope("run-1", "2026-08-28T10:11:12Z", RunOutcome::Clean).to_json_pretty(),
    )
    .unwrap();
    let element = value["elements"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    element["evidence"]["futureEvidence"] = serde_json::json!({ "opaque": [1, 2, 3] });
    VisualEnvelope::from_json(&serde_json::to_string(&value).unwrap()).unwrap();
}

#[test]
fn qualified_function_identity_keeps_same_named_claims_sources_and_diagnostics_apart() {
    let document = parse_document(
        "ply: 1\ncomponents:\n  billing:\n    anchor: app::billing\n    fns:\n      run: {}\n  shipping:\n    anchor: app::shipping\n    fns:\n      run: {}\n",
    )
    .unwrap();
    let leaf = || Node {
        id: "run".into(),
        kind: "fn".into(),
        verdict: "bounded(2)".into(),
        statuses: vec![],
        reused: false,
        evidence: None,
        children: vec![],
    };
    let component = |id: &str| Node {
        id: id.into(),
        kind: "component".into(),
        verdict: "bounded(2)".into(),
        statuses: vec![],
        reused: false,
        evidence: None,
        children: vec![leaf()],
    };
    let diagnostic = |node_id: &str, code: &str| Diagnostic {
        code: code.into(),
        severity: "warning".into(),
        phase: "verify".into(),
        engine: "kani".into(),
        check: "bounded(2)".into(),
        node_id: node_id.into(),
        title: format!("diagnostic for {node_id}"),
        primary_span: None,
        pointer: None,
        counterexample: None,
        fixes: vec![],
        assumptions: vec![],
        open_item: None,
    };
    let result = Envelope {
        command: "verify".into(),
        ply_version: "test-build".into(),
        root: Node {
            id: "workspace".into(),
            kind: "workspace".into(),
            verdict: "bounded(2)".into(),
            statuses: vec![],
            reused: false,
            evidence: None,
            children: vec![component("billing"), component("shipping")],
        },
        diagnostics: vec![
            diagnostic("billing::run", "W-BILLING"),
            diagnostic("shipping::run", "W-SHIPPING"),
        ],
        coverage: None,
        trust_surface: None,
        open_items: None,
        not_carried_forward: vec![],
    };
    let sources = BTreeMap::from([
        (
            "billing::run".into(),
            Span {
                file: "src/billing.rs".into(),
                start: [3, 4],
                end: [8, 1],
            },
        ),
        (
            "shipping::run".into(),
            Span {
                file: "src/shipping.rs".into(),
                start: [11, 2],
                end: [15, 1],
            },
        ),
    ]);
    let visual = build_visual_envelope_with_sources(
        &document,
        &result,
        RunMetadata {
            id: "qualified-run".into(),
            completed_at: "2026-08-28T10:11:12Z".into(),
            root: RootIdentity { path: ".".into() },
            tool: ToolIdentity {
                name: "cargo-ply".into(),
                version: "test-build".into(),
            },
            outcome: RunOutcome::Clean,
        },
        &sources,
    )
    .unwrap();

    let billing_id = stable_element_id("fn", "billing::run");
    let shipping_id = stable_element_id("fn", "shipping::run");
    assert_ne!(billing_id, shipping_id);
    assert_eq!(
        visual.elements[&billing_id].source.as_ref().unwrap().file,
        "src/billing.rs"
    );
    assert_eq!(
        visual.elements[&shipping_id].source.as_ref().unwrap().file,
        "src/shipping.rs"
    );
    assert_eq!(visual.elements[&billing_id].diagnostic_ids.len(), 1);
    assert_eq!(visual.elements[&shipping_id].diagnostic_ids.len(), 1);
    assert_eq!(
        visual.diagnostics[0].element_id.as_deref(),
        Some(billing_id.as_str())
    );
    assert_eq!(
        visual.diagnostics[1].element_id.as_deref(),
        Some(shipping_id.as_str())
    );
    assert!(
        visual
            .svg
            .contains(&format!("data-element-id=\"{billing_id}\""))
    );
    assert!(
        visual
            .svg
            .contains(&format!("data-element-id=\"{shipping_id}\""))
    );
}
