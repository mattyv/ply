//! `checks: []` means "check nothing" (The-Ply-Spec.md §5.4c).
//!
//! It used to mean the opposite. `verify` tested the list for emptiness and,
//! finding it empty, applied the shape-aware default -- so a contracted
//! function written `checks: []` was proved anyway, while `cargo ply check`
//! and the diagram read the same line as claiming nothing. A person who
//! wrote "do not check this" got a green proof they never asked for; a
//! person who meant "use the default" and wrote an empty list to say so had
//! no way to tell the two apart.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn an_empty_checks_list_runs_nothing_and_a_missing_one_still_runs_the_default() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("emptychecks");

    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let fns = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no fn nodes: {}", run.json));
    let verdict = |id: &str| -> String {
        fns.iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("no node `{id}`: {}", run.json))["verdict"]
            .as_str()
            .unwrap()
            .to_string()
    };

    assert_eq!(
        verdict("declared_unchecked"),
        "unclaimed",
        "an empty checks list asked for nothing, so nothing was checked and the node must say \
         so: {}",
        run.json
    );
    assert_eq!(
        verdict("left_to_the_default"),
        "bounded(2)",
        "a fn with no checks list at all still gets the check Ply picks from its shape -- the \
         empty-list rule must not swallow the default: {}",
        run.json
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "a run that checked nothing about a claimed function is not a clean run: {}",
        run.json
    );
}

/// Silence is the defect this whole tranche is about: an empty list must
/// produce a sentence, not merely a node nobody expands.
#[test]
fn the_empty_list_says_in_words_that_nothing_ran() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("emptychecks");

    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("90")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = unwrapped(&String::from_utf8_lossy(&out.stdout));

    assert!(
        stdout.contains(
            "`declared_unchecked` has an empty `checks:` list, so nothing was run against it and \
             it earned no evidence: an empty list means \"check nothing\", not \"use the \
             default\". Deleting the `checks:` line entirely would run `bounded(2)`, the check \
             Ply picks from this function's shape. Write the checks you want to run it; leave \
             the list empty to record a function you have deliberately not checked, and its \
             verdict stays `unclaimed` — Ply's word for \"nothing was checked here\". (W0515, \
             §5.4c)"
        ),
        "{stdout}"
    );
}
