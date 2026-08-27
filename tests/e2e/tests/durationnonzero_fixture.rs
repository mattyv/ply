//! Acceptance test for the 2026-08-27 task: `usize`/`isize`, the `NonZero`
//! family, and `Duration` must be accepted where they were refused before,
//! on **both** engines (`bounded` and `fuzz`), against a real
//! `cargo ply verify` run -- not merely the type classifier in isolation.
//!
//! Every function in this fixture holds for its entire domain, so a clean
//! `bounded(2)` (and no `fuzz` diagnostic) is the right outcome for every
//! one of them; a regression that reintroduces a false counterexample, a
//! harness that fails to compile, or a silently-dropped check all show up
//! here as a red test.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn usize_isize_nonzero_and_duration_are_accepted_on_both_engines() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("durationnonzero");

    let run = run_verify(&cargo_ply, fixture.path(), 180);

    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "no function in this fixture should produce a diagnostic -- every contract holds for \
         its entire domain, so a V0505/X0901/violation here means one of the four new shapes \
         regressed: {}",
        run.json
    );

    let root = &run.json["root"];
    assert_eq!(
        root["verdict"], "bounded(2)",
        "worst-of across every fn in this fixture must still be a clean bounded(2) -- envelope: {}",
        run.json
    );

    let children = root["children"][0]["children"].as_array().unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    for fn_name in [
        "bump_len",
        "double_delta",
        "tokens_requested",
        "shard_count",
        "round_trip",
        "from_whole_seconds",
    ] {
        // The tree's own node ids are bare fn names (unlike a diagnostic's
        // `node_id`, which is `component::fn`) -- confirmed against the
        // real envelope rather than assumed.
        let node = by_id
            .get(fn_name)
            .unwrap_or_else(|| panic!("missing node for {fn_name} in {root}"));
        assert_eq!(
            node["verdict"], "bounded(2)",
            "`{fn_name}` (previously an unsupported-signature shape) must now earn real \
             evidence on the Kani engine: {node}"
        );
    }
}
