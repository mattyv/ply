//! docs/review-silent-narrowing.md finding 3, 2026-08-28: `TwoCtor` has two
//! usable constructors (`new`, `preloaded`); Ply's receiver scan always
//! builds a receiver by calling exactly one of them (the first
//! fully-buildable one found, in source order), so every case this run
//! generates starts from `new` and states reachable only through
//! `preloaded` are never explored. `value`'s promise (always 0) is false
//! for anything built by `preloaded`, and this run genuinely cannot find
//! that by running cases, because it never calls `preloaded`.
//!
//! What this fixture pins is the disclosure: `preloaded` named as a
//! constructor this run never started a receiver from, and the verdict
//! marked `partial-history`/"narrower than it looks" rather than an
//! unqualified pass.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_unused_constructor_is_named_and_the_verdict_is_marked_narrower() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("secondctor");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // Ply genuinely never calls `preloaded`, so this run's real evidence
    // is real evidence about receivers built via `new` alone --
    // fuzzed(256), not weaker, and not silently unmarked either.
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "this run never starts a receiver from `preloaded`, so it cannot find the violation -- \
         but the verdict itself is still real evidence about the receivers it did build: {}",
        run.json
    );

    let fn_node = find_fn_node(&run.json["root"], "TwoCtor::value")
        .unwrap_or_else(|| panic!("no node for TwoCtor::value in {}", run.json));
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        statuses.contains(&"partial-history"),
        "a verdict resting on a receiver history that only ever starts from one of several \
         usable constructors must carry a status marking it narrower than a plain `fuzzed(n)` \
         looks -- statuses were {statuses:?}: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "secondctor::TwoCtor::value" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence disclosure: {}", run.json));
    assert_eq!(
        disclosure["severity"], "warning",
        "never starting a receiver from a real, usable constructor is a real coverage gap, not \
         a routine disclosure: {}",
        disclosure
    );
    let title = disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("TwoCtor::preloaded"),
        "the disclosure must name `TwoCtor::preloaded` as the constructor this run never \
         started a receiver from: {title}"
    );
    assert!(
        title.contains("never by calling") || title.contains("only ever built"),
        "the disclosure must say plainly that this run only ever built a receiver via `new`, \
         never via `preloaded`: {title}"
    );
    assert!(
        !title.contains("nothing here was assumed"),
        "the completeness claim is false the moment a usable constructor was never called: \
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
