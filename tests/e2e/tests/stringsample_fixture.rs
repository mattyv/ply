//! Acceptance test for the sampling/proving split's second headline case
//! (task, 2026-08-27):
//!
//! - a plain `String` parameter earns a real `fuzzed(n)` verdict via the
//!   shape-aware default route (never `bounded`, never silently nothing);
//! - **the decisive case**: a genuinely broken string function (a false
//!   promise) is caught, with the failing string value visible in the
//!   diagnostic;
//! - a `bounded` check explicitly asked for on that same shape is refused
//!   **by name** (`V0508`), naming what would work instead, never folded
//!   into the generic "none of its declared checks apply" wording.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// Also-fix, smaller (task 2026-08-27, docs/review-strings-receivers.md):
/// "the string control-character exclusion is real but never disclosed to
/// the user, while the float NaN exclusion is." The exclusion itself and
/// its own gate (`ContractFn::has_string_shape`) were already built and
/// pinned by code-level tests in `harness.rs`; nothing pinned that a run
/// over a string-shaped function actually tells the *user* what was left
/// out, the way the float precedent (`W0518`) already does. This is that
/// pin, from the outside -- reading `cargo ply verify`'s own JSON output,
/// never the generated strategy source.
#[test]
fn a_string_shaped_run_discloses_the_control_character_exclusion_like_floats_do() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stringsample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let diags = run.json["diagnostics"].as_array().unwrap();
    let w0521 = diags
        .iter()
        .find(|d| d["code"] == "W0521" && d["node_id"].as_str().unwrap().ends_with("byte_len"))
        .unwrap_or_else(|| {
            panic!(
                "expected a W0521 info-level disclosure for a string-shaped run, the same way \
                 floats get W0518: {}",
                run.json
            )
        });
    assert_eq!(w0521["severity"], "info");
    let title = w0521["title"].as_str().unwrap();
    assert!(
        title.contains("control character"),
        "must name what was excluded: {title}"
    );
    assert!(
        title.contains("Unicode") || title.contains("multi-byte"),
        "must also say what is NOT excluded, the way the float disclosure names both sides: \
         {title}"
    );
}

#[test]
fn a_string_parameter_is_sampled_and_earns_a_real_verdict_via_the_default_route() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stringsample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    let byte_len = by_id["byte_len"];
    assert_eq!(
        byte_len["verdict"], "fuzzed(256)",
        "a plain String parameter must default to fuzz(256), never bounded(2) and never \
         nothing: {}",
        run.json
    );
    // No Kani harness was ever generated for this fn: it never entered the
    // bounded/Kani path at all.
    assert!(!fixture.path().join("src/ply_generated.rs").exists());
}

#[test]
fn a_genuinely_broken_string_function_is_caught_with_a_false_promise() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stringsample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    let preview = by_id["preview"];
    assert_eq!(
        preview["verdict"], "violation",
        "a false promise on a String-typed function must be caught, not reported clean: {}",
        run.json
    );

    // This asserted a `W0541` witness-only violation until 2026-09-04: a
    // String failure could be found but not replayed as an ordinary Rust
    // test, and W0541 is the code that says so. String replay works now, so
    // the same failure arrives as an ordinary violation *with* a replay file,
    // and W0541 correctly no longer fires. What the test was really guarding
    // -- that the failing value is visible and is the real string -- is
    // asserted here on the code that does fire.
    let diags = run.json["diagnostics"].as_array().unwrap();
    let violation = diags
        .iter()
        .find(|d| d["code"] == "P0502" && d["node_id"].as_str().unwrap().ends_with("preview"))
        .unwrap_or_else(|| panic!("expected a P0502 violation for `preview`: {}", run.json));
    assert_eq!(violation["severity"], "error");
    assert!(
        !diags
            .iter()
            .any(|d| d["code"] == "W0541" && d["node_id"].as_str().unwrap().ends_with("preview")),
        "a String failure is replayable now, so the witness-only excuse must \
         not still be reported alongside the replay: {}",
        run.json
    );
    assert_eq!(
        violation["counterexample"]["cargo_test"], "src/ply_generated_cex.rs",
        "the reader must be handed a test that fails the same way: {violation}"
    );

    // The real failing string value must be visible, not just "a violation
    // happened" -- and it must be the *decoded* value, with no artefacts of
    // how it travelled: neither the marker wire form's backslashes nor the
    // quotes and `\u{...}` escapes proptest's `Debug` adds on the panic path.
    let inputs = &violation["counterexample"]["inputs"];
    let s = inputs["s"]
        .as_str()
        .unwrap_or_else(|| panic!("the failing input must be shown: {violation}"));
    assert!(
        !s.contains("\\\\"),
        "the reported value must be the real string, not the escaped wire form: {inputs}"
    );
    assert!(
        !s.starts_with('"') && !s.ends_with('"'),
        "the reported value must be the string itself, not the Rust literal \
         `Debug` prints -- a reader told the input was `\"a\"` will look for a \
         three-character value with quotes in it: {inputs}"
    );
    assert!(
        !s.contains("\\u{"),
        "a non-ASCII character must be reported as itself, not as the escape \
         `Debug` prints for it: {inputs}"
    );
}

