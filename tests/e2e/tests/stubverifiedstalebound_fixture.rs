//! §5.2a's fingerprint must cover what a claim composed against under D5's
//! first branch (The-Ply-Spec.md §5.5), not only the callee's source.
//!
//! Found by adversarial review, 2026-08-26, of the composition feature
//! itself: `f` stands on `g`'s own earned `bounded(k)`, recorded and reused
//! exactly like any other result -- but the fingerprint that guarded that
//! record only ever covered `g`'s *source*. Editing only `g`'s *declared
//! checks* (its bound going from 5 down to 2, no source touched anywhere)
//! correctly re-earned `g`'s own record, but left `f`'s fingerprint --
//! and therefore its stored, now-stale `bounded(5)` -- untouched. That is
//! the exact overclaim `stubverifiedminbound` exists to prevent at
//! composition time, reached instead through the record.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn editing_only_the_callees_declared_bound_re_composes_the_caller_not_a_stale_one() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverifiedstalebound");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(first.exit_code, Some(0), "envelope: {}", first.json);
    let first_fns = &first.json["root"]["children"][0]["children"];
    let first_f = first_fns
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "f")
        .unwrap_or_else(|| panic!("no `f` node in first run: {}", first.json));
    let first_g = first_fns
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "g")
        .unwrap_or_else(|| panic!("no `g` node in first run: {}", first.json));
    assert_eq!(first_g["verdict"], "bounded(5)", "envelope: {}", first.json);
    assert_eq!(
        first_f["verdict"], "bounded(5)",
        "first run: composed against g's own bounded(5): {}",
        first.json
    );

    // Edit ONLY `g`'s declared checks -- no source touched anywhere, in
    // either function.
    std::fs::write(
        fixture.path().join("ply.yaml"),
        r#"ply: 1
components:
  stubverifiedstalebound:
    anchor: ply_fixture_stubverifiedstalebound
    fns:
      g:
        checks: [bounded(2)]
      f:
        checks: [bounded(5)]
"#,
    )
    .unwrap();

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
        second_g["verdict"], "bounded(2)",
        "g's own checks changed, so g must re-run and earn its own new bound: {}",
        second.json
    );
    assert_ne!(
        second_g["reused"], true,
        "g's own declared checks changed -- its own record cannot still match: {}",
        second.json
    );

    assert_eq!(
        second_f["verdict"], "bounded(2)",
        "f's own source did not change, but the callee it composed against is now only \
         proved to bounded(2) -- reporting f's stale bounded(5) here would be the exact \
         overclaim §5.5's composition rule exists to prevent, arriving through the record \
         instead of through composition: {}",
        second.json
    );
    assert_ne!(
        second_f["verdict"], "bounded(5)",
        "f must never keep reporting a depth the callee it stands on no longer supports: {}",
        second.json
    );
}
