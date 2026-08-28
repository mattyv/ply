//! D3 (adversarial review, 2026-08-26): the tamper check that decides
//! whether a stored verdict is one the declared checks could actually have
//! earned (§5.2a's `Record::matching`/`verdict_is_earnable`) was widened,
//! after D5's first branch landed, to accept *any* `bounded(j)` with
//! `j <= k` for a claim declared `bounded(k)` -- because composing against
//! a shallower callee can genuinely produce a `j` below the caller's own
//! declared bound. That widening is too loose: it also accepts a
//! hand-edited record claiming a bound the composition never actually
//! produced. `f` here composes to exactly `bounded(2)` (it stands on `g`'s
//! own `bounded(2)` proof); a stored `bounded(4)` is still `<= f`'s
//! declared `bounded(5)`, so the old rule waved it through. The correct
//! rule pins the expected value to `min(declared_k, min(stood-on bounds))`
//! and requires *equality* against that computed number, not merely
//! `<=` against the declaration.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_hand_edited_bound_still_within_the_declared_ceiling_is_refused_not_reused() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverifiedminbound");

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
        "f composes to the shallower bound g actually proved: {}",
        first.json
    );

    let lock_path = fixture.path().join("ply.lock");
    let lock = std::fs::read_to_string(&lock_path).unwrap();
    // The map is keyed by node_id and serializes in BTreeMap (alphabetical)
    // order, so `::f`'s entry -- and its `verdict` field -- comes first in
    // the file, before `::g`'s. Replacing only the first occurrence edits
    // `f`'s recorded verdict, not `g`'s.
    let tampered_marker = "\"verdict\": \"bounded(4)\"";
    let edited = lock.replacen("\"verdict\": \"bounded(2)\"", tampered_marker, 1);
    assert_ne!(lock, edited, "f's stored verdict must have been rewritten");
    assert!(
        edited.contains(tampered_marker),
        "the hand-edit must actually have landed: {edited}"
    );
    std::fs::write(&lock_path, &edited).unwrap();

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let second_fns = &second.json["root"]["children"][0]["children"];
    let second_f = second_fns
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "f")
        .unwrap_or_else(|| panic!("no `f` node in second run: {}", second.json));

    assert_ne!(
        second_f["verdict"], "bounded(4)",
        "a hand-edited bound that still sits under f's own declared bounded(5) must never be \
         silently accepted just because it is <= the declaration -- the only honest value here \
         is min(declared, stood-on) = bounded(2): {}",
        second.json
    );
    assert_eq!(
        second_f["verdict"], "bounded(2)",
        "refusing the tampered record means checking the claim again, and it must recompose to \
         the same honest value as the first run: {}",
        second.json
    );

    let diagnostics = second.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|d| d["code"] == "W0516"),
        "the run must say the file was edited by something that is not Ply, exactly as it does \
         for any other impossible record: {}",
        second.json
    );

    assert_eq!(
        second.exit_code,
        Some(0),
        "the re-earned bounded(2) is real evidence and exits clean: {}",
        second.json
    );
}
