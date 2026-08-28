//! docs/review-silent-narrowing.md finding 1, 2026-08-28, extended by the
//! coordinator's own follow-up review of that fix: `Till`'s mutating
//! operations are spread across five files, one per ordinary spelling a
//! real crate uses to write "this `impl` is for the type declared over
//! there" -- the bare name after a `use`, `super::Till`, `crate::till::Till`,
//! `self::Till`, and a `use ... as` alias. The first fix caught only the
//! bare spelling; every qualified/aliased one fell straight through as "not
//! this type" and stayed invisible while the run kept claiming "nothing
//! here was assumed". `total`'s promise (always 0, false after any one of
//! the five real mutators runs) must now be caught regardless of which
//! spelling wrote the mutator that broke it -- one fixture, one test, so
//! the five spellings can never quietly drift apart into five different
//! answers.
//!
//! Unlike the trait-method and second-constructor cases (their own
//! fixtures), every one of these five is genuinely fixable rather than
//! merely disclosable: an ordinary `impl` block reached by any of these
//! spellings is code Ply's own scan can resolve to the real declaration.
//! So the acceptance bar here is stronger than "named honestly" -- the run
//! must actually FIND the violation, and the disclosure must show every
//! one of the five operations confirmed into the pool, none of them
//! excluded.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

#[test]
fn every_ordinary_spelling_of_the_same_impl_is_pooled_and_the_false_promise_is_caught() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("crossfilemutator");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // The whole point: this is not a disclosure case, it is a caught bug,
    // for every spelling at once. A pool that missed even one of the five
    // mutators would still likely go red here (the other four alone are
    // enough), so the disclosure text below is what actually proves each
    // spelling individually resolved -- the verdict alone cannot.
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "five real mutators exist, reached by five different ordinary spellings of `impl \
         Till` -- the receiver scan must resolve every one of them to catch `total`'s false \
         promise: {}",
        run.json
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "a real violation must fail the run: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let sequence_disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "crossfilemutator::till::Till::total" && d["code"] == "W0520")
        .unwrap_or_else(|| panic!("no W0520 sequence disclosure: {}", run.json));
    let title = sequence_disclosure["title"].as_str().unwrap();

    // Every one of the five spellings' own mutator must be in the
    // *confirmed* pool -- named as something this run called, not as
    // something excluded.
    for (op, spelling) in [
        (
            "Till::bare_bump",
            "the bare name after `use crate::till::Till;`",
        ),
        ("Till::super_bump", "`impl super::Till`"),
        ("Till::crate_bump", "`impl crate::till::Till`"),
        ("Till::self_bump", "`impl self::Till`"),
        ("Till::alias_bump", "`use crate::till::Till as T; impl T`"),
    ] {
        assert!(
            title.contains(op),
            "the pool must have actually included the mutator reached via {spelling} ({op}), \
             not merely found it and excluded it: {title}"
        );
    }

    // And none of the five should have landed in the excluded-operations
    // clause, or read as unconfirmed -- every spelling here is resolvable,
    // and a run that could resolve them but still hedged would be reporting
    // less confidence than it earned.
    assert!(
        !title.contains("could not confirm") && !title.contains("also be changed by"),
        "every one of the five spellings is resolvable to the same `Till`; none of them should \
         appear as an unresolved exclusion: {title}"
    );
    assert!(
        title.contains("nothing else was assumed"),
        "with every real mutator confirmed into the pool (none excluded, none unconfirmed), \
         this run's own coverage claim should be the unqualified one: {title}"
    );
}
