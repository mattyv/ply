//! A recorded result is only as good as the hash beside it, and that hash
//! has to cover the code the check actually runs (The-Ply-Spec.md §5.2a).
//!
//! Until 2026-08-25 it did not. It covered the checked function's own
//! tokens and the promises declared for callees a proof was allowed to
//! replace -- and nothing else. So a function with a contract, calling a
//! plain local helper, could have that helper broken and still report a
//! confident carried-forward pass in a thirtieth of a second, over code a
//! cold run proves is in violation. Every fixture in the branch that built
//! the feature used only functions with no helpers, or helpers behind a
//! declared promise: the one shape the hash did cover. That is why 284
//! green tests said nothing.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// The reproduction, as a test. Break the helper the check runs; the claim
/// must be checked again, and must report the violation that is really
/// there.
#[test]
fn breaking_a_helper_the_check_runs_re_earns_the_claim_and_finds_the_bug() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusehelper");

    let first = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &first.json["root"]["children"][0]["children"];
    assert_eq!(fns[1]["id"], "doubled", "envelope: {}", first.json);
    assert_eq!(fns[1]["verdict"], "fuzzed(64)", "envelope: {}", first.json);

    let src = fixture.read_lib_rs();
    let broken = src.replace(
        "pub fn scale(x: u32) -> u32 {\n    x * 2\n}",
        "pub fn scale(x: u32) -> u32 {\n    x / 2\n}",
    );
    assert_ne!(src, broken, "the helper body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[1]["reused"],
        serde_json::Value::Null,
        "the recorded result was about a helper body that is no longer there, so it says \
         nothing about the code as it stands now: {}",
        second.json
    );
    assert_eq!(
        fns[1]["verdict"], "violation",
        "and re-running must find the bug the broken helper really introduces -- a carried \
         forward `fuzzed(64)` here is a green verdict over code nobody checked: {}",
        second.json
    );
}

/// Soundness bought by throwing per-claim reuse away would be a different
/// feature. The claim whose own reachable code did not move keeps its
/// result.
#[test]
fn the_other_claim_whose_code_did_not_move_is_still_carried_forward() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusehelper");

    run_verify(&cargo_ply, fixture.path(), 120);
    let src = fixture.read_lib_rs();
    fixture.write_lib_rs(&src.replace(
        "pub fn scale(x: u32) -> u32 {\n    x * 2\n}",
        "pub fn scale(x: u32) -> u32 {\n    x * 3\n}",
    ));

    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(fns[0]["id"], "bumped", "envelope: {}", second.json);
    assert_eq!(
        fns[0]["reused"], true,
        "`bumped` cannot reach `scale`, so editing `scale` must cost it nothing -- a hash over \
         the whole crate would re-pay every claim for one edit: {}",
        second.json
    );
}

/// Nothing moved, so nothing is re-earned: the case the record exists for
/// still works with the reachable code in the hash.
#[test]
fn a_second_run_over_untouched_code_carries_everything_forward() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusehelper");

    run_verify(&cargo_ply, fixture.path(), 120);
    let second = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &second.json["root"]["children"][0]["children"];
    for i in 0..2 {
        assert_eq!(
            fns[i]["reused"], true,
            "nothing changed, so nothing may be re-earned: {}",
            second.json
        );
    }
}

/// A `test` check compiles each worked example into an assertion, so an
/// edited example is a changed check. Editing one used to be invisible:
/// the claim kept its `tested` verdict without the new example ever
/// running.
#[test]
fn editing_a_worked_example_re_runs_the_check_that_asserts_it() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusehelper");

    run_verify(&cargo_ply, fixture.path(), 120);

    let yaml_path = fixture.path().join("ply.yaml");
    let yaml = std::fs::read_to_string(&yaml_path).unwrap();
    let edited = yaml.replace("doubled(2) == 4", "doubled(2) == 5");
    assert_ne!(yaml, edited, "the fixture example must have been rewritten");
    std::fs::write(&yaml_path, &edited).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &run.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[1]["reused"],
        serde_json::Value::Null,
        "the example is what the check asserts, so changing it is changing the check: {}",
        run.json
    );
    assert_eq!(
        fns[1]["verdict"], "violation",
        "and the example that is now false must fail: {}",
        run.json
    );
}

/// A verdict no check in the file could ever have produced did not come
/// from a run of Ply. It is refused, said out loud, and the claim is
/// checked again -- rather than believed forever because a fingerprint
/// nobody touched still matches.
#[test]
fn a_recorded_verdict_those_checks_could_never_earn_is_refused() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusehelper");

    run_verify(&cargo_ply, fixture.path(), 120);

    let lock_path = fixture.path().join("ply.lock");
    let lock = std::fs::read_to_string(&lock_path).unwrap();
    let edited = lock.replace("\"verdict\": \"fuzzed(64)\"", "\"verdict\": \"proved\"");
    assert_ne!(lock, edited, "the stored verdict must have been rewritten");
    std::fs::write(&lock_path, &edited).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let fns = &run.json["root"]["children"][0]["children"];
    assert_ne!(
        fns[0]["verdict"], "proved",
        "sampling cannot earn a proof, so a stored `proved` beside a `fuzz` check must never \
         reach a reader: {}",
        run.json
    );
    let codes: Vec<&str> = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert!(
        codes.contains(&"W0516"),
        "and the run must say the file was edited by something that is not Ply: {}",
        run.json
    );
}

