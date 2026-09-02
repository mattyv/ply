//! The fourteenth false clean (docs/review-structs-enums.md finding 1,
//! 2026-08-28): `Acc::note` is the only operation that ever changes `Acc`'s
//! state, and it takes a `&mut u32` -- a type Ply's fuzz tier cannot build
//! an argument for. `Acc::get`'s promise (always 0) is false after one
//! ordinary `note` call, but Ply genuinely cannot call `note`, so this run
//! cannot find that violation by running cases. What it must do instead is
//! say so: name `note` in the disclosure, and mark the verdict with a
//! status that says this run's history is narrower than an unqualified
//! `fuzzed(n)` reads -- never claim, as the old wording did, that "nothing
//! here was assumed".

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_excluded_mutator_is_named_and_the_verdict_is_marked_narrower() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("excludedop");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // Ply genuinely cannot call `note` (a `&mut u32` argument -- a value
    // the function writes back through, which no engine here can construct
    // and observe), so this run cannot find `Acc::get`'s promise false by
    // running cases: the verdict really is
    // `fuzzed(256)`, real evidence about the receivers this run could
    // reach. What must NOT happen is that verdict standing alone, unmarked,
    // as if it meant "every history was explored".
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "this run cannot call `note`, so it cannot find the violation by running cases -- but \
         it must not silently report something stronger than it checked either: {}",
        run.json
    );

    let fn_node = find_fn_node(&run.json["root"], "Acc::get")
        .unwrap_or_else(|| panic!("no node for Acc::get in {}", run.json));
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        statuses.contains(&"partial-history"),
        "a verdict resting on a receiver history that could never include `note` must carry a \
         status marking it narrower than a plain `fuzzed(n)` looks -- statuses were {statuses:?}: \
         {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "excludedop::Acc::get" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence disclosure: {}", run.json));

    // The excluded operation must be named, not merely implied -- and the
    // diagnostic must have been escalated off `info`, since an operation
    // this run could never call is a real gap in what the verdict covers,
    // not a deliberate, documented sampling choice like the float/string
    // exclusions.
    assert_eq!(
        disclosure["severity"], "warning",
        "excluding a mutator from the pool is a real coverage gap, not a routine disclosure: {}",
        disclosure
    );
    let title = disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("Acc::note"),
        "the disclosure must name `Acc::note` as the operation this run never called: {title}"
    );
    assert!(
        !title.contains("nothing here was assumed"),
        "the old wording asserted every value this run saw was reachable by the type's own code \
         alone -- false the moment an operation was excluded, and it must not survive unchanged: \
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
