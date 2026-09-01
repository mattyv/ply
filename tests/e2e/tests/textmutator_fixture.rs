//! The fourteenth false clean, closed. See `tests/fixtures/textmutator/`
//! for the history; the short version is that `Acc::get` promises its
//! result is always 0, that promise is false after one call to the only
//! operation that changes the type, and Ply could not call that operation
//! because it took borrowed text.
//!
//! Its sibling `excludedop_fixture.rs` is the same shape with a parameter
//! Ply still cannot build, and asserts the opposite: a run that genuinely
//! cannot reach the broken state says so rather than reporting a pass.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_mutator_taking_text_is_now_reachable_and_the_false_promise_is_caught() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("textmutator");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`Acc::get`'s promise is false once `note` has been called, and `note` takes text, \
         which Ply can build now -- so this run must find the violation by running cases \
         rather than reporting the clean pass it used to: {}",
        run.json
    );

    let named_note = run.json["diagnostics"]
        .as_array()
        .map(|ds| {
            ds.iter()
                .any(|d| d["title"].as_str().is_some_and(|t| t.contains("Acc::note")))
        })
        .unwrap_or(false);
    assert!(
        named_note,
        "the run must still say which operations it used to build the receiver, so a reader \
         can tell how the broken state was reached: {}",
        run.json
    );
}
