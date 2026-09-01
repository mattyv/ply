//! `docs/reach-measurement-2.md`'s other honesty condition: when there is
//! no known-valid value to grow inputs from at all -- no `examples:` entry,
//! and the constructor never accepts a single generated draw -- the refusal
//! must name the action that would fix it, not just restate generic advice.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_gated_text_constructor_with_no_seeds_refuses_by_name() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("textseedempty");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "unclaimed",
        "a constructor that never accepts a single draw earns no fuzz evidence at all: {}",
        run.json
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
        title.contains("this type is built by parsing"),
        "must name why random text fails, in plain words: {title}"
    );
    assert!(
        title.contains("no case base to grow from"),
        "must say there was nothing to grow inputs from, not just that the rate was high: {title}"
    );
    assert!(
        title.contains("Add an `examples:` entry showing one valid call to `Strict::new`"),
        "must name the exact action -- which constructor to write an example for: {title}"
    );
    assert!(
        title.contains("Ply will grow inputs from it"),
        "must say what happens once that advice is followed: {title}"
    );

    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !statuses.contains(&"seeded"),
        "a run that earned no evidence must not also claim its (nonexistent) evidence was \
         seeded: {fn_node}"
    );
}
