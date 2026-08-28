//! §5.2a's reuse must actually fire on a warm, unchanged run for D5's first
//! branch (The-Ply-Spec.md §5.5), not merely compose the right bound on a
//! cold one. Found missing by adversarial review, 2026-08-26: a caller
//! standing on a proved callee composed correctly every time, but the
//! record's own "is this verdict one the declared checks could earn"
//! integrity check (`W0516`) still assumed a `bounded(k)` check could only
//! ever produce `bounded(k)` verbatim -- written before D5's first branch
//! existed, so a genuinely-composed `bounded(j)` for `j < k` looked
//! identical to a hand-edited record and was refused as impossible on
//! every single run after the first, forever. `stubverifiedstalebound_fixture`
//! covers the edit-between-two-runs shape; this covers the shape that
//! actually matters day to day -- nothing edited at all.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_caller_standing_on_a_proved_callee_reuses_on_an_unchanged_second_run() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverified");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(first.exit_code, Some(0), "envelope: {}", first.json);
    let first_fns = &first.json["root"]["children"][0]["children"];
    let first_f = first_fns
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "f")
        .unwrap_or_else(|| panic!("no `f` node in first run: {}", first.json));
    assert_eq!(
        first_f["verdict"], "bounded(2)",
        "first run: f composed cleanly against g's own proof: {}",
        first.json
    );
    assert_eq!(
        first_f.get("reused"),
        None,
        "a first run has nothing to reuse yet: {}",
        first.json
    );

    // Nothing a clone would not have either: the generated proof module.
    // A second run that genuinely reuses must not need to look at it, let
    // alone rewrite it -- mirroring `resultreuse_fixture`'s own proof for
    // the boundary-contract shape, here for D5's first branch.
    std::fs::remove_file(fixture.path().join("src/ply_generated.rs")).ok();

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(second.exit_code, Some(0), "envelope: {}", second.json);
    let second_fns = &second.json["root"]["children"][0]["children"];
    let second_f = second_fns
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "f")
        .unwrap_or_else(|| panic!("no `f` node in second run: {}", second.json));
    let second_g = second_fns
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "g")
        .unwrap_or_else(|| panic!("no `g` node in second run: {}", second.json));

    assert_eq!(
        second_g["reused"], true,
        "g's own inputs did not move: {}",
        second.json
    );
    assert_eq!(
        second_f["reused"], true,
        "f's inputs did not move either -- standing on a proved callee must not by itself \
         make a claim un-reusable on a second, unchanged run: {}",
        second.json
    );
    assert_eq!(
        second_f["verdict"], "bounded(2)",
        "the carried-forward verdict must be the same one, not re-derived: {}",
        second.json
    );

    let diagnostics = second.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "W0516"),
        "a genuinely composed bounded(j) for j <= the claim's own declared bound is not an \
         impossible record -- it must never be refused as one: {}",
        second.json
    );

    assert!(
        !fixture.path().join("src/ply_generated.rs").exists(),
        "a run that reused every claim must not write a proof module either"
    );
}
