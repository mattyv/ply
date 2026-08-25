//! A result Ply already earned is not re-earned when nothing it depended on
//! changed, and the run says which results it carried forward
//! (The-Ply-Spec.md §5.2a, D14).
//!
//! Before this landed, every run re-paid full engine cost: a crate whose
//! proofs took four minutes took four minutes again on a branch that touched
//! a README. Nothing was recorded, so nothing could be reused -- and nothing
//! could be reviewed either, since a diff never showed that a claim had been
//! checked at all.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// Both claims pass on the first run, and both are written into the record
/// beside the fingerprint of everything they stood on.
#[test]
fn a_first_run_records_what_it_earned() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultreuse");

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(run.exit_code, Some(0), "envelope: {}", run.json);

    let lock = fixture.path().join("ply.lock");
    assert!(
        lock.is_file(),
        "a first run must leave a record beside ply.yaml, or nothing can ever be reused: {}",
        run.json
    );
    let text = std::fs::read_to_string(&lock).unwrap();
    let record: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("ply.lock is not readable JSON: {e}\n{text}"));
    assert_eq!(
        record["results"]["resultreuse::safe_increment"]["verdict"], "bounded(2)",
        "the record must carry the verdict this run earned: {text}"
    );
    assert!(
        record["results"]["resultreuse::safe_increment"]["fingerprint"]
            .as_str()
            .is_some_and(|h| h.len() == 64),
        "and the hash of what it stood on: {text}"
    );
}

/// The point of the whole feature: nothing changed, so no engine runs, and
/// every node says it was carried forward rather than re-earned.
#[test]
fn a_second_run_reuses_what_the_first_one_earned_and_says_so() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultreuse");

    let first = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(first.exit_code, Some(0), "envelope: {}", first.json);

    let started = std::time::Instant::now();
    let second = run_verify(&cargo_ply, fixture.path(), 300);
    let elapsed = started.elapsed();

    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(fns[0]["id"], "safe_increment", "envelope: {}", second.json);
    assert_eq!(
        fns[0]["reused"], true,
        "a node whose inputs still hash the same must be carried forward, not re-proved: {}",
        second.json
    );
    assert_eq!(
        fns[1]["reused"], true,
        "including the one standing on a declared promise: {}",
        second.json
    );
    assert_eq!(
        fns[0]["verdict"], first.json["root"]["children"][0]["children"][0]["verdict"],
        "and it must be the same verdict, not a weaker one: {}",
        second.json
    );
    assert!(
        elapsed.as_secs() < 20,
        "a reused run must not pay engine cost; this one took {elapsed:?}"
    );
}

/// The case committing the record exists for: a clone that has the file and
/// none of the build output. Nothing in a fingerprint is keyed on the
/// generated proof module or on `target/`, so a fresh checkout reports the
/// same verdicts having compiled nothing at all.
#[test]
fn a_checkout_with_the_record_and_no_build_output_still_reuses() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultreuse");

    let first = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(first.exit_code, Some(0), "envelope: {}", first.json);

    // Everything a clone would not have: the build directory, the proof
    // module Ply generated, and the line declaring it.
    std::fs::remove_dir_all(fixture.path().join("target")).ok();
    std::fs::remove_file(fixture.path().join("src/ply_generated.rs")).ok();
    let lib = fixture.read_lib_rs();
    fixture.write_lib_rs(
        &lib.lines()
            .filter(|l| !l.starts_with("mod ply_generated;"))
            .collect::<Vec<_>>()
            .join("\n"),
    );

    let started = std::time::Instant::now();
    let clone = run_verify(&cargo_ply, fixture.path(), 300);
    let elapsed = started.elapsed();

    let fns = &clone.json["root"]["children"][0]["children"];
    for i in 0..3 {
        assert_eq!(
            fns[i]["reused"], true,
            "a checkout carrying the record must not re-prove anything: {}",
            clone.json
        );
    }
    assert_eq!(clone.exit_code, Some(0), "envelope: {}", clone.json);
    assert!(
        elapsed.as_secs() < 20,
        "and must not compile anything either; this took {elapsed:?}"
    );
    assert!(
        !fixture.path().join("src/ply_generated.rs").exists(),
        "a run that checked nothing must not write a proof module either"
    );
}

