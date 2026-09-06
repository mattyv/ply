//! Verifying a document says where a linked component's promises are checked.
//!
//! `cargo ply verify .` on this repository prints `workspace — unclaimed`,
//! because the root document declares no fn claims -- every one lives in
//! `crates/ply-core/ply.yaml`. That is correct about what the run checked.
//! But the root *drawing* shows the linked crate's interior, so a reader
//! sees ~70 grey chips and concludes nothing is checked, while another run
//! has earned evidence for all of them.
//!
//! The maintainer hit this within a minute of publishing fresh evidence:
//! "we have the local evidence there now, so why doesn't the vis look
//! green?" This is the cheap half of the answer -- the full fix is `verify`
//! following the link, which needs spec text first (2026-09-06).

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
fn verifying_the_root_says_which_run_checks_a_linked_components_promises() {
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

    assert!(
        text.contains("were not checked by this run"),
        "a linked component's promises must be reported as this run's \
         silence, not as an absence of evidence anywhere:\n{text}"
    );
    assert!(
        text.contains("`cargo ply verify crates/ply-core` is the run that checks them"),
        "and the note must name the command that does check them, or a \
         reader is told what is wrong without being told where to look:\n{text}"
    );
    // The sentence is built by concatenation precisely because a `\` line
    // continuation in a Rust string keeps the following line's indentation,
    // which shipped as a run of spaces mid-sentence twice in one day.
    assert!(
        !text.contains("  by this run") && !text.contains("checked  "),
        "the note must not carry the indentation of the source that wrote \
         it:\n{text}"
    );
}
