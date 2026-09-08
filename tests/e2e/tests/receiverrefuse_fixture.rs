//! Receiver construction's refusal-by-name half
//! (docs/review-self-construction.md's "fourth option", task 2026-08-27): a
//! type Ply cannot build a receiver for is refused, naming why -- never
//! guessed at, never filled in field by field.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no fn nodes: {json}"))
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node `{id}`: {json}"))
}

fn diag_for<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    let qualified = format!("receiverrefuse::{id}");
    json["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("no diagnostics: {json}"))
        .iter()
        .find(|d| d["node_id"] == qualified.as_str())
        .unwrap_or_else(|| panic!("no diagnostic for `{qualified}`: {json}"))
}

#[test]
fn a_type_with_no_constructor_is_refused_by_name() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverrefuse");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Gauge::read");
    assert_eq!(n["verdict"], "unsupported", "{}", run.json);
    let d = diag_for(&run.json, "Gauge::read");
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("Gauge") && title.contains("constructor"),
        "the refusal must name the type and say a constructor is what is missing, not merely \
         say \"unsupported\": {title}"
    );
}

#[test]
fn a_constructor_needing_an_unsupported_type_is_refused_by_name() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverrefuse");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Labelled::tag_value");
    assert_eq!(n["verdict"], "unsupported", "{}", run.json);
    let d = diag_for(&run.json, "Labelled::tag_value");
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("Labelled::new") && title.contains("Tag"),
        "the refusal must name the specific constructor and the specific type that blocked it, \
         never a generic \"not supported\" alone: {title}"
    );
}

/// This test used to pin the opposite, and its reasoning was wrong: a
/// method that changes the value is refused no longer (2026-09-07), because
/// a promise can say what it changes in terms of the value's own readings
/// before and after. `bump` carries no promise of its own, so what it now
/// reports is "makes no claims" -- and, crucially, no refusal naming the
/// receiver.
///
/// It is kept rather than deleted because a fixture whose whole job is
/// pinning refusals is exactly where a retracted refusal has to be pinned
/// as retracted, or the next reader takes the absence of a test for the
/// absence of the capability.
#[test]
fn a_mut_self_method_on_a_constructible_type_is_no_longer_refused() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverrefuse");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Counter::bump");
    assert_ne!(
        n["verdict"], "unsupported",
        "a method that changes the value is checkable now -- reporting it as a shape Ply \
         cannot check is the retracted claim, back: {}",
        run.json
    );
    assert!(
        run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["node_id"] != "receiverrefuse::Counter::bump"),
        "nothing should be refused about `bump` at all now: {}",
        run.json
    );
}

/// Owned `self` is the one receiver shape still refused, and the reason is
/// real: calling the method consumes the value, so a receiver Ply built
/// cannot be called into again. It is a second piece of codegen that does
/// not exist -- not something that cannot be said -- and admitting it
/// without generating it would report a claim the run cannot make.
#[test]
fn a_method_that_consumes_the_value_is_still_refused_by_name() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverrefuse");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Ledger::into_total");
    assert_eq!(
        n["verdict"], "unsupported",
        "a method that consumes the value it was called on is still out of reach: {}",
        run.json
    );
    let d = diag_for(&run.json, "Ledger::into_total");
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("consumes"),
        "the refusal has to say what is actually wrong -- that the call consumes the value -- \
         not merely that a receiver is involved: {title}"
    );
}
