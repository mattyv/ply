//! The machine-readable tree says what was *promised*, not only what came
//! of it.
//!
//! Someone using Ply on a real design reported that an agent reading a
//! function saw only its name — no way to infer intent. Part of that was
//! their own misuse of the grammar, and part was real: the envelope, which
//! is the agent-facing channel, carried verdicts and nothing else. A
//! consumer could read `fuzzed(64)` with no way to know what it was fuzzed
//! *for*.
//!
//! §7.1 already assumed otherwise — it says inline attributes "join when
//! `cargo ply` emits the §8 envelope", which only means something if the
//! envelope carries clauses. It did not. This pins that it does, and that
//! it does so whether the verdict was earned in this run or carried
//! forward: a promise is a property of the claim and does not stop existing
//! because a result was reused.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn fn_nodes(env: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    fn walk(n: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
        if n["kind"] == "fn" {
            out.push(n.clone());
        }
        for c in n["children"].as_array().unwrap_or(&vec![]) {
            walk(c, out);
        }
    }
    walk(&env["root"], &mut out);
    out
}

#[test]
fn the_envelope_carries_what_was_promised_and_what_rests_on_a_humans_word() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("envelopecontract");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = fn_nodes(&run.json);
    let node = fns
        .iter()
        .find(|n| n["id"] == "add")
        .unwrap_or_else(|| panic!("no node for `add`: {}", run.json));

    let ensures = node["contract"]["ensures"]
        .as_array()
        .unwrap_or_else(|| panic!("the node carries no promise at all: {node}"));
    let texts: Vec<&str> = ensures.iter().filter_map(|e| e.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("10_000")),
        "the clause declared in ply.yaml has to reach the envelope: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains(">= a")),
        "so does the inline attribute -- §7.1 says the two join here: {texts:?}"
    );

    let trusted = node["trusted"]
        .as_array()
        .unwrap_or_else(|| panic!("the node says nothing about what rests on trust: {node}"));
    assert_eq!(trusted.len(), 1, "{node}");
    assert!(
        trusted[0]["claim"].as_str().unwrap().contains("rounding"),
        "the claim itself: {node}"
    );
    assert!(
        trusted[0]["evidence"]
            .as_str()
            .unwrap()
            .contains("vendor docs"),
        "and the evidence named for it, which is the whole point of a trusted claim: {node}"
    );

    // Second run: the result is carried forward. A promise is a property of
    // the claim, so it must be there just the same -- this was a real gap
    // when the fields were first wired to the run rather than to the plan.
    let again = run_verify(&cargo_ply, fixture.path(), 120);
    let again_node = fn_nodes(&again.json)
        .into_iter()
        .find(|n| n["id"] == "add")
        .unwrap();
    assert_eq!(
        again_node["reused"], true,
        "this test is about the reuse path; nothing was reused: {again_node}"
    );
    assert_eq!(
        again_node["contract"], node["contract"],
        "a carried-forward verdict must carry the same promise: {again_node}"
    );
    assert_eq!(again_node["trusted"], node["trusted"], "{again_node}");
}