/// Editing the function re-earns *that* claim and leaves the other one
/// alone: the fingerprint is per claim, not per crate.
#[test]
fn editing_a_function_re_runs_that_claim_and_only_that_claim() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultreuse");

    run_verify(&cargo_ply, fixture.path(), 300);

    let src = fixture.read_lib_rs();
    let edited = src.replace(
        "pub fn safe_increment(x: u32) -> u32 {\n    x + 1\n}",
        "pub fn safe_increment(x: u32) -> u32 {\n    let step = 1;\n    x + step\n}",
    );
    assert_ne!(src, edited, "the fixture body must have been rewritten");
    fixture.write_lib_rs(&edited);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let fns = &run.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[0]["reused"],
        serde_json::Value::Null,
        "the edited function's recorded result is about code that is no longer there, so it \
         must be checked again: {}",
        run.json
    );
    assert_eq!(
        fns[0]["verdict"], "bounded(2)",
        "and it must earn its verdict again: {}",
        run.json
    );
    assert_eq!(
        fns[1]["reused"], true,
        "the untouched claim must still be reused -- a per-crate hash would re-pay every \
         proof for one edit: {}",
        run.json
    );
}

/// The input that makes the record worth more than a cache of bodies: a
/// proof standing on a declared promise is *about* that promise, so editing
/// it in ply.yaml must re-run the caller even though no Rust changed.
#[test]
fn editing_only_a_declared_promise_re_runs_the_claim_that_rests_on_it() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultreuse");

    run_verify(&cargo_ply, fixture.path(), 300);

    let yaml_path = fixture.path().join("ply.yaml");
    let yaml = std::fs::read_to_string(&yaml_path).unwrap();
    let edited = yaml.replace("*result <= 10_000", "*result <= 9_000");
    assert_ne!(yaml, edited, "the fixture promise must have been rewritten");
    std::fs::write(&yaml_path, &edited).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let fns = &run.json["root"]["children"][0]["children"];
    assert_eq!(fns[1]["id"], "total", "envelope: {}", run.json);
    assert_eq!(
        fns[1]["reused"],
        serde_json::Value::Null,
        "the recorded result assumed a promise that has since been rewritten, so it says \
         nothing about what is written now: {}",
        run.json
    );
    assert_eq!(
        fns[0]["reused"], true,
        "the claim that assumed nothing is untouched by the edit: {}",
        run.json
    );
}

/// The sampling tier reaches `verify` by a different route from the proof
/// tier -- it generates a whole test crate -- and a run that reuses every
/// sampled result must not write or compile that crate at all.
#[test]
fn a_sampled_result_is_reused_with_the_run_that_produced_it() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultreuse");

    let first = run_verify(&cargo_ply, fixture.path(), 300);
    let fresh = &first.json["root"]["children"][0]["children"][2];
    assert_eq!(fresh["id"], "widen", "envelope: {}", first.json);
    assert_eq!(fresh["verdict"], "fuzzed(64)", "envelope: {}", first.json);
    assert!(
        fresh["evidence"]["seed"]
            .as_str()
            .is_some_and(|s| s.len() == 64),
        "the run that happened names its seed, or the comparison below is vacuous: {}",
        first.json
    );

    let second = run_verify(&cargo_ply, fixture.path(), 300);
    let reused = &second.json["root"]["children"][0]["children"][2];
    assert_eq!(
        reused["reused"], true,
        "a sampled result whose inputs still hash the same is carried forward too: {}",
        second.json
    );
    assert_eq!(
        reused["evidence"], fresh["evidence"],
        "and it carries the seed and case count of the run that actually happened -- a \
         reused verdict that named no run would be a verdict nobody could repeat: {}",
        second.json
    );
}

/// A reused result reaches the terminal marked as reused, and the mark is
/// explained where it is used -- a marker nobody can decode is decoration.
#[test]
fn the_terminal_says_which_results_were_carried_forward() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("resultreuse");

    run_verify(&cargo_ply, fixture.path(), 300);

    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("300")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        stdout.contains("safe_increment — bounded(2)  [reused]"),
        "the node line must say the result was carried forward: {stdout}"
    );
    assert!(
        stdout.contains(
            "[reused]         this result was not re-run: an earlier run recorded it, and \
             everything it depended on — the code, the promises it assumes, the checks, the \
             engines, Ply's own version — hashes the same today"
        ),
        "and the mark must be explained in plain words: {stdout}"
    );
}
