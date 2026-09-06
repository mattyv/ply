//! A recorded result is only as good as the hash beside it, and that hash
//! has to cover the code the check actually runs (The-Ply-Spec.md §5.2a).
//!
//! `reusereach` proves it for a helper the function calls, and
//! `reuseexamplehelper` for one only a worked example calls. This file
//! proves it for a helper named only in a contract **written in the
//! document** rather than as a Rust attribute on the function.
//!
//! The walk reads contract expressions off the function item's attributes.
//! A contract declared in `ply.yaml` is merged in later and never appears
//! there, so until 2026-09-06 the helper it called was hashed nowhere:
//! rewriting it changed what every generated case asserts and moved nothing
//! in the record.
//!
//! Reported by external review, 2026-09-06.

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
        fns[0]["verdict"], "violation",
        "and re-running must find the broken promise: `answer` returns its input up to 5 and the \
         postcondition now demands under 3, so a carried-forward green here is a verdict over a contract that \
         fails: {}",
        second.json
    );
}

/// Soundness bought by throwing per-claim reuse away would be a different
/// feature. The claim whose contract names nothing keeps its result.
#[test]
fn the_claim_whose_contract_names_no_helper_is_still_carried_forward() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reuseyamlcontract");

    run_verify(&cargo_ply, fixture.path(), 120);
    let src = fixture.read_lib_rs();
    fixture.write_lib_rs(&src.replace(
        "pub fn expected() -> u32 {\n    7\n}",
        "pub fn expected() -> u32 {\n    9\n}",
    ));

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(fns[1]["id"], "bumped", "envelope: {}", second.json);
    assert_eq!(
        fns[1]["reused"], true,
        "`bumped` cannot reach `expected` and its contract names nothing, so rewriting it must \
         cost this claim nothing: {}",
        second.json
    );
}
