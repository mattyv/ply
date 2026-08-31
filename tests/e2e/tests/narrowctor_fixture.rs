//! Adversarial review, 2026-08-31: "a fallible constructor's rejection arm
//! can be turned into a vacuous pass and nothing notices". The existing
//! receiver-constructor fixture (`receiverresultctor`) only rejects a
//! single value, and its promise (`*result >= 0` on a `u64`) is true no
//! matter what the constructor does -- so mutating the constructor's
//! rejection arm into `Ok(Self { v })` changes nothing about that fixture's
//! result, and the rejection counter the high-rejection warning depends on
//! silently reads zero instead of anything real.
//!
//! `Narrow::new` here rejects most of its input domain (`v > 3`, against a
//! generator drawing mostly from `0..=16`), and `doubled`'s promise is not
//! vacuous -- it holds only because the constructor really is narrowing the
//! receiver. This is the fixture that lets the high-rejection warning
//! (`W0503`) actually fire and be asserted on.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_constructor_rejecting_most_inputs_earns_the_high_rejection_warning() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("narrowctor");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(64)",
        "the promise genuinely holds for every accepted case -- this must be real evidence, \
         not a violation or an abort: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let warning = diagnostics
        .iter()
        .find(|d| d["code"] == "W0503" && d["open_item"] == "high_rejection_rate")
        .unwrap_or_else(|| {
            panic!(
                "expected the high-rejection warning to fire when most draws are thrown away: {}",
                run.json
            )
        });
    let title = warning["title"].as_str().unwrap();
    assert!(
        title.contains("Narrow::doubled"),
        "must name the fn the warning is about: {title}"
    );
    assert!(
        title.contains("draws rejected"),
        "must say how many draws were thrown away: {title}"
    );
}
