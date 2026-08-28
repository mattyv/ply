//! The narrowing this fix leaves open, made explicit (docs/review-caveats.md
//! N1): `mutate` genuinely needs the generated harness to share one Cargo
//! workspace with the crate under test (`engines::mutants`), and Ply no
//! longer creates that sharing uninvited on a crate with no `[workspace]`
//! table of its own. So on this crate's layout `mutate` must be refused by
//! name, honestly, before cargo-mutants is ever spawned -- never a crash,
//! and never silently dropped while `fuzz` on the same function still runs
//! and passes for real.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn mutate_is_refused_by_name_on_a_crate_with_no_workspace_while_fuzz_still_runs() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("plainmutate");

    let run = run_verify(&cargo_ply, fixture.path(), 150);

    // `fuzz` genuinely ran and the contract genuinely holds -- untouched by
    // `mutate`'s inability to run here.
    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "envelope: {}",
        run.json
    );
    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        statuses.contains(&"unsupported"),
        "mutate's refusal on this layout must be named `unsupported`, an absence of evidence, \
         not folded away: {fn_node}"
    );

    let codes: Vec<&str> = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"V0505"), "{}", run.json);
    let title = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "V0505")
        .unwrap()["title"]
        .as_str()
        .unwrap();
    assert!(
        title.contains("mutate") && title.contains("workspace"),
        "the refusal must name both the check and the real reason, not a generic failure: {title}"
    );

    // An absence of evidence still fails the run by default (§1) -- but
    // never as "engine missing" or "tool error", since the engine is
    // present and nothing crashed; it is a plain unsupported layout.
    assert_eq!(
        run.exit_code,
        Some(1),
        "§6: a declared check with no evidence and no crash is exit 1: {}",
        run.json
    );
}
