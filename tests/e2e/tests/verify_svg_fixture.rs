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
