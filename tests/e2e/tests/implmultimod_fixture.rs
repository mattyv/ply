//! The blocker this project's ninth adversarial review found (2026-08-27):
//! Ply used to decide which function a promise is ABOUT by reading `impl`
//! blocks, and separately decide which function a generated harness would
//! CALL by re-spelling the claim's own key into a path. Nothing tied those
//! two together, so a claim could read one function's contract and run a
//! different function's body -- reported as a clean pass.
//!
//! Every fixture that exercised method resolution before this
//! (`implmethod`, `implambiguous`) was a single file with everything
//! public and no modules at all -- the one arrangement where the path Ply
//! writes back out is guaranteed to be the path it read in, so none of
//! them could have caught this. This fixture is the missing shape: more
//! than one module, two same-named types, an `impl` written from inside a
//! submodule for its *parent's* type, a type re-exported under another
//! name, an `impl` in a different file from its own type, and an
//! ambiguity that spans two files instead of one.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn node<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["root"]["children"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("no fn nodes: {json}"))
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node `{id}`: {json}"))
}

fn diag_for<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    let qualified = format!("implmultimod::{id}");
    json["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("no diagnostics: {json}"))
        .iter()
        .find(|d| d["node_id"] == qualified.as_str())
        .unwrap_or_else(|| panic!("no diagnostic for `{qualified}`: {json}"))
}

/// The blocker itself: the promise "the answer is 999" is written on the
/// CRATE-ROOT `Root` (`impl super::Root` inside `inner.rs`, which names
/// `crate::Root` -- `super::` from inside `inner` is the crate root). The
/// only spelling that reaches this function is the unqualified `Root::five`.
/// Checking it must report the real violation -- the function's own body
/// returns 5, not 999 -- never a clean pass.
#[test]
fn the_function_that_actually_carries_the_promise_is_checked_and_found_violating_it() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmultimod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "Root::five");
    assert_eq!(
        n["verdict"], "violation",
        "`Root::five`'s own promise says the answer is 999; its body returns 5. This must be \
         reported as a violation, not a clean pass borrowed from an unrelated function: {}",
        run.json
    );
}

/// The other half of the same reproduction: `inner::Root::five` is the
/// literal, real-Rust name of a DIFFERENT function -- `inner::Root`'s own
/// `five`, declared in `inner/sub.rs`, which carries no promise at all.
/// Before this fix, this exact spelling resolved to the crate-root
/// function above instead and reported its (false) promise as holding.
/// Whatever this run reports about `inner::Root::five`, it must never be a
/// clean pass of a promise this function does not carry.
#[test]
fn the_wrong_spelling_never_borrows_a_false_promise_from_a_different_function() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmultimod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "inner::Root::five");
    assert_ne!(
        n["verdict"], "fuzzed(16)",
        "`inner::Root::five` carries no `#[ply::ensures]` of its own -- a clean `fuzzed(16)` \
         here would mean the crate-root `Root::five`'s promise got attached to this function \
         instead, which is exactly the false pass this fixes: {}",
        run.json
    );
    assert_ne!(
        n["verdict"], "violation",
        "a violation here would report a promise this function was never given: {}",
        run.json
    );
}

/// A correct claim in a multi-module crate, with its `impl` block in a
/// different file from its type's own declaration -- the ordinary,
/// non-adversarial case this fix must not break.
#[test]
fn a_correct_claim_across_two_files_resolves_and_checks() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmultimod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "widgets::Widget::three");
    assert_eq!(
        n["verdict"], "bounded(2)",
        "`Widget` is declared in one file and its `impl` sits in another -- real Rust -- and \
         this claim must resolve and check normally: {}",
        run.json
    );
    assert!(
        !run.json["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["node_id"] == "implmultimod::widgets::Widget::three"),
        "a clean verdict must carry no diagnostic at all: {}",
        run.json
    );
}

/// A type re-exported under another name still resolves its methods to the
/// same declaration the real name would reach.
#[test]
fn a_type_re_exported_under_another_name_resolves_and_checks() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmultimod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "ExportedWidget::four");
    assert_eq!(
        n["verdict"], "bounded(2)",
        "`ExportedWidget` is a `pub use` re-export of `widgets::Widget` -- the claim must \
         resolve to `widgets::Widget::four`'s real declaration and check normally: {}",
        run.json
    );
}

/// Whatever the canonical-path work makes newly refusable, refused by
/// name: two `impl` blocks for one type, in DIFFERENT files, defining the
/// same method -- exactly as ambiguous as two in one file (`implambiguous`
/// already pins that shape), and this crate-wide resolver must refuse
/// rather than silently pick whichever file its walk reaches first.
#[test]
fn two_impl_blocks_for_one_type_in_different_files_are_refused_as_ambiguous() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("implmultimod");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let n = node(&run.json, "pairs::Pair::describe");
    assert_eq!(
        n["verdict"], "unsupported",
        "two impl blocks defining the same method for the same type, even split across two \
         files, must be refused rather than silently resolved to one of them: {}",
        run.json
    );
    let d = diag_for(&run.json, "pairs::Pair::describe");
    assert_ne!(
        d["code"], "E0301",
        "\"could not find\" is false -- Ply found two real candidates, not zero: {d}"
    );
    let title = d["title"].as_str().unwrap();
    assert!(
        title.contains("Pair") && title.contains("describe"),
        "the diagnostic must name the claim: {d}"
    );
    assert!(
        title.to_lowercase().contains("does not name one function")
            || title.to_lowercase().contains("ambiguous")
            || title.contains('2'),
        "the diagnostic must say plainly that more than one candidate matched: {d}"
    );
}
