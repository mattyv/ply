//! A declaration-only render has checked nothing, so its envelope must not
//! roll up to `clean`. Every item in it is `unclaimed`, which the verdict
//! vocabulary already counts as an absence of evidence -- so the run's own
//! outcome has to say the evidence is missing, not that the run came back
//! clean. An editor client colouring a badge from `outcome` would otherwise
//! show green for a document nothing has ever checked, which is the exact
//! failure this project exists to refuse.

use std::process::Command;

fn cargo_ply() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("cargo-ply")
}

#[test]
fn a_declaration_only_render_does_not_report_its_run_as_clean() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let doc = repo.join("vetting/001-spsc-disruptor.ply.yaml");

    let out = Command::new(cargo_ply())
        .args(["ply", "render"])
        .arg(&doc)
        .arg("--json")
        .output()
        .expect("running cargo-ply render --json");
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("render --json emitted valid JSON");

    // The premise of the assertions below, checked rather than assumed: a
    // declaration-only render carries no evidence at all.
    let verdicts = json["elements"]
        .as_object()
        .expect("the envelope carries an element map")
        .values()
        .map(|element| {
            element["evidence"]["verdict"]
                .as_str()
                .unwrap_or("<missing>")
        })
        .collect::<Vec<_>>();
    assert!(
        !verdicts.is_empty() && verdicts.iter().all(|verdict| *verdict == "unclaimed"),
        "every item in a declaration-only render should still be unclaimed, but the \
         verdicts were {verdicts:?}"
    );

    let outcome = json["run"]["outcome"].as_str().unwrap_or("<missing>");
    assert_ne!(
        outcome,
        "clean",
        "nothing has been checked in a declaration-only render -- all {} items are \
         `unclaimed` -- so the run must not report itself clean",
        verdicts.len()
    );
    assert_eq!(
        outcome, "missing_evidence",
        "the run's outcome must say the evidence is missing, which is what a tree of \
         `unclaimed` items means"
    );
}
