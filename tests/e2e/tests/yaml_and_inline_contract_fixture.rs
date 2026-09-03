//! Adversarial review, 2026-08-31: "deleting the surviving warning outright
//! survives the whole suite -- nothing asserts it fires when inline
//! attributes do exist." That review was about a *warning* saying a ply.yaml
//! contract went unchecked; `envelopecontract`'s `add` is the fixture that
//! carries both halves, an inline `#[ply::ensures]` and a ply.yaml
//! `ensures:`, and nothing asserted anything about it.
//!
//! Since 2026-09-03 both halves are checked (§5.4's "ANDed in"), so the
//! warning is gone and the review's worry moves with it: what could now be
//! deleted without the suite noticing is the *merge*. So that is what this
//! asserts -- both clauses in the contract the node carries, and a verdict
//! that could only come from checking them together.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn both_halves_of_a_split_contract_are_checked() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("envelopecontract");
    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let add = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(add["id"], "add", "expected `add`: {}", run.json);

    let ensures = add["contract"]["ensures"]
        .as_array()
        .unwrap_or_else(|| panic!("`add` carries no promise at all: {}", run.json));
    let texts: Vec<&str> = ensures.iter().filter_map(|e| e.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains(">= a")),
        "the inline attribute has to be in the contract that was checked: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("10_000")),
        "and so does the clause written in ply.yaml -- one of them silently winning is \
         the failure this fixture exists to catch: {texts:?}"
    );

    assert_eq!(
        add["verdict"], "fuzzed(64)",
        "both clauses hold of a saturating add over the bounded inputs the document \
         declares, so checking them together earns a clean verdict. A verdict here that \
         did not depend on the ply.yaml clause is what a deleted merge would look like: {}",
        run.json
    );
}
