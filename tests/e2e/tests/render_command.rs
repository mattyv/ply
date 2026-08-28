//! `cargo ply render` (§7.1's renderer, reachable as a subcommand).
//!
//! The development loop's step 2 is "Ply renders that intent before
//! implementation begins", and until this existed that meant building a
//! second binary from a different workspace and invoking it by path. A user
//! trying Ply on a real project said exactly that.
//!
//! What is pinned is the property that makes one entry point rather than
//! two worth having: the subcommand and the standalone binary produce the
//! same bytes, and the notices written for a first-time reader reach both.
//! A second entry point that quietly disagrees with the first is worse than
//! no second entry point.

use std::process::Command;

use ply_e2e::build_cargo_ply;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("tests/e2e lives two levels below the repository root")
        .to_path_buf()
}

#[test]
fn render_matches_the_committed_drawing_the_standalone_binary_produces() {
    let cargo_ply = build_cargo_ply();
    let root = repo_root();
    let out = tempfile::tempdir().unwrap();
    let svg_path = out.path().join("out.svg");

    let status = Command::new(&cargo_ply)
        .current_dir(&root)
        .args([
            "render",
            "vetting/001-spsc-disruptor.ply.yaml",
            "-o",
            svg_path.to_str().unwrap(),
        ])
        .status()
        .expect("spawning cargo-ply render");
    assert!(
        status.success(),
        "render must succeed on a committed document"
    );

    assert_eq!(
        std::fs::read_to_string(&svg_path).unwrap(),
        std::fs::read_to_string(root.join("vetting/001-spsc-disruptor.svg")).unwrap(),
        "one renderer, two entry points: the bytes have to be identical"
    );
}

/// The document is all it reads. Pointed at a `ply.yaml` in a directory
/// with no crate, no `Cargo.toml` and no Rust at all, it still draws --
/// which is what "renders intent before implementation begins" has to mean.
#[test]
fn render_needs_no_code_no_manifest_and_no_engines() {
    let cargo_ply = build_cargo_ply();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ply.yaml"),
        "ply: 1\n\ncomponents:\n  alpha:\n    anchor: alpha\n    fns:\n      go:\n        checks: [fuzz(64)]\n",
    )
    .unwrap();

    let out = Command::new(&cargo_ply)
        .current_dir(dir.path())
        .args(["render", "ply.yaml"])
        .output()
        .expect("spawning cargo-ply render");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let svg = String::from_utf8_lossy(&out.stdout);
    assert!(
        svg.starts_with("<svg "),
        "the SVG goes to stdout: {svg:.80}"
    );
    assert!(
        svg.contains("alpha"),
        "the component must be drawn: {svg:.200}"
    );
}

/// The "this folded nothing away" notice exists for someone using the tool
/// for the first time. It would be a poor joke for it to appear from the
/// standalone binary and not from the command most people will type.
#[test]
fn a_selection_that_folds_nothing_says_so_from_this_entry_point_too() {
    let cargo_ply = build_cargo_ply();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ply.yaml"),
        "ply: 1\n\ncomponents:\n  alpha:\n    anchor: alpha\n  beta:\n    anchor: beta\n",
    )
    .unwrap();

    let out = Command::new(&cargo_ply)
        .current_dir(dir.path())
        .args(["render", "ply.yaml", "--depth", "1", "-o", "out.svg"])
        .output()
        .expect("spawning cargo-ply render");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("this drawing is identical to the one with no --depth/--focus/--collapse"),
        "a flat document folds nothing, and the reader has to be told: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
