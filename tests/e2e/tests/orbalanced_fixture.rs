//! The other direction of the branch-decided measurement fixture
//! (CLAUDE.md, 2026-09-02): a promise whose `||` arms are genuinely
//! balanced must still print the split -- disclosure is unconditional,
//! never gated on skew (CLAUDE.md's second trap) -- but must not be marked
//! narrow, since nothing here is.

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
fn a_balanced_or_promise_prints_the_split_without_the_lopsided_mark() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("orbalanced");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "measuring which side of the promise decided each case must never itself change the \
         verdict: {}",
        run.json
    );

    let fn_node = find_fn_node(&run.json["root"], "thirds")
        .unwrap_or_else(|| panic!("no node for orbalanced::thirds in {}", run.json));
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        !statuses.contains(&"promise-lopsided"),
        "no single side of this three-way tautology decides almost every case -- it must not \
         be marked narrow: statuses were {statuses:?}: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "orbalanced::thirds" && d["code"] == "W0526")
        .unwrap_or_else(|| {
            panic!(
                "the split must print even when it is balanced -- disclosure is never gated on \
                 skew: {}",
                run.json
            )
        });
    assert_eq!(
        disclosure["severity"], "info",
        "nothing here is wrong or owed, only worth naming, the same reasoning the float/string \
         sampling disclosures already use: {}",
        disclosure
    );
    // Exact-string (CLAUDE.md: "assert the sentence a user reads, exact-
    // string"), pinned against a real run of this fixture -- the seed is
    // derived from the function's own name and contract text (§5.4c), never
    // from entropy, so these counts are the same on every run.
    assert_eq!(
        disclosure["title"].as_str().unwrap(),
        "`thirds`'s postcondition joins 3 conditions with `||`: `|result|x % 3 == 0||x % 3 == \
         1||x % 3 == 2`. Rust only evaluates a later one when every earlier one already came \
         back false, and Ply's count preserves that order. Of the 64 cases where the promise \
         held, `x % 3 == 0` decided it 22 times, `x % 3 == 1` decided it 24 times and `x % 3 == \
         2` decided it 18 times. That count says which side of the promise did the work; it \
         says nothing about which lines inside `thirds` itself ran. (W0526)"
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "an info-severity disclosure must not fail the run: {}",
        run.json
    );
}
