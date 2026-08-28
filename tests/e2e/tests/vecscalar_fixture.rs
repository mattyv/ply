//! Acceptance test: `Vec<T>` for an already fuzz-supported scalar `T`
//! (`Vec<u32>`) earns a real verdict via the shape-aware default route, and
//! is refused by name on the proving engine -- the M4 fuzz-vs-bounded
//! asymmetry (pre-dating this task) already pinned at the unit level in
//! `harness.rs`, given its own end-to-end fixture here (task, 2026-08-27).

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn vec_of_u32_is_sampled_and_earns_a_verdict_while_bounded_is_refused_by_name() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("vecscalar");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    let count = by_id["count"];
    assert_eq!(
        count["verdict"], "fuzzed(256)",
        "a Vec<u32> parameter must default to fuzz(256), never bounded(2) and never nothing: {}",
        run.json
    );
    assert!(!fixture.path().join("src/ply_generated.rs").exists());

    let bounded = by_id["count_bounded"];
    assert_eq!(
        bounded["verdict"], "unsupported",
        "bounded on Vec<u32> must be an honest absence, not a pass: {}",
        run.json
    );
    let diags = run.json["diagnostics"].as_array().unwrap();
    let v0508 = diags
        .iter()
        .find(|d| d["code"] == "V0508" && d["node_id"].as_str().unwrap().ends_with("count_bounded"))
        .unwrap_or_else(|| panic!("expected a V0508 refusal-by-name: {}", run.json));
    assert!(
        v0508["title"].as_str().unwrap().contains("Vec<u32>"),
        "must name the actual blocking parameter: {v0508}"
    );

    // `Vec<u8>` by *value*, never read back in its own postcondition --
    // regression coverage for the marker-precompute fix this task made
    // (found via `String`, but general to every by-value moved parameter,
    // `Vec<u8>` included): "Vec<u8> already exists for both engines; do not
    // regress it" (task brief), fuzz side.
    let sum_moved = by_id["sum_moved"];
    assert_eq!(
        sum_moved["verdict"], "fuzzed(64)",
        "a by-value Vec<u8> parameter not read post-call must still fuzz cleanly: {}",
        run.json
    );
}
