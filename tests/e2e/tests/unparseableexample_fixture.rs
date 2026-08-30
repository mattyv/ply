//! Regression e2e for the external review of 2026-08-30, whose blocker was
//! the worst kind of defect this project can have: a **green lie that
//! persists**.
//!
//! An `examples:` entry that is not a Rust expression died in `syn`, and the
//! error was discarded by an `if let Ok(...)` while the harness was built.
//! The malformed example was silently dropped, the sound one ran, and the
//! claim earned `tested` — exit 0, no diagnostics — after which the result
//! was recorded in `ply.lock` and reused on the next run. The generator's
//! own doc comment promised the opposite ("never a silently skipped
//! example") for as long as the call site had broken it.
//!
//! Two things make this worse than an ordinary bug. A typo'd example is the
//! one assertion the author wrote out by hand, so it is exactly the check
//! they most wanted. And the examples' *text* is a fingerprint input, so the
//! broken entry was faithfully hashed into a verdict it never contributed
//! to — the bad evidence did not merely appear, it was cached and served
//! again.
//!
//! Sibling to `badexample`, which covers the entry that parses and then
//! fails to compile. That one never reaches this bug: it dies in the
//! compiler, which Ply already reported honestly.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn an_example_that_cannot_be_parsed_stops_the_claim_earning_anything() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("unparseableexample");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let verdict = run.json["root"]["verdict"].as_str().unwrap_or("");
    assert_ne!(
        verdict, "tested",
        "one of this fn's two examples was never compiled or run, and the verdict says it was \
         tested. That is a claim earning evidence from a check that does not exist: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert!(
        !diagnostics.is_empty(),
        "a run that silently dropped one of the author's own assertions must say so: {}",
        run.json
    );
    // The machine-readable identity has to be E0501, not a title substring.
    // A reader's tool matches on `code`, never on `title` text -- and this
    // fixture's own bug used to be exactly that gap: the diagnostic's real
    // `code` was `V0507` (a code no documentation names, `severity:
    // "warning"` for something that refuses the claim and exits non-zero,
    // and `open_item: "unsupported_signature"`, which is false -- the
    // signature is fine, the document is malformed) while "E0501" only ever
    // appeared inside the human-readable `title`. A test that checks the
    // title substring passes on both the right diagnostic and the wrong
    // one, which is why this fixture's regression survived undetected.
    let malformed = diagnostics
        .iter()
        .find(|d| {
            d["title"]
                .as_str()
                .is_some_and(|t| t.contains("`add_small(`"))
        })
        .unwrap_or_else(|| {
            panic!(
                "no diagnostic quotes the offending entry — with two examples on one function, a \
                 reader cannot otherwise tell which one is malformed: {}",
                run.json
            )
        });
    assert_eq!(
        malformed["code"], "E0501",
        "the machine-readable code must be E0501 (the documented code for an unparseable \
         contract expression), not whatever refused-anchor code the tool happens to reuse: {}",
        run.json
    );
    assert_eq!(
        malformed["severity"], "error",
        "refusing a claim and exiting non-zero is an error, not a warning: {}",
        run.json
    );

    assert_ne!(
        run.exit_code,
        Some(0),
        "a refused claim must not exit 0: {}",
        run.json
    );
}
