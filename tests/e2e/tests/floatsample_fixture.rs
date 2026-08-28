//! Acceptance test for the sampling/proving split (task, 2026-08-27):
//!
//! - a plain float parameter earns a real `fuzzed(n)` verdict via the
//!   shape-aware default route (never `bounded`, never silently nothing);
//! - a `bounded` check explicitly asked for on that same shape is refused
//!   **by name** (`V0508`), never folded into the generic "none of its
//!   declared checks apply" wording that used to be false for exactly this
//!   case;
//! - a `fuzz` check on that same shape runs and earns a verdict.
//!
//! `increment`'s own contract is also the NaN/infinity decision's real-world
//! stake, spelled out in the fixture's own doc comment: it is only honestly
//! clean because Ply's default float sampling excludes NaN.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_float_parameter_is_sampled_and_earns_a_real_verdict_via_the_default_route() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("floatsample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    let increment = by_id["increment"];
    assert_eq!(
        increment["verdict"], "fuzzed(256)",
        "a plain f64 parameter must default to fuzz(256), never bounded(2) and never nothing: {}",
        run.json
    );
    // No Kani harness was ever generated for this fn: it never entered the
    // bounded/Kani path at all.
    assert!(!fixture.path().join("src/ply_generated.rs").exists());

    // The NaN/infinity disclosure (W0518, info) must name this run.
    let diags = run.json["diagnostics"].as_array().unwrap();
    let w0518 = diags
        .iter()
        .find(|d| d["code"] == "W0518" && d["node_id"].as_str().unwrap().ends_with("increment"))
        .unwrap_or_else(|| panic!("expected a W0518 float-sampling disclosure: {}", run.json));
    assert_eq!(w0518["severity"], "info");
    assert!(w0518["title"].as_str().unwrap().contains("NaN"));
}

#[test]
fn bounded_on_a_float_is_refused_by_name_while_fuzz_on_the_same_shape_earns_a_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("floatsample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    // `checks: [bounded(2)]` on a plain f32 -- refused by name, not silently
    // downgraded and not silently skipped.
    let bounded = by_id["mirror32_bounded"];
    assert_eq!(
        bounded["verdict"], "unsupported",
        "bounded on a sample-only type must be an honest absence, not a pass: {}",
        run.json
    );
    let diags = run.json["diagnostics"].as_array().unwrap();
    let v0508 = diags
        .iter()
        .find(|d| {
            d["code"] == "V0508" && d["node_id"].as_str().unwrap().ends_with("mirror32_bounded")
        })
        .unwrap_or_else(|| panic!("expected a V0508 refusal-by-name: {}", run.json));
    assert!(
        v0508["title"].as_str().unwrap().contains("x: f32"),
        "must name the actual blocking parameter: {v0508}"
    );
    assert!(
        v0508["title"].as_str().unwrap().contains("fuzz")
            || v0508["fixes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["title"].as_str().unwrap().contains("fuzz")),
        "must say what would work instead: {v0508}"
    );
    assert!(
        !v0508["title"]
            .as_str()
            .unwrap()
            .contains("none of its declared checks apply"),
        "this is the exact false sentence a sample-only type must never get: {v0508}"
    );

    // `checks: [fuzz(64)]` on the identical shape -- runs, and earns a real
    // verdict.
    let fuzzed = by_id["mirror32_fuzzed"];
    assert_eq!(
        fuzzed["verdict"], "fuzzed(64)",
        "the same shape, asked for on the engine that supports it, must actually run: {}",
        run.json
    );
}
