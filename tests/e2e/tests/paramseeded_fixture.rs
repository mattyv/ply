//! TODO.md, "an example does not unblock a parameter Ply cannot build": a
//! plain function's own `Option<String>` parameter -- a type Ply's ordinary
//! codegen never builds at all -- is seeded from one `examples:` entry the
//! same way a receiver constructor's own gated text already is, and the
//! verdict carries the same `seeded` status and an honest count of real,
//! distinct cases.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_option_string_parameter_with_one_example_earns_a_seeded_fuzzed_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("paramseeded");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the promise genuinely holds for every case -- seeding a parameter must not turn a \
         real pass into anything else: {}",
        run.json
    );

    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        statuses.contains(&"seeded"),
        "a verdict whose cases were grown from a known-valid value must carry that fact \
         structurally, not just as a diagnostic aside: {fn_node}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "W0524")
        .unwrap_or_else(|| panic!("expected the parameter-seeding diagnostic: {}", run.json));
    assert_eq!(d["severity"], "info", "a disclosure is not a failure: {d}");
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("label"),
        "must name the parameter that was seeded: {title}"
    );
    assert!(
        title.contains("64"),
        "must name the real number of cases that ran: {title}"
    );
    assert!(
        title.to_lowercase().contains("example"),
        "must say the seed came from an `examples:` entry: {title}"
    );
}

/// The honesty condition CLAUDE.md names outright: a seeded verdict must
/// never be indistinguishable from an unseeded one. An ordinary fn with no
/// unbuildable parameter at all must carry neither the status nor the
/// diagnostic, even one declared right beside `paramseeded` in the same
/// crate shape.
#[test]
fn without_an_example_the_parameter_stays_refused_and_unseeded() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("paramseeded");
    // Remove the one seed: the parameter must go back to being refused
    // outright, exactly as it always was without this task's change.
    let ply_yaml_path = fixture.path().join("ply.yaml");
    std::fs::write(
        &ply_yaml_path,
        "ply: 1\ncomponents:\n  paramseeded:\n    anchor: ply_fixture_paramseeded\n    fns:\n      width:\n        checks: [fuzz(64)]\n",
    )
    .unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    assert_eq!(
        run.json["root"]["verdict"], "unsupported",
        "no example, no seed -- `Option<String>` is exactly as unbuildable as it always was: {}",
        run.json
    );
    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !statuses.contains(&"seeded"),
        "must not carry the seeded status when nothing was seeded: {fn_node}"
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "W0524"),
        "must not emit the parameter-seeding diagnostic for a fn nothing seeded: {}",
        run.json
    );
    let refusal = diagnostics
        .iter()
        .find(|d| d["code"] == "V0505")
        .unwrap_or_else(|| panic!("expected the unsupported-shape refusal: {}", run.json));
    let title = refusal["title"].as_str().unwrap();
    assert!(
        title.to_lowercase().contains("example"),
        "the refusal must now say an example would unblock this parameter, not just restate \
         the flat refusal: {title}"
    );
}
