//! The machine-readable tree says what was *promised*, not only what came
//! of it.
//!
//! Someone using Ply on a real design reported that a function reaches an
//! agent as a bare name. Part of that was their own misuse of the grammar
//! -- they had written an invariant as an `examples` string, where
//! `ensures` states the property -- and part was real: the envelope, which
//! is the agent-facing channel, carried verdicts and nothing else. A
//! consumer could read `fuzzed(64)` with no way to know what it was fuzzed
//! *for*.
//!
//! §7.1 already assumed otherwise: it says inline attributes "join when
//! `cargo ply` emits the §8 envelope", which only means something if the
//! envelope carries clauses. It did not.
//!
//! Pinned here: that it does, and that it does so whether the verdict was
//! earned in this run or carried forward. A promise is a property of the
//! claim and does not stop existing because a result was reused.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn fn_node(env: &serde_json::Value, id: &str) -> serde_json::Value {
    fn walk(n: &serde_json::Value, id: &str, out: &mut Option<serde_json::Value>) {
        if n["kind"] == "fn" && n["id"] == id {
            *out = Some(n.clone());
        }
        for c in n["children"].as_array().unwrap_or(&vec![]) {
            walk(c, id, out);
        }
    }
    let mut out = None;
    walk(&env["root"], id, &mut out);
    out.unwrap_or_else(|| panic!("no node for `{id}`: {env}"))
}

#[test]
fn the_envelope_carries_what_was_promised_and_what_rests_on_a_humans_word() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("envelopecontract");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let node = fn_node(&run.json, "add");

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
        "and the evidence named for it, which is the whole point: {node}"
    );

    // Second run: the result is carried forward. A promise belongs to the
    // claim, so it must be there just the same -- a real gap when these
    // fields were first wired to the run rather than to the plan.
    let again = run_verify(&cargo_ply, fixture.path(), 120);
    let again_node = fn_node(&again.json, "add");
    assert_eq!(
        again_node["reused"], true,
        "this half is about the reuse path; nothing was reused: {again_node}"
    );
    assert_eq!(
        again_node["contract"], node["contract"],
        "a carried-forward verdict must carry the same promise: {again_node}"
    );
    assert_eq!(again_node["trusted"], node["trusted"], "{again_node}");
}
