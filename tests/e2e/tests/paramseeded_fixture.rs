//! Composition (TODO.md, "make the sampling engine's decision recursive"),
//! 2026-09-02: a plain function's own `Option<String>` parameter -- a type
//! Ply's ordinary codegen refused outright before this task -- earns a real
//! `fuzzed(n)` verdict on its own, with no `examples:` entry needed. This
//! fixture used to demonstrate a narrower corpus-seeding workaround for
//! exactly this shape; composition supersedes it, so this test now pins the
//! real capability instead.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_option_string_parameter_earns_a_fuzzed_verdict_with_no_example_needed() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("paramseeded");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "an Option<String> parameter must be checked directly now, no seed required: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "V0505"),
        "must not still report the parameter as unsupported: {}",
        run.json
    );
}

/// Revert-proof (CLAUDE.md: prove every fix bites): make the promise false
/// and confirm a real violation with a real failing input, for the exact
/// shape this task's own probe measured as refused (an optional string).
#[test]
fn a_false_promise_over_an_optional_string_earns_a_real_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("paramseeded");
    let lib_path = fixture.path().join("src/lib.rs");
    // `*result >= 0` is trivially true for a `usize` no matter what the
    // real code does -- broken here into a promise the real body actually
    // can (and, for any non-empty text, does) violate.
    let broken = std::fs::read_to_string(&lib_path)
        .unwrap()
        .replace("*result >= 0", "*result == 0");
    std::fs::write(&lib_path, broken).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "the promise is false for every non-empty input -- this must be caught, not reported as \
         a clean pass: {}",
        run.json
    );
    // `Option<String>` is not witness-renderable (§ RustType::is_witness_
    // renderable's own doc), so the real failing value is reported as a
    // witness-only violation (W0541), same as a bare `String` already is
    // (see `stringsample_fixture.rs`) -- never a fabricated Rust literal,
    // but the real value must still be visible.
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let w0541 = diagnostics
        .iter()
        .find(|d| d["code"] == "W0541" && d["node_id"] == "paramseeded::width")
        .unwrap_or_else(|| panic!("expected a W0541 witness-only violation: {}", run.json));
    assert_eq!(w0541["severity"], "error");
    let inputs = &w0541["counterexample"]["inputs"];
    assert!(
        inputs["label"].is_string() && !inputs["label"].as_str().unwrap().is_empty(),
        "the real failing input must be shown, not just claimed: {w0541}"
    );
}
