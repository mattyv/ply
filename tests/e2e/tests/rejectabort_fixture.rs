//! Regression e2e for the 2026-08-24 M4 review's D4: when proptest abandons
//! a run because almost every generated input was rejected, the shipped
//! adapter still reported the full `fuzzed(256)` -- a verdict claiming 256
//! cases of evidence for a run that executed approximately none. A warning
//! does not license a verdict string that overstates the evidence (D6: the
//! evidence order is what the verdict carries; qualifiers sit beside it).

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_run_proptest_abandoned_earns_no_fuzz_evidence() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("rejectabort");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert!(
        !verdict.starts_with("fuzzed"),
        "a run that checked approximately nothing must not claim fuzz evidence, got `{verdict}`: {}",
        run.json
    );
    assert_eq!(verdict, "unclaimed", "envelope: {}", run.json);

    // §1: "every verdict must name the evidence that produced it concretely
    // enough to reproduce it". The `evidence` block said `cases: 256` for
    // this run too -- attached because `fuzz(256)` was *declared*, not
    // because 256 cases were reached (adversarial review of the post-004
    // fixes, D5). The count that goes in the envelope is the count proptest
    // actually accepted, which is what "cases the engine reached" means.
    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let cases = fn_node["evidence"]["cases"].as_u64();
    assert!(
        cases != Some(256),
        "a run proptest abandoned reached nowhere near the 256 cases it was asked for, and the \
         envelope must not report the declared number as if it had: {fn_node}"
    );
    assert!(
        fn_node["evidence"]["seed"].is_string(),
        "the seed is still real -- it is what this run used, and it is what replays it: {fn_node}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let warnings: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["code"] == "W0503")
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "the run must say why it produced nothing: {}",
        run.json
    );
    let title = warnings[0]["title"].as_str().unwrap();
    assert!(
        title.starts_with(
            "proptest gave up on `narrow_window` before it could run the 256 cases \
                           `fuzz(256)` asked for"
        ),
        "the warning must name the cause in plain words (newbie bar, exact wording): {title}"
    );
    assert!(
        title.contains("So this function has no fuzz evidence at all"),
        "the warning must say what the run is worth, not just what happened: {title}"
    );
    assert!(
        !warnings[0]["fixes"].as_array().unwrap().is_empty(),
        "§8: a non-result diagnostic SHOULD carry concrete fixes: {}",
        warnings[0]
    );
    assert!(
        warnings[0]["counterexample"].is_null(),
        "nothing failed, so there is no witness: {}",
        warnings[0]
    );
}
