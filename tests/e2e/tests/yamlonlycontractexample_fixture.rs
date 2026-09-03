//! Defect 1 (2026-08-30, regression, "a promise nobody checks is now
//! reported green in total silence"): a fn declaring `checks: [test]` with a
//! passing `examples:` entry, and a `ply.yaml` `ensures` clause naming the
//! wrong answer. The example passed, `test` reported `tested`, and the wrong
//! promise was never checked and never mentioned. Zero diagnostics.
//!
//! The fix at the time was a warning: say out loud that the clause was not
//! checked. This test asserted that warning fired.
//!
//! Since 2026-09-03 the clause *is* checked (§5.4's "ANDed in"), so the
//! warning is gone and the assertion moves to the thing that actually
//! protects a reader. `seven` returns 7; the document promises 99. That is
//! a violation, and a passing example beside it does not make it anything
//! else. This is the sharpest case the merge exists for: everything about
//! this fixture looks green until someone checks the promise.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_false_promise_beside_a_passing_example_is_still_a_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("yamlonlycontractexample");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    let seven = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(seven["id"], "seven", "expected `seven`: {}", run.json);
    assert_eq!(
        seven["verdict"], "violation",
        "`seven` returns 7 and the document promises 99. The declared example passes, \
         which is exactly how this used to come back a clean `tested`: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d["node_id"] == "yamlonlycontractexample::seven"),
        "and the violation has to be reported, not merely scored: {run:#?}",
        run = run.json
    );
}
