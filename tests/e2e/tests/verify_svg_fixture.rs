//! `verify --svg` writes the real, evidence-coloured drawing to a file --
//! the same picture `--publish-view` already builds internally and wraps in
//! JSON for an editor, just saved as a plain `.svg` a person or a CI step
//! can open directly. Before this, nobody could get that picture out of Ply
//! at all: `cargo ply render` only ever draws from the document alone, and
//! was never meant to turn green.

use ply_e2e::{build_cargo_ply, copy_fixture};
use std::process::Command;

/// Asserting the file merely exists, or merely contains `<svg`, would prove
/// almost nothing -- `cargo ply render` on the same document would satisfy
/// both and never touch a single real result. So this asserts the thing
/// that only a real run produces: a genuine evidence sentence Ply's own
/// verdict kernel had to compute, naming the actual check that actually
/// ran, on the actual claim this fixture declares.
#[test]
fn writes_the_real_evidence_coloured_drawing_not_the_declared_one() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("textseeded");
    let svg_path = fixture.path().join("verified.svg");

    let output = Command::new(&cargo_ply)
        .args([
            "verify",
            fixture.path().to_str().unwrap(),
            "--engine-timeout",
            "60",
            "--svg",
        ])
        .arg(&svg_path)
        .output()
        .expect("spawning cargo-ply verify --svg");

    assert!(
        svg_path.is_file(),
        "verify --svg must write the file even though the run's own exit code reports \
         the verdict, not the write -- stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let svg = std::fs::read_to_string(&svg_path).expect("reading the written svg");
    assert!(svg.starts_with("<svg"), "not a drawing at all:\n{svg}");

    // `fn-chip-box-earned` only ever appears when a fn chip carries real
    // `DisplayState::Earned` evidence -- the declared-only render has no
    // such state to attach and never emits this class, whatever the check
    // kind or case count declared for it. This fixture's one claim is
    // seeded to pass (`docs/reach-measurement-2.md`'s own probe), so a real
    // run earns it.
    assert!(
        svg.contains("fn-chip-box-earned"),
        "the written drawing must carry this run's own earned evidence, not \
         just the document's declared shape:\n{svg}"
    );

    // And the declared form really has none, so the assertion above is
    // discriminating between two real, distinguishable outputs -- not
    // trivially true because both paths happen to produce the same markup.
    let declared = Command::new(&cargo_ply)
        .args(["render", fixture.path().to_str().unwrap()])
        .output()
        .expect("spawning cargo-ply render");
    let declared_svg = String::from_utf8_lossy(&declared.stdout);
    assert!(
        !declared_svg.contains("fn-chip-box-earned"),
        "the plain declared render must never carry a real run's evidence -- \
         if it does, this test cannot tell the two apart:\n{declared_svg}"
    );
}

/// `--svg-overview` exists so a deep workspace has a drawing that fits on a
/// page, and CI can publish both from **one** verification.
///
/// One run is the whole point, and the reason this asks for both files in a
/// single command rather than two. Two runs would generate two sets of
/// inputs from two seeds, so one could find a counterexample the other
/// missed -- and the project's front page would then show a summary that
/// disagreed with the full drawing beside it.
///
/// What makes this a real test of the fold rather than of plumbing: the
/// overview must carry `1 of 1 earned`, a count only a completed run can
/// produce, on a box whose contents have been folded away. A fold that
/// dropped the evidence, or an overview quietly rendered from the document
/// instead of the run, fails here.
#[test]
fn one_run_writes_both_the_full_drawing_and_a_folded_overview() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("textseeded");
    let full_path = fixture.path().join("verified.svg");
    let overview_path = fixture.path().join("overview.svg");

    Command::new(&cargo_ply)
        .args([
            "verify",
            fixture.path().to_str().unwrap(),
            "--engine-timeout",
            "60",
            "--svg",
        ])
        .arg(&full_path)
        .arg("--svg-overview")
        .arg(&overview_path)
        .output()
        .expect("spawning cargo-ply verify --svg --svg-overview");

    let full = std::fs::read_to_string(&full_path).expect("the full drawing must be written");
    let overview =
        std::fs::read_to_string(&overview_path).expect("the overview must be written too");

    assert!(
        overview.starts_with("<svg"),
        "the overview is not a drawing at all:\n{overview}"
    );
    assert!(
        overview.contains("collapsed-stack"),
        "the overview must actually be folded, or it is just a second copy of the full \
         drawing:\n{overview}"
    );
    // The count only a completed run can compute, on the folded box itself.
    // The declared drawing of this same document cannot produce it, so this
    // is what separates a real overview from a re-rendered declaration.
    assert!(
        overview.contains("1 of 1 earned"),
        "folding must not drop the run's results -- the box should still say what it \
         earned:\n{overview}"
    );
    assert_ne!(
        overview, full,
        "this document has something to fold, so the two files must differ"
    );
    assert!(
        overview.len() < full.len(),
        "the overview is meant to be the smaller drawing: overview {} bytes, full {} bytes",
        overview.len(),
        full.len()
    );
}
