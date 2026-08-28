//! The safety net the qualified-spelling resolution needs (coordinator
//! review of docs/review-silent-narrowing.md's own fix, 2026-08-28): "if
//! any of those cannot be resolved confidently, they must land in the
//! excluded-operations list by name with a true reason -- never silently
//! absent while the run claims completeness."
//!
//! `till::Till`'s bare name is ambiguous crate-wide (a second, unrelated
//! `Till` sits at the crate root), so Ply cannot confirm that
//! `till::ops`'s own `impl super::Till` really means `till::Till` rather
//! than the other one -- even though, read as ordinary Rust, it obviously
//! does. Ply must not guess: this run cannot call `take`, so it cannot
//! catch `total`'s false promise, but it must say so rather than either
//! silently pooling a possibly-wrong merge or silently dropping the
//! operation and reading as a complete pass.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_unresolvable_qualified_spelling_is_named_as_an_exclusion_never_silently_dropped_or_merged() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("unconfirmedspell");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // Ply genuinely cannot confirm the ambiguous spelling, so this run's
    // real evidence is only about the receivers it could build without
    // `take` -- fuzzed(256), not a stronger or a weaker verdict.
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "this run cannot confirm `super::Till` names the same `Till` (the bare name is \
         ambiguous crate-wide), so it cannot call `take` -- but it must not silently claim a \
         different verdict either: {}",
        run.json
    );

    let fn_node = find_fn_node(&run.json["root"], "till::Till::total")
        .unwrap_or_else(|| panic!("no node for till::Till::total in {}", run.json));
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        statuses.contains(&"partial-history"),
        "a verdict resting on a receiver history that could never include an unconfirmed \
         operation must carry a status marking it narrower than a plain `fuzzed(n)` looks -- \
         statuses were {statuses:?}: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "unconfirmedspell::till::Till::total" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence disclosure: {}", run.json));
    assert_eq!(
        disclosure["severity"], "warning",
        "an operation this run could not confirm and so never called is a real coverage gap, \
         not a routine disclosure: {}",
        disclosure
    );
    let title = disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("Till::take"),
        "the disclosure must name `Till::take` as the operation this run never called: {title}"
    );
    assert!(
        title.contains("could not confirm"),
        "the disclosure must say *why*: this scan found a real `impl` block ending in the same \
         bare name, but could not confirm it is the same type, which is a different fact (and a \
         different fix -- disambiguate the name) from an unbuildable argument or a trait method: \
         {title}"
    );
    assert!(
        !title.contains("nothing else was assumed"),
        "the completeness claim is false the moment an operation was excluded, confirmed or not: \
         {title}"
    );
}

fn find_fn_node<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|c| find_fn_node(c, id))
}
