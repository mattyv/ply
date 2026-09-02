//! Regression e2e for the confident-verdict defect this task fixes
//! (CLAUDE.md, 2026-09-02, modelled on the real defect found pointing Ply
//! at `semver`'s `Version::parse`): a promise written as a top-level `||`
//! can be satisfied almost entirely by one side of it, and the run must not
//! print an unqualified `fuzzed(n)` about that -- it must say which side
//! decided how often, and mark the verdict when one side did almost all of
//! the deciding.
//!
//! This fixture is also the short-circuit proof CLAUDE.md's first trap
//! names by name: `maybe_pass_through` returns `None` whenever its guard
//! (`x < 100`) is true, so if the measurement ever forced the promise's
//! other half (`result.unwrap() == x`) instead of stopping the way `||`
//! itself stops on its first true arm, this fixture would panic on nearly
//! every case instead of holding.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn find_fn_node<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|c| find_fn_node(c, id))
}

#[test]
fn a_lopsided_or_promise_is_measured_marked_and_never_changes_the_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("orskewed");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // The verdict itself must not move: this is disclosure, never a
    // different result (CLAUDE.md: "Verdicts must not change").
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "measuring which side of the promise decided each case must never itself change the \
         verdict: {}",
        run.json
    );

    // The short-circuit proof: if the generated check ever forced
    // `result.unwrap()` on a case where the guard (`x < 100`) already
    // decided the promise true, `maybe_pass_through` returns `None` there
    // and `.unwrap()` panics -- which would turn this clean `fuzzed(64)`
    // into a tool error or a violation. It did not, so evaluation order
    // was preserved, not merely asserted.
    assert!(
        run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["severity"] != "error"),
        "the right-hand side of the promise must never be forced on a case the left side \
         already decided -- an error diagnostic here means it was: {}",
        run.json
    );

    let fn_node = find_fn_node(&run.json["root"], "maybe_pass_through")
        .unwrap_or_else(|| panic!("no node for maybe_pass_through in {}", run.json));
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        statuses.contains(&"promise-lopsided"),
        "one side of this `||` decides almost every case -- the verdict must carry a status \
         saying so: statuses were {statuses:?}: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "orskewed::maybe_pass_through" && d["code"] == "W0526")
        .unwrap_or_else(|| panic!("no W0525 branch-split disclosure: {}", run.json));
    assert_eq!(
        disclosure["severity"], "warning",
        "one side deciding almost every case is weaker evidence than the case count alone \
         suggests, the same reasoning W0503's high-rejection warning uses: {}",
        disclosure
    );
    // Exact-string (CLAUDE.md: "assert the sentence a user reads, exact-
    // string"), pinned against a real run of this fixture -- the seed is
    // derived from the function's own name and contract text (§5.4c), never
    // from entropy, so these counts are the same on every run.
    assert_eq!(
        disclosure["title"].as_str().unwrap(),
        "`maybe_pass_through`'s postcondition joins 2 conditions with `||`: `|result|x < \
         100||result.unwrap() == x`. Rust only evaluates a later one when every earlier one \
         already came back false, and Ply's count preserves that order. Of the 64 cases where \
         the promise held, `x < 100` decided it 49 times and `result.unwrap() == x` decided it \
         15 times. That count says which side of the promise did the work; it says nothing \
         about which lines inside `maybe_pass_through` itself ran. (W0526)"
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "a warning-severity disclosure must not fail the run under the default `--fail-on`, \
         the same rule W0503's own high-rejection warning already follows: {}",
        run.json
    );
}
