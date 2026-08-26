//! §9's own rule in action: a defect found by review enters the suite as a
//! fixture of its own shape, not a spot-check on the line that was wrong.
//!
//! The shape: `fuzz`/`test` checks in one crate share a single generated
//! harness crate (The-Ply-Spec.md §5.4c). Before the misattribution fix,
//! one function's harness module failing to *compile* took the whole shared
//! crate's compile down with it, and every other claim in the crate --
//! however correct -- was reported `tool_error`, quoting the same compiler
//! error about a variable it does not have. Reproduced here with
//! `bad_examples_fn` (a broken `examples:` entry, so its own module never
//! compiles) sharing a crate with `good_fn` (unrelated, and correct).

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    fn find<'a>(n: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
        if n["id"] == id {
            return Some(n);
        }
        n["children"]
            .as_array()?
            .iter()
            .find_map(|child| find(child, id))
    }
    find(&json["root"], id).unwrap_or_else(|| panic!("no node `{id}` in envelope: {json}"))
}

#[test]
fn one_broken_functions_harness_never_blames_its_crate_mate() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("sharedharness");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let diagnostics = run.json["diagnostics"].as_array().unwrap();

    // The good function earns its own real verdict -- not `tool_error`,
    // not anything short of the fuzz evidence it actually earned.
    let good = node(&run.json, "good_fn");
    assert_eq!(
        good["verdict"], "fuzzed(32)",
        "good_fn is completely correct and must be checked for real, whatever else in this \
         crate is broken: {}",
        run.json
    );
    assert!(
        good["evidence"]["cases"].is_number(),
        "a real run must name the evidence it produced: {good}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "sharedharness::good_fn"),
        "good_fn has nothing wrong with it and must carry no diagnostic at all -- especially \
         not another function's compiler error: {}",
        run.json
    );

    // The broken function is reported against itself, and only itself.
    let bad = node(&run.json, "bad_examples_fn");
    assert_eq!(bad["verdict"], "tool_error", "envelope: {}", run.json);
    let bad_diags: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["node_id"] == "sharedharness::bad_examples_fn")
        .collect();
    assert!(
        !bad_diags.is_empty(),
        "the broken function must say so: {}",
        run.json
    );
    for d in &bad_diags {
        assert_eq!(d["code"], "X0901");
        let title = d["title"].as_str().unwrap();
        assert!(
            title.contains("E0308") || title.contains("mismatched types"),
            "the diagnostic must carry the real compiler error for *this* function's own \
             generated code: {title}"
        );
        assert!(
            !title.contains("v.len()") && !title.contains("borrow of moved value"),
            "this function's diagnostic must never carry a different function's compiler \
             error: {title}"
        );
    }

    // No diagnostic anywhere may misattribute one function's cause to the
    // other -- the defect's exact shape.
    assert!(
        diagnostics
            .iter()
            .filter(|d| d["node_id"] == "sharedharness::good_fn")
            .count()
            == 0,
        "envelope: {}",
        run.json
    );
}
