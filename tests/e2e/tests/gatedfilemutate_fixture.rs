//! A crate-wide conservative fingerprint must retain the exact owner and
//! source file of every function Ply did resolve. Otherwise a function at
//! the top of `src/pipeline.rs` is selected as `pipeline::count_row`, while
//! cargo-mutants names it `count_row`, and no deliberate bug is planted.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_crate_wide_gate_still_mutates_the_claimed_file_module_function() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("gatedfilemutate");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    let function = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(function["id"], "pipeline::count_row", "{}", run.json);
    assert_eq!(
        function["verdict"], "tested·spec-strong",
        "every viable mutation in count_row's body should be caught: {}",
        run.json
    );
    assert!(!run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diag| diag["open_item"] == "no_mutants"));
}