/// The same shape, without a panic: a byte-length-vs-char-count mismatch
/// that returns a *wrong value* rather than crashing, so this exercises the
/// ordinary (non-panic) postcondition-failure path -- where the reported
/// string comes back through Ply's own marker decoding (escaped on the way
/// out, unescaped on the way back in) rather than proptest's own panic-
/// shrink report. The two are routed differently internally but must both
/// report a clean, unescaped, real failing value.
#[test]
fn a_non_panicking_wrong_value_string_bug_is_also_caught_with_a_clean_reported_value() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stringsample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    let bad = by_id["char_count_wrong"];
    assert_eq!(
        bad["verdict"], "violation",
        "byte length vs char count is a real mismatch for multi-byte input, and must be caught: {}",
        run.json
    );

    // Was a `W0541` witness-only violation until 2026-09-04; String replay
    // works now, so the failure comes back as an ordinary violation with a
    // replay file. The value assertions below are the point of this test and
    // are unchanged.
    let diags = run.json["diagnostics"].as_array().unwrap();
    let violation = diags
        .iter()
        .find(|d| {
            d["code"] == "P0502" && d["node_id"].as_str().unwrap().ends_with("char_count_wrong")
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a P0502 violation for `char_count_wrong`: {}",
                run.json
            )
        });
    let s = violation["counterexample"]["inputs"]["s"].as_str().unwrap();
    assert!(
        !s.is_empty(),
        "the failing string must be shown: {violation}"
    );
    assert!(
        !s.contains('\\'),
        "the reported value must be the real, decoded string -- no wire-escaping artefacts: {s:?}"
    );
    // Debug-formatted text (proptest's own panic-shrink fallback) would wrap
    // the value in quotes; the marker-decode path Ply owns must not.
    assert!(
        !(s.starts_with('"') && s.ends_with('"')),
        "must be the plain decoded string, not a Debug-quoted rendering: {s:?}"
    );
}

#[test]
fn bounded_on_a_string_is_refused_by_name_while_fuzz_on_the_same_shape_earns_a_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stringsample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let children = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap();
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = children
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();

    // `checks: [bounded(2)]` on a plain String -- refused by name, not
    // silently downgraded and not silently skipped.
    let bounded = by_id["byte_len_bounded"];
    assert_eq!(
        bounded["verdict"], "unsupported",
        "bounded on a sample-only type must be an honest absence, not a pass: {}",
        run.json
    );
    let diags = run.json["diagnostics"].as_array().unwrap();
    let v0508 = diags
        .iter()
        .find(|d| {
            d["code"] == "V0508" && d["node_id"].as_str().unwrap().ends_with("byte_len_bounded")
        })
        .unwrap_or_else(|| panic!("expected a V0508 refusal-by-name: {}", run.json));
    assert!(
        v0508["title"].as_str().unwrap().contains("s: String"),
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

    // `byte_len` -- the identical shape, sampled -- runs and earns a real
    // verdict (checked more thoroughly in the default-route test above;
    // repeated here to pin the fuzz/bounded contrast in one place).
    let fuzzed = by_id["byte_len"];
    assert_eq!(fuzzed["verdict"], "fuzzed(256)");
}
