//! Defect 2 (2026-08-30, "a documented way of writing contracts is
//! accepted, then silently ignored"): a fn claimed entirely through
//! `ply.yaml`'s `requires:`/`ensures:` -- no `#[ply::requires]`/
//! `#[ply::ensures]` attribute anywhere in the source. `check` resolves the
//! claim fine ("N of N fn claims ... point at a function Ply can find") and
//! used to say nothing more; `verify` then ran none of the declared checks
//! and explained that with two warnings on the same fn that flatly
//! contradicted each other -- one saying the ply.yaml contract "is used ...
//! so this run checked `seven` against its inline attributes only", the
//! other saying "there is nothing to check its result against, so nothing
//! was run". Neither command told the reader the one fact that actually
//! matters: a contract written in ply.yaml is not read for a fn's own
//! checks, only an inline attribute is.
//!
//! A same-day fix (2026-08-30) narrowed the second warning so it never
//! fired unless there *was* an inline attribute -- which reopened the
//! silence for a fn whose `checks:` finds nothing to run at all in a
//! different way (2026-08-31 regression, see `yamlonlycontractexample_fixture`).
//! The real fix is to always say the ply.yaml fact once, worded so it is
//! never false regardless of what else did or did not run -- so `verify`
//! now reports *two* diagnostics for this fixture, and they no longer
//! contradict each other, because neither claims more than it knows.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// The CLI reflows diagnostics to a fixed column width, so where a sentence
/// wraps depends on the tempdir path's length. Collapsing whitespace keeps
/// these assertions exact-string about the words themselves.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run(args: &[&str]) -> (i32, String, String) {
    let cargo_ply = build_cargo_ply();
    let out = std::process::Command::new(&cargo_ply)
        .args(args)
        .output()
        .expect("spawning cargo-ply");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `check` is the command people run first, and it used to give no hint at
/// all that this claim's contract did nothing -- then, once it did, said so
/// on the line reporting the claim as resolved. Since the contract is
/// merged into the function's own checks (2026-09-03), that sentence would
/// be false, so it now says what actually happens: both ways, not one.
/// `seven` has its own `checks:` list here, so this is the "you asked for
/// this to be checked" case, not the boundary-declaration case (see
/// `boundarycontract_check`).
#[test]
fn check_says_a_yaml_contract_is_used_both_ways() {
    let fixture = copy_fixture("yamlonlycontract");
    let (code, stdout, stderr) = run(&["check", fixture.path().to_str().unwrap()]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let flat = unwrapped(&stdout);
    assert!(
        flat.contains("1 of 1 fn claims in this crate point at a function Ply can find"),
        "the claim must still resolve: {flat}"
    );
    assert!(
        flat.contains(
            "1 of them also writes a `requires:`/`ensures:` contract directly in ply.yaml. \
             That contract is used both ways: `verify` checks `seven` against it, alongside \
             any `#[ply::requires]`/`#[ply::ensures]` attribute written on `seven` itself, \
             and a caller of `seven` may assume it at a boundary."
        ),
        "check must name the fact plainly, before verify is ever run: {flat}"
    );
}

/// `verify` used to run none of the declared checks here and explain that
/// with two diagnostics that contradicted each other; then with two that
/// agreed. Both were descriptions of a contract nobody checked.
///
/// Now it is checked (§5.4's "ANDed in", 2026-09-03), so the interesting
/// assertion is not about diagnostics at all: it is that the promise the
/// document makes reaches the verdict.
#[test]
fn a_contract_written_only_in_the_document_is_checked_and_holds() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("yamlonlycontract");
    let run = run_verify(&cargo_ply, fixture.path(), 60);

    let seven = &run.json["root"]["children"][0]["children"][0];
    assert_eq!(
        seven["id"], "seven",
        "expected the one claim this fixture declares: {}",
        run.json
    );
    assert_eq!(
        seven["verdict"], "tested",
        "`seven` returns 7 and the document promises 7. It takes no input, so one call is \
         the whole input space -- checked, and it holds: {}",
        run.json
    );

    let ensures = seven["contract"]["ensures"].as_array().unwrap();
    assert_eq!(
        ensures.len(),
        1,
        "the contract Ply checked, once. Listing the document's clause and the merged text \
         both put every clause in twice: {ensures:?}"
    );
    assert!(
        ensures[0].as_str().unwrap().contains("== 7"),
        "and it is the clause the document wrote: {ensures:?}"
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    for d in diagnostics
        .iter()
        .filter(|d| d["node_id"] == "yamlonlycontract::seven")
    {
        let title = d["title"].as_str().unwrap_or("");
        assert!(
            !title.contains("nothing to check its result against")
                && !title.contains("not** yet ANDed"),
            "nothing may say the contract went unchecked: {title}"
        );
    }
}
