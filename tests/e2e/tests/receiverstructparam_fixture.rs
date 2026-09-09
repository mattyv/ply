//! A state-changing method may take a user-defined structure by value.
//! Every preparatory repeat needs its own constructed argument; consuming
//! the final checked call's value makes the generated harness fail with
//! E0382 before a single case runs.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_receiver_history_rebuilds_a_by_value_struct_argument_for_each_call() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverstructparam");
    let run = run_verify(&cargo_ply, fixture.path(), 120);

    assert_eq!(
        run.exit_code,
        Some(0),
        "the generated harness must compile and run: {}",
        run.json
    );
    let ingest = find_node(&run.json["root"], "Sink::ingest")
        .unwrap_or_else(|| panic!("the checked method must appear in the result: {}", run.json));
    assert_eq!(
        ingest["verdict"], "fuzzed(64)",
        "all requested cases must reach the checked method: {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().all(|d| d["code"] != "X0901"),
        "a moved generated argument must never surface as a harness failure: {}",
        run.json
    );

    let reach = diagnostics
        .iter()
        .find(|d| d["code"] == "W0418")
        .unwrap_or_else(|| panic!("the private operation must be disclosed: {}", run.json));
    let title = reach["title"].as_str().unwrap();
    assert!(
        title.contains("`Sink::period_us`")
            && title.contains("not `pub`")
            && title.contains("outside this crate cannot call it"),
        "the state warning must name visibility as the reason: {title}"
    );
    assert!(
        !title.contains("takes an argument Ply cannot build"),
        "a no-argument private method must not be described as having an unbuildable argument: {title}"
    );
}

fn find_node<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| find_node(child, id))
}
