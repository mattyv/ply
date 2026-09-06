//! Fixture regression: a contract written in `ply.yaml` names helpers that
//! run on every generated case, and they have to be in what the fingerprint
//! covers (The-Ply-Spec.md §5.2a). Reported by external review 2026-09-06.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// The reproduction. Rewrite the helper the document's postcondition
/// compares against; the claim must be checked again, and must report the
/// violation that is now really there.
#[test]
fn rewriting_a_helper_named_only_in_a_yaml_contract_re_earns_the_claim() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reuseyamlcontract");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &first.json["root"]["children"][0]["children"];
    assert_eq!(fns[0]["id"], "answer", "envelope: {}", first.json);
    assert_eq!(
        fns[0]["verdict"], "fuzzed(64)",
        "the promise holds on a cold run: {}",
        first.json
    );

    let src = fixture.read_lib_rs();
    let broken = src.replace(
        "pub fn expected() -> u32 {\n    7\n}",
        "pub fn expected() -> u32 {\n    3\n}",
    );
    assert_ne!(src, broken, "the helper body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[0]["reused"],
        serde_json::Value::Null,
        "the recorded result was about a promise that allowed up to 6 and now allows up to 2, so it says \
         nothing about the check as it stands -- a contract in the document is as much code as \
         one in an attribute: {}",
        second.json
    );
    assert_eq!(
        fns[1]["reused"], true,
        "and the claim whose contract names no helper must still be carried forward -- this \
         must not have bought soundness by re-earning everything: {}",
        second.json
    );
    assert_eq!(
        fns[0]["verdict"], "violation",
        "and re-running must find the broken promise: `answer` returns its input up to 5 and the \
         postcondition now demands under 3, so a carried-forward green here is a verdict over a contract that \
         fails: {}",
        second.json
    );
}
