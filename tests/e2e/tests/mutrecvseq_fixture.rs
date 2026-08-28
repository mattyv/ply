//! Adversarial review, docs/review-caveats.md N3: "the twelfth false
//! clean". A type mutating through `&mut self` -- most Rust types -- with a
//! promise false after two ordinary calls must be caught, not reported as a
//! clean pass. Before the fix, Ply's bounded operation sequence only ever
//! pooled `&self` operations whose own parameters matched the checked
//! method's exact shape, so `Acc::add` (`&mut self`, one `u32` parameter)
//! could never be spliced in before `Acc::get` (`&self`, no parameters) --
//! every generated case called `get` on a freshly constructed, never-
//! mutated `Acc`, and the false promise (`< 5`, broken the moment two
//! `add(3)` calls push the total to 6) reported a clean `fuzzed(256)`
//! forever.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn a_promise_false_after_two_ordinary_mut_self_calls_is_caught_not_green() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("mutrecvseq");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`Acc::get`'s promise (`< 5`) is false after two ordinary `add` calls -- a receiver \
         sequence that can build a `&mut self` operation into its history must find it: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let violation = diagnostics
        .iter()
        .find(|d| d["node_id"] == "mutrecvseq::Acc::get" && d["severity"] == "error")
        .unwrap_or_else(|| panic!("no violation diagnostic: {}", run.json));
    assert_eq!(violation["code"], "W0541");

    // The disclosure must actually name `add` as part of the pool it drew
    // from -- not just claim a bound that was never really exercised (N3's
    // whole point: the sentence must never claim coverage that did not
    // happen).
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "mutrecvseq::Acc::get" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence disclosure: {}", run.json));
    let title = disclosure["title"].as_str().unwrap();
    assert!(
        title.contains("Acc::add"),
        "the disclosure must name `Acc::add` as an operation the sequence could call -- \
         otherwise the honest reading is that nothing here could ever change state: {title}"
    );
}
