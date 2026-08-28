//! Blocker 2 (task 2026-08-27, docs/review-strings-receivers.md finding 2):
//! a top-level `parse` and a `util::parse` -- ordinary name pairs, nothing
//! exotic. `parse_harness::` is a plain substring of `util_parse_harness::`,
//! so `cargo test`'s own filter for `parse` alone used to execute
//! `util::parse`'s own tests too, and the *correct* `parse` was reported
//! `violation`, citing `util::parse`'s own failing tests as "its own". Each
//! function's verdict must be attributed to the right one.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no fn nodes: {json}"))
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node `{id}`: {json}"))
}

#[test]
fn two_same_named_functions_in_different_modules_are_each_attributed_to_their_own_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("nameoverlap");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let parse = node(&run.json, "parse");
    assert_eq!(
        parse["verdict"], "tested",
        "`parse`'s own promise is true on every input its precondition allows -- it must hold, \
         never borrow `util::parse`'s broken one: {}",
        run.json
    );

    let util_parse = node(&run.json, "util::parse");
    assert_eq!(
        util_parse["verdict"], "violation",
        "`util::parse`'s own promise is false on every input -- it alone must report the \
         violation: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    // No diagnostic may be attached to the correct `parse`.
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "nameoverlap::parse"),
        "the correct `parse` must carry no violation diagnostic at all: {}",
        run.json
    );
    let d = diagnostics
        .iter()
        .find(|d| d["node_id"] == "nameoverlap::util::parse" && d["code"] == "R0502")
        .unwrap_or_else(|| panic!("no R0502 diagnostic for `util::parse`: {}", run.json));
    let title = d["title"].as_str().unwrap();
    assert!(
        title.starts_with("`util::parse`"),
        "the diagnostic must name the function it is actually about: {title}"
    );
    assert!(
        title.contains("util_parse_harness::"),
        "the tests named must be `util::parse`'s own generated tests: {title}"
    );
}
