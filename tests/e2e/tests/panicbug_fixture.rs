//! A body that panics is a real bug, and Ply must be able to say so.
//!
//! The 2026-08-24 M4 review's D6 fixed half of this: the adapter used to
//! label the node `violation` while carrying no witness, which §5.4c's MUST
//! forbids outright. The fix at the time was to report `X0901`/`tool_error`
//! instead -- honest, but it meant a genuine crash bug could **never** be
//! reported as a violation at any seed, because Ply's own failing-input
//! marker prints only from the postcondition arm and a panic skips it
//! (docs/review-post-004-strategy.md's correction to vetting 004's finding
//! 4: the fuzz tier's only two answers for this bug were "all green" and
//! "Ply's harness had a problem").
//!
//! proptest catches the panic, shrinks it, and prints the minimal failing
//! input in its own report -- which the adapter was discarding. It now reads
//! that report, so this fixture earns a `violation` **with** a witness. The
//! MUST is unchanged: no witness, no violation. What changed is that the
//! witness was there all along.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_panicking_body_earns_a_violation_with_the_input_that_crashes_it() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("panicbug");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "a body that panics on a legal input has broken its promise, and Ply can now show \
         the input: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "P0502", "envelope: {}", run.json);
    assert!(
        !diag["counterexample"].is_null(),
        "§5.4c MUST: a violation without a witness is exactly the report this project \
         exists to prevent: {diag}"
    );
    let x = diag["counterexample"]["inputs"]["x"].as_str().unwrap();
    let x: u64 = x.parse().unwrap_or_else(|_| panic!("input `x` was `{x}`"));
    assert_eq!(
        x % 2,
        1,
        "the witness must be an input that really does crash `halves` -- it panics on odd \
         numbers, and proptest shrinks to the smallest: {diag}"
    );

    let title = diag["title"].as_str().unwrap();
    assert!(
        title.contains("does not return at all for this input"),
        "the diagnostic must say what actually happened, in plain words (newbie bar): {title}"
    );
    assert!(
        title.contains("panicked before its postcondition"),
        "and why Ply's own marker never printed: {title}"
    );

    // The seed is on the node, so this exact run can be replayed (§1, §8).
    let fn_node = &run.json["root"]["children"][0]["children"][0];
    let seed = fn_node["evidence"]["seed"].as_str().unwrap();
    assert_eq!(seed.len(), 64, "node: {fn_node}");
    // ...and `cases` is *absent*, deliberately (changed 2026-08-25,
    // adversarial review D5). This assertion used to read `cases == 256`,
    // which described a run that did not happen: proptest stops at the first
    // failing case and shrinks from there, so a violation never reaches the
    // declared count. The number asked for is still on the diagnostic's
    // `check` field (`fuzz(256)`); what the envelope must not do is report it
    // as a count the engine reached.
    assert!(
        fn_node["evidence"]["cases"].is_null(),
        "a violation stopped the run early -- the declared count is not a count of cases run: \
         {fn_node}"
    );

    assert_eq!(run.exit_code, Some(1), "a violation fails the run (§6)");
}
