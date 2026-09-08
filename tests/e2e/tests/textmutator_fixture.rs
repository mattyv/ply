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

/// The recipe Ply prints has to be the recipe that ran (2026-09-08).
///
/// An adversarial review found it was not: a text argument was escaped once
/// on its way into the history and once more on its way onto the marker
/// line, while the reader unescapes only once -- so a call really made with
/// `[` was reported as `note(\[)`, and a call made with the empty string was
/// reported as `note()`, which is a call to a function that takes an
/// argument, with no argument. A wrong recipe is worse than no recipe:
/// someone follows it, cannot reproduce the failure, and concludes the
/// report is noise.
///
/// The fixture is rigged so the promise can only break on text containing
/// one of the characters the marker's own wire format is sensitive to, so
/// the failing case is guaranteed to be one that would expose the bug.
#[test]
fn the_text_a_call_was_made_with_is_reported_as_it_was_written() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("textmutator");

    let src = fixture.read_lib_rs();
    let rigged = src.replace(
        "    pub fn note(&mut self, _s: &str) {\n        self.total += 5;\n    }",
        "    pub fn note(&mut self, _s: &str) {\n        if _s.contains(';') || _s.contains('[') || _s.contains('=') {\n            self.total += 5;\n        }\n    }",
    );
    assert_ne!(src, rigged, "the mutator must have been rigged");
    fixture.write_lib_rs(&rigged);

    let run = run_verify(&cargo_ply, fixture.path(), 120);
    let history = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|d| d["counterexample"]["receiver_history"].as_str())
        .unwrap_or_else(|| panic!("the run must report how the value got there: {}", run.json));

    let arg = history
        .rsplit_once("Acc::note(")
        .map(|(_, rest)| rest.trim_end_matches(')'))
        .unwrap_or_else(|| panic!("the history must name the call that broke it: {history}"));

    assert!(
        arg.starts_with('"') && arg.ends_with('"'),
        "text has to be written the way it would be written in Rust -- quoted -- or an empty \
         string reads as no argument at all: {history}"
    );
    let inner = &arg[1..arg.len() - 1];
    assert!(
        inner.contains(';') || inner.contains('[') || inner.contains('='),
        "the rigged mutator only changes anything on text containing one of those characters, \
         so the reported text has to contain one -- if it does not, the report is describing a \
         call that would not have broken anything: {history}"
    );
    assert!(
        !inner.contains(r"\;") && !inner.contains(r"\[") && !inner.contains(r"\="),
        "and it must not carry the marker format's own escaping -- that is the double-escaping \
         bug, and it makes the printed text differ from the text the call was made with: \
         {history}"
    );
}
