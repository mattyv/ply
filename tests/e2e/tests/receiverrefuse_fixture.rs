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

/// Everything currently refused stays refused: `Counter` is constructible,
/// but `bump` takes `&mut self`, and this task never widens past `&self`
/// (Ply still has no way to state what a `&mut self` call is supposed to
/// change about the receiver).
#[test]
fn a_mut_self_method_on_a_constructible_type_still_stays_refused() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverrefuse");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Counter::bump");
    assert_eq!(
        n["verdict"], "unsupported",
        "a `&mut self` method must stay refused even when its type is constructible: {}",
        run.json
    );
    let d = diag_for(&run.json, "Counter::bump");
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("&mut self") || title.contains("receiver"),
        "the refusal must still name the receiver/`&mut self` reason: {title}"
    );
}
