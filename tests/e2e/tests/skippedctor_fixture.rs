//! Is the disclosure enough? (docs/review-structs-enums.md finding 2,
//! 2026-08-28). `Point::labeled` is a real constructor Ply's scan finds but
//! cannot use (its `&mut u32` argument is unbuildable), so Ply correctly
//! falls through to direct field construction. The old W0522 wording never
//! said whether a constructor existed at all -- this test pins that it now
//! does.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_found_but_unusable_constructor_is_named_in_the_disclosure() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("skippedctor");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "envelope: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "skippedctor::always_nonneg" && d["code"] == "W0522")
        .unwrap_or_else(|| panic!("no W0522 public-fields disclosure: {}", run.json));
    let title = disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("Point::labeled"),
        "the disclosure must name the constructor Ply found but could not use, or a reader has \
         no way to know direct field construction was not the only route this type offered: \
         {title}"
    );
    assert!(
        title.contains("mut"),
        "the disclosure should say *why* the constructor could not be used, not just that one \
         exists: {title}"
    );
}
