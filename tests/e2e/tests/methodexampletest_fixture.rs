//! Real-world reproduction (2026-09-01, verified by hand against `semver`'s
//! `Version::cmp_precedence`): Ply's own refusal for a shape `fuzz`/
//! `bounded` cannot build says, verbatim, to "declare `test` instead, with
//! an `examples:` entry, to run the concrete case directly". Doing exactly
//! that on a method used to fail to compile at all -- the generated test's
//! own name spliced the checked function's `::`-qualified path in verbatim,
//! which is not a legal Rust identifier (`error: invalid path separator in
//! function definition`). Every fixture exercising this codegen before this
//! one used a free function, whose path has no `::` to go wrong -- and
//! nearly everything in a real library is a method.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_method_declared_under_test_with_examples_actually_runs_and_earns_a_real_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("methodexampletest");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    assert_eq!(
        run.json["diagnostics"].as_array().unwrap().len(),
        0,
        "the worked example is true and the contract holds -- no diagnostic should appear: {}",
        run.json
    );
    assert_eq!(
        run.json["root"]["verdict"], "tested",
        "a method's `test` check with a true worked example must actually run it and earn a \
         real `tested` verdict, not a tool error from a harness that failed to compile: {}",
        run.json
    );

    // The catching direction is not optional (CLAUDE.md: assert the
    // observable outcome, not the shape of the output): the same escape
    // hatch must actually catch a false claim about the method, with a
    // named, reproduced failure -- never a comfortable pass.
    let ply_yaml_path = fixture.path().join("ply.yaml");
    let original = std::fs::read_to_string(&ply_yaml_path).unwrap();
    let broken = original.replace(
        "Weight::new(3).matches(&Weight::new(3)) == true",
        "Weight::new(3).matches(&Weight::new(4)) == true",
    );
    assert_ne!(
        broken, original,
        "replacement text did not match the fixture's ply.yaml verbatim"
    );
    std::fs::write(&ply_yaml_path, broken).unwrap();

    let run2 = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run2.json["root"]["verdict"], "violation",
        "`Weight::new(3).matches(&Weight::new(4))` is false -- the rewritten example asserts a \
         false claim about the method, and Ply must report a real, reproduced violation, not a \
         pass: {}",
        run2.json
    );

    let diagnostics = run2.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["node_id"] == "methodexampletest::Weight::matches" && d["code"] == "R0502")
        .unwrap_or_else(|| {
            panic!(
                "no R0502 diagnostic naming the failing example: {}",
                run2.json
            )
        });
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("failed"),
        "the diagnostic must say plainly that the example failed: {title}"
    );
}