/// A full-price re-run that says nothing about what changed is the exact
/// experience the record exists to end. When a stored result is displaced,
/// the run names the input that displaced it.
#[test]
fn a_result_that_could_not_be_carried_forward_says_which_input_moved() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusehelper");

    run_verify(&cargo_ply, fixture.path(), 120);
    let src = fixture.read_lib_rs();
    fixture.write_lib_rs(&src.replace(
        "pub fn scale(x: u32) -> u32 {\n    x * 2\n}",
        "pub fn scale(x: u32) -> u32 {\n    x * 3\n}",
    ));

    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("120")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        stdout.contains(
            "  Checked again rather than carried forward from an earlier run, because what \
             each one depended on has changed:"
        ),
        "a surprise re-run must explain itself: {stdout}"
    );
    assert!(
        stdout.contains(
            "reusehelper::doubled — the code it runs changed since that result was recorded"
        ),
        "and must name the claim and the input that moved: {stdout}"
    );
}

/// The proof tier's half of the same defect: `bounded` descends into a
/// callee that carries its own contract, so that callee's real body is
/// part of what was proved. Editing it must re-earn the proof.
#[test]
fn editing_a_body_a_proof_descends_into_re_earns_the_proof() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reuseproof");

    let first = run_verify(&cargo_ply, fixture.path(), 300);
    let fns = &first.json["root"]["children"][0]["children"];
    assert_eq!(fns[0]["id"], "outer", "envelope: {}", first.json);
    assert_eq!(fns[0]["verdict"], "bounded(2)", "envelope: {}", first.json);

    let src = fixture.read_lib_rs();
    let edited = src.replace(
        "pub fn inner(x: u32) -> u32 {\n    x * 2\n}",
        "pub fn inner(x: u32) -> u32 {\n    x * 3\n}",
    );
    assert_ne!(src, edited, "the callee body must have been rewritten");
    fixture.write_lib_rs(&edited);

    let second = run_verify(&cargo_ply, fixture.path(), 300);
    let fns = &second.json["root"]["children"][0]["children"];
    assert_eq!(
        fns[0]["reused"],
        serde_json::Value::Null,
        "the proof read `inner`'s body, so a rewritten `inner` is a different proof: {}",
        second.json
    );
}

/// The coarse mode's honest price is that editing one claim re-runs its
/// neighbours. Ply already works out *why* it widened -- it has to, to decide
/// -- and until now kept it. "the code it runs changed" is true and useless
/// when you did not touch anything that claim calls: the sentence a person can
/// act on names the construct that cost them the walk.
#[test]
fn a_run_that_widened_the_walk_says_which_construct_cost_it() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusewiden");

    run_verify(&cargo_ply, fixture.path(), 120);

    let src = fixture.read_lib_rs();
    let edited = src.replace(
        "pub fn bumped(x: u32) -> u32 {\n    x + 1\n}",
        "pub fn bumped(x: u32) -> u32 {\n    x + 2\n}",
    );
    assert_ne!(src, edited, "the edit must have landed");
    fixture.write_lib_rs(&edited);

    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("120")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        stdout.contains("reusewiden::halved"),
        "the untouched claim is re-run under the coarse mode -- that is the whole point \
         of explaining it: {stdout}"
    );
    assert!(
        stdout.contains("declares an `impl` block for `Scaler`"),
        "a person who edited `bumped` and watched `halved` re-run needs the construct \
         named, not just \"the code it runs changed\": {stdout}"
    );
    assert!(
        stdout.contains("means every line of the crate, not only the functions they call"),
        "and needs telling that the whole crate is the unit now, so the re-run is \
         explained rather than merely announced: {stdout}"
    );
    assert!(
        stdout.contains("even an edit in code they never call"),
        "the sentence has to close the loop the person actually noticed -- they edited \
         something these claims do not call: {stdout}"
    );
    assert_eq!(
        stdout.matches("means every line of the crate").count(),
        1,
        "the reason belongs to the crate, not to each claim: repeating the same paragraph \
         once per displaced claim is noise, and on a crate with twenty claims it would \
         bury the list it is explaining: {stdout}"
    );
}

/// The explanation must stay rare. A bounded walk is the ordinary case, and
/// a paragraph about widening printed when nothing widened would train the
/// reader to skip the one that matters.
#[test]
fn an_ordinary_bounded_walk_is_re_run_without_the_widening_paragraph() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("reusehelper");

    run_verify(&cargo_ply, fixture.path(), 120);
    let src = fixture.read_lib_rs();
    fixture.write_lib_rs(&src.replace(
        "pub fn scale(x: u32) -> u32 {\n    x * 2\n}",
        "pub fn scale(x: u32) -> u32 {\n    x * 4\n}",
    ));

    let out = std::process::Command::new(&cargo_ply)
        .args(["verify", fixture.path().to_str().unwrap()])
        .arg("--engine-timeout")
        .arg("120")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(
        stdout.contains("the code it runs changed"),
        "the helper edit must still displace the claim that calls it: {stdout}"
    );
    assert!(
        !stdout.contains("means every line of the crate"),
        "this crate's walk was bounded -- Ply hashed the helper because the claim really \
         does call it, and saying the whole crate was hashed would be false: {stdout}"
    );
}
