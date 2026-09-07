//! A root verification follows the same links its drawing follows.

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
fn verifying_the_root_checks_linked_components_in_the_same_run() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let out = Command::new(cargo_ply())
        .args(["ply", "verify", "."])
        .current_dir(&repo)
        .output()
        .expect("cargo-ply runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "the composed root verification should complete:\nstdout:\n{text}\nstderr:\n{err}"
    );
    assert!(
        text.contains("workspace — tested"),
        "the root must aggregate the linked crates' real evidence:\n{text}"
    );
    assert!(
        !text.contains("were not checked by this run"),
        "the obsolete split-run explanation must be gone:\n{text}"
    );
}
