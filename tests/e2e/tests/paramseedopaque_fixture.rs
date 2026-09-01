//! TODO.md's own honesty condition: "one value run 256 times is one test
//! and must never be reported as 256." `Opaque` has no part Ply knows how
//! to vary, so its one `examples:` entry must earn `tested` -- the exact
//! number of distinct cases it actually had -- never a `fuzzed(n)` that
//! reads as n independent draws.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_opaque_parameter_with_one_example_earns_tested_never_a_fabricated_fuzz_count() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("paramseedopaque");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict = run.json["root"]["verdict"].as_str().unwrap();
    assert_eq!(
        verdict, "tested",
        "an opaque type's one example is one real case -- the existing `tested` word already \
         means exactly that, and must not be inflated into a `fuzzed(n)`: {}",
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
        "nothing here was mutated or grown -- this is an ordinary example replay, not seeded \
         generation, and must not be marked as though it were: {fn_node}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "W0524"),
        "the parameter-seeding diagnostic is for a growable corpus -- this one never grew: {}",
        run.json
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d["title"].as_str().unwrap_or("").contains("fuzzed(")),
        "no diagnostic here may claim a fuzzed case count that never happened: {}",
        run.json
    );
}
