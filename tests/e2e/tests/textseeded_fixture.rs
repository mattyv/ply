//! `docs/reach-measurement-2.md`: a receiver built from free-form text
//! cannot be constructed by random text alone -- almost none of it parses.
//! With one `examples:` entry, Ply grows a corpus of known-valid values
//! (the entry itself, plus every value the constructor accepts during the
//! run) and mutates that corpus instead of guessing uniformly, and the
//! verdict carries a `seeded` status so the evidence's real provenance is
//! never mistaken for an unbiased sample of all possible text.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_gated_text_constructor_with_one_example_earns_a_seeded_fuzzed_verdict() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("textseeded");

    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the promise genuinely holds for every accepted case -- seeding must not turn a real \
         pass into anything else: {}",
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
        "a verdict whose cases were grown from known-valid values must carry that fact \
         structurally, not just as a diagnostic aside: {fn_node}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d["code"] == "W0523")
        .unwrap_or_else(|| panic!("expected the seeded-generation diagnostic: {}", run.json));
    assert_eq!(d["severity"], "info", "a disclosure is not a failure: {d}");
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("Prerelease::new"),
        "must name the constructor the corpus was grown for: {title}"
    );
    assert!(
        title.contains("1 from the `examples:` you wrote")
            || title.contains("from the `examples:` you wrote"),
        "must name how many seeds came from `examples:`: {title}"
    );
    assert!(
        title.contains("that `Prerelease::new` accepted from random draws during this run"),
        "must name how many seeds the constructor accepted at runtime, not just the ones \
         written by hand: {title}"
    );
    assert!(
        title.contains("evidence about inputs")
            && title.contains("near")
            && title.contains("not about the whole space of text"),
        "must state the limitation as what the evidence means, not as an apology: {title}"
    );
    assert!(
        title.contains("The 64 cases are real and each one ran"),
        "must pre-empt the reasonable suspicion that seeded means fewer than 64 really ran: \
         {title}"
    );

    // The terminal tree carries the same mark, explained once beneath it.
    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("300")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // This is the fixture's second `cargo-ply verify` invocation (the first
    // ran above, via `run_verify`, to read `--json`), so the result is
    // carried forward -- `[seeded, reused]`, not bare `[seeded]`. Either way
    // the mark this test checks for is present.
    assert!(
        stdout.contains("Prerelease::is_empty — fuzzed(64)  [seeded"),
        "the node line must carry the mark: {stdout}"
    );
    assert!(
        stdout.contains("[seeded]") && stdout.contains("random text almost never parses"),
        "the mark must be explained in plain words beneath the tree: {stdout}"
    );
}

/// The honesty condition CLAUDE.md names outright: a seeded verdict must
/// never be indistinguishable from an unseeded one, in either direction.
/// `narrowctor` (a receiver constructor gated on a plain `u64`, nothing
/// text-shaped at all) must carry no seeded status and no seeded
/// diagnostic, even though it earns the same `fuzzed(64)` verdict this
/// fixture does and even trips the same high-rejection warning code path.
#[test]
fn an_unseeded_high_rejection_run_carries_no_seeded_status() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("narrowctor");

    let run = run_verify(&cargo_ply, fixture.path(), 120);
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
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !statuses.contains(&"seeded"),
        "a constructor gated on a plain integer has nothing to do with text seeding -- carrying \
         the status here would make a seeded run indistinguishable from one that never was: {fn_node}"
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.iter().any(|d| d["code"] == "W0523"),
        "must not emit the seeded-generation diagnostic for a fn nothing seeded: {}",
        run.json
    );
}

/// Reuse must not lose the status: a verdict resting on seeded evidence
/// still says so on the second run, even though nothing re-ran.
#[test]
fn a_reused_seeded_verdict_still_carries_the_status() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("textseeded");

    let first = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(
        first.json["root"]["verdict"], "fuzzed(64)",
        "envelope: {}",
        first.json
    );

    let second = run_verify(&cargo_ply, fixture.path(), 300);
    let fn_node = &second.json["root"]["children"][0]["children"][0];
    assert_eq!(
        fn_node["reused"], true,
        "nothing changed, so the second run must carry the first one's result forward: {}",
        second.json
    );
    let statuses: Vec<&str> = fn_node["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        statuses.contains(&"seeded"),
        "a reused verdict that quietly dropped what its evidence rests on would be a worse \
         report than no reuse at all: {fn_node}"
    );

    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("300")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("Prerelease::is_empty — fuzzed(64)  [seeded, reused]"),
        "the terminal must still show the mark on a carried-forward result: {stdout}"
    );
}
