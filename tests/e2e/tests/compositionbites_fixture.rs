//! Composition (TODO.md, "make the sampling engine's decision recursive",
//! 2026-09-02) must not just refuse *less* -- turning every one of the
//! shapes this task's own probe measured as refused into a comfortable
//! green would be worse than the defect it fixes. Each function in this
//! fixture carries a promise that is genuinely false; this test proves a
//! real run finds each one, with a real failing input, not a clean pass.

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
fn a_false_promise_over_a_list_of_strings_earns_a_real_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("compositionbites");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let node = find_fn_node(&run.json["root"], "longest_len")
        .unwrap_or_else(|| panic!("no node for longest_len: {}", run.json));
    assert_eq!(
        node["verdict"], "violation",
        "a list of strings is buildable now -- the promise must actually be checked, and it is \
         false: {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let diag = diagnostics
        .iter()
        .find(|d| {
            d["node_id"] == "compositionbites::longest_len" && d["counterexample"].is_object()
        })
        .unwrap_or_else(|| panic!("expected a real counterexample: {}", run.json));
    assert!(
        diag["counterexample"]["inputs"]["xs"].is_string(),
        "the real failing list must be shown, not just claimed: {diag}"
    );
}

#[test]
fn a_false_promise_over_a_slice_earns_a_real_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("compositionbites");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let node = find_fn_node(&run.json["root"], "sum_slice")
        .unwrap_or_else(|| panic!("no node for sum_slice: {}", run.json));
    assert_eq!(
        node["verdict"], "violation",
        "a slice is buildable now -- the promise must actually be checked, and it is false: {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let diag = diagnostics
        .iter()
        .find(|d| d["node_id"] == "compositionbites::sum_slice" && d["counterexample"].is_object())
        .unwrap_or_else(|| panic!("expected a real counterexample: {}", run.json));
    assert!(
        diag["counterexample"]["inputs"]["xs"].is_string(),
        "the real failing slice must be shown, not just claimed: {diag}"
    );
}

#[test]
fn a_false_promise_over_a_nested_user_struct_earns_a_real_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("compositionbites");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let node = find_fn_node(&run.json["root"], "total_n")
        .unwrap_or_else(|| panic!("no node for total_n: {}", run.json));
    assert_eq!(
        node["verdict"], "violation",
        "a list of a user struct is buildable now -- the promise must actually be checked, and \
         it is false: {}",
        run.json
    );
}
