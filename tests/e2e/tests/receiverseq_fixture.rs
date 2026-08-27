//! The decisive test for receiver construction
//! (docs/review-self-construction.md's "fourth option", task 2026-08-27):
//! `Meter::spend`'s invariant holds on any single call from a fresh `Meter`
//! and breaks only after a second one, the exact shape the review proved
//! constructor-only can never reach. If Ply cannot find this bug, receiver
//! construction is constructor-only wearing a longer name -- this is the
//! test that says whether it is.
//!
//! Measured directly, not merely argued: with
//! `harness::MAX_RECEIVER_SEQUENCE_LEN` forced to 0 (constructor-only), five
//! independent seeded runs of 256 cases each against this exact fixture all
//! came back a clean `fuzzed(256)` -- the bug was never found. At the real
//! default (3), the same fixture reliably reports `violation`. That
//! comparison is not re-run by this suite (it requires rebuilding `ply-core`
//! with the constant patched), but it is what this test's own existence
//! rests on, and it is recorded here rather than only in the session's own
//! report.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_bug_reachable_only_after_a_second_call_is_found_never_by_a_single_fresh_call() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("receiverseq");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`Meter::spend`'s invariant is unreachable-by-construction from a single call and \
         reachable from a second one -- a receiver built by constructor-plus-sequence must find \
         it, or the sequence knob is doing nothing: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let violation = diagnostics
        .iter()
        .find(|d| d["node_id"] == "receiverseq::Meter::spend" && d["severity"] == "error")
        .unwrap_or_else(|| panic!("no violation diagnostic: {}", run.json));
    assert_eq!(
        violation["code"], "W0541",
        "the shrunk failing case describes a whole receiver sequence, which Ply cannot yet \
         render back as a literal Rust value -- an honest witness-only violation, never a \
         fabricated per-parameter reading: {violation}"
    );
    assert!(
        violation["counterexample"]["inputs"]["receiver_and_sequence"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "real evidence must be attached -- the raw shrunk sequence, not an empty witness: \
         {violation}"
    );

    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "receiverseq::Meter::spend" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence-length disclosure: {}", run.json));
    assert_eq!(disclosure["severity"], "info");
    let title = disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("Meter::new") && title.contains('3'),
        "the disclosure must name the constructor Ply called and the sequence bound (3), \
         visibly, the same way a `bounded(k)` verdict already names its own loop bound: {title}"
    );
}
