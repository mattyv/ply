//! A recorded result is only as good as the hash beside it, and that hash
//! has to cover the code the check actually runs (The-Ply-Spec.md §5.2a).
//!
//! `reusereach` proves that for a helper the claimed function calls. This
//! file proves it for a helper only the **worked example** calls. Until
//! 2026-09-06 the walk started at the claimed function and nowhere else, so
//! a helper named by the example and by nothing else was hashed nowhere at
//! all: rewriting it changed what the assertion demanded, left the
//! fingerprint byte-identical, and served the old pass over an assertion
//! that had just changed meaning.
//!
//! The reviewer who reported it asked for this test specifically -- fresh
//! versus cached, end to end -- rather than a unit test of the walk, on the
//! grounds that "the walk returns the right set" and "the record is actually
//! invalidated" are two different claims and only the second is the promise.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// The reproduction. Rewrite the helper the example asserts against; the
/// claim must be checked again, and the run must go red because the
/// author's own assertion is now false.
#[test]
fn rewriting_a_helper_only_the_example_calls_re_earns_the_claim() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reuseexamplehelper");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &first.json["root"]["children"][0]["children"];
    assert_eq!(fns[1]["id"], "tripled", "envelope: {}", first.json);
    assert_eq!(
        fns[1]["verdict"], "fuzzed(64)",
        "the example runs and passes on a cold run: {}",
        first.json
    );

    let src = fixture.read_lib_rs();
    let broken = src.replace(
        "pub fn expected() -> u32 {\n    6\n}",
        "pub fn expected() -> u32 {\n    7\n}",
    );
    assert_ne!(src, broken, "the helper body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[1]["reused"],
        serde_json::Value::Null,
        "the recorded result was about an assertion that demanded 6 and now demands 7, so it \
         says nothing about the check as it stands: {}",
        second.json
    );
    assert_ne!(
        fns[1]["verdict"], "fuzzed(64)",
        "and re-running must not report a pass -- `tripled(2)` is 6 and the example now demands \
         7, so a carried-forward green here is a verdict over an assertion that fails: {}",
        second.json
    );
}

/// Soundness bought by throwing per-claim reuse away would be a different
/// feature. The claim that names no example keeps its result.
#[test]
fn the_claim_with_no_example_is_still_carried_forward() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reuseexamplehelper");

    run_verify(&cargo_ply, fixture.path(), 120);
    let src = fixture.read_lib_rs();
    fixture.write_lib_rs(&src.replace(
        "pub fn expected() -> u32 {\n    6\n}",
        "pub fn expected() -> u32 {\n    8\n}",
    ));

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(fns[0]["id"], "bumped", "envelope: {}", second.json);
    assert_eq!(
        fns[0]["reused"], true,
        "`bumped` names no example and cannot reach `expected`, so rewriting it must cost this \
         claim nothing -- seeding the walk with examples must not have widened every claim: {}",
        second.json
    );
}
