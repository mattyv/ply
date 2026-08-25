//! Closes docs/m4-findings.md's own NOT RUN item and the 2026-08-24 M4
//! review's D7/O2/O4 in one fixture: a real violation on the Kani-excluded
//! shape (`BTreeSet<u8>`), reported witness-only because Ply has no way to
//! write a `BTreeSet` out as a Rust literal.
//!
//! Three things this pins that nothing else did. (1) The milestone's
//! headline shape can actually *catch* a bug, not just pass cleanly. (2)
//! `W0541`'s wording is true for the case that triggers it -- it used to
//! tell a user whose type is `BTreeSet<u8>` that the problem was types
//! "other than u8", and then (2026-08-25) to tell a user with any of the
//! newly admitted shapes about `BTreeSet`s and `Vec`s they do not have. (3) Shrinking is load-bearing: the bug fires for any
//! set containing 3, so an unshrunk witness would be some larger set, and
//! only real shrinking reports `[3]`.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_violation_on_a_btreeset_is_reported_witness_only_with_the_shrunk_input() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("btreesetbug");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "envelope: {}",
        run.json
    );
    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "envelope: {}", run.json);
    let diag = &diagnostics[0];
    assert_eq!(diag["code"], "W0541", "envelope: {}", run.json);
    assert_eq!(diag["engine"], "proptest");
    assert_eq!(diag["open_item"], "inputs_unrenderable");

    // The witness is real, and it is the *minimal* one: the bug fires for
    // any set containing 3, so this value can only come from shrinking.
    assert_eq!(
        diag["counterexample"]["inputs"]["xs"], "[3]",
        "proptest must shrink the failing set to the one element that matters: {}",
        run.json
    );
    // Witness-only: no rendered test is claimed, and none is written.
    assert!(
        diag["counterexample"]["cargo_test"].is_null(),
        "no renderer exists for this shape: {diag}"
    );
    assert!(
        !fixture.path().join("src/ply_generated_cex.rs").exists(),
        "no cex test may be written for a shape Ply cannot spell"
    );

    let title = diag["title"].as_str().unwrap();
    assert!(
        title.contains("no way yet to write parameter(s) `xs: BTreeSet<u8>` back out as a literal"),
        "the diagnostic must be true for the type that triggered it, and say which parameter \
         blocked the rendering -- it once told a user whose type is `BTreeSet<u8>` that the \
         problem was types \"other than u8\", and after the 2026-08-25 fragment widening it told \
         a user with a `[u32; 4]` about `BTreeSet`s they do not have (newbie bar, exact \
         wording): {title}"
    );
    assert!(
        title.contains("`|result|*result == xs.len() as u32`"),
        "the diagnostic must quote the contract the way the user wrote it: {title}"
    );
    assert!(
        title.contains("Ply never invents one"),
        "the diagnostic must say why there is no runnable test here: {title}"
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "a violation fails the run (§6: exit 1)"
    );
}
