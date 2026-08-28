//! docs/review-silent-narrowing.md finding 2, 2026-08-28: `TraitTill`'s
//! only mutating operation (`take`) is a `trait` method
//! (`impl Fill for TraitTill`), never an inherent one. Calling through a
//! trait is genuinely out of scope for the receiver scan (it does not
//! resolve which trait a method needs or attempt a trait-qualified call),
//! so this run genuinely cannot call `take` and genuinely cannot find that
//! `total`'s promise (always 0) is false after one real `take` call.
//!
//! What must happen instead -- and what this fixture pins -- is that the
//! run says so: names `take` as excluded because it is a trait method
//! (never claiming, as the pre-fix wording did, that "nothing here was
//! assumed"), and marks the verdict `partial-history`/"narrower than it
//! looks" rather than reading as an unqualified, complete pass.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_trait_mutator_is_named_as_out_of_scope_and_the_verdict_is_marked_narrower() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("traitmutator");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // Ply genuinely cannot call a trait method, so the verdict is real
    // evidence about the receivers this run *could* reach -- fuzzed(256),
    // not weaker. What must not happen is that verdict standing alone,
    // unmarked, as though every operation had a chance to run.
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "this run cannot call a trait method, so it cannot find the violation -- but it must \
         not silently claim a different (or a stronger, unqualified) verdict either: {}",
        run.json
    );

    let fn_node = find_fn_node(&run.json["root"], "TraitTill::total")
        .unwrap_or_else(|| panic!("no node for TraitTill::total in {}", run.json));
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        statuses.contains(&"partial-history"),
        "a verdict resting on a receiver history that could never include a trait-only mutator \
         must carry a status marking it narrower than a plain `fuzzed(n)` looks -- statuses were \
         {statuses:?}: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "traitmutator::TraitTill::total" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence disclosure: {}", run.json));
    assert_eq!(
        disclosure["severity"], "warning",
        "a mutator left out of the pool for a reason unrelated to the promise being checked is a \
         real coverage gap, not a routine disclosure: {}",
        disclosure
    );
    let title = disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("TraitTill::take"),
        "the disclosure must name `TraitTill::take` as the operation this run never called: \
         {title}"
    );
    assert!(
        title.contains("trait"),
        "the disclosure must say *why* -- a trait implementation, not an unbuildable argument, \
         which is a different fact and a different fix: {title}"
    );
    assert!(
        !title.contains("could not build an argument"),
        "the old, argument-specific wording must not be reused for a reason that has nothing to \
         do with buildability: {title}"
    );
    assert!(
        !title.contains("nothing here was assumed"),
        "the completeness claim is false the moment an operation was excluded, whatever the \
         reason: {title}"
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
