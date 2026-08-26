//! D5's ordering rule and its fallback (The-Ply-Spec.md §5.5): "within a
//! crate, verify claimed functions callees-before-callers" -- a cycle
//! cannot be ordered, so every claim in it falls back to the second
//! branch. The point of this test is what does NOT happen: no hang, no
//! tool error, no silent full-descent inlining that might never terminate
//! -- just the same `conditional`/`owed-evidence` shape D5's second branch
//! has always produced, applied here because ordering could not establish
//! anything stronger.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_cycle_cannot_be_ordered_so_both_sides_fall_back_and_stay_conditional() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverifiedcycle");

    let run = run_verify(&cargo_ply, fixture.path(), 180);

    let fn_nodes = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let f = fn_nodes
        .iter()
        .find(|n| n["id"] == "f")
        .unwrap_or_else(|| panic!("no `f` node in envelope: {}", run.json));
    let g = fn_nodes
        .iter()
        .find(|n| n["id"] == "g")
        .unwrap_or_else(|| panic!("no `g` node in envelope: {}", run.json));

    for (name, node) in [("f", f), ("g", g)] {
        assert_eq!(
            node["verdict"], "bounded(2)",
            "`{name}` must still earn real evidence -- not `timeout`, not `tool_error`, not \
             `unclaimed` -- a cycle is not an engine failure: {}",
            run.json
        );
        let statuses: Vec<&str> = node["statuses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(
            statuses.contains(&"conditional"),
            "`{name}` cannot be placed before the other in the callees-before-callers order, so \
             it falls back to D5's second branch -- assumed, not proved -- and must carry \
             `conditional`: {node}"
        );
        assert!(
            statuses.contains(&"owed-evidence"),
            "and the assumption is owed evidence, exactly as any other D5-second-branch \
             verdict: {node}"
        );
    }

    let root_statuses: Vec<&str> = run.json["root"]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        root_statuses.contains(&"conditional"),
        "`conditional` propagates to the root (D6): {}",
        run.json
    );

    // Neither side may claim to have stood on the other's proof -- that
    // would be exactly the unsoundness Ply's own ordering exists to
    // prevent (nothing here was independently established).
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "W0517"),
        "no clean-dependency diagnostic may appear when ordering could not establish either \
         side: {}",
        run.json
    );
    assert!(
        diagnostics.iter().any(|d| d["code"] == "W0511"),
        "each side's own W0511 conditional-verdict diagnostic must still be present: {}",
        run.json
    );

    assert_eq!(
        run.exit_code,
        Some(0),
        "`conditional` is real evidence and exits clean (D5's second branch always has): {}",
        run.json
    );
}
