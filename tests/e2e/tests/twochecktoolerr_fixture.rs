//! Adversarial review, docs/review-caveats.md, "also fix": declaring
//! `[test, fuzz(n)]` together on one fn used to silently drop whichever
//! check ran zero cases, undoing the guard added the same day ("a pass
//! must prove a case ran"). `test` alone never runs anything for a receiver
//! method with no `examples:` (that is `fuzz`'s own job), so when both are
//! declared together the module-wide test count is nonzero the moment
//! `fuzz` runs -- and before this fix, that let `test`'s own zero cases
//! read as `tested`. The promise here is deliberately TRUE and `fuzz`
//! deliberately passes: a false promise would let worst-of aggregation
//! report `violation` regardless of what `test` did, hiding the exact bug
//! this fixture exists to catch. Since 2026-09-08 the successful fuzz
//! evidence remains the verdict while the empty test remains a `tool_error`
//! status: neither fact is allowed to erase the other.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_check_that_ran_nothing_is_never_hidden_by_a_sibling_check_that_passed() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("twochecktoolerr");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the sibling fuzz check really ran, so its evidence must not disappear: {}",
        run.json
    );

    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let statuses = fn_node["statuses"].as_array().unwrap();
    assert!(
        statuses.iter().any(|status| status == "tool_error"),
        "zero cases ran for `test`, so its non-result must remain beside the fuzz evidence: {}",
        run.json
    );
    assert_ne!(
        run.exit_code,
        Some(0),
        "preserving the fuzz evidence must not turn the unresolved test into a clean run"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let test_diag = diagnostics
        .iter()
        .find(|d| {
            d["node_id"] == "twochecktoolerr::Gauge::value"
                && d["check"] == "test"
                && d["code"] == "X0901"
        })
        .unwrap_or_else(|| {
            panic!(
                "no X0901 tool-error diagnostic for the `test` check: {}",
                run.json
            )
        });
    let title = test_diag["title"].as_str().unwrap();
    assert!(
        title.contains("ran zero cases"),
        "the diagnostic must say plainly that `test` executed nothing: {title}"
    );

    // `fuzz` itself must still have run and still be visible as evidence --
    // this fixes the check that ran nothing without breaking the one that
    // didn't.
    let fuzz_diag = diagnostics
        .iter()
        .find(|d| d["node_id"] == "twochecktoolerr::Gauge::value" && d["code"] == "W0520")
        .unwrap_or_else(|| {
            panic!(
                "`fuzz` must still have run and disclosed its sequence bound: {}",
                run.json
            )
        });
    assert_eq!(fuzz_diag["severity"], "info");
}
