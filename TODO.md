# TODO

## The acceptance test ran: `semver` reach moved from 1 of 16 to 4 of 16 — 2026-09-01

Measured, not inferred: every property below was written as a real promise on the real
function and run. Full write-up in `docs/reach-measurement-2.md`; the vendored copy it was
run against is at `/home/user/semvercheck` (outside this repository).

Newly reachable, each `fuzzed(64)`: parse rejects whitespace; an accepted identifier is
stored verbatim; at most 32 comparators. The check was proved to bite — a deliberately
false promise came back `violation` with a shrunk failing input.

**The honest caveat, and it is a big one: three of the four are checked almost entirely on
inputs the function rejects**, because random text essentially never parses. The evidence
is real; the author's rules about what is *accepted* are barely exercised. Ply prints
`fuzzed(64)` unqualified and does not say this.

- [ ] **Ply does not disclose that a run took the reject path nearly every time.** It
      already has the vocabulary (`narrower than it looks`, `seeded`) and does not reach for
      it here. Writing `examples:` does not help either: seeding only engages where the
      ordinary generator cannot build the value at all, and a plain text parameter is one it
      can — so the escape hatch a user would reach for is silently inert. This is the same
      species of gap the `seeded` status was invented to close, in the one place it does not
      apply.
- [ ] **A promise comparing non-numeric values with `==` or `!=` does not compile.** The
      generated harness casts both sides of a comparison to `i128` (so it can report a
      broken promise rather than overflow while checking one); against a string, an
      `Option` or a struct that cast is invalid — `error[E0606]: casting &str as i128 is
      invalid`. Reported honestly as a tool error, never as a pass. Not new, but far more
      reachable now the return-type gate no longer refuses these functions first. This is
      what blocks the natural phrasing of property 15.
- [ ] **`Self` as a parameter spelling is refused where the same type by name is checked.**
      `cmp_precedence(&self, other: &Self)` is `unsupported`; change nothing but `&Self` to
      `&Version` and it is `fuzzed(64)`. Mirror image of this document's original headline,
      which turned on the author having typed `-> Self` rather than `-> Version`. Property 6
      is reachable in substance and unreachable as written.

Still blocked and untouched today: trait methods (properties 1, 2, 3, 4, 7, 12), and a
`VersionReq` or `Comparator` built from text in a parameter position (5, 8, 9, 10, 11).

## Fixed: a declaration-only render no longer calls its own run clean — ffacd9b, 2026-09-01

`cargo ply render --json` (the editor-facing envelope behind semantic focus) built a tree
in which every item is `unclaimed` — nothing has been checked — and then reported
`"outcome": "clean"`. A plugin colouring a badge from that field would have shown green for
a document no run has ever looked at.

The builder now derives the outcome from the tree it constructs rather than trusting the
caller, so it reports `missing_evidence`. Two tests, both watched to fail against the old
code: an end-to-end one over the real CLI that checks its own premise (all six items still
unclaimed) before asserting the outcome, and a unit test that hands the builder `clean` and
requires it back as `missing_evidence`.

## Measured: the return-type gate can come off, and what it hides — 2026-09-01

Fable's ranking put this first and it is not a declaration at all: the gate refusing a
function because Ply cannot *construct* its return type blocks 10 of the 16 properties in
`docs/reach-measurement-2.md`, and the gate's own doc comment already concedes it blocks
nothing technically -- *"nothing in this codegen ever names or constructs a return type.
This gate is therefore a deliberate, requested narrowing... on principle."*

Measured rather than taken on the comment's word. With the gate temporarily removed:

- A function returning `std::cmp::Ordering` -- a type Ply models nowhere -- **earns
  `fuzzed(64)`**, and a false promise about it is caught: `!result.is_lt()` on `a.cmp(&b)`
  gives `violation` with a shrunk failing input. So the comment is right, and the refusal
  really is costing real evidence for no technical reason.
- **But removing it exposes a separate defect the gate was hiding.** A contract that *names*
  the return type -- `|result| *result != Ordering::Greater || a > b` -- fails to compile:
  `error[E0433]: cannot find type Ordering in this scope`. The generated harness brings
  parameter types into scope and not types the contract text names.

- [x] **The import defect, fixed** (`cc2121e`). A contract may now name any type the file
      it lives in can see -- `use_aliases_in_file`'s own scan is carried on `ContractFn` and
      resolved against every identifier the contract text references
      (`fuzz_gen::contract_referenced_use_imports`), not only the parameter/receiver types
      `extra_type_imports` already walked. A glob import of the target crate was considered
      and rejected: it cannot reach `std::cmp::Ordering` at all (an external crate's own
      root export list, not the target crate's), and blindly re-emitting every `use` in the
      file risks importing something private for an unrelated reason and breaking a
      neighbour function's harness that never asked for it. Proved both directions: a unit
      test and a new e2e fixture (`tests/fixtures/ensuresimport`) go red with the exact
      `error[E0433]` before the fix and green after; the fixture also proves the catching
      direction (a broken implementation still reports `violation`).
- [x] **The gate decided: off, on both engines, measured** (`51ef480`). Measured directly
      before removing anything, per the maintainer's brief: a function returning
      `std::cmp::Ordering` earns `fuzzed(64)` on the fuzz engine and a genuine `bounded(2)`
      proof on the bounded (Kani) engine, both completing in seconds -- not a timeout
      mislabeled -- and both independently report `violation` with a real witness on a
      broken promise about the same type. Both engines pay this gate's cost for nothing in
      return, so `is_bounded_return_supported`/`is_fuzz_return_supported`
      (`crates/ply-core/src/harness.rs`) now always answer `true`; the history of why the
      gate was added is kept in their doc comments, retracted rather than deleted. §5.4b
      amended in the same change: "parameters and return type" is no longer true --
      the list binds parameters only, and a function's return type is never a reason
      either engine refuses it. New permanent fixture `tests/fixtures/orderingreturn`
      proves the same clean-and-catches pair against a real `cargo ply verify` run on both
      engines together. **Not yet measured: whether this moves the real 16-property count**
      -- that re-measurement is the maintainer's to run, per `docs/reach-measurement-2.md`'s
      own acceptance rule.

## Design principle, from the maintainer — 2026-09-01

**Requiring the user to write a small declaration is cheap, because an agent writes it.**
Stated plainly by the maintainer when seeded generation landed: "one line is one line from
an LLM".

*The example originally cited here has been withdrawn.* It said one `examples:` line took a
real library's function from no evidence to sixty-four real checks; re-measured by hand
against `semver` 1.0.28, that function gets there with no example at all (see the correction
above). The principle is the maintainer's and stands on its own; the measurement that was
offered in support of it did not.

This changes the economics of a decision already recorded. `docs/rule-registry-design.md`
and the seeding design both weighed "adds something new for the user to write and keep true"
as a significant cost, and rejected options partly on those grounds. That cost is lower than
assumed for anything an agent can write from reading the code. It is **not** lower for
things a user must keep true by hand over time -- a declaration that drifts from reality is
still the failure mode that matters, and an agent writing it once does not keep it true.

So the rule is: **prefer a small declaration over inference where an agent can supply it and
Ply can check it against reality.** Prefer inference where the declaration would have to be
maintained by hand and nothing would notice it going stale.

- [x] **An example now unblocks a parameter Ply cannot build, for the shapes whose parts Ply
      already knows how to vary -- 2026-09-01.** `width(label: Option<String>) -> usize` (the
      measured gap, verbatim) now earns `fuzzed(64) [seeded]` from one `examples:` entry --
      `tests/fixtures/paramseeded`. `Vec<String>` opens the same way (elements and length
      both vary), sharing the exact corpus/mutate/trickle apparatus the constructor path
      already built (`fuzz_gen::plan_param_seeding`, `SeedableWrap`) rather than a second one
      -- the two mechanisms are mutually exclusive by construction (this one only ever fires
      for a non-receiver fn), which is what makes reusing the apparatus's own generated
      variable names safe. **Not opened, disclosed rather than attempted:**
      `Result<String, E>`, and nested `NonZero`/`Duration`/`f32`/`f64` inside any wrapper --
      each needs its own construction or mutation story, which `String`'s existing text
      apparatus does not hand over for free (a number has no character-level mutator to
      reuse; a `Result`'s `Err` arm needs its own construction path).

      The counting condition: `plan_param_seeding` refuses (stays `None`, parameter stays
      refused) whenever the type is not one of the two classified shapes, whenever more than
      one parameter is otherwise unbuildable, or whenever no `examples:` entry supplies a
      seed -- an opaque type never borrows the seeded machinery just because an example
      exists. For an opaque type, `examples:` now still unlocks `test` alone (never `fuzz`,
      which cannot grow a case count for it): `generate_example_test`'s own codegen never
      depended on the parameter being buildable in the first place, so the gate widening
      there is a real bug fix, not new machinery -- `tests/fixtures/paramseedopaque` earns
      `tested`, the vocabulary this project had already written down for "a concrete case
      was run and held", never a fabricated `fuzzed(n)`. A new diagnostic, `W0524`, carries
      the growable case's own provenance (parameter name, example count, real case count) the
      way `W0523` already does for the constructor case, worded for its own honesty
      condition: there is no rejection rate to report here (nothing gates an
      `Option<String>`/`Vec<String>` value the way a fallible constructor gates text), so
      every one of the `n` cases genuinely ran.

      The refusal itself was fixed in the other direction too: `V0505`'s message now names,
      per unbuildable parameter, whether an example would actually help (and what to write)
      or would not (an opaque type, told plainly rather than given false hope).

      Found along the way and fixed as necessary plumbing, not scope creep: the generated
      harness module only ever imported types a fn's own resolved parameters referenced, so
      `test`'s newly-unlocked opaque-type path failed to compile (`error[E0433]: cannot find
      type ... in this scope`) the moment an example's own literal source named a type
      nothing else in the fn's signature resolved -- fixed with a glob import of the target
      crate, which an explicit `use` always outranks on a name clash, so no existing
      generated harness resolves any name differently.

      Every fixture that depends on a shape staying unbuildable (`excludedop`, `skippedctor`,
      `textmutator`) and every one depending on the constructor-seeding behaviour
      (`textseeded`, `textseedempty`) still passes unchanged -- none of them declare an
      `examples:` entry for the parameter this widening reaches, which is exactly the gate
      that keeps their premises intact.

## Seeded generation moved a real library's reach — 2026-09-01

The acceptance test Fable named: not a green fixture, but whether a real library's number
moves. It moved.

`semver`'s `Prerelease::is_empty` -- a method whose receiver must be built by parsing text,
the exact shape that produced the dead end:

- **Before:** 1025 of 1074 generated strings rejected, 49 ever checked, verdict `unclaimed`.
  No evidence at all.
- **After:** 43 of 107 rejected, verdict `fuzzed(64) [narrower than it looks, seeded]`.
  Sixty-four real cases, each one run.

**Correction, 2026-09-01, after re-running this by hand against a fresh vendored copy of
`semver` 1.0.28 rather than trusting the earlier record: the `examples:` entry is not what
moved it, and the earlier version of this section said it was.** The same function reaches
`fuzzed(64)` with no `examples:` line at all, and the run says so itself: *"the 64 cases
were grown from 64 known-valid values: 0 from the `examples:` you wrote, 64 that
`Prerelease::new` accepted from random draws during this run."* What did the work is the
free half of the mechanism -- harvesting every value the constructor accepts during the run,
which used to be thrown away. Adding the example changes the verdict not at all.

Both numbers above were re-measured directly, the "before" by building the product as it
stood at 83949d6 and running it against the same crate, so the delta is real. Only the
attribution was wrong.

This matters beyond bookkeeping: the design principle recorded below was written on the
belief that *one line from a model* bought the evidence. On this function nothing was
bought -- it was free. Examples still matter where the constructor accepts essentially
nothing and there is no case base to grow from (the `textseedempty` fixture: 1025 of 1025
rejected), which is a narrower claim than the one that was recorded.

**This is the first change today that moved a number rather than a failure mode.** Both
reach fixes yesterday and the text fix this morning were real and necessary and left
`semver` at one checkable property. This one produces evidence where there was none.

Verified by hand before being believed: the status appears on a seeded run and propagates to
the root; it survives result reuse (`[seeded, reused]`); an unseeded run carries no such
mark, so the two are never confused; and a seeded run still catches a false promise --
breaking the function under test gives `violation`, not a comfortable pass.

- [ ] **Not measured: whether the whole 1-in-16 count moves.** One property is now reachable
      that was not. The other fifteen are each held by two to four blockers, and this
      addresses one of them. The full re-measurement is owed before any claim about the count.
      A vendored copy set up for it now lives at `/home/user/semvercheck` (outside this
      repository; nothing here modifies `semver` upstream).
- [ ] **KNOWN GAP, disclosed not detected: seeded runs miss the extremes.** Mutations of
      short valid values reach a 280-character identifier or a 20-digit overflow essentially
      never -- and those are exactly the cases `semver`'s author wrote down. The `[seeded]`
      status is honest cover for this, not a fix. The user's defence is to write the extreme
      case as an example, where it becomes a seed; that should be said somewhere a user reads.

## Seeded generation for text-parsed types — 2026-09-01

`docs/reach-measurement-2.md`'s open blocker ("a type built by parsing text cannot be
constructed by random text") is closed for the shape it was measured against: a receiver
whose own constructor takes a `&str`/`String` and rejects most input (a `#[ply::requires]`,
or a fallible `Result<Self, E>` return). Test-first throughout; every fix below was proved
by reverting it and watching the exact same failure come back.

- [x] **Corpus and mix, implemented.** `fuzz_gen::plan_receiver_seeding` decides, per
      receiver constructor, whether its (first) `String`-typed parameter is gated at all;
      if so, `fuzz_gen::extract_examples_seed_strings` pulls every literal string argument
      passed to that constructor anywhere in the crate's `examples:` entries (syntactic
      only, zero new vocabulary), and the generated harness (`fuzz_gen::seed_apparatus`)
      grows that pool at runtime with every value the constructor actually accepts. Future
      draws for that parameter come from a 4:1 mix (`SEED_MUTATE_WEIGHT`:
      `SEED_TRICKLE_WEIGHT`, `fuzz_gen.rs`) of mutating a random corpus entry (character
      edit, splice, truncation, repetition, or a verbatim replay) against a continuing
      uniform trickle -- justified in a doc comment on those two constants and repeated in
      the diagnostic below so the ratio reaches the JSON envelope, not just source. An
      unseeded (ungated, or non-text) constructor takes byte-identical code paths to
      before this feature existed -- pinned by
      `fuzz_gen::tests::an_infallible_unconstrained_text_constructor_is_not_seeded`,
      comparing the seeded and plain entry points on the same fn and asserting equal
      output.
- [x] **The verdict carries its own provenance, honestly.** A `fuzzed(n)` verdict earned
      this way carries a `seeded` status -- structurally the same way `conditional`
      already travels (a plain flag in the same `statuses` list the tree and the record
      already carry, propagated and reused with zero extra plumbing, per
      `crates/ply-cli/src/verify.rs`'s own comment at the push site) -- plus a new,
      `info`-severity diagnostic (`W0523`, never a warning: this describes what the
      evidence *is*, not something incidental about the run) naming the real counts a
      given run produced: how many seeds came from `examples:`, how many the constructor
      accepted from generated draws, and the actual rejected/total split. A verdict
      earning `fuzzed(n)` with nothing ever seeded carries neither the status nor the
      diagnostic -- proved both directions by an e2e fixture pair
      (`tests/fixtures/textseeded`, whose receiver constructor parses text and is fed one
      `examples:` seed, vs. the pre-existing `narrowctor`, whose receiver constructor
      rejects on a plain `u64` and must show neither) and confirmed to survive a reused
      (carried-forward) verdict unchanged.
- [x] **The other honesty condition: no seeds at all names the fix.** When a gated text
      constructor's corpus never grows past zero (no `examples:` entry, and not one
      generated draw was ever accepted), the existing high-rejection abort (`W0503`)
      switches from its generic "widen your `requires`" wording to naming the exact
      action -- add an `examples:` entry for the specific constructor, quoted by name --
      only for this shape; every other cause of that same abort (a plain numeric
      `requires`, unrelated to text) keeps its original wording verbatim, unaffected.
      Fixture: `tests/fixtures/textseedempty` (a constructor accepting essentially none of
      the space uniform sampling could draw, no `examples:` entry at all).
- [x] **Real bugs found by actually compiling the generated harness, not just asserting on
      its text (CLAUDE.md: "assert the observable outcome, not the shape of the output").**
      The corpus's embedded `examples:` literals were spliced as bare `&str` literals into
      a `Vec<String>` initializer (`error[E0308]`, caught only once the `textseeded`
      fixture was actually built and run, not by the unit tests, which merely checked the
      literal text was present) -- fixed by appending `.to_string()` to each; a unit test
      now pins the exact well-typed form. Both new fixtures also needed their receiver's
      field made `pub`, the same requirement every other receiver-postcondition fixture in
      this codebase already has.
- [ ] **KNOWN GAP, disclosed rather than hidden (the design brief's own "known failure
      mode").** Seeded generation is an honest *disclosure*, not a *detection* mechanism:
      64 cases mutated from short, ordinary seeds essentially never reach an extreme an
      author actually cared about (a 280-character identifier, a 20-digit overflow) --
      mutation just does not walk that far from a short starting point in a handful of
      edits. The `seeded` status and its diagnostic say what the evidence *is*; they do
      not detect what it *misses*. The user's defence is unchanged from what `examples:`
      already offers: write the extreme case by hand as its own `examples:` entry, and it
      becomes a seed like any other. Second-order, also disclosed rather than fixed: seeds
      anchor the draw distribution near what is already known-valid, so a pathological
      input that would crash the parser becomes measurably *less* likely to turn up than
      under pure uniform sampling -- which is the whole reason the uniform trickle stays
      in the mix rather than being dropped for a purer (and more self-referential) corpus.
      No code change is proposed for this; it is a property of the technique, named so it
      is never mistaken for closed.
- [ ] **KNOWN GAP: only a receiver's own constructor parameter is seeded.** A plain
      (non-receiver) function whose own text parameter is itself gated by its own
      `#[ply::requires]` -- e.g. a parser checked directly rather than through a
      receiver -- is not seeded at all yet; nor is a `String` parameter nested two levels
      deep (a struct field built via its own constructor, itself an argument to another
      constructor). Scoped out deliberately for this session: the measured probe
      (`docs/reach-measurement-2.md`) and the acceptance shape are both squarely the
      receiver-constructor case, and widening to every gated `String` parameter
      everywhere is a larger, separately-reviewable change. Not yet re-measured against
      `semver` itself -- that measurement is explicitly the maintainer's to run, not
      mine to claim.

## Two more harness-generation compile defects fixed — 2026-08-31

Both were the two `KNOWN GAP`s recorded just below (found the same day, pointing Ply at
`semver`, `docs/reach-measurement-2.md`), confirmed pre-existing and almost certainly a
real share of that measurement's 1-in-16 reach. Test-first, revert-and-confirm-red on
both.

- [x] **A method's own postcondition could not mention the receiver it is called on.**
      `#[ply::ensures(|result| *result >= self.a)]` generated a harness that did not
      compile: `error[E0424]: expected value, found module `self``, because the
      postcondition is spliced into the generated test as a free-standing expression
      outside any `impl` block, where the literal keyword `self` means nothing. Fixed by
      rewriting a bare `self` to the binding the generated harness already builds the
      receiver under (`__ply_receiver`), before `old()` is lifted (so `old(self.x)` still
      reads the receiver's value on entry) and before the postcondition is widened. New
      helper: `contract_rt::rewrite_self_to_receiver`, wired into
      `fuzz_gen::generate_fuzz_test` (the only place a receiver method's postcondition is
      ever spliced into a runnable test -- `contract_rt::render_cex_test`'s replay-test
      path already refuses every receiver method by design, so it needed no change).
      Fixture: `tests/fixtures/selfreceiver/`; test:
      `tests/e2e/tests/selfreceiver_fixture.rs`, covering `self` read alongside the
      result (the reported repro, verbatim), `self` read alongside a parameter, and a
      receiver built through a fallible (`Result<Self, E>`) constructor whose own
      postcondition also reads `self` (the constructor-scan fix and this fix now
      interacting in one run). Reverting the fix reproduces the original `E0424` verbatim
      for all three.
- [x] **A comparison nested inside another comparison as a leaf did not compile.**
      `*result == (a == b)` (a boolean postcondition stated as an equality of two other
      equalities) rendered as `a == b as i128` -- `contract_rt::widen`'s catch-all leaf
      case cast the nested comparison's token stream to `i128` with no parens of its own,
      and because `as` binds tighter than `==`, that parses as `a == (b as i128)`,
      comparing `u64` to `i128` (`error[E0308]`). Fixed by giving `widen_leaf` its own
      case for a nested comparison or logical operator (`==`, `!=`, `<`, `<=`, `>`, `>=`,
      `&&`, `||`): recurse through `widen` itself (which already widens *that*
      expression's own leaves correctly, arithmetic included, so a mixed case like
      `a + 1 == b` nested as a leaf still cannot overflow while being checked), then
      parenthesise the whole result before casting it -- never taking the nested
      expression's tokens verbatim. Fixture: `tests/fixtures/nestedcomparison/`; test:
      `tests/e2e/tests/nestedcomparison_fixture.rs`, covering the reported repro
      (verbatim), a comparison nested under `&&`, one nested under `||`, a comparison of
      two expressions rather than two bare names, and a mixed arithmetic case -- all five
      needed the fix. Two more (`&&`/`||` as the postcondition's own outermost operator,
      no wrapping equality) are in the same fixture and confirmed, by testing against the
      pre-fix binary, to already have worked -- `widen`'s own `&&`/`||` recursion never
      routes through the leaf path those exercise. Reverting the fix reproduces the
      original `E0308` verbatim for all five.

## Silent-green regression and two false sentences, closed — 2026-08-31

An adversarial review of the two 2026-08-30 wording fixes above found the narrower
`W0510` gate had reopened the exact silence it was meant to close, plus one more false
sentence in `check`'s new boundary-contract wording, plus one false "a test reproduces
this" claim when two fns break their promise in the same run, plus four planted bugs
none of the new tests caught. All closed; test-first, revert-and-confirm-red on every
fix.

- [x] **Silent-green regression, closed.** `checks: [test]` + a passing `examples:`
      entry + a *wrong* ply.yaml `ensures:` (no inline attribute) reported `tested` with
      zero diagnostics — `V0505` never fires when there is something to actually run
      (the example), so narrowing `W0510` to only fire alongside an inline attribute
      left nothing to say the ply.yaml contract was ever declared, let alone unchecked.
      Fixed by restoring `W0510`'s original unconditional firing (`declares_contract`,
      not `declares_contract && cf.has_contract()`) and instead fixing the actual false
      clause: "so this run checked `{fn}` against its inline attributes only" (false
      with no inline attribute) is now "so this run does not check `{fn}` against it;
      only an inline attribute on `{fn}` itself counts toward `{fn}`'s own checks" — true
      either way. `V0505`'s own ply.yaml aside (added in the 2026-08-30 fix) is removed
      as now-redundant, since `W0510` always fires alongside it. New fixture:
      `tests/fixtures/yamlonlycontractexample/`; new test:
      `tests/e2e/tests/yamlonlycontractexample_fixture.rs`. Existing tests updated to
      expect two non-contradictory diagnostics instead of one
      (`yamlonlycontract_fixture.rs`, `verify.rs`'s own unit test).
- [x] **`check` told a boundary-only fn's author to destroy the feature, with a false
      sentence.** For `legacy_rate` (declares a ply.yaml contract, no `checks:` of its
      own — §5.5's boundary declaration, working as intended), `check` said "`verify`
      does not read a contract written there yet" (false — it reads it and uses it as a
      caller's assumption) and "Move the contract onto `legacy_rate` as an attribute if
      you want it checked" (advice to delete the feature the fixture demonstrates). Now
      distinguishes two cases (`AnchorTally`'s `yaml_contract_checked_fns` vs.
      `yaml_contract_boundary_fns`): a fn with its own `checks:` gets told to move the
      contract onto it if it wants that checked; a fn with no `checks:` of its own gets
      told this is deliberate, and that any caller's result will say it rests on an
      unchecked promise. New test: `tests/e2e/tests/boundarycontract_check.rs`; existing
      `check.rs` unit test updated, new one added for the boundary case.
- [x] **"Ply wrote a test that reproduces this" was false when two fns broke their
      promise in one run.** `harness::write_generated_test` overwrote
      `ply_generated_cex.rs` wholesale on every call, and `verify` called it once per
      broken fn — so the terminal printed the line twice but only the *last* fn's test
      survived on disk. Fixed by accumulating every fn's rendered cex test into one
      `Vec<RenderedTest>` across the whole run (`push_cex_test`, deduped by test name so
      a fn re-rendered mid-run for §9's oracle check does not produce two `fn` items with
      the same name) and writing the combined file exactly once, after every fn has been
      checked. New fixture: `tests/fixtures/fuzzbugtwo/` (two fns, both broken); new
      test: `tests/e2e/tests/fuzzbugtwo_fixture.rs`, asserting both rendered tests
      survive and both actually run under `cargo test`.
- [x] Two smaller wording repairs: "run `cargo test` and it fails with the same message
      above" (false — `cargo test` prints the postcondition failure text, never the
      diagnostic's own title) is now "run `cargo test` from this crate's root directory
      and it fails the same way this run just did" (`main.rs`'s `counterexample_report`,
      pinned by a new unit test). `check`'s plural wording ("Move the contract onto the
      function as an attribute" when several are involved, naming none of them) now says
      "those functions"/"them" throughout, pinned by a new unit test with four fns split
      across both cases.
- [x] **Four planted bugs, each closed with a test that kills it** (adversarial review
      measured all four surviving the existing suite):
      - A fallible constructor's rejection arm turned vacuous (`Ok` instead of rejecting)
        went unnoticed by the existing receiver-constructor fixture
        (`receiverresultctor`), because its constructor rejects only one value and its
        promise (`u64 >= 0`) is vacuously true regardless. New fixture:
        `tests/fixtures/narrowctor/` — a constructor rejecting most of its domain
        (`v > 3`, against a generator drawing mostly from `0..=16`) behind a non-vacuous
        promise (`*result <= 6`, true only because the rejection is real). New test:
        `tests/e2e/tests/narrowctor_fixture.rs`, asserting the high-rejection warning
        (`W0503`/`high_rejection_rate`) fires — confirmed to fail (verdict flips to
        `violation`) when the constructor's rejection is defeated.
      - `||` mutated to `&&` in both new yaml-contract detectors (`verify.rs`'s
        `declares_contract`, `check.rs`'s walk) survived because every existing fixture
        with a ply.yaml contract declared both `requires:` and `ensures:`. Already
        closed incidentally by the two fixtures above (`yamlonlycontractexample`,
        `boundarycontract` via `boundarycontract_check.rs`), both `ensures:`-only —
        confirmed by mutating both conditions to `&&` and watching both tests fail.
      - Deleting `W0510` outright survived the whole suite, since the one place it was
        tested (`yamlonlycontract_fixture.rs`) has no inline attribute, and nothing
        asserted it also fires when one *does* exist. New test:
        `tests/e2e/tests/yaml_and_inline_contract_fixture.rs`, reusing the existing
        `envelopecontract` fixture (`add` carries both an inline `#[ply::ensures]` and a
        ply.yaml `ensures:`) — confirmed to fail when the diagnostic push is deleted.

## Two wording defects found pointing Ply at semver — 2026-08-30

Both defects were in what Ply *says*, not what it computes — found by pointing Ply at
`semver` (the brief cited `docs/reach-measurement-2.md` for this, which is not present in
this checkout). Both fixed, with a failing test written first for each, an end-to-end
fixture, and a revert-and-confirm-red pass on every fix.

- [x] **A counterexample was announced and then withheld.** The terminal printed a
      diagnostic's title — which can promise "proptest shrank a failing case to this
      minimal example" — and stopped there: no failing input, no mention that Ply had
      just written a runnable red test into the user's own `src/`, even though `--json`
      carried both the whole time. `crates/ply-cli/src/main.rs`'s `print_human` now
      reuses the same `counterexample` field `--json` does, printing the failing input
      plainly (never fabricated — the W0541 "cannot render as Rust" case still names no
      test file, since none was written) and the path of the written test when there is
      one. Fixture: `tests/fixtures/fuzzbug/` (existing); new tests: `fuzzbug_fixture.rs`'s
      `the_terminal_shows_the_promised_counterexample_and_where_the_test_was_written`,
      plus three unit tests in `main.rs`.
- [x] **A contract written in `ply.yaml` was accepted by `check`, then silently ignored
      by `verify`, which explained the silence with two contradictory warnings.** One
      (`W0510`) said the ply.yaml contract "is used ... so this run checked `{fn}`
      against its inline attributes only" — false when there are no inline attributes,
      since nothing was checked against them. The other (`V0505`) correctly said "there
      is nothing to check its result against, so nothing was run." Fixed here by
      narrowing `W0510` to fire only when there genuinely are inline attributes to
      check against (`cf.has_contract()`).
      **RETRACTED, 2026-08-31 (adversarial review): that narrowing was itself a
      regression** — a fn with `checks: [test]`, a passing `examples:` entry, and a
      *wrong* ply.yaml `ensures:` (no inline attribute) reported a clean `tested` with
      zero diagnostics, because `V0505` does not fire when there is something to run
      (an example), so nothing was left to mention the ply.yaml contract at all. See
      "Silent-green regression and two false sentences, closed" below for the real fix:
      `W0510` fires unconditionally again whenever ply.yaml declares a contract, and its
      own wording is what changed to stop being false, not the condition it fires under.
      `check`'s anchors line ("N of N fn claims ... point at a function Ply can find")
      now also names this up front, before `verify` ever runs. This is a wording fix
      only — `ply.yaml` contract merge stays out of scope, per the spec's own status
      list (§2226-2229 area, M3 thin-slice status). Fixture:
      `tests/fixtures/yamlonlycontract/` (new); new test:
      `tests/e2e/tests/yamlonlycontract_fixture.rs` (two tests, `check` and `verify`).
## KNOWN GAP: a method's promise cannot mention its own receiver — CLOSED, see top of file

- [x] **`#[ply::ensures(|result| *result >= self.a)]` generates a harness that does not
      compile:** `error[E0424]: expected value, found module `self``. Any promise that
      refers to `self` — which is most of what a method's promise would naturally say —
      is affected, whatever its parameters.

      Found while verifying the two fixes above, and **confirmed pre-existing**: the same
      case run against the binary built before those fixes produces the identical error,
      so neither fix caused it. It surfaced only because the reproduction taken from the
      `semver` measurement happened to write `self.a == other.a`; a promise about the
      arguments alone hides it completely, which is why the same-type-parameter fix looked
      finished when it was not.

      This is very likely a real share of the 1-in-16 reach recorded in
      `docs/reach-measurement-2.md`. A method that cannot say anything about the object it
      is called on can only promise things about its arguments, and the interesting
      promises about a method are usually about the receiver.

      Reported honestly when it happens — a tool error, never a pass, with the compiler's
      own words quoted — so nobody is misled. It is still a check that cannot run.

      **Fixed 2026-08-31** — see "Two more harness-generation compile defects fixed" at
      the top of this file.

## The text fix closed a recorded false clean — 2026-09-01

CI caught this, and it is the opposite of a regression. `excludedop` exists to record
"the fourteenth false clean" (`docs/review-structs-enums.md` finding 1): `Acc::get`
promises its result is always 0, that promise is false after one call to `Acc::note`,
and Ply could not call `note` because it took borrowed text. So every generated case
ran against a receiver only the constructor had touched, and a broken function reported
a clean pass.

Text arguments now work, so Ply calls `note`, reaches the broken state, and reports a
`violation`. The fixture's test still asserted the old, weaker truth and went red.

- [x] **`excludedop` keeps testing what it was written to test.** Its `note` now takes
      `Option<String>` — a `String` nested inside another type, deliberately never
      built — so the run still genuinely cannot call it and must say so. Verified: the
      verdict is `fuzzed(256)` marked narrower, and the warning names `note` and why.
- [x] **`textmutator` records the win.** The same shape with a `&str`, asserting the
      `violation` Ply now finds. Proved to bite: reverting the one-line text fix brings
      back `fuzzed(256)` on the broken function and the test fails demanding
      `violation`. That is the false clean itself, reproducible on demand.

- [x] **`skippedctor` too, same cause.** Its premise is a constructor Ply finds but cannot
      use, because the constructor took borrowed text. It is usable now, so no constructor
      was skipped and the disclosure it tests never fired. Its argument is now an
      `Option<String>`, and the test passes again.

Swept the rest rather than waiting for CI to find them one at a time: exactly three fixtures
depended on borrowed text being unbuildable, all three are handled, and the only fixture left
with a `&str` parameter is `textmutator`, which uses one deliberately.

Worth stating plainly because it is the first time this has happened today: a capability
improvement made tests fail by making Ply *better*, and the fix was to preserve both truths
rather than weaken any test.

## Text parameters landed, and the next blocker is a design problem — 2026-09-01

`&str` now reaches the sampler (one line: `str` maps to the same type `String` already used;
references were already looked through, so only the borrowed spelling was missing). Measured
as the largest single blocker — 11 of `semver`'s 16 properties.

Re-measured against `semver` immediately. **The count is still one in sixteen, but the
failure mode moved, and that is the finding.**

Probe: `Prerelease::is_empty(&self) -> bool`, whose receiver must be built by
`Prerelease::new(text: &str) -> Result<Self, Error>` — text parameter and fallible
constructor at once, both fixed within the last day. Before today this was refused before
anything ran. Now:

- Ply **builds the receiver and runs the check**. `Prerelease::new` is called with generated
  text, exactly as intended.
- The verdict is `unclaimed`, not `fuzzed(64)`, because **1025 of 1074 generated strings were
  thrown away** by the constructor's precondition. Random text essentially never parses as a
  valid pre-release identifier.
- Ply says so itself, unprompted: *"So this function has no fuzz evidence at all -- its
  verdict is `unclaimed`, not `fuzzed(64)`."* That is the high-rejection machinery working —
  the same machinery whose test was proved to bite this morning by planting a bug that turned
  rejections into passes.

- [ ] **NEXT BLOCKER, and it is a design problem rather than a defect: a type built by
      parsing text cannot be constructed by random text.** Uniform sampling will not produce
      a valid version string, identifier, or any other parsed format, so every property about
      such a type reaches the engine and comes back with no evidence. The honest reporting is
      right and worth keeping. What is missing is a way to generate values that satisfy a
      constructor — seeding from `examples:`, sampling a grammar, or reusing values the
      crate's own tests already contain. None of these is a one-line change, and choosing
      between them is a design decision, not an implementation.

The scoreboard, stated plainly: reach on `semver` has gone from "refused before anything ran"
to "ran, and honestly reported that it learned nothing". No property moved into the checkable
column. That is progress in honesty and none in coverage, and the two should not be confused.

## Re-measured `semver` after the reach fixes: it has not moved — 2026-09-01

The two reach defects fixed today (a promise may now mention its receiver; a comparison may
now nest inside a promise) plus yesterday's receiver-constructor fix were checked against the
library that produced the 1-in-16 result. **Reach is unchanged.** That is exactly what
`docs/reach-measurement-2.md` predicted — every unreached property is held by two to four
independent blockers, and its table records "unblocks alone: 0" for every capability
including the ones just shipped — but it was worth measuring rather than trusting, and the
measurement found something the table missed.

Probe: `Version::cmp_precedence(&self, other: &Self) -> Ordering`, the property about
comparing versions while disregarding build metadata. It converges three defects — a receiver
that must be built, a parameter of the receiver's own type, and a return type Ply can observe
but not construct. Two of those three are now fixed.

- [ ] **NEW BLOCKER: a parameter written as `Self` is refused, where the same type spelled by
      name is not.** `other: &Self` gives "parameter(s) other: Self use a type neither the
      bounded nor the fuzz codegen builds inputs for". Rewriting it as `other: &Version` --
      which no compiler or reader would call a change -- gets past that check entirely. This
      is the same asymmetry the measurement already found between `-> Self` and `-> Version`
      in the return position, now confirmed in the parameter position too. It is not in the
      measurement's blocker table, so that table understates the problem: properties it
      attributed to other causes may be blocked by this as well.
- [ ] **Confirmed still open: the refusal that names nothing.** With the parameter spelled
      out, the same function is refused with "none of its declared checks apply to this
      function's shape" -- no mention of the return type that is actually stopping it. The
      measurement flagged this ("`unsupported_shape_diag` inspects only parameters, so when
      the blocker is the return type it falls back to a sentence carrying no information a
      user can act on"). It is unchanged.

The honest scoreboard: today's fixes are real and were verified to work in both directions,
but they move `semver` from one checkable property to one checkable property. A blocker only
becomes visible once the ones in front of it are gone, and removing two of four revealed a
fifth rather than a verdict.

## Two harness-generation defects fixed — 2026-08-31

Both were found by pointing Ply at `semver` -- see `docs/reach-measurement-2.md`,
which landed on `main` while this work was in progress and so was not visible
from the branch it was written on. The agent that fixed these noted, correctly
for what it could see, that the cited file did not exist and declined to
either invent the measurement or drop the citation. It exists; that note is
withdrawn rather than left to confuse a later reader.

- [x] **Defect 1 — a receiver's own constructor scan disagreed with the
      parameter path about what counts as a constructor.** A
      `Result<Self, E>`-returning `new` (or one spelling the type's own name
      instead of `Self`, bare or `Result`-wrapped -- four spellings of one
      shape) was recognised when building a *parameter* and reported as not
      existing when the very same scan was asked to build a *receiver* for
      the identical type in the identical run. Fixed by making the receiver
      scan (`scan_file_for_receiver`, `crates/ply-core/src/harness.rs`) call
      `ctor_return_kind` -- the one classifier the parameter path
      (`scan_ctor_candidates`) already used -- instead of carrying its own
      narrower, separate check, and by threading the resulting `CtorReturn`
      through `ReceiverPlan` instead of hardcoding `CtorReturn::Bare`.
      `fuzz_gen::receiver_preamble` now renders the same rejecting `match`
      around a fallible constructor call that `build_user_value_stmt`
      already renders for the parameter path. Fixture:
      `tests/fixtures/receiverresultctor/`; test:
      `tests/e2e/tests/receiverresultctor_fixture.rs` (all four spellings,
      plus the exact `A`/`Bad`/`read_it` reproduction, in one run). Reverting
      the fix reproduces the original false `V0507` refusal verbatim.
- [x] **Defect 2 — a method whose parameter shares its receiver's type
      generated a harness with the same `use` line twice.** The generated
      harness's extra-type-import scan (`extra_type_imports`,
      `crates/ply-core/src/fuzz_gen.rs`) deduplicated against its own output
      only, never against the primary `use` `wrap_fn_harness_module` already
      emits for the checked function's own type -- so a `&self` method
      taking another value of its own type imported that type twice
      (`error[E0252]: the name `Pair` is defined multiple times`). Fixed by
      backing the dedup with a real `HashSet`, seeded with the primary
      import up front, so "the receiver's type" and "a second parameter of
      the same type" are the same case as the existing two-parameters dedup,
      not a second special case beside it. Fixture:
      `tests/fixtures/sharedtypeparam/`; test:
      `tests/e2e/tests/sharedtypeparam_fixture.rs` (receiver+parameter,
      two parameters with no receiver, and receiver+parameter+return all
      naming the same type). Reverting the fix reproduces the original
      `E0252` verbatim.
- [x] **KNOWN GAP, found while writing the defect-2 fixture -- `#[ply::ensures]` on a
      receiver method cannot read `self`.** Nothing rewrites a bare `self` in the
      postcondition closure to the actual receiver binding before splicing it into the
      generated free-standing assertion, so `self.a == other.a` renders as a
      literal `self.a`, which does not exist outside an `impl` block --
      `error[E0424]: expected value, found module `self``. No fixture in the
      crate exercised this before (`grep`-confirmed: no existing
      `#[ply::ensures]` or `#[ply::requires]` on a receiver method reads
      `self`), so it was invisible until this task's own reproduction
      (`same_as(&self, other: &Pair)`, ensures reading `self.a`, matching the
      literal Defect 2 repro handed to this session) tried it. Fixing defect
      2 alone does *not* make that literal reproduction pass -- it trades
      `E0252` for `E0424`. The committed `sharedtypeparam` fixture avoids
      reading `self` in every postcondition so it isolates defect 2 cleanly.
      **Fixed 2026-08-31** — see "Two more harness-generation compile defects fixed" at
      the top of this file.
- [x] **KNOWN GAP, found the same way -- postcondition widening mis-parenthesises a
      nested comparison.**
      `contract_rt::widen`'s catch-all leaf case casts a whole nested
      comparison's token stream to `i128` without wrapping it in its own
      parens first, so `*result == (a.a == b.a)` (a boolean-returning
      postcondition stated as an equality of two other equalities) renders
      as `a.a == (b.a as i128)` -- because `as` binds tighter than `==`,
      that compares `u64` to `i128`, `error[E0308]`. No existing fixture
      wrote a boolean postcondition that way either. Worked around in the
      `sharedtypeparam` fixture by stating the same property as an `iff`
      (`(!*result || lhs == rhs) && (*result || lhs != rhs)`), which
      `widen`'s existing `&&`/`||` recursion handles correctly.
      **Fixed 2026-08-31** — see "Two more harness-generation compile defects fixed" at
      the top of this file.
## Ply pointed at a stranger's code: 1 of 16 — 2026-08-30

`docs/reach-measurement-2.md`. The method of `docs/invariant-reachability.md`, repeated on a
second library chosen before reading what Ply supports: `semver` 1.0.28, 2,117 lines, whose
author documents his guarantees unusually well. Sixteen stated properties. **Ply checks one.**

**Zero of the sixteen are out of the tool's shape.** No threads, no sequences, no hidden
state — sixteen pure single-function properties. This is the most favourable library the
project is likely to meet, and reach is 1 in 16. The single reachable property survives only
because the author wrote `-> Self` instead of `-> Version`; spelling the type out, which no
compiler or reader would call a change, turns the verdict into `unsupported`.

**It contradicts the first measurement almost item for item, which is the point.** Floats
ranked first there; `semver` has no float anywhere, so they unblock zero. Structs and enums
ranked last with "zero effect"; here they gate twelve of sixteen. The two dominant blockers
here — `&str` arguments, and refusal on the *return* type — never appeared on the first list
at all, because in the rate limiter everything was already refused at the parameters, so
nothing ever reached the return check. **A blocker only becomes visible once the ones in
front of it are gone.** One library's ranking does not generalise, and a standing list of
types to build is the wrong instrument; measurement per codebase is the right one.

Also measured: no capability unblocks even one further property on its own. `&str` blocks
eleven of sixteen, but every unreached property is held by two to four independent blockers
at once, so the first fix moves the count by nothing.

### Three defects, each reproduced independently before being written down

- [ ] **Ply generates a harness that does not compile whenever a method's parameter has the
      same type as its receiver.** `a.same_as(b)`, `merge`, `union`, `cmp`, `min` — all of
      them. Reproduced from scratch in a nine-line crate: `error[E0252]: the name Pair is
      defined multiple times`. The report of it is good — refuses to call it a pass or a
      violation, quotes the compiler — but the harness is still wrong.
- [ ] **A user-facing sentence is false, and one run proves it false by itself.** For a type
      whose constructor returns `Result<Self, E>`, Ply says: "it has no associated function
      in the file it is declared in that builds a `A` value ... and none was found."
      Reproduced with the same type, same constructor, same file, same run: used as a
      *parameter* it earns `fuzzed(64)`, because Ply built the value by calling exactly the
      constructor it says does not exist; used as a *receiver* it is refused with that
      sentence. The `Result<Self, E>` widening reached the parameter path and not the
      receiver path. Every constructor in `semver` is that shape and every `semver` type is
      used as a receiver.
- [ ] **Contracts written the documented out-of-source way are accepted, then ignored.**
      `check` reports "6 of 6 fn claims point at a function Ply can find"; `verify` runs none
      of them and explains it with two warnings on the same function that contradict each
      other — one saying the contract exists and was used, one saying there is no contract.
      Neither states the actionable fact: only source attributes reach the engines. `check`
      is the command people run first and gives no hint of it.

Two smaller ones, recorded but not ranked. **When the tool was made to go red on purpose** --
`Version::new` was deliberately broken to return a non-empty pre-release, because a check
that never fails proves nothing -- it caught it, with an input strategy that makes the catch
real rather than lucky. But the terminal said "proptest shrank a failing case to this minimal
example" and then showed no example: the values live in `--json` only, alongside a runnable
failing test Ply wrote into the crate's `src/` without mentioning it. **To be explicit,
because the shorthand invites the opposite reading: no defect was found in `semver`. None was
looked for.** The other smaller one is that
the return-type gate causing much of the loss is documented in its own code comment as
blocking nothing technically, which makes it a deliberate narrowing that is now the
second-largest blocker in the measurement.

Nothing hung and nothing crashed. Cold run 21 seconds, warm run half a second.

## KNOWN GAP: the required-check names can be made impossible to satisfy — 2026-08-30

Branch protection is on, so `main` now requires named checks to pass. That makes the
*names* of CI jobs load-bearing, and four of the six are generated rather than fixed:

    shard: [0, 1, 2, 3]
    name: product-e2e (${{ matrix.shard }}/4)

The shard count is written twice — once as the list, once as the literal `4` in the
display name — and the name is what branch protection matches on.

- [ ] **Change the shard count and every pull request blocks forever.** Going to six
      shards produces jobs called `product-e2e (0/6)`…`(5/6)`, so a rule requiring
      `product-e2e (0/4)` waits on a check that will never report again. The pull
      request cannot merge and nothing explains why — the failure is a *missing*
      check, not a failing one, which is the harder kind to read. Editing only the
      list and not the string is worse still: the jobs are then named
      `product-e2e (4/4)` and `(5/4)`.

      The standard fix is a gate job that does nothing but depend on every shard and
      succeed, with a fixed name, and require that instead. Then the shard count is
      free to change and the required name never moves. Not built — it is a change to
      CI that cannot be tested without merging it, so it wants a deliberate decision
      rather than being slipped in.

      Cheap partial mitigation available today: require `product` and `kernel-mutants`
      (both fixed names) and leave the shards advisory. That protects the fast checks
      and the mutation gate but not the end-to-end suite, which is the one that has
      actually caught a regression on this branch.

## Review of the scheduler unification — 2026-08-30

An independent adversarial pass over the five commits. It confirmed the soundness argument
link by link, and confirmed the ordering code is byte-identical to what shipped before. It
also found a seventh bug the exhaustive check could not see, after six earlier planted bugs
had all died — which is the point the project keeps having to relearn: a check's adequacy is
measured, never standing.

- [x] **The check could not tell a tie broken on name from a tie broken on position.**
      Every test used names `n0`, `n1`, `n2`, `n3` — sorted in the same order as the
      positions they sat at, so the two rules were indistinguishable. Swapping one for the
      other left all 1,048,576 cases green, all eight smaller tests green, and green the
      test *named after the property it broke*. Names are now `d`, `b`, `a`, `c`, which sort
      neither with the positions nor against them, so neither substitution can imitate the
      real rule. Verified: replanted, and it now dies. This was not academic — real names are
      `component::function`, and a nested component makes name order and position order
      genuinely disagree.
- [x] **The check read its output as a set, so placing something twice was invisible.** One
      line comparing counts closes it.
- [x] **The spec never stated the rule the whole change turns on.** It said a claim *in* a
      cycle falls back; it never said the fallback also covers every claim that reaches one.
      The implementation has always behaved that way and no artifact said so. §5.5 now does,
      including why the coarse rule is the safe one.
- [x] **The unused stub-permission gate reads as though it agrees with the shipped rule.**
      It does not: it refuses only a caller inside the callee's own cycle, so it is looser
      exactly where it matters, and its own exhaustive test cannot notice — that corpus has
      two nodes and the disagreement needs three. Its crate doc now says so, and says that
      adopting it is a deliberate relaxation needing an argument the spec declines to make.
- [x] **Two claims in the spec were not true of the evidence they cited.** The measurement
      was dated 2026-08-30; it was made 2026-08-27. And it was described as "a real outside
      library", which it is not — it is `tests/fixtures/ratelimiter/`, in this repository,
      written from a design brief by someone told not to think about checkability and not
      told this project existed. That provenance is what makes the measurement worth citing,
      so overstating it as third-party was both false and unnecessary. Corrected in the spec
      and here; the commit message that carries it is already pushed and cannot be corrected
      in place, which is why it is written down here instead.

### KNOWN GAP: a function that calls itself is never denied credit by the ordering

- [ ] **Self-recursion is filtered out before the ordering ever sees it**, so a
      self-recursive claim is placed normally rather than denied. Credit for the self-call
      is still refused, but only because the claim's own result is not yet available when
      the decision is taken — an accident of sequence that no test pins. Meanwhile the
      exhaustive check *does* include self-loops and requires them denied, so the tested
      rule and the real input space quietly disagree about this one case. Pre-existing, not
      introduced by this change; found by review 2026-08-30. Wants a test pinning that a
      self-recursive claim earns nothing from itself.

### KNOWN GAP: the spec still claims a restriction nothing enforces

- [ ] **§5.4a says contract strings are restricted to a closed subset. Nothing checks
      that**, and this repository's own rate-limiter fixture violates it. Flagged inside
      `docs/invariant-reachability.md`, which the spec now cites as evidence — so the spec
      leans on a document that names one of the spec's own claims as needing retraction, and
      the retraction is still undone. Predates this branch; recorded 2026-08-30 rather than
      left to be found again.

## What widening the types is actually worth — decided 2026-08-30

Agreed with the maintainer: **stop treating "support more types" as the roadmap.** The
evidence against it is already in this repository and it is unusually direct.

On the one library anyone measured against -- `tests/fixtures/ratelimiter/`, designed by
someone told not to think about checkability and not told this project existed, who wrote
down eleven properties they cared about -- the share of supported types went from 21% to about
80%, and the number of those properties that became checkable went from zero to zero.
Sixty points of work, no movement in the thing the work was for. The number was counting
how often a type appears on a public surface, which turned out to be nearly unrelated to
whether anything could be proved. It was also dominated by getters and configuration,
while the type that library's whole correctness argument rested on had a public-surface
count of zero, because it was internal state.

What actually blocked those eleven: finding the function at all, building the object a
method needs before it can be called, mutation, and floating point. Floats have since
landed (`2443b85`), which is the strongest form of the argument -- the single
highest-ranked blocker is discharged, so "more types" is not what stands between Ply and
the next real property.

**The replacement question, to be answered before any further type work is scheduled:**
take a library whose author enumerated their own properties, and for each one record
whether it is a single-function property at all, what specifically stops it, and whether
the author flagged it as risky. Then rank by *which single missing capability unblocks
the most properties*. That ranking put structs and enums last on the one library it was
run against -- the opposite of what type coverage implied.

**Two reasons a function cannot be checked, and they are not the same thing.** Conflating
them is what makes "Ply's checkable subset is too narrow" sound damning when it mostly is
not:

- **Out of shape, permanently.** A sixteen-thread stress test is better evidence than
  anything a single-function checker could produce. Refusing by name is the product
  working. This category should be counted and reported, never quietly widened toward.
- **In shape, unplumbed.** A genuine single-function property over a value Ply could
  sample, blocked only by not being able to build the argument. This is a gap.

Only the second is measurable, so effort drifts toward it whether or not it is where the
value is. That drift is exactly what the 21%-to-80% episode was.

**Consequence for the plan:** the honesty machinery (the rule registry, staleness
reporting, the "what was NOT checked" output) is scheduled ahead of further type work.

**Corrected the same day, before this was acted on.** The first version of this paragraph
said the ledger "works across a whole codebase regardless of types" and called the proof
engine "a bonus on the slice where it happens to be cheap". The second half is wrong and
is withdrawn. A ledger with no engine behind it is a spreadsheet of assertions: the
evidence ladder only means anything because its top rungs are sometimes reached, and if
nothing ever reaches them the whole design collapses into "tested / not tested". Proof is
what makes an entry in the ledger worth more than a claim; the ledger is what makes proof
safe to trust and safe to lack. They are complementary, not ranked.

What is true is narrower: **proof pays where it is concentrated, not where it is spread.**
A small pure core carrying consequence out of proportion to its size is exactly where it
earns its cost -- which is why this repository's own kernel gets exhaustive enumeration
over every tree to a bound plus an unbounded inductive proof, and why the mutation run
that guards it has already found real dead code and a blanked failure message that would
have left every future counterexample unreadable. The mistake was never valuing proof; it
was expecting proof to spread evenly across a codebase.

That gives a targeting rule rather than a coverage programme: **widen toward the shapes
where proof is cheap AND the consequence is concentrated**, and let the rest be recorded
honestly. It also makes the gap below worse rather than acceptable -- Ply's kernel is
precisely the shape where proof pays most, and Ply cannot reach it.

### KNOWN GAP: Ply's own file does not record what Ply's own evidence is

- [ ] **Ply's self-declaration is silent about both halves of its own honesty.** `ply.yaml`
      declares which crates exist and which may depend on which, and stops. It does not
      record that the verdict kernel and the check scheduler carry the strongest evidence
      in the repository -- exhaustive enumeration over every tree to a bound, an unbounded
      inductive proof for the kernel, a mutation run in CI that checks the check can still
      see. Nor does it record that the parsing, rendering and process-driving shell is out
      of reach and always will be.

      Both omissions matter, and the second more. Ply's own argument is that saying "I
      cannot see this" is the whole premise, and that the count of out-of-reach things
      should be reported proudly rather than hidden. Ply does not do that for itself: a
      reader of its self-portrait sees neither the proof nor the gap.

      Sharpest form of it: **Ply cannot check its own most-proved code.** The kernel's
      entry point takes a reference to a recursive tree; the scheduler takes a set of
      numbers, a slice of strings and a map of sets. Ply can build a value for none of
      those. Pointed at its own core, it would report that it cannot see it.

      This is a gap in the one file whose entire purpose is that its claims are checked,
      and it was not written down anywhere before today.

## The source copy followed a hand-written list — 2026-08-30 (2c9e343)

The first CI run after the workspace merge went red, and it was the merge's
fault. One test builds Ply from a private copy of its own source tree, and
which directories that copy took was written out by hand. Four crates joined
the workspace; the copy did not get them; cargo refused to load a workspace
root naming members that are not on disk.

What made it expensive is what CI reported: `cargo build ... failed`, from a
test about result caching. Three layers from the cause and saying nothing
about it. The copy now reads the member list out of the root manifest it is
already copying, and a new test states the invariant rather than trusting the
routine — every member the manifest declares has a manifest in the copy. Run
against the old code it names the four missing crates.

The class of defect is worth naming: a second, hand-kept list of something the
build system already knows. It was silent until the first change in eight
months touched it.

## Verification results now change what the drawing looks like — 2026-08-30

Left in the working tree, not committed (explicit constraint for this session) —
`crates/ply-core/src/visual/svg.rs` and `tools/render/tests/visual.rs` only.

- [x] Fn chips now colour by the five display states (declared/earned/violated/
      unanswered/stale), computed purely from the stored evidence a run already
      reported — never fabricated. Each state pairs its own fill/border with its own
      drawn character (a reader with no colour vision still tells them apart), and
      "earned on assumptions" reuses the earned colour with an attached mark rather
      than inventing a sixth state, per the settled state model. `violated` is the
      only new red; `unanswered`/`stale` are neither red nor the existing findings-red.
- [x] The opening verdict strip now states result counts ("2 earned, 1 broken, ...")
      alongside its existing promise counts, only once a run's evidence actually
      settles something — a document with no evidence, or evidence that resolves to
      nothing beyond "declared", renders its strip exactly as before (checked, not
      assumed: `the_strip_states_no_results_when_evidence_settles_nothing`).
- [x] A collapsed box now states its earned-over-promised split as a plain count
      (`"1 of 3 earned"`), never a percentage — the rejected two-part-meter design
      that would let nine-earned-one-untouched read as "90% healthy" stays rejected.
      `a_collapsed_boxs_earned_split_equals_the_counts_folded_beneath_it` renders the
      same evidence both expanded and collapsed and checks the two counts agree.
- [x] New public API: `render_svg_with_evidence_and_options`, so evidence and
      `--depth`/`--focus`/`--collapse` can be exercised together (previously only
      `render_svg_with_evidence`, always fully expanded, existed). Not wired into
      `cargo ply verify`'s own publish path — out of scope for this change, which is
      the renderer only.
- [x] 12 new tests in `tools/render/tests/visual.rs`, including the two invariants
      named above and one confirming red still means only `violated`/`deny`/`finding`
      for evidence-drawn output specifically (the pre-existing red test only ever
      renders fixtures with no evidence, so it could not have caught a regression
      here). `cargo test --workspace --exclude ply-e2e`: 616 passed, 0 failed (604
      baseline + 12). `cargo fmt --all` and `cargo clippy --all-targets -- -D
      warnings` both clean. `git diff --stat -- vetting/ docs/` is empty — no
      committed artifact changed.
- [ ] Not attempted, named rather than silently skipped: wiring
      `render_svg_with_evidence_and_options` into `cargo ply verify`'s actual publish
      path (`visual::build_visual_envelope_with_sources` still always renders fully
      expanded); a CLI flag to preview a folded evidence render outside of `verify`.
      Neither was asked for.

## Verus pin moved forward — 2026-08-30

Done. The spike pinned **0.2026.08.15.7d4628a**; it now pins **0.2026.08.23.fbbbbcf**,
the current stable. (A 0.2026.08.30.b432e82 rolling build also exists and was not used:
a rolling build is the wrong thing to rest a recorded proof on.)

- [x] **Kernel proof moved to Verus 0.2026.08.23.fbbbbcf.** Re-obtained rather than
      bumped, because the claim rests on what the verifier said and not on a string:
      **22 verified, 0 errors**, identical to the old pin, with the proof file needing no
      edits at all -- no syntax migration, no deprecation, same required toolchain.
      1.43s against the old ~2s, which is one measurement on one machine and is not
      claimed as a result.
- [x] **The vacuity checks were re-run too, and they are the load-bearing half.** A proof
      that verifies against a broken kernel proves nothing, so both recorded mutations
      were replanted on the new release: each still produces 20 verified / 2 errors, in
      the same two obligations as before. Reverted, 22/22 clean afterwards. The newer
      Verus is not passing this proof more easily -- it fails in the same places.
- [x] Incidental: `diff/Cargo.lock` was stale (the spike had not run since `ply-core` grew
      its dependencies) and is refreshed by 248 lines. Not an effect of the upgrade.
- [ ] **The honesty condition is unchanged and still applies**: the proof runs over a
      shadow of the kernel, not its production source, and the differential test is what
      licenses the shadow to speak for `aggregate()`. Re-check it whenever either side is
      edited. Not a task so much as a standing condition, kept here so it travels.

## External review, and the honesty boundary — 2026-08-30

A third review (Codex) read the merged transcript work; a second model verified every
finding by building and running the counterexamples. **All nine held.** They are fixed
across two commits, except the three recorded below as open.

- [x] **Both views described enforcement this build does not perform.** They said an
      undeclared cross-component call, capability use in a sealed component, and `strict`
      escalation were architecture findings that fail the build. Those rules are
      implemented nowhere — their codes appear in no checker — and `ply check` already
      says so in its own output. The views contradicted the tool they belong to. One
      shared sentence now says declared-and-unchecked, and the codes are gone.
- [x] **Two sentences claimed checks that never ran**, both now derived from the
      function's effective list: a contract on a function with no checks said "the checks
      above test this promise" four lines under "nothing about this function is verified";
      and worked examples were called "compiled into a test" when the verifier only
      compiles them under `test`. The committed sample shows the second landing.
- [x] **The two views printed different headline counts** for the same document. One
      shared calculation now; the drawing's boolean walk is deleted.
- [x] **`render ply.yaml -o ply.yaml` destroyed the document.** Refused before reading,
      on canonicalized paths.
- [x] **An unsupported `ply:` version rendered confidently.** Refused now, and the version
      is printed in both views. Deliberately *not* full validation: render draws
      half-written documents on purpose, and the version is the one field whose wrongness
      is not survivable, because it selects the rules every other line is read under.
- [x] **Every remaining "has been run" claim is gone.** Reworded conditionally ("if every
      declared check ran and passed") rather than negatively, because a negation is one
      feature away from false — this repo already has an evidence-overlay path.
- [x] **Terminal control bytes from author-written text are neutralised** at a single
      choke point on each renderer's output, so a future insertion site cannot forget it.
- [x] **The completeness walk now binds `Document` field by field.** Its absence is why
      the format version went unrendered and unnoticed.
- [x] **A committed transcript for vetting 004 exists** and is drift-gated. README and the
      module doc claimed one sat beside every scenario; making that true beat softening it.
- [x] **A ratchet for the whole class**: no sentence in either view may cite a diagnostic
      code this build cannot raise, checked against the codes actually present in the
      checker sources rather than a hand-kept list. Verified to bite by injecting one.

## One workspace, and evidence that reaches the drawing — 2026-08-30

- [x] **`tools/` merged into the product workspace.** The split existed because the
      tooling "predates the product", while every crate in it already depended on
      `ply-core` — one dependency graph pretending to be two. It cost three real things:
      `cargo mutants` could see neither side (pointed at tools it found no members;
      pointed at the product it ran `ply-core`'s thin suite while the renderer's 91-test
      suite sat across the boundary), the tests for `visual/svg.rs` lived in the other
      workspace, which is how a green test came to pin a false sentence, and the two
      clippy invocations differed so a lint firing in one was invisible in the other.
      `tests/spike` and `tests/fixtures` stay excluded, and that exclusion is principled:
      each carries its own workspace root and several exist to be built in isolation.
- [x] **Ply's own document now describes all eight of its crates**, not four. The four
      tooling crates and their six real dependency edges were invisible to Ply's
      self-check while they lived in the second workspace — a file whose entire purpose
      is that its claims are checked, silently omitting half its own codebase.
- [x] **Evidence attaches while the picture is drawn.** It used to render the SVG, then
      search its own output as a string to find the shapes it had just drawn. Its doc
      comment conceded the consequence — elements "left unattached rather than guessed" —
      and it was happening: a nested component never attached at all, because the matcher
      compared a bare function name against a dotted path it could never equal. Top-level
      components worked by coincidence, which is why every existing test looked right.
      ~230 lines of re-parsing deleted; output byte-identical with no evidence passed.

### The two schedulers order cycles differently — analysis, 2026-08-30

Before anyone unifies these, the mapping is not vocabulary. The two implementations
disagree about **where a call cycle goes**, and only one of them is tested.

**Shipped** (`crates/ply-cli/src/verify.rs`, `topological_order`, ~50 lines): returns
`(order, cyclic)` — a linear order of everything it could place, plus the set it could
not. The caller concatenates them, so **every cycle member is processed last, in a
lump**, no matter where the cycle sits in the dependency graph.

**Pure** (`tools/schedule`, `plan`): collapses strongly-connected components and returns
layered batches, so **a cycle is processed at its own layer** — early if things depend on
it, late if it depends on things.

The pure version is the more defensible ordering: a cycle that half the codebase depends
on should not be verified after its dependents. But the shipped version is the one that
has absorbed real review fixes — the `domain` restriction (adversarial review,
2026-08-26) exists because an earlier version sized everything off `node_ids.len()` and
silently admitted reused and fuzz-only claims into the ordered pass. The pure copy never
saw that class of bug because nothing calls it.

So unification is a decision, not a move:

- [x] **Decided which ordering ships: the shipped one** (`4dd4d30`). Its leftover
      set is not merely the cycle's own members — a function only becomes orderable
      once every function it calls has been placed, so a function that calls into a
      cycle, however many steps removed, never becomes orderable either. Adopting
      `plan`'s layering instead would have handed assumed-contract credit to exactly
      those steps-removed dependents, which the shipped ordering deliberately
      withholds. The ordering moved into `crates/ply-core/src/schedule.rs` unchanged
      (same Kahn's-algorithm body, same id tie-break), and the returned leftover set
      got a name that says what is in it (`tainted`, not `cyclic`).
- [x] Carried across unchanged, so every review fix travelled with the code rather
      than needing to be re-found: the `domain` restriction (2026-08-26, the one with
      its own comment) and the reuse-decided-after-ordering fix are both still in the
      moved function's doc comment and behaviour, verbatim.
- [x] **The exhaustive check moved with it and grew a second dimension**
      (`crates/ply-core/tests/schedule_enumeration.rs`): it now varies which
      functions are in scope, not just the call graph's shape — 1,048,576
      combinations, because scope (a claim that should never have entered the
      ordered pass at all) is exactly where the 2026-08-26 bug lived. Checked
      against an oracle computed a different way (SCC + reachability) than the
      implementation (indegree counting).

### KNOWN GAP that outranks the rest, found 2026-08-30 — CLOSED (`4dd4d30`)

- [x] **The check scheduler no longer exists twice.** `tools/schedule`'s `plan`/
      `Batch` (the untested, more permissive copy) are deleted; `may_stub` and its
      own exhaustive test are untouched and still live there. The one real ordering
      is `ply_core::schedule::order`, called directly by `crates/ply-cli/src/
      verify.rs`, and it is the copy the exhaustive test now covers. Verified after
      the fact, independently: three deliberate breakages of the new module (a node
      placed before one of its callees, a cycle's dependent left out of the tainted
      set, the id tie-break replaced with a hash-based one) each made the
      enumeration fail, and each failure named the actual defect rather than a bare
      assertion. The full engine-backed suite (`cargo test -p ply-e2e`, 89 fixture
      binaries, Kani proofs included) was run to completion afterward: 163 tests,
      0 failures — the landing commit's own note ("the engine-backed suite has not
      reported yet") is now resolved.

### A fourth review, and the half-fixes it found — 2026-08-30

The pattern in all three: I had fixed the instance in front of me and reported the class
as done.

- [x] **The drawing still told both lies the text form had been taught out of.** The
      morning's fix landed on the transcript only, and a green test *required* the
      drawing to say an example was "compiled into a test" against a function declaring
      `[bounded(3), fuzz(1024)]` — a passing test pinning a false sentence in exactly the
      configuration where it is false. Both sentences now come from two shared helpers,
      and both are future-conditional: neither view runs a compiler, so neither may say
      one ran. The TODO tick claiming otherwise was an overclaim about my own work.
- [x] **`SCHEMA.md` §8 still opened "None of the rules in this section is enforced"** while
      §2 and §14, corrected earlier the same day, said crate-level `edges:`/`deny:` are
      checked. Replaced with a tier matrix stating row by row what is enforced (`A0401`,
      `A0405`) and what is only recorded, verified against the code rather than the prose.
- [x] **The malformed-example refusal wore the wrong identity**: `V0507` (a code in no
      documentation anywhere), `severity: "warning"` for something that refuses a claim
      and exits non-zero, and `open_item: "unsupported_signature"` — false, since the
      signature is fine and the document is malformed. A dedicated constructor emits a
      real `E0501` at error severity now. The regression asserted on a substring of the
      human title, so it passed with the code wrong; it asserts the `code` field.

### KNOWN GAPS, left open deliberately

- [ ] **The doc test is a substring ratchet, and its weakness is measured, not assumed.**
      A reworded blanket lie inserted under §8 alongside an intact matrix passes all three
      assertions. The real fix is the rule registry below; until it exists this catches
      the historical sentence and the matrix vanishing, which is worth having and is not
      a proof.
- [ ] **The malformed-example diagnostic carries no pointer at the offending YAML line.**
      `diag.rs` documents pointers as present only on E0201/E0204, so this is consistent —
      but that rationale ("a diagnostic about a function points at source, not at YAML")
      argues a diagnostic that *is* about a YAML line deserves one. Follow-up, not a
      defect; the title quotes the entry.
- [ ] **The tier matrix omits a `profile:` row** that §8 documents in its own subsection.
      Nothing is claimed enforced that is not, but the honest summary skips one construct.


- [ ] **The rule registry.** The ratchet above catches a phantom *code*; it cannot catch a
      phantom claim with no code in it ("compiled into a test" had none). The real fix is
      a table of rules as data — code, tier, implemented, severity, gloss — that the
      checker, its NOT-CHECKED paragraph, and both views all derive from, so an unenforced
      rule cannot be described as enforced by construction. That is a design change and
      wants review, not an agent's initiative.
- [ ] **Multi-line author text can still impersonate transcript structure.** A note
      containing a newline renders at column 0 and can look like a heading the renderer
      wrote. Control bytes are handled; layout is not. Re-flowing multi-line prose legibly
      has real design questions (indent, wrap, quoting) and the threat model today is a
      trusted author, so half-solving it would be churn.
- [ ] **`block()` finds the first heading with a given name**, so two functions sharing a
      name across components remain a blind spot in the scoped needle checks.

## Coverage audit, and the four faults it found — 2026-08-30

A cheaper model swept the text renderer and the drawing for tests that pass without
proving anything. It confirmed the repair below held — the whole class of bug the first
review found is now caught — and found four more faults that the suite could not see. All
four are verified by hand, fixed with tests that kill them, and the tests were watched
going red under each fault before being kept.

**Line coverage was not measurable: neither tool is installed, and nothing was installed
to get a number.** That is less of a loss than it sounds. Every line involved in all four
faults below *executes* under the old tests; a coverage tool would have called them
covered. Mutation survival is what found them, and it is the number this project should
keep quoting.

- [x] **An arrow touching the outside world could silently lose the only words saying it
      is unchecked.** The note fires when either end is an outside party; requiring
      *both* ends — which essentially never happens — deleted it from every real edge and
      no test noticed. Now every such edge is checked against the document.
- [x] **`--focus` could show the wrong half of the tree in detail.** What is inside the
      component you named is meant to be spelled out; the boxes above it stay plain so
      they do not bury it. Swapping those two was invisible to roughly ninety lines of
      focus tests, because they all check the geometry of whatever got drawn and never
      that the right things got drawn. My first attempt at the guard was itself useless —
      I picked the target and an unrelated component, which sit on the same side of the
      swap and pass either way. It needed one component inside the target and one above
      it.
- [x] **Nesting in the text was not checked at all.** Indentation is the only thing
      grouping a function with its component there — no boxes, no lines — and handing a
      child the same depth as its parent, flattening a whole subtree, left every test
      green. Every assertion was "these words appear somewhere", and somewhere is not the
      same place. Depth is now checked against the document for every component and
      function.

**Recorded, not fixed.** The installed command's `--text` test compares the command's
output against a second call to the same function, so it proves the wiring and cannot
prove the content. That is the right division of labour — content is the render tests'
job — but it is worth knowing that assertion is wiring-only.

**Still unmeasured.** The drawing module is 4,000+ lines and only three points in it were
mutation-tested. Standing mutation coverage exists for the verdict kernel and nothing
else; extending it past the kernel remains open.

## Review of the transcript, and what it found — 2026-08-30

A second model reviewed the feature below. It was right about almost everything, and the
headline is bad: **the safety net was largely an illusion.** Thirteen deliberate
breakages of the text renderer were run against the whole suite and only one died. All
are now fixed and all die.

- [x] **The worst one: the feature lied in the exact place it was sold on.** A function
      that wrote no checks line at all, and inherited an empty list from an ancestor, was
      told it had *written* an empty list. Same sentence, byte for byte, as a function
      that really did write one — the two opposite statements this view exists to keep
      apart. It now says which ancestor switched checking off, and says the function did
      not ask for it.
- [x] **The completeness test skipped four fields it never mentioned:** the seal, the
      build-fails-here flag, machine-written functions, and worked examples. Deleting the
      entire worked-examples block left every test green. Both structures are now bound
      field by field with no catch-all, so a field added later stops the test compiling
      rather than being quietly unchecked. Two fixtures that exercise those fields were
      added to the set it reads; there was no `mode: synth` in any of them before.
- [x] **The derived sentences had no test at all.** They restate no field, so a walk over
      fields cannot see them — deleting how strongly a component is checked, or why, or
      that it declares nothing yet, all passed. Every component block must now answer both
      questions, and each of the sentences that answers them is pinned word for word.
- [x] **Wrong rule, wrong severity, in the sentence a reader would quote.** Both views
      said a sealed component touching a capability "is an error (A0408)". It is A0403,
      and a warning unless the component is also marked to fail the build. A0408 is a
      different rule about helpers used inside contracts. Pre-existing in the drawing; the
      text form copied it onto a second surface instead of catching it.
- [x] **A component marked sealed *and* declaring capabilities silently lost the
      capability list** — the view telling a reader the document said less than it did,
      and dropping the half that would explain a surprising finding.
- [x] **Two header sentences were false.** "Nothing here has been run" is not something a
      renderer handed a parsed document can know, and is flatly wrong for anyone who just
      ran a verification; it now says no result reaches this page. And the summary's gloss
      called the counted functions "code this document says nothing about" — in the
      trading-system example both counted functions wrote `checks: []`, so the document
      says something very deliberate about 2 of the 2 it described.
- [x] **Both views claimed an enforcement that does not exist.** An open question was said
      to cap a function's checks; §5.6 says in as many words that the cap is not enforced,
      and a verification runs the full claim anyway. `worklist` has always said so on
      every marker line. The two views now do too, and share one copy of the sentence
      instead of two.
- [x] Newbie bar: "contract at the watermark" (jargon, glossed by more jargon) and "the
      level above" (reads as the parent component, not the previous line) rewritten.
- [x] **`plural()` leaked a fresh allocation on every call**, under a comment claiming no
      allocation could enter. In this repo of all repos.
- [x] Spec §7.1a said the walk "visits every field" and that there is "only one wording"
      of a shared fact. Neither was true when written; both are retracted and replaced
      with what actually holds, including the day the seal sentence was worded two ways.

Goldens moved and were read, not accepted: the three vetting drawings and the
architecture diagram changed by exactly the three corrected sentences, no geometry.

**KNOWN GAP, left open on purpose.** A component's stated level ignores open questions,
so a component can say it declares checks up to the strongest level while a function
inside it says an open question holds it down. Both sentences are individually true and
they sit four lines apart. The fix belongs in the shared ceiling computation and changes
the drawing too, so it is its own change rather than a rider on this one.

## The transcript: the render as text — 2026-08-30

Measured, not assumed: on the committed trading-system diagram 474 characters are drawn
on the canvas and 9,923 are reachable only by hovering. 95% of what the render says --
and all of the reasoning -- is invisible to anyone who cannot hover, and a model reading
the document cannot hover at all.

- [x] **`ply-render --text` writes the whole document as prose.** Same facts as the
      drawing, including every sentence the drawing only shows on hover; generated on
      demand and never committed, so it cannot go stale. Goes to stdout or `-o`, exactly
      like the SVG.
- [x] **Combining `--text` with `--depth`/`--focus`/`--collapse` is refused, not
      ignored.** Those fold a drawing to fit a screen; the text has no screen. A reader
      handed a quietly-folded transcript would believe they had the complete view.
- [x] **Component-level default `checks:` are now stated.** Found by the new invariant,
      not by reading: `full.ply.yaml` declares `checks: [bounded(2)]` on a component and
      the transcript said nothing about it. That is the §5.4c distinction the transcript
      exists to make legible -- a default is invisible on every function that inherits
      it, and "nothing written" and "written empty" mean opposite things.
- [x] **The load-bearing invariant drives from the document, not the drawing**
      (`the_transcript_leaves_nothing_in_the_document_out`): every component, function,
      check, contract clause, capability, owned type, profile rule, default list, trusted
      claim, edge, forbidden rule, external and open question must be findable in the
      text. Four planted breakages (drop a forbidden rule, drop all but the first check,
      print the trusted claim where the evidence belongs, drop the component default) all
      die, each naming the dropped item.
- [x] **The older drawing-vs-text test had a doc comment that overclaimed** and now says
      what it checks: names only. About a third of what the picture says is glyph
      shorthand (`B2 F1024`, `e×1`, `⛉`, `*`) that the text spells out in words --
      demanding verbatim agreement would force the text to be as terse as the picture.
- [x] **The label/line gap ratchet came out.** It had been pinned down to 0 earlier in
      the same session, which made its `<=` a comparison that could not fail -- the same
      silence the ratchet was built to prevent. It is a flat assertion now.
- [x] Spec amended: §7.1a.

## Component notes, and the envelope's reasoning — 2026-08-28

Both from a second person's smoke test of the grammar, decided by review rather than on
the spot: the first instinct -- add a free-text `description` -- would have been wrong.

- [x] **A component may carry a `note:`.** Every prose slot in this grammar already sits
      where checking is impossible, and `externals`' note is *required* on exactly the
      reporter's own argument -- "a bare name tells a newbie nothing". A component's
      rationale is the fourth of those. Ply's own three load-bearing rules moved out of
      comments and into notes, so the file keeps its reasoning instead of discarding it.
- [x] **Not on functions.** The reporter had written a real invariant as an `examples`
      string because no better slot was visible; the answer there is `ensures`. A note
      beside it makes that mistake comfortable -- the next invariant lands in the note
      and never becomes a promise.
- [x] **The envelope carries the contract and the trusted claims.** The tree said only
      what came out, so an agent could read `fuzzed(64)` and not know what it was fuzzed
      for -- and §7.1 already assumed otherwise. Set from the claim rather than the run,
      so a reused verdict carries it too; wiring it to the run first and watching the
      second run come back bare is how that gap was found.
- [x] **`cargo ply render` says when a selection folded nothing away.** The subcommand
      that landed on main does not carry that notice; the standalone binary did. It was
      written for a first-time reader -- one of them recorded the silence as a bug before
      deciding it was correct behaviour -- and delivering it from one entry point and not
      the other would be a poor joke.
- [x] Half the report was wrong and is recorded as such rather than quietly fixed:
      `audit --json` has always carried the trust surface. What was missing was the tree.
- [ ] Comments still evaporate, deliberately: making them survive needs a position-aware
      second parse this codebase does not have, and would let prose reach output without
      passing validation. The component note is the declared home instead.
- [x] `cargo ply render --help` advertises the global `--json` flag. It now emits the
      declaration-only visual envelope, so clients can navigate the YAML hierarchy before
      code or evidence exists; every item is explicitly `unclaimed`.

## Agreed 2026-08-28, not yet done

**Order the maintainer set: land the release, then the bug backlog, then suite speed.**
Nothing below jumps that queue.

- [x] **Put the pre-code renderer on the installed command (`15adc47`).** The documented loop says
      Ply can render a `ply.yaml` before code exists, but the working renderer is shipped
      only as a separate development tool. `cargo ply render` must use that same renderer,
      accept its existing depth/focus/collapse controls, and write either stdout or an
      explicit SVG path. A second renderer is refused.

- [x] **The end-to-end CI job is sharded across a matrix of four.** The suite is 84
      independent test binaries that ran as one job for over an hour, which was most of
      the wait on every pull request. No test, no product code and nothing about what is
      checked changed. The split is computed at run time from the files on disk rather
      than written into the workflow, so a test added later lands in a shard by its
      position and cannot be silently left out of all of them -- a hand-maintained list
      would rot the first time someone forgot it, and a test nobody runs is the kind of
      absence this project treats as a defect. Round-robin rather than contiguous
      blocks, so the slow Kani-backed tests, which sit together alphabetically, spread
      3/2/2/3 instead of landing in one shard. Verified before pushing: the four shards
      are disjoint and cover all 84 files, and the exact command the workflow runs
      compiles a real shard. The honest cost: every shard pays the engine install, only
      partly cached, so the real figure sits above total/4.

- [ ] **Cut the duplicate proofs out of the end-to-end suite.** Measured today: 137
      fixture copies across 71 distinct fixtures, so the same code is proved many times
      over -- `resultreuse` and `implmethod` nine times each, `structenumparam` and
      `clamp` eight, `reusehelper` seven. Kani-heavy binaries alone are 1,020s of the
      3,053s of test time. The fix is test design, not infrastructure: several tests
      that each prove one fixture to assert on different parts of the same run become
      one test making several assertions about one run. Caching was already tried and
      measured (docs/suite-speed-finding.md): 2,533s before, 2,569s after, no speed-up,
      because those tests run concurrently and all miss together. Deferred by the
      maintainer until after the bug backlog -- a large refactor of the tests that
      vouch for a release is the wrong thing to do while landing one.

- [x] **The Rust toolchain is pinned (`rust-toolchain.toml`, 1.98.0).** Done 2026-08-30:
      CI and a contributor's machine now agree by construction rather than by anyone
      remembering to type `cargo +stable`. Raising it is deliberate — bump the line, run
      the suite, and fix the lints the new release brings in the same change.
- [ ] ~~Consider pinning the Rust toolchain (`rust-toolchain.toml`).~~ CI installs
      `stable`, which was 1.98.0; this container had 1.94.1, four releases behind, and
      clippy gained lints in between. Two `-D warnings` failures therefore could not be
      reproduced locally and were pushed red. Installing `stable` here and running
      `cargo +stable clippy` reproduces CI exactly and is the working practice from now
      on, but a pinned toolchain would make the two agree by construction rather than
      by remembering.

- [ ] **Make CI a required check on `main`, so a red pull request cannot be merged.**
      Asked for by the maintainer after PR #4 offered a merge button while all three
      jobs were failing -- twice, on two different causes. GitHub will not block a merge
      on a failing check unless the branch is protected and the check is named as
      required. Needs a branch protection rule on `main` requiring `product`,
      `product-e2e` and `tools` to pass. This is a repository setting, not a code
      change, so it needs doing in GitHub's settings by the repository owner (or via
      the API with admin rights) -- Ply cannot set it from here.

## Second smoke-test impression — 2026-08-28

- [x] **"The check badges are the one thing with no tooltip, while the canvas tooltip
      promises hover anything for its meaning."** Checked rather than assumed: the
      badges do resolve a tooltip, and it glosses each one in plain language
      ("bounded(2) — proves the contract for every input, unrolling loops at most 2
      times"). The claim was wrong; the instinct behind it was not.
- [x] **The invariant test that should have settled that question could not.**
      `every_drawn_item_resolves_a_tooltip` walked a hand-maintained list of classes: of
      the 35 the renderer emits, it named 14. A construct added later was explained only
      if someone remembered to add it, and nothing failed if they did not -- the same
      silent absence this project treats as a defect everywhere else. Inverted: every
      class emitted must resolve a tooltip, and anything that genuinely cannot has to be
      named as decoration, so a new construct fails until someone decides which it is.
- [x] Inverting it found exactly one real gap: the `ply.yaml` title on the canvas had no
      tooltip. It has one now.
- [x] **Every box now says why it is the colour it is.** The canvas tooltip explained
      the scale; no box said which function set its own shade, so finding the drag meant
      opening every chip in turn. Each box now names the weakest thing inside it and
      what that thing declares -- by path, so a function several components down is
      findable rather than merely blamed. A box that is white says which claim declares
      nothing at all.
- [x] The words cannot drift from the colour: a test walks every component in six
      documents and fails if the level the sentence names is not the level the box is
      painted. Watched red by making the search pick the strongest declaration instead
      of the weakest.

## Smoke test on a real project — 2026-08-28

The maintainer ran Ply against a project of their own and reported what broke. Findings
in their words, and what happened to each.

- [x] **`cargo ply --version` did not exist.** It reports two numbers now, because they
      answer different questions: the release, and the build identity -- the content
      hash that decides whether a stored result may be carried forward, so the number a
      run means when it says "the build of Ply that checked it changed".
- [x] **The help text contradicted itself**, describing the CLI as a slice that
      "implements only `verify`" while listing four working commands under it.
- [x] **`--depth`/`--focus` on a flat document emitted identical output silently**, which
      the maintainer initially recorded as a bug before deciding it was correct. It now
      says so: the note is earned by rendering the default drawing and comparing, so it
      cannot disagree with what was drawn.
- [x] **Flow labels stranded at `--depth 1`** -- one in the title band, one at y=162 on a
      canvas 152 tall, invisible. Reproduced exactly. The label placement escalates away
      from its line until it clears every box, and between two boxes side by side there
      is no such spot, so the search ran off the page. A position outside the drawn
      content is no longer a candidate. Pinned by an invariant test that walks the real
      output of several documents at several depths and fails on the first label outside
      the canvas.
- [ ] **KNOWN GAP, reproduced and not yet fixed: edge lines strike through box text.**
      An edge between two boxes with a third between them is drawn as a straight line
      through the middle box. Repro: three top-level components in a row, an edge from
      the first to the third. Fixing it means routing a line around obstacles -- a
      waypoint on the path and an obstacle test -- rather than the single straight
      segment `render_edge` draws today.
- [ ] **The architecture summary counts crates it never names.** "1 of 2 crates in this
      workspace belong to a declared component" is honest but not actionable: the crate
      that belongs to nothing is not named, so the reader cannot tell which. Found while
      checking a reviewer's claim that an undeclared dependency is skipped silently --
      it is skipped, but the coverage count does disclose it, so the fix is naming
      rather than counting.
- [x] **`check` cannot check architecture without a Cargo workspace**, so the pre-code
      half of the loop is render-only. True, and the README implied otherwise. The
      development-loop section now says which half of step 2 works before the code
      exists: the drawing always, the document's grammar always, the architecture check
      not until there is a Cargo project to read a dependency graph from.

## Bug fixes after 0.1.0 — branch `claude/bugfix-post-0-1-0`

- [x] **The README says how to install it.** There was no install path written down at
      all, which is a strange gap in a tool someone is about to try on a real project.
      Every command in the new section was run before it was written: installing the
      CLI straight from the repo, adding the attribute crate to a project that had
      never seen Ply, and getting a real `fuzzed(64)` out of `cargo ply verify` -- with
      the project's `Cargo.toml` byte-identical afterwards. The engine prerequisites
      (`kani-verifier` for `bounded`, `cargo-mutants` for `mutate`) are named, along
      with what Ply leaves on disk and what it does not.

- [x] **A constructor in a qualified `impl` block was invisible on the parameter path,
      and Ply said the type had none.** The receiver path learned in 2026-08-27 that
      `impl super::T`, `impl crate::T` and `impl Alias` name the same type as
      `impl T`; the parameter path kept the older bare-name-only rule. So a type
      declared in `lib.rs` with its `impl` block in a submodule -- which has no other
      spelling available -- was reported as having "no constructor Ply can call", about
      a type with a public `new`. A false sentence rather than a missing feature. Both
      paths now use the same resolution. An `impl` that ends in the same bare name but
      cannot be resolved to the type's own canonical module is still refused: building
      the wrong type is worse than refusing to build one.
- [x] **The same false sentence had a second cause, on both paths: a constructor that
      returns the type by name rather than `Self`.** `pub fn new(..) -> super::Quota`
      is ordinary Rust; only `-> Self` was recognised, so the constructor was invisible
      for an ordinary parameter *and* for a receiver, the receiver message reading
      "none was found". Found by probing the same family rather than by the suite. Both
      paths now accept the type's own name, resolved to its canonical declaration
      (`Confirmed` only -- another module's same-named type is a different type).
- [x] **Two more causes of the same sentence, found by writing ordinary Rust and
      watching Ply be wrong about it.** An `impl` block inside an inline `mod` in the
      same file was never looked at -- the scan walked only the file's top-level items;
      it now flattens inline modules, carrying the module path down so `super::` still
      resolves to the right place. And a parameter spelled `crate::Beta` rather than
      `Beta` was carried as the rendering of a token stream, `crate :: Beta`, which the
      by-bare-name type lookup could never match and no sentence should quote at a
      reader; a plain path now keeps its bare last segment. `ordinaryspellings` fixture
      and test, both watched red.
- [x] **REGRESSION I INTRODUCED, found by review and fixed.** The first version of the
      qualified-parameter fix trimmed every path to its last segment before looking the
      type up. A parameter naming another crate's type (`v: depx::Thing`) then resolved
      to a same-named local type, built the wrong thing, and reported a compile failure
      in Ply's own generated code -- a calm, correct refusal turned into an internal
      error blaming Ply. Reproduced outside the repo before fixing. A plain path now
      keeps its qualifiers, and a qualified spelling is accepted only when those
      qualifiers match the module the type is really declared in; `super::` with no
      module context to resolve against is refused rather than guessed at. Pinned in
      `ordinaryspellings` and watched red against the trimming version.

- [ ] **OPEN, out for review: a type whose only constructor is `impl Default`.** Ply
      says "it has no constructor Ply can call", which is false -- `T::default()` is a
      constructor anyone can call. Building via Default yields exactly one value, so a
      `fuzz(256)` claim would report 256 cases having tried one distinct input, which
      is the silent-narrowing failure this project exists to prevent. Three options
      (correct the sentence only; build via Default; build via Default plus the bounded
      operation sequence the receiver path already uses) went to a reviewer, whose
      answer is: take the third, and correct the sentence now regardless, because it is
      false on both the parameter and the receiver path today. The second option cannot
      be made honest by disclosure, because the case count in the verdict is itself the
      claim: 256 runs of one value is one test run 256 times. Two cautions recorded
      with it -- `#[derive(Default)]` declares no `fn default` in the source, so a scan
      reading only `impl` blocks would recognise the hand-written one and miss the
      derived one; and when a type has no operations at all the sequence degenerates
      back to a single value, which needs the count clamped or the disclosure escalated
      rather than the general sentence quietly covering it.
- [x] **A constructor found and then found unusable is no longer reported as absent.**
      Every refusal opened "it has no constructor Ply can call", which is true only
      while nothing was found. A constructor Ply found and could not use -- private, or
      with an argument it cannot build, `fn new(n: impl Into<u32>)` being the ordinary
      case -- was recorded and then dropped by every refusal arm. The note now replaces
      that clause wherever it exists, so the sentence names what was found and why.
- [x] Fixed alongside: types were quoted with token-stream spacing (`impl Into < u32 >`,
      `Vec < u8 >`) in every message that fell back to that rendering; and
      `NotFound`'s wording told someone whose parameter was `impl Into<u32>` that Ply
      found no such struct declared, sending them to look for a declaration they never
      wrote.
- [x] **The Default-only sentence is corrected**, as the reviewer said to do regardless
      of the construction work. A type whose only constructor is `Default` -- written by
      hand or derived, both now recognised -- is told exactly that, and told why Ply
      does not build through it yet: one value is not many sampled cases, however many
      times it is run. Construction itself (option (iii)) stays open below.
- [x] **A type declared inside an inline `pub mod` was invisible to the type index.**
      The blindness fixed in the constructor scan was still in
      `scan_crate_type_locations`, which walked only each file's top-level items and so
      recorded an inline-module type as living at the crate root -- where it matched
      neither the `holder::Gauge` a caller writes nor the path the generated harness
      has to import. A fully public type with a public `new` was refused as if Ply had
      never heard of it. The index now records the inline `mod` chain alongside the
      file, and the harness imports the type by its real path. A type inside a
      *private* inline module is real and unreachable, which is a different answer
      again, and now gets its own sentence rather than the generic refusal.
- [x] KNOWN GAP, unchanged and now written where it is: a `Result<Self, E>`
      constructor is recognised for a parameter and still not for a receiver.
- [x] `qualifiedctor` fixture and end-to-end test, watched red. The test also weakens
      what the constructor guarantees and requires the verdict to go red, so a green
      run cannot be Ply quietly not calling it. It asserts each path's own leaf verdict
      rather than the worst-of root, which either path alone could satisfy.
- [x] Fixed while there: a diagnostic quoted a type as `super :: Quota` -- token-stream
      spacing leaking into a sentence held to the newbie bar. It now reads
      `super::Quota`, as written.

## Forced colour made Ply blind to its own engines — 2026-08-28

- [x] **Every compiler error reached Ply as `\x1b[1m\x1b[91merror\x1b[0m: ...` under
      `CARGO_TERM_COLOR=always`, and nothing matched.** Ply reads its engines'
      output line-first -- a compiler error is a line beginning `error`, attributed to
      a function by the `-->` span under it -- so forced colour meant it could neither
      pin a build failure to the function that caused it nor quote the compiler. It
      fell back to "the compiler gave no specific error line": a sentence written for a
      failure genuinely beyond attribution, printed for one that was entirely
      attributable. A true sentence in the wrong place, which reads like the tool
      working.
- [x] Found by CI, which sets that variable, on a test that had been green locally for
      months. Engine output is now stripped of ANSI escapes (CSI and OSC, including
      terminal hyperlinks) before anything parses it -- at every engine, not just
      cargo, so the next tool to add colour costs nothing.
- [x] Regression test runs the real fixture with the variable set and asserts the
      compiler's own message survives; watched red, and the pre-existing test stayed
      green under the same sabotage, which is why it never caught this.

## Ply's own architecture, rendered and checked — 2026-08-28

- [x] **ARCHITECTURE.md**, with the diagram rendered from the root `ply.yaml` rather
      than drawn by hand, and linked from the README. A test in the render tools fails
      if the committed SVG stops matching what the spec renders to, so the page cannot
      go stale quietly; watched red by adding a crate to the spec.
- [x] **Running Ply on Ply found a real violation of Ply's own rule, and it is fixed
      rather than declared away.** `ply_e2e` had grown a dependency on `ply_core` that
      no edge allowed -- the type-coverage measurement reads core's classifier directly
      so the published count cannot drift. Declaring `e2e -> core` would have made the
      run green by widening the rule to the whole suite to excuse one file (the crate
      tier cannot say "one test file may"), leaving every other e2e test resting on a
      convention the checker could no longer enforce. The measurement moved to
      `ply-core`'s own tests instead, beside the classifier it measures: the rule is
      intact, the edge is gone, `e2e` depends on nothing again.
- [x] Fixed while there: the architecture summary said "1 real crate dependencies
      cross" -- now "1 real crate dependency crosses". The unit test had been pinning
      the ungrammatical wording, so it was updated to the corrected sentence.
- [x] Fixed while there: a document that declares no fn claims at all was told "NOT
      RESOLVED ... none of the 0 fn claims were ever looked for" -- a failure that did
      not happen, the mirror image of the bug the previous entry fixed. It now says
      there were no fn claims to resolve.

## `check` on a crate with no library stops reading clean — 2026-08-28

- [x] **A binary-only crate got a clean `check` and a refusal from `verify`.** Found
      while answering whether the fast command needs code to be there. With no
      `src/lib.rs`, every claim was counted as "anchored to another crate" -- the shape
      of a boundary somebody chose -- and the run exited 0 having resolved nothing.
      `verify` on the same crate said `E0301`, exit 1. The two commands now agree: the
      claim is unresolved, the missing library is named as the obstacle, and the
      summary says a search did not happen instead of reporting a zero.
- [x] End-to-end test in `check_command`, watched red against the old behaviour; both
      commands asserted on the same crate in the same test. The-Ply-Spec.md §5.2
      amended.
- Note: the document half still needs no code at all -- a `ply.yaml` alone in an empty
  directory gets its grammar checked, which is the spec-first loop working as intended.

## Ply borrows the user's Cargo.toml and gives it back — 2026-08-28

- [x] **A run on a crate with its own workspace no longer leaves an edit behind.**
      Found while cleaning up a stray modified file in this checkout, which turned out
      to be a Ply run artifact, then reproduced on a scratch crate outside the repo.
      Registering the generated harness as a workspace member is still how the mutate
      engine finds it, but the registration is now held by a guard that puts the
      original manifest back byte-for-byte when the run ends, error paths included.
      Confirmed end to end: a real crate with `members = ["."]`, checked with both
      `fuzz` and `mutate`, comes back `fuzzed(64)·spec-strong` with its `Cargo.toml`
      byte-identical to what was there before.
- [x] **The generated failing test survives the cleanup.** Removing the membership
      alone would orphan the harness -- neither a workspace root nor a member of one,
      so unbuildable -- and the counterexample Ply just rendered would be unrunnable.
      The same guard release rewrites the harness manifest into the standalone shape;
      `cargo test` in `target/ply/fuzz/<name>/` fails on the seeded bug afterwards,
      checked directly rather than assumed.
- [x] Four unit tests, each watched red against the specific defect it names (restore,
      whole-line removal, don't-touch-what-changed, clear a stale entry), plus the
      `existingworkspace` end-to-end test rewritten to assert the new contract.
      The-Ply-Spec.md §5.4c amended.
- KNOWN GAP (in the spec, deliberately): a run killed outright runs no guard, so the
  `members` entry survives a `SIGKILL` or a crashed container. The next run clears it,
  since the restore target is always the original minus the harness entry.

## D5's first branch lands: `stub_verified` — 2026-08-26

Full write-up of both red-first passes (the feature, then the reuse gap an adversarial
review found in it) belongs beside this entry's own literal failures, recorded here
since no separate doc was asked for this session.

- [x] **`stub_verified` works, mechanically, against the generated harnesses.** Confirmed
      by direct reproduction against real `cargo kani` runs (not just unit tests): a
      caller stubbing a callee proved clean this run verifies in a fraction of a second
      via `#[kani::stub_verified]` plus a never-run "existence" harness satisfying
      Kani's purely-syntactic existence check (`tests/spike/FINDINGS.md` item 4 --
      confirmed again here, not just cited). §5.5's opening claim ("verification runs
      callees-before-callers") is real now: a topological order over the call graph,
      ties broken by node id, cycles falling back to the second branch.
- [x] **Ordering** (`callgraph`/`verify.rs`): claimed functions with a `bounded` check
      are ordered callees-before-callers; a cycle (mutual recursion, direct or
      transitive) cannot be ordered and every claim in it falls back to D5's second
      branch, `conditional`, exactly as before this feature -- not an error, not a hang
      (`stubverifiedcycle` fixture).
- [x] **The bound composes to the weaker of the two, never the caller's own declared
      one** (`stubverifiedminbound` fixture) -- the anti-overclaim test, and the one that
      matters most.
- [x] **A real Kani limitation found and worked around, not papered over**: plain
      `#[kani::stub]` cannot target a function that itself carries a contract (issue
      #4591, "Failed to find contract closure" -- a compile error killing the whole
      crate, reproduced here against this feature after `tests/spike/kani-pin` found it
      for a different case). Both of D5's branches, when reached through a same-crate
      contracted callee, therefore use `#[kani::stub_verified]` mechanically; what marks
      one `conditional` and not the other is entirely Ply's own bookkeeping (did the
      ordering above establish the callee clean this run), never anything Kani checks.
- [x] **A second defect, found by adversarial review of this feature and not by any
      test already in the suite**: the composed bound depends on a callee's *earned*
      verdict, and nothing hashed it. Editing only a callee's declared `checks:` (its
      bound going from `bounded(5)` down to `bounded(2)`, no source touched anywhere)
      correctly re-earned the callee's own record while the caller's record -- and its
      now-stale deeper bound -- went untouched. Closed by adding `verified_bounds` to
      `FingerprintInputs` (`record.rs`, new `INPUT_GROUPS` entry "the callees it stands
      on") and by *deferring* a bounded-eligible claim's reuse lookup until its
      fingerprint is finalised in dependency order, rather than deciding it from the
      Pass-1 fingerprint before that composition is known. Pinned permanently:
      `stubverifiedstalebound` fixture, red-first against the isolated defect, green
      with the fix restored.
- [x] **The same restructuring incidentally fixed a second, independent bug the review
      also caught**: an earlier version of the ordered pass decided "reused" before
      ordering and then unconditionally re-ran every bounded-eligible claim's engine
      regardless, wasting the exact cost reuse exists to avoid and writing a proof
      module for a claim the envelope reported `reused: true` -- caught by
      `resultreuse_fixture` going from 5/7 to 7/7 once the ordered pass was made to
      consult the reuse decision instead of ignoring it.
- [x] The missing "§5.5's limits" subsection §5.5 already pointed at ("see this
      section's limits below") now exists, gathering: cross-crate `stub_verified` (out
      of scope for v1, unchanged), a call outside the workspace, a call Ply's reader
      cannot see, branch one requiring a callee *clean* (never merely `bounded`-shaped
      -- a conservative, stated restriction on composing across more than one hop of
      assumption), the cycle fallback being decided per claim rather than per edge, and
      the whole mechanism's soundness resting entirely on Ply's own scheduler, never on
      Kani (`tests/spike/FINDINGS.md` item 4, restated where a reader of this rule would
      need it).
- [x] **A third defect, found independently by re-verifying this commit against a
      fresh fixture rather than trusting the 313 green tests that had just landed**:
      standing on a proved callee made a caller *permanently* unreusable, not merely
      briefly stale. The `verified_bounds` fix above closed the stale-number gap, but
      `record.rs`'s own "is this verdict one the declared checks could earn" integrity
      check (`W0516`) predates D5's first branch and still assumed a `bounded(k)` check
      could only ever produce `bounded(k)` verbatim -- so a claim declaring `bounded(5)`
      that genuinely composed down to `bounded(2)` looked identical to a hand-edited
      record on every run after the one that earned it, and was silently re-verified
      from scratch, forever, paying full engine cost each time. Confirmed by direct
      instrumentation before touching anything (the lookup and stored fingerprints for
      the caller were byte-identical, including the new "callees it stands on" group --
      ruling out the first, more obvious suspicion before fixing anything): the actual
      divergence was `W0516` itself, refusing an exact-fingerprint match because the
      composed verdict's number differed from the check's own. Fixed by making
      `bounded(k)` earn any `bounded(j)` with `j <= k`, never only `k` itself, and never
      a `j` deeper than declared -- the one place a stored verdict is allowed to differ
      from its own check's number, stated as exactly that in `verdict_is_earnable`.
      Pinned permanently: `stubverifiedwarmreuse` test (reusing the plain `stubverified`
      fixture, two runs, nothing edited between them) -- red-first against the isolated
      defect, green with the fix restored, run three consecutive warm runs by hand
      first to confirm the fix holds indefinitely and not just once.
- [x] Fixtures: `stubverified`, `stubverifiedminbound`, `stubverifiedcycle`,
      `stubverifiedfuzzedcallee`, `stubverifiedstalebound` (`tests/fixtures/`), each
      with its own e2e test under `tests/e2e/tests/` (`stubverifiedwarmreuse`'s test
      reuses the `stubverified` fixture rather than needing its own). `cargo test
      --workspace`: 315 passed, 0 failed, fmt and clippy clean.
- [ ] KNOWN GAP, deliberate: a claim declaring **both** `bounded` and `fuzz`/`test` in
      the same `checks:` list is bounded-eligible (needs the ordered pass) but its
      fuzz/test portion needs the harness crate built in the *unchanged*, earlier pass
      that the ordered pass's own reuse decision now runs after. No current fixture
      declares such a mixed list, so this has not bitten anything real, but it is not
      solved either -- recorded here rather than discovered later.
- [ ] KNOWN GAP, stated in §5.5's new limits paragraph: branch one requires a callee's
      own verdict to be clean (never `conditional`) before this claim can stand on it.
      Composing branch one across more than one hop of assumption -- does a claim
      resting on a clean callee that itself rested on a clean callee inherit anything
      the second hop assumed, transitively -- is a real question this design declines
      to answer rather than guesses at.

## Third adversarial review of D5's first branch — 2026-08-26

`docs/review-callees-first.md`, three BLOCKING findings (D1, D2, D3) plus three
non-blocking-but-real ones (D4, D5, D6). Every fix red-first, with the literal failure
text captured before the fix went back in.

- [x] **D1 (BLOCKING) — branch one composed against a callee whose own proof did not
      cover the caller's argument.** A `bounded(k)` proof over a length-indexed
      parameter (`Vec<u8>`, a slice, `BTreeSet`, an array) only ever builds values up to
      length `k`, not the type's full value space -- composing a caller's bound against
      it assumes the callee's contract holds on arguments its own proof never touched.
      Reproduced live: a callee proved only over vectors of length <= 2 returns a value
      breaking its own postcondition at length 3; a caller always passing length 3
      composed to a clean `bounded(2)`, exit 0, false. Red-first (`stubverifiedveclen`
      fixture, domain gate disabled): `f` came back `"verdict":"bounded(2)"`,
      `"statuses":[]` -- no `conditional`, no `owed-evidence`, the exact false-clean
      shape the review named. Fixed with `RustType::is_full_domain()`
      (`crates/ply-core/src/harness.rs`): a callee with any non-full-domain parameter is
      excluded from branch one, whatever its own verdict, falling back to branch two
      exactly like a cycle does. **Narrowed, not proved**: a fixed-size `[T; N]` array is
      excluded too, conservatively, even though its size is part of the type and an
      argument-containment argument might one day admit it safely -- that argument is
      not made here. Green: `f` composes to `bounded(2)` with `conditional`/
      `owed-evidence`, `W0511` present, no `W0517`, exit 0.
- [x] **D2 (BLOCKING) — a same-crate contracted callee Ply cannot build a stub for was
      silently inlined.** A tuple-pattern parameter, a `self` parameter, or an
      unparseable contract attribute made `build_contract_fn` fail for the callee, and
      the `if let ... && ...` chain deciding D5's first two branches had no `else`: the
      failure fell through with no stub, no refusal, no diagnostic, contradicting this
      commit's own "always stubbed, never inlined" claim. Red-first (`stubverifiedtuparg`
      fixture, the new refusal gate disabled): `f` came back a real, freshly-computed
      `"verdict":"bounded(2)"` in 32.55s -- Kani had genuinely compiled and inlined `g`'s
      real tuple-pattern body, exactly the silent inlining the review named. Fixed: the
      match rewritten with every arm explicit, a new `unstubbable_contracted` field on
      `BoundaryPlan`, and a new diagnostic (`W0512`, `unbuildable_contracted_stub_diag`)
      naming the callee and why its stand-in could not be built. Green: `g` and `f` both
      report `unclaimed`, `W0512` names `g`, exit 1.
- [x] **D3 (BLOCKING) — the widened tamper check accepted a hand-edited overclaim.**
      Round 2's fix for the stale-bound defect widened `W0516`'s "is this verdict
      earnable" check to accept any `bounded(j)` with `j <= k` for a claim declared
      `bounded(k)` -- necessary, because branch one can genuinely compose to a shallower
      `j`. But the review showed this was a strict superset of what composition can
      produce: a hand-edited `bounded(4)` for a claim that had actually composed to
      `bounded(2)` passed, because `4 <= 5` (the claim's own declared bound) was the only
      thing checked. Red-first (`stubverifiedtamperedbound` fixture: run once, hand-edit
      the stored `bounded(2)` to `bounded(4)`, re-run against the pre-fix "any `j <= k`"
      rule): the tampered `bounded(4)` was silently accepted, no `W0516`, exactly the
      overclaim the review reproduced. Fixed (`record.rs`'s `verdict_is_earnable`): the
      expected value is now pinned exactly, `min(declared_k, min(the bound each
      stood-on callee earned))`, read from `verified_bounds`, and a stored verdict must
      equal that number, never merely sit under it. Green: the tampered record is
      refused, `W0516` present, re-verified to the honest `bounded(2)`, exit 0.
- [x] **D4 (non-blocking, real) — `audit`/`worklist` never saw an inline-contracted
      assumption.** Both commands read only the `ply.yaml`-declared boundary-contract
      route; a same-crate callee assumed through its own inline `#[ply::requires]`/
      `#[ply::ensures]` (branch two, reached whenever that callee is not itself an
      independently bounded-checked claim) reported `conditional`/`owed-evidence`
      correctly at `verify` time while `audit`'s trust surface and `worklist`'s count
      both stayed silent -- §5.5's own honesty condition 3 not holding for this class.
      Red-first (`stubverifiedinlineaudit` fixture, the new listing arm disabled): both
      commands reported "(0)" -- zero assumed contracts, zero owed evidence -- for a
      callee `verify` itself marks conditional. Fixed in `shared.rs`'s
      `assumed_contracts`, narrowly: listed whenever the callee carries no `bounded`
      check anywhere in the document. **Known gap, not solved**: a same-crate callee
      that *is* claimed `bounded` elsewhere but still lands on branch two at `verify`
      time (a cycle, or an unclean run) needs the same ordering computation `verify`
      does to tell "stood on" from "assumed" -- this listing does not attempt that and
      under-reports exactly that case. Green: both commands report "(1)", naming the
      callee, the caller, and the promise text.
- [x] **D5 (non-blocking, real) — the vacuity gate fired on a proved callee's inline
      contract.** §5.5's emptiness/vacuity check (E0502/E0503) exists to interrogate
      branch two's *assumed* clauses; it was running over every stub's contract
      regardless of branch, including a callee proved clean this run (branch one), whose
      inline contract is real evidence rather than a promise being trusted sight-unseen.
      Fixed by filtering the stub list fed to the vacuity gate to assumed-only
      (`is_assumed()`) before it runs. A second, smaller wording defect went with it:
      diagnostic text in this area said a contract was "declared in ply.yaml" in
      contexts where it could just as easily be an inline `#[ply::requires]`/
      `#[ply::ensures]` on the callee itself -- corrected in `W0511`'s
      `conditional_verdict_diag` and the three E0502/E0503 title strings to describe the
      contract neutrally rather than naming the wrong source.
- [x] **D6 (non-blocking, real) — a mutation-decorated verdict broke bound parsing.**
      `parse_bound` (`verify.rs`, feeds the `known_bounded` map branch one composes
      against) matched only a bare `bounded(k)` string; a `·spec-strong`-decorated
      verdict failed to parse, silently dropping that claim out of `known_bounded` and
      its callers to branch two. Fixed by stripping the `·spec-strong` suffix before
      parsing, matching the same handling `record.rs`'s `verdict_is_earnable` already
      had for the identical shape.
- [x] Fixtures added, each with its own permanent e2e test:
      `stubverifiedveclen` (D1, the reviewer's length-indexed-parameter shape, entered
      the suite permanently as required), `stubverifiedtuparg` (D2, a tuple-pattern
      parameter), `stubverifiedtamperedbound` (D3, a live hand-edit-and-reverify
      reproduction), `stubverifiedinlineaudit` (D4, `audit` + `worklist`). All four
      confirmed red-first against the isolated defect and green with the fix restored,
      including a red-first pass over D3's own live e2e reproduction (hand-editing
      `ply.lock` and re-running against the pre-fix "any `j <= k`" rule accepted the
      tampered `bounded(4)`; the fix refuses it and re-earns `bounded(2)`). D6 also
      carries its own unit test (`parse_bound`, spec-strong-decorated input).
      `cargo test --workspace`: 321 passed, 0 failed, 0 ignored, across 50 test
      binaries; `cargo fmt --all` made no changes; `cargo clippy --workspace
      --all-targets -- -D warnings` clean.

## Phase 1a — landed 2026-08-25

Full write-up with verbatim red-first failures: `docs/phase-1a.md`.

- [x] **The `ply.yaml` model lives once, in the product** (`ceb52aa`). `tools/model` and
      `tools/check`'s library became `ply_core::{model,check}`; the hand-rolled subset in
      `config.rs` is deleted, closing that file's own `TODO(M1)` ("promote one, delete the
      other"). `tools/render` and `tools/check`'s binary consume them by path dependency.
      Behaviour-preserving: 169 passed / 0 failed on the full suite, committed SVGs
      byte-identical, fmt and clippy clean in both workspaces.
- [x] **`schema/ply.schema.json` exists and is normative** (`c8528ce`) — §5/D3 have called
      it that since the spec was written while the file did not exist. Load-bearing, not
      decorative: the `E0204` key vocabulary and required-field list are read out of it at
      runtime and six Rust constants that duplicated them are gone. It now rejects things
      the product silently accepted: non-snake_case names (§5.1a rule 2 was enforced by
      nothing), unknown capabilities and bans, `unresolved` id 0 — and, found by the
      schema-vs-parser invariant test rather than by design, `fuzz(0256)`/`fuzz(+5)`, which
      the parser had inherited from `u32::from_str`. All 49 existing documents still pass.
- [x] **`cargo ply check`** (`5212cfa`) — schema + anchor tiers, `--json`, exit 0/1/2,
      **0.074s with no engine installed**. It reports what it did NOT check
      (`coverage.not_checked`) and says plainly that every node reading `unclaimed` means
      this command gathered no evidence, not that the code is unverified.

- [ ] `check`'s staleness tier — blocked on `ply.lock` (Phase 1c); its absence is currently
      declared in `coverage.not_checked`.
- [x] `check`'s architecture tier — crate level BUILT (`6fac707`), and it reads the real
      dependency graph rather than guessing. Carries a known defect found by review: it is
      blind to binary-only crates, so it reports a clean pass on this repo's own violation.
      FIXED (`a4c8675`), verified against this repo in both directions, and the repo now
      declares and checks its own architecture (`ply.yaml`, committed). A second review
      then found a blocker that outranks it: a run in which the architecture check could
      not happen at all — a broken manifest, no cargo, or a dependency cycle — prints "No
      problems found" and exits 0, so CI goes green on a run that checked nothing. That is
      the eighth instance of absence-of-evidence reading as success. Reproduced
      independently; fix dispatched. The item level is CANCELLED as specified — see the resolvability
      measurement (`fed5bf3`): one call site in five is resolvable from source, so that tier
      would report on a minority of the program and its silence would read as approval.
- [ ] JSON-pointer → (line, col) index for `E0201`/`E0204` (§5). The pointer ships; the line
      does not, and §5 now says a guessed line is worse than none.
- [ ] Multi-file `ply.yaml` discovery and merge (§5) — and with it, `E0202` across files,
      currently unreachable.
- [ ] Wire `--fail-on` / `--only-changed` to `check` once a tier exists they can mean
      something for.
- [ ] `check` should accept a loose `*.ply.yaml` path, so `tools/check`'s binary can retire.
- [ ] `.archi/ply.json`'s "Tooling Today" diagram still shows `cargo ply` as not built —
      stale since M3, not since this phase.
- [ ] KNOWN GAP, deliberate: `discover_fn` sees only top-level fns in `src/lib.rs`. `check`
      inherits the limit so it never passes an anchor `verify` would fail; the diagnostic
      now says which of the two failures you hit, which makes the limit visible.

## Result reuse, and the gap a review found in it — landed 2026-08-25

Full write-up with the literal red-first failures and the timings: `docs/reuse-hash-gap-closed.md`.
The review that forced it: `docs/review-result-reuse.md`.

- [x] **Ply remembers a checked result and skips re-checking while nothing it depended on
      moved** (`107a491`, superseded below). Cold 11.8s / warm 0.028s on the small fixture;
      97.3s / 0.067s on the older one.
- [x] **The review found the feature's load-bearing claim false, and it was fixed before
      merge** (`c650e55`, write-up `eca129f`). The hash covered the checked function's own
      lines and the promises written for old code it calls — not the ordinary helpers the
      check actually runs, not the bodies a proof walks into, not the worked examples, not
      the resolved dependency versions. Break a helper so the checked function genuinely
      violates its own guarantee, and the tool answered "carried forward, still fine" in
      0.03s while printing a line claiming the code hashed the same. That is this
      project's own worst failure mode — a green result over code nobody checked — and it
      is the seventh instance of it found and closed.

      The hash now covers every first-party body a check can reach: through calls, through
      a function named as a value, and through the claim's own contract expression. Where
      the walk cannot be trusted — a method call, a hand-written operator, a macro, an
      unrecognised attribute — it is abandoned and the whole crate is hashed instead:
      coarser, never wrong, and which mode ran is itself hashed. Allowlist on purpose, so
      an unanticipated construct costs engine time rather than a false pass.

      Verified independently of the agent that built it, on a fixture written for the
      purpose: helper broken → the claim re-runs and reports the violation with a
      counterexample, while an unrelated claim in the same crate keeps its result; only the
      unrelated claim edited → the reaching claim still reuses; nothing touched → both
      reuse in 0.039s; and the method-behind-a-type case, which no syntactic walk could
      follow, caught by the coarse mode with the unrelated claim honestly re-run too.

- [x] **Two smaller review findings closed in the same commit.** A stored verdict none of
      its own checks could ever have produced is now refused, said out loud, and re-run,
      instead of being believed forever. And a run that cannot use a stored result now
      names the input that moved — distinguishing "the function's own source changed" from
      "the code it runs changed" — rather than silently re-paying full cost.
- [x] **Every place claiming the hash covered "everything" now states what it covers and
      what it does not** — the spec, the schema page, the module comment, the fixture
      comment, the exact-string test, and the line printed on every reused run. §5.2a
      carries a "what it does not cover, stated rather than implied" paragraph.

- [ ] KNOWN GAP, recorded not hidden: build environment that never appears in a file
      (`RUSTFLAGS`, `[profile]` settings such as `overflow-checks`, a `#[path]` attribute)
      is not hashed; nor is what an outside proc macro expands to beyond its crate
      identity. Reuse across machines needs a committed lockfile — without one Ply records
      that it does not know the versions rather than guessing.
- [ ] KNOWN GAP: a hand-edited record is caught only where the stored verdict is one the
      stored checks could never earn. A hash cannot defend the file against a text editor.
- [ ] The fuzz engine's recorded version is the requirement written in the manifest, not
      the version actually resolved.
- [x] **The coarse mode now explains itself** (`bf6048f`). It already worked out why
      it abandoned the call walk and kept it to itself, so a person who edited one function
      and watched an unrelated claim re-run was told only "the code it runs changed" — true,
      and useless. The run now names the construct that cost the walk and says the crate is
      the unit: *"For `x` and `y`, \"the code it runs\" means every line of the crate, not
      only the functions they call, because src/lib.rs declares an `impl` block for
      `Scaler`, and Ply cannot tell by reading the source which of its bodies a method call
      or an operator would run."* Said once per crate however many claims it displaced —
      the first build repeated it per claim, which reading the real output caught — and
      never printed for a bounded walk, where it would be false. Both of those are pinned
      by tests, the second one negatively. New fixture `reusewiden` carries the shape.
- [ ] Open question, deliberately left: whether every Ply release should invalidate every
      record. Fable's answer was yes — a hand-maintained "only when it matters" flag
      recreates the judgment call the design exists to eliminate. Not yet revisited.

## Reach — types, methods, and four false cleans found closing them (2026-08-27)

Driven by a yardstick rather than by opinion: `tests/fixtures/ratelimiter/` is a working
rate limiter designed by someone told NOT to think about checkability and not told this
project existed. Every number below is measured against it.

- [x] **`usize`, `isize`, the `NonZero` family and `Duration`** (`4ce1c1c`). Type coverage
      on the yardstick went from 3 of 70 uses to 25. My own estimate had been 82 percent;
      the real figure is 36, because a duration nested inside an `Option` is not a bare
      one and covering those would reintroduce the false-counterexample risk the work
      exists to prevent. Second over-estimate of the day, same mistake both times:
      counting something adjacent to the question and reporting it as the answer.
- [x] **Methods resolve, and receiverless functions check** (`c1ea364`). Ply could not find
      methods at all — its own schema documented `Type::method` and the anchor did not
      resolve — and **no config in this repository that had ever been run through a real
      check claimed a method**, which is why nobody noticed. Two defects were found by hand
      before it landed: the generated harness imported a method as if it were an importable
      item, and separately **any** zero-parameter function failed the same way, latent since
      the sampling tier was built.
- [x] **The ninth and tenth false cleans** (`62f4c74`, review `7f6bfe8`). Ply decided which
      function a promise was ABOUT separately from which function the test would CALL, so
      two same-named types in different modules made them disagree: a promise saying the
      answer is 999, on a body returning 5, reported a clean pass on a different function
      entirely. Fixed structurally — the called path is now derived from the resolution, so
      the two cannot drift. Building the multi-module fixture that proved it then exposed
      the tenth and worse: **the filter selecting which generated test to run matched
      nothing for any method, so zero tests ran and the result was reported as held.** Every
      method check had been passing without executing anything.
- [x] **Floats, and one type list per engine** (`2443b85`). A type the sampler can build is
      now checkable even where the prover cannot reason about it; a proof requested on such
      a type is refused by name. Floats reach the property the yardstick's author named as
      least trusted. Three honesty features arrived unasked: a run whose precondition
      rejected 92 of 156 draws says its 64 accepted cases are weaker evidence than the
      number suggests; NaN and infinity are excluded by default and the run says it
      therefore says nothing about them; an unrenderable failing input says so and adds
      that Ply never invents one.

- [ ] **Strings and collections on the sampling tier.** Did not land; the mechanism now
      exists. **Demoted 2026-08-30, and the old justification here was withdrawn** -- it
      read "this is where parsing and validation bugs live, so it is the highest-value
      remaining type work", which is a guess dressed as a ranking. Build it when a
      property somebody wrote down is blocked on it and nothing else, not as a programme.
      See "What widening the types is actually worth" below.
- [ ] KNOWN GAP: a constructor returning `Result<Self, E>` — a real shape in the yardstick
      — is still refused.
- [ ] KNOWN GAP: `NonZero` and `Duration` are top-level only, never nested inside `Option`,
      `Result`, arrays or collections.
- [ ] Building a receiver, so methods that need one become checkable. Design settled in
      `docs/review-self-construction.md`; the fourth option there (constructor plus a
      bounded sequence of the type's own operations, with the length reported) is the one
      to build, not constructor-only.
- [ ] **The suite re-proves the same fixture up to eight times per run**, which is most of
      the wall clock on every verification cycle. Making a proof earned once in a run serve
      the other tests that need it would turn forty-minute waits into minutes. Deferred all
      day behind more interesting work; it is now the biggest drag on the loop.

**The method that actually worked, recorded because it is the transferable part:** verify
with a promise that is FALSE. A passing check proves nothing — my own verification of the
methods work used a true promise, saw a pass, and called it verified while nothing was
running at all. Ten false cleans on this branch, every one found by real code or
adversarial review, none by the suite.

## Agreed with the maintainer, not yet started

- [x] **The promise ramp widened, and a contrast invariant added** (2026-08-29). The
      maintainer said the render had gone monotone. Measured, and he was right for a
      reason worth recording: adjacent rungs of the ramp sat 1.13-1.33 contrast apart,
      close to indistinguishable, so the evidence ladder -- this tool's main lever -- was
      encoded in the least readable way available. Now 1.27-1.54 per step, full range
      2.18 -> 3.15. Writing the check also found a real legibility defect that predated
      it: the anchor and ownership lines sat at 2.2 against the strongest fill, i.e.
      unreadable on exactly the boxes a reader most wants to read. Both fixed, and held by
      a contrast floor over every ink/fill pair.

- [ ] **KNOWN GAP -- the render is monotone by construction until results reach pixels.**
      The palette is a two-state design: grey means promised, green means earned. Only the
      first state can be drawn today, because `render_svg_with_evidence` post-processes a
      finished string, so no run result can change a pixel. That means every diagram
      anyone sees is the "before" half, permanently, and the moment the design pays off
      never arrives. The colour was removed and the thing that puts it back was deferred.
      This is the strongest argument yet for doing the evidence plumbing next: it is not
      one blocked feature among five, it is the half of the palette that makes the other
      half worth having.

- [x] **A dark palette, following the reader's system setting** (2026-08-29). The render
      paints its own near-white background, so a dark-mode reader got a bright panel. One
      alternative palette, explicitly **not** a theming hook: the colour meanings are
      enforced by CI, and a user-redefinable palette would make every one of those
      guarantees unenforceable. The colour-blindness floor now runs against both palettes
      and immediately earned its keep -- it rejected the first dark red proposed, which sat
      0.200 from ordinary structure, inside the measured confusable band. Replaced with one
      that clears the floor against both structure and the attention amber.

- [x] **Diagram-layout and cartography evidence applied** (2026-08-29). The research doc
      cited perception and notation theory but no experimental work on diagram *layout*.
      Two fields have it. Landed: (a) **a shallow-crossing invariant** — crossings are the
      one layout property with a large replicated effect on reading speed, and the useful
      form is the refinement (a near-90° crossing is ignored by the eye; a shallow one
      costs accuracy), so shallow ones are now forbidden in CI while ordinary ones are
      not; (b) **"route edges orthogonally" struck from the research doc** as folklore --
      controlled studies found no measurable benefit, and it manufactures exactly the
      shallow near-parallel runs the eye-tracking work condemns; (c) **position and size
      named in the spec's channel table**, including the ones that mean nothing. Evidence
      for (c): on the Underground map the drawn geometry moved route choice about twice as
      much as travellers' own journey times, so readers trust geometry whether or not it
      was meant to carry meaning.

- [ ] **KNOWN GAP, newly measured: four forbidden-call lines are drawn along each other.**
      Found by writing the crossing invariant, not known before. Three share one vertical
      corridor in vetting 003 and two share a horizontal run, so they render as a single
      line and a declared rule goes invisible. Worse than a crossing: a crossing slows a
      reader, an overlap hides a rule. Pinned at 4 as a ratchet. The fix is giving deny
      routes their own lanes, the way regular edges already have them.
      **Correction worth recording:** an earlier measurement in-session reported "zero
      crossings in every diagram". That was true of X-shaped crossings and completely
      missed these overlaps, because the detector used treated collinear segments as
      non-crossing. The repo's own `segments_cross` does not, which is how they surfaced.

- [ ] **When results can reach the drawing: the chips are the promise/earned encoding,
      not a meter.** Supersedes the research doc's two-part meter *and* the split-fill idea
      proposed in-session. Both were wrong for the same reason, from opposite directions:
      a small meter is small first and an encoding second (size is the most detectable
      visual variable, so a tiny track is the least noticeable thing available); and a
      split box fill creates a second enclosed region, and enclosure is the strongest
      grouping cue there is -- it would perceptually sort the chips inside into "earned"
      and "not", which is a lie unless made true. The evidence favours discrete countable
      units over continuous fills for part-whole reading, with the advantage largest for
      untrained readers -- which is the newbie bar. So: colour earned *chips*, sort them
      together so the grouping the eye infers is true, and use a split fill only on
      collapsed boxes where no chips can contradict it. Invariant when it lands: a
      collapsed box's split must equal the earned-over-promised count folded beneath it,
      and no chip may be coloured earned without a result behind it.

- [ ] **If a diff view is ever wanted: side-by-side, with changes drawn as marks.** A
      reader comparing two renders by eye *will* miss changes -- change blindness is among
      the most robust findings in vision science, and two separately-opened files are its
      worst case. Small multiples measured faster than animation on every comprehension
      task; animation won only on "what was added just now". Never ship "spot the
      difference".

- [ ] **Decided against, with reasons, so they are not re-proposed:** crossing-*count*
      minimisation in the layout (effect sizes come from dense abstract graphs; Ply's are
      single-digit box counts, and reordering rows trades away the version-to-version
      stability a diff view would want); symmetry as a layout goal (a weak effect, weaker
      still in the study closest to Ply); a fold-prominence test (marks already draw at
      fixed size at every depth -- verified -- so the test would compare two constants);
      and Lynch's *Image of the City* as evidence (thirty sketch-map interviews per city,
      no tasks, no measurements -- useful vocabulary, not a source you can test against).

- [x] **Four more of the visual-language items applied** (2026-08-29), after the maintainer
      correctly pointed out that the first three changed almost nothing visible: they were
      all *subtractive* (green removed, red removed, two labels moved), and every additive
      idea was still outstanding. (a) **Absence is now hatched** rather than left blank --
      the single biggest visible change, and the document's sharpest perceptual point.
      (b) **A verdict strip** opens every render with what is declared and how much of it
      promises nothing. (c) **Checks read as words when zoomed in** -- `B3 F4096 M` becomes
      "proves for all inputs, loops up to 3 / tries 4096 random inputs / plants bugs; the
      checks must catch them", following the same zoom rule the contract clauses use.
      (d) **A colour-blindness gate in CI.** Its first version was nearly vacuous -- the
      distance metric passed pure red against pure green, the textbook confusion -- so it
      was rewritten around the guarantee that actually holds: every meaning also carries a
      non-colour mark. The weak floor is kept, documented as weak.

- [ ] **KNOWN GAP -- five of the document's items are blocked on one missing thing.**
      `render_svg_with_evidence` calls `render_svg(doc)` and then post-processes the
      finished string, so run results cannot affect a single pixel. Broken, evolving, the
      promise/earned split, "working well", and the result-side counts in the strip all
      need evidence to reach the drawing stage. That plumbing, not any individual glyph, is
      the next real piece of work, and the research document never mentions it.

- [ ] **Judgement call to revisit: at focus, a fn chip now shows both `B3 F4096 M` and the
      spelled-out lines.** The document wanted the letters confined to hover. Kept both so
      the chip looks the same at either zoom, with the words as the expansion -- but it is
      redundant, and if the band gets busier the letters should go.

- [ ] **Decided against: making the strict notch and pure seal more visible.** The document
      asks for one or the other (amplify or demote); demote is the honest answer. "Strict"
      means findings here are errors rather than warnings, which only matters when there IS
      a finding -- at which point red already shouts. Spending a glance-level channel on a
      modifier only legible alongside another signal is a poor trade. Hover-tier.

- [x] **The diagram no longer paints promises green** (2026-08-29, from
      `docs/visual-language-research.md` via Fable's review). Two of the review's three
      "land now" items done. (a) Capability tags were red — the same red as a real
      failure — though a declared capability is neither forbidden nor wrong; they are now
      neutral, and the deny lines visibly alarm for the first time. (b) The promise ramp
      moved off green onto ordered neutral greys, so a project where nothing has run no
      longer renders as a field of healthy green. That was §1's absence-of-evidence
      failure drawn in pixels, by the tool that exists to prevent it. Both are guarded by
      invariants over the emitted stylesheet, not spot-checks: red must belong to
      something forbidden or wrong, and un-run work must contain no green. The-Ply-Spec.md's
      channel-discipline rule is amended, retracting "pastel = promised, saturated = earned"
      with the argument for why it neither held nor could hold.

- [x] **Edge lines no longer strike through labels** (2026-08-29). The third and last of
      the review's "land now" items, and the longest-standing recorded render defect. The
      ratchet that pinned this debt at 13 collisions is now at **0**. Two causes, both
      real: the placement search checked candidate positions against the canvas edge and
      the boxes but never against the *lines*, and it ran before the forbidden-call routes
      existed, so a label could not avoid a line not yet routed. Placement now runs as a
      second pass once every line exists, and candidates vary where along the edge the
      label sits as well as how far out it is pushed -- without that second axis a steep
      edge's perpendicular is nearly horizontal, so every candidate slid the label *along*
      the horizontal line it was stuck under. Exactly 2 labels moved across the whole
      corpus; 001 and 002 are byte-identical.

- [ ] **Deferred with a bug to fix first: the two-part promise/earned meter.** Good idea,
      wrong arithmetic as written — it folds by *summing*, so a collapsed box with nine
      earned functions and one untouched reads 90% healthy, which is exactly what the
      kernel's first standing obligation forbids. Fold by weakest-descendant plus a count
      instead. Also needs earned-result data to reach pixels, a path that today exists only
      as tooltip text — that, not the drawing, is the real lift.

- [ ] **Cut from the proposal, recorded so it is not re-proposed:** rank-band layout
      (position already means containment; a second meaning breaks the one-meaning rule and
      would draw layers nobody declared), demoting deny bars to a lock glyph (a red barred
      line is the most instinctive form in the grammar, and it draws a *rule*, not an
      alarm), and a third amber corner flag (two markers for that already exist, one in the
      same corner as the strict notch). Also: the document claims the check badges carry no
      tooltip — false, verified in the renderer; the defensible version is that hover is
      not glanceable.

- [x] **A focused function now draws its promise, instead of hiding it in hover** (2026-08-29).
      Prompted by the maintainer's question of whether the visual language is sufficient --
      "at a glance it is not always obvious what a function does". It already was declared:
      `requires`/`ensures` are contract clauses, and unlike free pseudocode they cannot drift
      into fiction, because a check stands behind them. They were simply hover-only. Now
      `--focus` draws them as `needs`/`gives` lines under the fn name. Rejected in the same
      breath: adding a separate unchecked pseudocode block, which would put the only ink on
      the canvas with no evidence behind it -- the exact failure Ply exists to prevent.

- [ ] **KNOWN GAP -- only one function in the whole vetting corpus declares a contract.**
      Found while looking for an example to render: 001 and 002 declare none at all, 003
      declares exactly one (`check_order`). So the clause band is real but barely exercised
      by our own scenarios, and the vetting corpus is not currently evidence that contracts
      are pleasant to write at scale. Worth writing clauses into 001/002 as a grammar
      exercise in their own right -- that is what `vetting/` is for.

- [x] **Automatic bug-planting now runs against the kernel on every build** (2026-08-28).
      Fable's call, taken: the exhaustive tree check is the gate, but whether it can SEE
      is a measured property that can regress, and nothing was keeping the 2026-08-25
      repair true. First run: 35 planted bugs, 24 caught, **6 survived** -- none in the
      aggregation logic, all six in the status-set helper, and every one genuine rather
      than an artefact. Three were an unused `union` the merge site was hand-rolling
      around (now called, so the duplication is gone and those die by construction); one
      an unpinned `FromIterator`; one `is_empty`, whose only behaviour-changing consumer
      is the renderer in another crate; and one the `Debug` impl, which could be blanked
      leaving every test green while every future counterexample printed unreadable.
      After the fixes: **30 caught, 5 unviable, 0 survivors.** The CI job requires zero
      and deliberately carries no excused-failures list.

- [ ] **Mutation coverage stops at the kernel.** Fable's second recommendation, not yet
      done: an occasional (not gating) run over the model/check parsing and validation
      code -- the surface neither the exhaustive check nor the unbounded proof reaches,
      and where the third hand-planted fault of 2026-08-25 lived ("check 0 examples"
      accepted). Expect a noisy first survivor list; triage into this file rather than
      gating on it, because a gate with a hastily-blessed baseline is theatre.

- [ ] **The "your filter hid nothing" notice is written out twice.** `cargo ply render`
      and the standalone renderer each build the same read-parse-draw-and-warn sequence,
      so the notice a first-time user relies on lives in two places and can drift out of
      one of them. Closed PR #8 already factors it into one shared helper: fetch it with
      `git fetch origin refs/pull/8/head` (commit `968a274`; that reference outlives the
      branch being deleted) and lift the helper onto current `main` rather than rewriting
      it. Recorded 2026-08-28 when #8 was closed as superseded, so the one good idea in it
      is not lost with the branch.

- [x] **DECIDED 2026-08-26: the architecture verdict is code, never a model.** The
      question was raised and settled: since the architecture tier is *approximate* by
      nature — warnings by default, an escape hatch that takes a written reason — would a
      model be the better enforcer? No. Approximate and nondeterministic are different
      properties: a source-reading checker is wrong in the same places every run and can
      enumerate what it could not see, while a model is wrong in different places each run
      and cannot. Full argument, including two repairs to the reasoning that reached the
      right answer for partly wrong reasons: `docs/review-architecture-enforcement.md`
      (`b32deba`).

      What that review changed about the plan, and what to actually do when this starts:

      - **Crate tier first.** Cargo already knows the dependency graph exactly. Cheap,
        sound, errors rather than warnings. Then turn it on this repository immediately —
        self-hosting is the point, not a follow-up.
      - **Measure before committing to depth.** A rough count says ~60% of this
        workspace's own call sites are method calls, the shape a source reader handles
        worst. Turn that estimate into a real number first; it decides how much
        function-level checking is worth building at all.
      - **The hand-run spike is rejected as designed** — circular (a model drafts the
        description it then checks itself against), and unscoreable. Replaced with seeded
        violations, including one behind dynamic dispatch, measured against ground truth.
      - **The unexhausted middle rung**: a real type-resolving backend sits between
        reading source and asking a model, and the extractor was made swappable for
        exactly that. "Source reader or model" was a false choice.
      - **A model's place is upstream only** — drafting the architecture description,
        triaging call sites the reader could not place, proposing where an exception is
        warranted. Propose, never decide. Anything it proposes gets confirmed
        mechanically (the span exists, the item exists, the name matches) before it is
        ever reported.
      - **KNOWN RISK, recorded because it fails quietly**: "propose, never decide"
        collapses the moment proposals are rubber-stamped. A model-drafted architecture
        description that someone skims and approves is architecture-as-vibes laundered
        through a deterministic checker. The rule that goldens are reviewed rather than
        blind-accepted has to reach it.
      - **Not an exception to the thesis.** A model rung has no oracle, and `strict: true`
        means this tier is designed to gate merges — so "it's only warnings" is not a
        defence available here.


- [ ] **D7 for stubbed crossings — `W0541 stub_substituted`** (planned
      2026-08-25, `docs/plans/d7-stub-failures.md`). The Kani-pin spike proved a
      stub-caused failure has no faithful plain-Rust reproduction, on any engine version:
      the rendered test calls the real callee, which never returns the stub's invented
      value, so it is emitted **green** (that test is in `tests/fixtures/boundarycontract`
      right now and passes). D7's unqualified red-test promise is corrected in the spec
      today. Build: a third `W0541` reason, the fabricated value + admitting clause in the
      diagnostic, a `fixes` entry proposing the tightening, and *stop emitting the passing
      `ply_cex_*` test* — a green reproduction that reproduces nothing is worse than none.
      Refused by name: rendering the test against a rewritten body, which would go red for
      a program the user does not run.

- [x] **Trusted boundary declared in `ply.yaml`** — CLOSED 2026-08-25, will not be built.
      The gate below answered no on evidence, and the maintainer closed the idea in
      conversation the same day: a per-function promise is the whole of what was wanted,
      and there is to be no region-wide or module-wide variant. Kept here for the
      evidence, not as pending work. Original framing follows. — the
      coarse-grained sibling of §5.5's per-callee rule: declare a region taken as given,
      rather than writing a contract per legacy function a new feature happens to touch.
      Fills a real hole in §7.2's taxonomy — *our code, checkable in principle,
      deliberately not checked* — which is distinct from an `external` (someone else's
      system, permanently). Three conditions agreed up front, all learned the hard way
      here: it must never read as evidence (crossing one marks the caller `conditional`,
      never clean, or trusting the whole tree goes green); it must be counted on the
      audit surface so the trusted region is under pressure to shrink; and it must draw,
      per §7.1's gate. Carry `trusted`'s own lesson: it shipped with no staleness and an
      attestation would have outlived the code it vouched for. Proposal first, gate,
      then adopt — the sequence that worked for external elements.

      **GATE RUN 2026-08-25 — the answer is no; take the fallback.**
      `docs/plans/trusted-boundary.md` bound itself to one empirical claim: that real
      callers are defensive enough to verify with the callee replaced by an
      unconstrained symbolic return, and that "if most crossings fail under havoc, this
      is a hint generator wearing a grammar construct." **Most crossings fail: 2 of 8
      pass (25%), and both passes are 004's own functions.** Zero of six callers written
      without the experiment in mind passed; six of six failed, five with a
      counterexample and one by timing out at the 300s floor with no witness at all.
      The falsifiable prediction on record **held** — `tier_fee_cents` passes under
      havoc (133.74s, inside the floor) because of its own `.min(10_000)`, and so does
      `approve_withdrawal` above it (212.63s) — but it held only on the two functions
      the plan already knew about. Cost is not the objection: havoc costs the same as a
      declared contract stub (133.74s vs 148.62s on the same function) and lands inside
      §6's floor. Three findings the plan has no row for: a havoc'd loop bound turns a
      22s proof into a 300s timeout with no diagnostic; the breaking value names the
      callee and the direction but never the threshold (`2_813_465` where the contract
      needed is `<= 100_000`); and Ply would print the *least* useful witness where
      several exist. **Recommendation: do not build `given:` as a grammar construct;
      adopt the plan's own open question 6 fallback** — let a clause-free boundary entry
      mean havoc, per callee, no new grammar (the codegen already emits it; only
      `verify.rs`'s `if claim.requires.is_empty() && claim.ensures.is_empty() { continue; }`
      stands in the way). Evidence, fixtures and a reproducing `run.sh`:
      `tests/spike/havoc/FINDINGS.md`. Commit `ff15b23`.

## Kani pin — spiked 2026-08-25; recommendation: stay put, two gaps left open

- [x] **Bump the Kani pin — a D13-shaped spike, not a fork.** Ran, against Kani `main`
      built from source (`245709373965fcb78209135822cbafb59c08d036`, 2026-08-25, CBMC
      6.10.0, `nightly-2026-04-01`) beside the untouched 0.67.0. **Recommendation: do
      not move the pin, and do not fork.** Four measured reasons.
      (1) There is nothing to bump *to* — crates.io's newest `kani-verifier` is still
      0.67.0, so a bump means pinning a commit of an unreleased branch that still
      reports itself as `0.67.0`, which would stamp two different engines with one D14
      fingerprint.
      (2) Blocker 2 is **not fixed**: `#[kani::stub]` over a contracted target still
      fails with `Failed to find contract closure __kani_recursion_check_<fn>` on
      today's `main` (Kani #4591, open).
      (3) Blocker 1 as recorded here was **never true** — at 0.67.0 a stubbed harness
      that fails *does* print a concrete-playback witness, and Ply's own
      `extract_witness_bytes` would accept it. The real limit, identical on both
      toolchains and stated in Kani's own generated doc comment, is that the playback
      test **does not apply the stub**: replaying a stub-caused failure panics on
      leftover concrete values instead of reproducing anything. That is worse than the
      documented blocker because a naive "the test is red" check passes.
      (4) Ply's real §5.5 shape already works at the pin — `boundarycontract`'s stubbed
      proof verifies (94.6s, 85 checks) and a violation in the same configuration yields
      a witness — at ~12-14% *lower* cost than the candidate (107.7s, 110 checks).
      Evidence, fixtures and a reproducing `run.sh`: `tests/spike/kani-pin/FINDINGS.md`.
      Commit `82555a9`.
- [ ] **KNOWN GAP, raised by the spike, no Kani version fixes it.** §5.5 can produce a
      violation that no test of the real code can reproduce: the counterexample's third
      value is the *stub's* invented return, and the real callee never returns it. Written
      out D7-style at the two real inputs, the test is green
      (`tests/spike/kani-pin/boundary/src/lib.rs::witness_replay`, observed passing).
      §8 forbids a witness-free `violation`; here the witness exists but is not
      replayable. A spec conversation about §5.5/§8, not an engine upgrade.
- [ ] **`boundarycontract`'s clean proof does not exercise its own assumption.** Delete
      the generated stub's `kani::assume` so the callee is unconstrained and
      `ply_proof_tiered_fee` still verifies (86.4s at the pin, 107.1s on the candidate):
      `legacy_rate(tier).min(10_000)` clamps whatever comes back, so the proof holds for
      *any* callee. The `conditional` verdict is still honest, but the fixture does not
      show the assumption doing work. Consider adding a harness that does (the spike's
      `tiered_fee_halfclaim` is one).

## Post-004 review closure — landed 2026-08-25

Disposition of every finding in `docs/review-post-004-fixes.md`, with the red-first
failure message and literal before/after for each: `docs/post-004-review-closure.md`.
Six commits, one per finding.

- [x] **Review D1 (MAJOR) — an ordinary `use` import bypassed the boundary rule.** The
      resolver never read `use` declarations, so a bare-name call classified `Unresolved`
      and `Unresolved` meant descend: `bounded(2)`, zero diagnostics, **exit 0** in
      40.562s over an unclaimed body, against `unclaimed`/`W0512`/**exit 1** in 0.007s for
      the identical claim spelled with a qualified path. Resolution now follows `use`
      (renames, groups, globs), inline and file modules, re-exports, and a path
      dependency's `src/lib.rs`; first-party source Ply cannot read is refused (`W0513`,
      new) rather than descended into. New fixture `useimport` + e2e, nine `ply-core` unit
      tests. §5.5's "just never a first-party one" retracted here, in
      `docs/post-004-fixes.md` and in this file. Commit `e83ccb9`.
- [x] **Review D2 — the fail-by-default rule missed absences encoded as statuses.**
      `mutate` with cargo-mutants masked: `fuzzed(64)` + status `inconclusive` + exit 0,
      against §6's own exit-3 row. The rule is now over **names, not slots** — one
      absence vocabulary read against a node's verdict *and* its statuses — and `mutate`'s
      non-results name which absence they are, so exit 3/2/1 follow the fact. Masked-engine
      e2e (§9's matrix, first entry built) + unit tests, including that `conditional`,
      `owed-evidence`, `weak-spec` and `stale` still exit 0. §3's "It never fails the run"
      reconciled with §6. Commit `a92e61f`.
- [x] **Review D4 — `W0541`'s wording was false for the shapes it now fires on.** It named
      `BTreeSet`/`Vec` to users whose parameter is a `[u32; 4]`. It now names the
      parameters and types that blocked the rendering; `RustType::display_name` fixes the
      same omission in `X0901` ("`xs: `", type missing). Commit `681fc75`.
- [x] **Review D5 — `evidence` described runs that never happened.** `cases: n` was
      attached whenever `fuzz(n)` was *declared*: a harness that never compiled reported
      `cases: 64`. Now built where the run happens, with `cases` only when the count is
      real. Commit `283cd83`.
- [x] **Review D6 — `owed-evidence` was emitted but defined nowhere.** Defined in §0's
      glossary and D6's status list as the debt half of `conditional`; §5.5 calls it a
      status; the verdict kernel gains the variant and a round-trip test over the whole
      vocabulary. Enumeration unchanged and green. Commit `9c730dd`.
- [x] **Review G1 — the `conditional` path was dead at the tool's own defaults.** 004's
      `tier_fee_cents` is scalar-signature, so plain `cargo ply verify` gave it 60s and
      reported `timeout` in 1m6.776s, saying nothing about the assumption. A **stubbed**
      `bounded` harness now gets a 300s floor — derived split (a stub is knowable before
      the run and always trades concrete values for a symbolic one), fitted constant
      (201.77s measured, plus the ~107s CBMC variance the M3 findings recorded), with
      9.72s and ~110s as the second and third measurements showing the cost is the body's
      as much as the stub's. `K0601` explains the premium when there was one. The
      `boundarycontract` fixture now carries 004's body shape and its e2e passes **no**
      `--engine-timeout`: the only test that observes §6's default end to end.
      Commit `182e9e1`.
- [x] **Review O2, O3, O5 — overstatements corrected in place** ("Nothing was lost" on
      seeding; §5.5's present-tense `audit`/`worklist`; "s1/s2 behaviour unchanged", which
      item 2 falsified by flipping their exit codes).
- [x] **Review O4 — DISPUTED, with evidence.** The tree holds **21** e2e `#[test]`
      functions at `3adca0e`, counted file by file, so `70 + 11 + 21 = 102` was right and
      the review's 22 was wrong. Nothing changed.
- [ ] **KNOWN GAP — the boundary rule inspects the claimed function's own body only.**
      Until D5's first branch lands, a contracted callee `g` is inlined rather than
      stubbed, so an unclaimed callee one level below `g` still travels into the caller's
      proof unnamed. Same pattern as review D1, a different bypass; stated in §5.5's
      limits. Not started deliberately (out of that task's scope).
- [ ] **KNOWN GAP — calls Ply's reader cannot see are not call sites for the rule**:
      macro-generated calls, `#[path = "..."]` module attributes, function pointers and
      trait methods.
- [ ] **KNOWN GAP (review G2) — the assumed-contract enforcement loop, as ONE item**,
      because the three parts are one loop and their conjunction is the risk: (1) no
      vacuity check — a declared `ensures: ["|result| false"]` makes the stub's
      `kani::assume` unsatisfiable and the caller's proof vacuously green, and a
      `kani::cover!` after the stubbed call would catch it cheaply; (2) no staleness — D14
      fingerprints trusted claims, nothing fingerprints a declared boundary contract
      against the callee's body, so legacy code can change under a standing assumption
      (the hazard §5.4d closed for `trusted`, reopened one mechanism over); (3) no
      accumulating surface — `audit`/`worklist` are not built, so the debt lives only in
      per-run output that scrolls away, and the run is CI-green by default.
- [ ] **KNOWN GAP (review G3) — declared-contract keying assumes the anchor equals the
      Cargo.toml dependency key.** `ledger = { package = "real-name", path = ... }` with
      `anchor: real-name` would not match the path a caller writes, and the callee would
      classify `Unclaimed`. It fails **closed** (a loud `W0512` naming a callee whose
      contract the user just wrote), so this is a usability gap, not an honesty one. Fix:
      resolve the anchor through the same rename logic the resolver already has.
- [ ] **Recorded-entropy fuzz mode** (the review's complement to the seeding decision):
      vary the seed by default in some contexts and *always* record it, so cross-run
      detection accumulation comes back without reopening the re-roll-until-green channel
      that determinism closed.

## Post-004 fixes — landed 2026-08-25

Closes the five items `docs/review-post-004-strategy.md` sequenced after vetting 004.
Full write-up with literal before/after output, red-first failure messages and measured
costs: `docs/post-004-fixes.md`. Four commits, one per item plus item 1's spec-and-code.

- [x] **Finding 2 / D5's third branch — the boundary rule.** §5.5 rewritten (three-way
      split, three honesty conditions, and its own stated limits); §2's D5 row amended.
      Built: a `bounded` check whose fn calls a callee no contract describes refuses to
      descend and names it (`W0512`), 004's `run.sh s3` going from `timeout` after
      **11m23.094s** to a named refusal in **0m0.005s**. With a contract declared in
      `ply.yaml`, the callee is stubbed (`#[kani::stub]`, cross-crate, real) and the
      caller earns `bounded(2)` + statuses `["conditional", "owed-evidence"]` + `W0511`
      listing the assumption — 004's `run.sh s5`, **3m15.9s** wall at a 600s budget.
      Commit `2cf09c2`.
- [x] **Finding 7, `anchor:` half.** A component anchored at another crate is a boundary
      component: contracts read, `checks` not run here (`W0303`), no node. A fn entry
      declaring only `requires`/`ensures` is a boundary contract declaration, not a claim.
      Commit `2cf09c2`.
- [x] **Finding 1 — a run that checked nothing exits 0.** §1 gains the
      absence-of-evidence principle; §6's exit table gains the missing row and
      `--fail-on=warn|evidence|error` (default `evidence`, `error` the documented
      opt-out). Exit codes 2 and 3 are returned for the first time. Commit `d73558f`.
- [x] **Finding 4 — `fuzzed(n)` is not reproducible**, and the escalation the review
      added to it. Seed derived per fn, recorded in the §8 envelope as
      `evidence: { engine, seed, cases }`, `--seed <hex>` replays; proptest's own
      persisted-failure replay switched off. `run.sh s8`: six identical `fuzzed(256)`,
      where it used to split 3/3. And a panicking body now earns a `violation` carrying
      proptest's own shrunk input instead of `X0901` — the class of real bug that could
      not be reported at any seed. Commit `c8e231b`.
- [x] **Finding 7, `ensures:` half.** `config::validate_keys` enforces §5.1a rule 1 on the
      verify path (`E0204`, location, nearest key) against the **whole** §5 key
      vocabulary, so the keys `verify` ignores are still accepted. §5.1a amended to say
      the rule binds every reader of the file, and so does its converse. Commit `23e8f67`.
- [x] **Finding 5 — the implemented fragment is narrower than §5.4b.** `char`,
      `Option<T>`, `Result<T,E>`, `[T; N]` and top-level type aliases are in. Measured
      first (Kani `Verification Time`, trivial bodies): 0.028s `u32`, 0.064s `char`,
      0.036s `Option<u32>`, 0.040s `Result<u32,u8>`, 0.036s `[u32; 4]`, 0.041s
      `[u32; 16]`, 0.028s alias. No unwind annotation for an array — its bound is a
      compile-time constant. Commit `593cf9a`.
- [x] **D5's first branch IS implemented** (`5671ab5`, then `dc1e7ed`/`4ca1c9e` closing six
      defects an adversarial review found — one of them a false clean verdict). Superseded
      text follows, kept because its concrete example is still the right one. **KNOWN GAP
      (was) — D5's *first* branch is still not implemented.** A callee that passed
      its own Kani proof this run is inlined, not `stub_verified`, because callees-first
      scheduling (ADR-0003's "entire soundness guarantee", living unlinked in
      `tools/schedule`) is not promoted into the product. Concretely: 004's
      `total_debit_cents` still times out at 120s with `fee_cents` inlined. The review
      sequences this as the next tranche.
- [ ] **KNOWN GAP — §5.5's rule does not reach `std`/`core`/registry callees.** A call
      into a crate whose source Ply cannot read is left alone, so a `bounded` verdict can
      still include a body Ply never examined. Stated in §5.5, not left to be discovered.
      (The clause "just never a first-party one" was **retracted 2026-08-25**: an
      ordinary `use` import bypassed the rule entirely — see the closure of the review's
      D1 below.)
- [ ] **KNOWN GAP — a boundary assumption is reported as owed, and nothing exercises it.**
      §5.5 says an unexercised assumption is owed evidence and that `audit`/`worklist`
      list it. The `owed-evidence` status and `W0511` are built; `cargo ply audit` and
      `cargo ply worklist` are **not built**, and fuzz-checking a declared contract
      against the real legacy body is not built either.
- [ ] **KNOWN GAP — `ply.yaml` `requires`/`ensures` are still not ANDed into the fn's own
      check** (§5.4 says they are). They are read, and used for §5.5's boundary
      assumption; `W0510` now says out loud which of the two a user is getting.
- [ ] **KNOWN GAP — no witness decoder for the newly admitted shapes.** `char`,
      `Option`, `Result` and `[T; N]` reach the engines, but `WitnessValue` cannot spell
      them, so a Kani violation on one is reported `X0901`/`tool_error` naming the
      parameter (never a witness-free `violation`) and a fuzz violation lands on the
      existing `W0541` witness-only path.
- [ ] **NOT DONE, deliberately deferred**: cross-crate type-alias resolution (004's
      `withdraw` takes `ledger::AccountId`; resolving it changes nothing, because the
      `&mut ledger::Ledger` beside it keeps the fn `unsupported` either way), structs of
      scalars, `--only-changed`, `cargo ply check`, `schema/ply.schema.json`, and the
      renderer's earned-vs-declared split (finding 8).
- [ ] **Rendered cex test for a panicking body fails with the function's own panic**, not
      the contract message, because the call sits outside the test's `catch_unwind`.
      §9's cex-oracle clause "failure output states the contract" therefore does not hold
      for that shape; the contract is named in the generated test's comment. The oracle
      test itself (`clamp_oracle.rs`) is unaffected and green.
- [x] **`run.sh` budgets raised, annotated in place**: s5 120s → 600s (the stubbed proof
      needs ~202s of Kani time), s7 120s → 600s (arrays are cheap, this fn's body is not).
      Both original runs are quoted in `docs/post-004-fixes.md`. (A record of a done thing;
      the unchecked box was a bookkeeping slip the 2026-08-25 review caught.)

## Vetting 004 — legacy boundary, fragment-first — landed 2026-08-24

The first vetting scenario designed inside §5.4b's fragment from line one, and the first
run against the real `cargo ply verify` (Kani 0.67.0) rather than reasoned about on
paper. Write-up: `vetting/004-legacy-extension.md`; two crates + `ply.yaml` + `run.sh`
under `vetting/004-legacy-extension/`; SVG committed. Nothing in `crates/`, `tools/` or
`The-Ply-Spec.md` was touched — this scenario finds, a later session decides.

- [x] `legacy/` (ordinary `BTreeMap`/`Vec`/generic-helper module, no `ply::` anywhere) +
      `feature/` (five fns, all claimed, contracts inline) + one `ply.yaml` read by
      `verify`, `ply-check` (clean, exit 0) and `ply-render`.
- [x] Twelve `verify` invocations across `run.sh s1..s8`, all reproducible; every verdict
      quoted in the write-up is literal tool output (two long envelopes are cut to their
      verdict spine, and say so).
- [x] **The boundary's answer is `timeout`.** `tier_fee_cents` (fragment-clean signature,
      body calling one unclaimed `BTreeMap`-backed legacy fn) never finished: `timeout` at
      120s and again at **600s** (11m23s wall). Control: the identical fn with the legacy
      call replaced by a `match` earns `bounded(2)` in 1m20s total. `conditional`/D5 never
      fired — none of D5 (`stub_verified`, `W0511`, `ply-schedule`) is linked into
      `crates/` at all.
- [x] `--only-changed` and `cargo ply check` confirmed **absent** (§6 specifies both);
      recorded as findings, not built.
- [x] **Finding 1 — a run that checked nothing exits 0.** CLOSED 2026-08-25 (`d73558f`). `K0601 timeout` is warning
      severity, `--fail-on` is unimplemented, so a run whose root verdict is `timeout` is
      CI-green. Proposal in the write-up: absence of evidence fails by default.
- [x] **Finding 2 — D5 has no branch for an *unclaimed* callee.** CLOSED 2026-08-25 (`2cf09c2`). Both its branches assume
      the callee has a contract. Needs an explicit third rule, and the diagnostic must name
      the callee that was descended into (K0601 today names only the caller).
- [ ] **Finding 3 — checkability is about bodies, and §5.4b gates on types.**
      `total_debit_cents` (no legacy contact at all) also timed out at 120s in the same run
      where `fee_cents` passed.
- [x] **Finding 4 — `fuzzed(n)` is not reproducible.** CLOSED 2026-08-25 (`c8e231b`), with the panic-witness escalation. Six fresh runs of the *same*
      unfixed source: 3 × `fuzzed(256)`, 3 × `tool_error` (X0901, the real panic). Seed is
      entropy-derived (`Config { cases, ..default() }`) and recorded nowhere; exit code
      flips with it. The §8 envelope needs the seed, and a `--seed`/lockfile replay.
- [x] **Finding 5 — the implemented fragment is narrower than §5.4b.** CLOSED 2026-08-25 for arrays, aliases, `char`, `Option`, `Result`; structs of scalars still open. `[u32; 4]` (the
      spec's own *preferred* bounded shape) is `Unsupported`; so is a `type X = u64` alias.
      No `Type::Array` arm and no alias resolution in `rust_type_from_syn`.
- [ ] **Finding 6 — V0505's fix names a mechanism that does not exist** ("add a
      `pure`-marked generator hook"): no `#[ply::pure]` macro, no ply.yaml key.
- [x] **Finding 7 — `verify` is single-crate** — CLOSED 2026-08-25 for both halves (`anchor:` consumed in `2cf09c2`, `E0204` parity in `23e8f67`); multi-crate *verification* is still out of scope.: `anchor:` is parsed and never used, every
      component's fns are looked for in one `src/lib.rs`, and ply.yaml `requires`/`ensures`
      are silently dropped (unknown serde fields) while `ply-check` on the same file
      enforces `additionalProperties: false`.
- [ ] **Finding 8 — the render draws declared ceilings as earned.** `tier_fee_cents B2`
      and `withdraw B2` (unsupported!) draw exactly like the fn that really earned
      `bounded(2)`. Already on this list as "separate declared ceilings from earned
      verdicts"; 004 is the first live instance.
- [ ] **Finding 9 — `--only-changed` is the delta thesis's mechanism**, not a convenience.
- [ ] **Finding 10 — `verify` writes into the crate under test** (generated modules in
      `src/`, harness member appended to `[workspace]`), which is why `run.sh` copies to a
      scratch tree. Second vote for the "where the harness crate should live" item below.
- [ ] **NOT COVERED: 004's document is outside the renderer's invariant sweep.**
      `tools/render/tests/render.rs` walks a hardcoded list of 001/002/003 plus its own
      fixtures. Adding 004 means editing `tools/`, which this session was not permitted to
      do. The committed SVG was rasterised (CairoSVG) and checked by eye instead.
- [ ] NOT RUN, recorded: `mutate`/`prove` in this scenario; a boundary callee with a
      non-scalar signature; any bound other than `bounded(2)`; any budget above 600s.

## External systems and actors — landed 2026-08-24 (31a669d)

Full detail in `docs/external-elements-adoption.md`; the gate this was conditioned
on (vetting re-run before any spec amendment) is recorded as a numbered finding
plus an "external-elements gate" section in `vetting/003-trading-system.md`.

- [x] `tools/model`: `externals:` (`External { note }`, required field) and
      per-fn `entry: Vec<String>` on `FnClaim`.
- [x] `tools/check`: five new document-local rules — `E0202` (name collides with a
      component), `E0207` (external in a `->`/`deny`), `E0208`
      (`external ~> external`), `E0209` (`entry:` names an undeclared external),
      `W0410` (external declared, never referenced) — all fixture-tested,
      `tools/check/tests/externals.rs`.
- [x] `tools/render`: external box outside the frame, `~>`/derived `entry:` edges
      routed around intervening components, frame border weight bumped to read as
      a boundary. New invariant `frame_boundary::no_external_box_intersects_
      the_frame_deny_wildcards_stay_inside_and_external_edges_cross_once`
      (`tools/render/tests/render.rs`) — written red first (confirmed: it failed
      on its own vacuous-pass guard before the renderer had any external support,
      not a compile error), green after, mutation-tested (two real mutations,
      each reverted). Fixed two pre-existing routing-algorithm limitations the
      real 003 picture exposed (wrong rail-side heuristic, obstruction filter too
      narrow) in a new dedicated function, without touching the existing
      (already-tested) deny-line routing at all.
- [x] **Correction, same session, coordinator review**: the committed
      `003-trading-system.svg` (`--collapse ingest`) drew `venue ~> ingest.feed`
      straight through `strategy`/`signals` — a real crossing
      `no_drawn_element_intersects_a_box_it_is_not_inside` did not catch, because
      that test never rendered `--collapse <name>` for any single component, only
      "default" and "--depth 1" (collapse-everything). Root cause and full fix in
      `docs/external-elements-adoption.md`; short version: the test now sweeps
      one `--collapse` per top-level component per fixture (watched go red on
      the exact defect, plus a second, previously-unknown one on
      `--collapse gateway` crossing `pnl`, before the routing fix landed), and
      the router's first-leg sweep — sound for deny's always-off-to-the-side
      `from`, unsound for an external edge's `from` (an ordinary component
      border, which can sit inside another component's column) — now tries a
      straight vertical run first and only detours sideways when that specific
      run is blocked. Both 003 SVGs regenerated again and rasterised with
      headless Chromium; confirmed by eye, no line crosses a box it shouldn't.
- [x] **Second correction, same session, coordinator review**: the committed
      `003-trading-system.svg`'s `RawFrame` edge label was struck by a drawn
      line (the derived `entry` edge's, not even its own path) — same shape of
      gap as the correction above, this time in text, not boxes. Extended
      `no_drawn_element_intersects_a_box_it_is_not_inside` to check every
      `edge-label` against every drawn line (tried and rejected two narrower
      forms first — all-text-vs-all-lines produced false positives on `any`
      `*`/deny `except` labels, own-path-only produced a false negative on this
      exact bug, since the striking line belonged to a *different* edge);
      watched red first, naming the exact label and line, before any placement
      code changed. Fixed by splitting external-edge rendering into a
      route-then-draw two-pass structure so each label can be checked against
      every sibling line (regular edges, deny lines, and other external
      routes), not just its own, plus widening the label-placement escalation
      to vary the anchor point along the segment as well as the perpendicular
      offset. Mutation-tested (line-avoidance clause disabled, confirmed red,
      reverted, confirmed green). 13 pre-existing, out-of-scope violations on
      edges that predate this feature (`BookUpdate`/`OrderIntent`/`Order`/
      `Fill`) are now surfaced by the general check but not failed on —
      recorded, not fixed, per `docs/external-elements-adoption.md`. Both 003
      SVGs regenerated a third time and rasterised again; confirmed by eye, no
      label is struck by a line in either image.
- [x] `vetting/003-trading-system.ply.yaml`: `venue` external, three flow edges,
      `entry: [venue]` on `Oms::submit`; `ply-check` clean. Both committed SVGs
      regenerated and diffed line-by-line before accepting; `vetting/001-*.svg`,
      `vetting/002-*.svg`, and the disruptor insta snapshot regenerated too (the
      only diff in each: the frame stroke-width bump, confirmed by diffing).
- [x] `The-Ply-Spec.md` amended: §5.1 (structure + example), §5.1a rule 6, §5.3
      (external edges), §7.1 (two table rows + the dash-channel restatement),
      §7.2 (the fourth kind of unspecified — "out of scope by ownership").
- [x] Gate passed — no fallback to the flag-only form was needed.
- [x] `cd tools && cargo test`, `cargo fmt --check`, and
      `cargo clippy --release --all-targets -- -D warnings` all green/clean.
- [ ] **Left for the maintainer, not attempted**: the *holistic* squint test —
      does this read well, beyond "nothing overlaps" — is explicitly the
      maintainer's own call, per the task brief. The specific correctness
      property (no line crosses a box it shouldn't) is now confirmed two ways,
      not just judged: the extended invariant, and a direct-eye check of both
      committed SVGs rasterised with headless Chromium.
- [ ] NOT RUN: a document with more than one external, or with two external
      edges to the same external from different components sharing no lane —
      the layout code has a defensive width-overflow guard but no fixture
      exercises multi-external layout or that specific lane-fan gap.
- [ ] Out of scope by the task brief, not attempted: `crates/` (the
      `entry:`/audit surface lands there at M5); `tools/kernel` untouched (and
      correctly so — externals never enter the verdict tree).
- [ ] NOT FIXED, recorded: 13 pre-existing `edge-label`-vs-line violations on
      edges that predate this feature (`BookUpdate`, `OrderIntent`, `Order`,
      `Fill` between `gateway`/`oms`/`pnl`), now surfaced by the general
      check added above but not failed on. Fixing them means extending this
      session's two-pass restructure and multi-anchor escalation to the
      regular-edge and deny-edge label placement code too — a larger, riskier
      change to well-tested code, not attempted here. Full list printed by the
      test itself; see `docs/external-elements-adoption.md`.

## M4 — fuzz + test + mutate tier — landed 2026-08-24 (2520f8b)

Note on provenance: 2520f8b's own message flags that the full-suite result was "NOT
yet independently confirmed" at commit time (a session-salvage commit, written before
verification finished). It now is: `cargo test --workspace` (single-threaded) is green,
5m31.8s wall clock, zero warnings on a fresh `cargo check --workspace --tests` — recorded
in docs/m4-findings.md along with two deliberate self-mutations, each caught and reverted.

- [x] Task 0: engine-timeout default made shape-aware
      (`verify::default_engine_timeout_secs`) — a `Vec`-typed `bounded(k)` harness now
      gets `30 + 15·k` seconds (reproduces the M3-measured 150s for `bounded(8)` exactly);
      scalar-only stays at 60s. §6 amended with the reasoning.
- [x] `fuzz(n)` check: proptest harness generation (ints biased small, `Vec`/`BTreeSet`
      length 0-8, `requires` as a reject filter with a >50%-rejection `W0503` warning),
      shrink-on-failure rendered through the *same* `contract_rt` renderer the Kani path
      uses. Struct-parameter fuzzing NOT implemented -- deliberate scope cut, recorded in
      docs/m4-findings.md, not silently skipped.
- [x] `test` check: `examples` entries (parsed as arbitrary `==` Rust exprs, §5.4a) +
      generated direct-contract boundary cases.
- [x] `mutate` check: cargo-mutants wired via the spike's verified mechanism, with a real
      correction the spike didn't know about -- `--copy-target true`, not `--gitignore
      false` (see below). `E0504` (mutate with no test/fuzz kill signal) implemented and
      fixture-tested.
- [x] Shape-aware default-check routing (§5.4c's own MUST, unimplemented in M3):
      `default_checks_for` -- `[bounded(2)]` only when Kani-supported, `[fuzz(256)]` when
      the shape is fuzz-supported but Kani-excluded, `[]` otherwise.
- [x] 5 new fixtures + e2e tests (`fuzzbug`, `weakspec`, `strongspec`, `mutatetier`,
      `btreeset`) -- all 6 of the M4 brief's acceptance criteria pass, including the
      milestone's own headline case: a `BTreeSet<u8>` fixture (Kani-excluded per §5.4b)
      earning an honest `fuzzed(256)` verdict via the default route, no `checks:` declared
      by hand.
- [x] **Falsified spec claim, real cost**: §5.4c's mutate mechanism said `--gitignore
      false` was the fix for the harness crate's git-ignored `target/ply/fuzz/`
      placement. Real runs found this wrong on two counts -- `--gitignore`'s own default
      is already off, and there is a *separate*, gitignore-independent skip
      (cargo-mutants prunes any directory literally named `target` at the copy root,
      unconditionally) that hit every real `mutate` run. Fixed with `--copy-target true`
      (which cannot even be passed alongside `--gitignore` -- confirmed, they share a
      clap argument group). Honest cost: this copies the crate's entire `target/` build
      cache into every scratch tree cargo-mutants builds (~13s against a 189MB `target/`
      in this session's fixtures) -- a real, size-dependent tax flagged for M5, not a free
      fix. §5.4c amended.
- [x] Falsified: `engines::fuzz`'s failed-test-name parser looked for libtest's per-test
      `---- name stdout ----` header, which never appears under `--nocapture` (which this
      adapter always passes, for the high-rejection marker) -- caught because it silently
      reported a real seeded bug as a clean pass on the first real run against a fixture,
      not by a unit test in isolation. Fixed; a regression test pins the real output shape.
- [x] Two deliberate self-mutations, each caught and reverted (docs/m4-findings.md):
      suppressing the `·spec-strong` suffix append (caught by `strongspec_fixture`);
      removing `--copy-target true` again (caught by `weakspec_fixture`, reproducing the
      exact tool-error this session hit for real before the fix).
- [ ] **KNOWN GAP, recorded not hidden**: fuzz-found witnesses are not persisted across
      `verify` runs the way Kani's are (M3 finding 6's `target/ply/witness/<fn>.json`
      convention has no fuzz-path equivalent yet) -- a fix that narrows a bug to a
      *different* input than the one already rendered would leave a stale red test behind.
      Needs its own `<fn>_fuzz.json` path (never the same file Kani writes to, since one
      fn could in principle declare both `bounded` and `fuzz`).
- [x] `W0541` (unrenderable fuzz witness) was implemented but NOT exercised against a real
      failing case. **Now run** (2026-08-24 review closure): `tests/fixtures/btreesetbug` --
      a `BTreeSet<u8>` violation reported witness-only, shrunk to `[3]`, no `cargo_test`
      artifact, exit 1. The item's original wording ("`Vec`/`BTreeSet` of non-`u8`") was
      itself wrong: the path fires for every `BTreeSet`. Still not run: a `Vec<i32>`-shaped
      witness.
- [ ] `mutate`'s `--re <fn>` is an unanchored substring match on cargo-mutants' own
      descriptive mutant names (anchoring with `^fn$` matched *zero* mutants in a real
      run -- confirmed and fixed to the unanchored form). This means a fn whose name is a
      substring of another's in the same crate could see cross-fn mutate scope leak; no
      fixture here exercises more than one fn per crate, so this was not reproduced, only
      named.
- [ ] TODO(M1, carried from M3): reconcile the hand-rolled `ply.yaml` model in
      `crates/ply-core/src/config.rs` with `tools/model`'s full model.

## M4 adversarial review — closed 2026-08-24 (see docs/review-m4-2026-08-24.md and docs/m4-review-closure.md)

Every item below was fixed red-first: the test that fails *because of that defect* was
written and watched fail before the fix, and its failure message read to check it named
the defect. `cargo test --workspace -- --test-threads=1` green afterwards: 405s (6m45s)
wall clock, 72 tests (was 53), zero warnings on `cargo check --workspace --tests`.

- [x] **D1 (SEVERE) — the fuzz/test adapter failed open on a harness that would not
      compile**: an ill-typed `examples` entry earned `fuzzed(64)`/`tested` with zero
      diagnostics and exit 0. A run that did not succeed, did not time out and named no
      failing test ran *zero* cases: now `X0901` + verdict `tool_error` for every check in
      that harness, carrying the compiler's own first error and two concrete fixes. Pinned
      by `tests/fixtures/badexample` + `tests/e2e/tests/badexample_fixture.rs`. §5.4c
      amended with the rule.
- [x] D2 — counterexample `inputs` mislabeled for non-alphabetical parameter order: fixed
      by the reviewer in `94e0a2d`, not redone here.
- [x] D3 — the `>50%` rejection `W0503` was arithmetically unreachable (rejected draws
      counted on both sides of the ratio, i.e. `accepted < 0`). Now `rejected/total`;
      `tests/fixtures/highreject` (~62% rejection) pins it, and the wording no longer
      claims fewer cases ran than the verdict says.
- [x] D4 — a fuzz run proptest *abandoned* (global-reject abort) still earned
      `fuzzed(256)`. Now `unclaimed` + a `W0503` naming the real accepted/rejected counts,
      via a distinct `PLY_FUZZ_ABORT` marker. `tests/fixtures/rejectabort`. §5.4c amended.
- [x] D5 — `M0601` was dead code and cargo-mutants ran with no wall-clock cap (`-t` caps
      only each mutant's test phase, so a hung copy or baseline build hung `verify`
      silently, which §5.4c forbids). The invocation is now wrapped in `timeout` like the
      fuzz and Kani adapters, exit 124 classifies as `Timeout`, and the cap is 10x the
      per-mutant budget (min 120s; measured runs use ~4% of it). M0601's wording no longer
      says "per mutant".
- [x] D6 — a `violation` could be emitted with no witness (marker-parse-failure path),
      breaching §5.4c's MUST. The label now comes from what the renderer could establish;
      `tests/fixtures/panicbug` (a panicking body — an ordinary case, not a contrived one)
      pins `tool_error` with rewritten text and fixes.
- [x] D7 — `W0541`'s wording was false for the exact shape that triggers it (it fires for
      every `BTreeSet`, `BTreeSet<u8>` included). Reworded and exact-string tested; the
      same false claim corrected in three doc comments and docs/m4-findings.md.
      `harness::tidy_contract_text` also widened for method calls, so the quoted contract
      reads `xs.len() as u32` instead of `xs . len () as u32`.
- [x] D8 — five in-tree doc comments asserting claims the M4 commit itself falsified
      (`--gitignore false` "must always pass it explicitly", the mutants mechanism block,
      `failed_tests`' `---- name stdout ----` claim, "Ply always anchors this",
      `write_harness_cargo_toml`'s phantom parameter) all corrected.
- [x] O1 — "derived, not guessed" overstated two fitted constants: `verify.rs`'s doc
      comment and §6 now separate the derived shape split from the fitted coefficients,
      and both record that no e2e exercises the default.
- [x] O2/O4 — the shrinking claim and the `btreeset` acceptance were weaker than their
      names. `tests/fixtures/btreesetbug` (the Kani-excluded shape with a real bug) closes
      both *and* docs/m4-findings.md's own NOT RUN item: witness-only `W0541`, shrunk to
      `[3]`, no `cargo_test` artifact, exit 1.
- [x] O3 — "every M4 non-result diagnostic carries a concrete `Fix`" was not true; the
      claim is corrected in docs/m4-findings.md and all five named paths now carry fixes.
- [x] O5 — partly closed (no-violation-without-witness, witness-only, and
      never-claim-evidence-you-lack are now all tested end to end); the remainder is
      recorded below.
- [x] Found while fixing the above: an `examples` entry containing a `"` generated invalid
      Rust (the entry is echoed into the assert message unescaped), and a `mutate` run that
      produced no result at all was reported as `weak-spec` — a finding no engine made. Both
      fixed red-first; inconclusive mutate runs now carry D6's own `inconclusive` status.
- [ ] **NOT RUN, recorded not hidden**: `M0601` against a genuinely hung cargo-mutants, and
      `P0601`/`R0601` against a genuinely slow harness. The caps and classifications are
      unit-tested; no fixture is slow enough to trip them without making the suite
      timing-fragile.
- [ ] **NOT RUN**: the `W0110` engine-missing paths (`prove`, cargo-mutants absent) — no
      fixture masks an engine, so their newly populated `fixes` are unobserved. §9's own
      engine-absence matrix is the right home for this.
- [ ] §5.4c's "MUST carry the distinguishing engine output into the diagnostic" is now met
      by the new `X0901` (carries the compiler's error line) and `W0503` (real counts), but
      every other adapter still drops `raw_output` (`let _ = raw_output;`) — M3-inherited,
      unchanged.
- [ ] The shape-aware engine-timeout default is exercised by a unit test only: every e2e
      passes `--engine-timeout` explicitly, so no test observes the default in real use.
- [ ] `ensure_workspace_member` bails on any crate whose `Cargo.toml` lacks a `[workspace]`
      table — i.e. every ordinary crate inside a larger workspace, and every standalone
      crate without the marker. `fuzz`/`test`/`mutate` therefore work only on
      fixture-shaped crates today. Needs the same decision as the `--copy-target true`
      cost: where the harness crate should live.
- [ ] `mutants.out/` is left in the user's crate root after every mutate run (removed at the
      *start* of the next one) — outside the `target/ply/` housekeeping convention.
- [ ] A missing-engine label beats a passing check in `combine_fn_check_verdicts`
      (`checks: [prove, fuzz(256)]` with fuzz passing yields `engine-missing`), contradicting
      D9 and D6's status-vs-order split. Unreachable until M7 declares `prove` fixtures —
      fold into the M5 verdict-kernel work.
- [ ] `checks: [fuzz(n), test]` on a fn with no `ensures` silently drops the `test` check
      too (the no-`ensures` `V0505` branch returns before the harness runs, examples
      included). Examples need no postcondition, so this is a routing fix with its own
      fixture, not a wording change.
- [ ] Mutants whose tests time out land in cargo-mutants' `timeout.txt` and do not block
      `all_caught()`, so a fn can earn `·spec-strong` with timed-out (uncaught) mutants.
      Defensible as cargo-mutants' own convention; undocumented until now.
- [ ] §6's exit-code table reserves 2 for a tool error, but `main::exit_code_for` returns 1
      for every error-severity diagnostic (M3-inherited; now visible on D1's new path). The
      new e2e tests assert *non-zero* rather than pinning 1, so no test blesses either
      behaviour.


- [x] `ply-render --depth N` / `--focus` / `--collapse <component>` (8d8910f) —
      collapsed box shows contents line, rolled-up capability badges, pin and finding
      counts; edges reattach; default output byte-identical without flags.
- [x] Collapsed boxes draw as a stack (dc1ad4b, repaired in 26cdeb6); 003's canonical
      artifact is now the collapsed system view, full depth moved to -full.svg.
- [ ] **Color SVG config** — make the renderer's palette configurable (the style
      constants: ceiling scale, finding red, ink, amber) instead of hardcoded; must
      keep the §7.1 channel discipline (a config can retune a hue, not repurpose a
      channel) and the style-rule invariant test.
- [ ] `ply-render --legend` — opt-in legend strip below the frame, generated from the
      live style constants (§7.1, specced 2026-08-23).
- [x] `W0409` redundant parent-to-descendant edge lint (7d4c6fc) — both directions,
      both edge kinds; brought a W-warns/E-fails severity model with it.
- [x] Edge and deny routing + collision-freedom invariant (b3da43c, 2b07bd0) — 003
      render findings 1, 3, 4 closed. KNOWN GAP left open deliberately: deny lines in
      *different* margin columns can still cross (repro:
      tools/render/tests/fixtures/deny_stress.ply.yaml). Needs a routing policy
      decision (§7.1), not a guess.
- [x] Gate debt closed for real — `strict` notch, `mode: synth` violet chip, `examples`
      e×N token all drawn and test-pinned (worktree merge).

## Engine strategy — settled 2026-08-24 (fable review)

- [x] **No pivot to VeriFast, and no additional engine now.** It is a category error:
      Ply is multi-engine by design (D9, §5.4c), so there is no "primary engine" to
      swap — only check kinds and adapters. Three independent reasons VeriFast is the
      wrong first tenant: (1) it emits a symbolic-execution trace, never a concrete
      counterexample, and a failure means *unproved*, not *false* — so a VeriFast-primary
      Ply could never emit a `violation` from its main engine, deleting §1's core
      mechanism rather than weakening it; (2) LLM proof-closure measures 31.4%
      (arXiv:2606.26490, C) against Verus's 44% / AutoVerus ~90%, and our users are
      agents; (3) measured cost on the real verify-rust-std proofs: linked_list
      2,254 → 4,390 lines (+95%; 39 lemmas, 166 `open`, 229 `close`), raw_vec
      854 → 3,246 (+280%).
- [x] **Today's answer for external proofs**: they enter as `trusted` claims — no new
      grammar, and safe now that attestations go stale with the code.
- [ ] **When we reach M7**, the `prove` slot takes a deductive engine, with **Verus as
      first tenant** (Rust-shaped, better agent proof-closure), not VeriFast. Adding any
      engine is milestone-sized, and we are one milestone of seven in with no working
      `cargo ply` — so the next step stays the thin end-to-end slice, not a second engine.
- [x] **Verus feasibility spike done** (`tests/spike/verus/`, FINDINGS.md) — the
      deductive-vs-bounded question the scale spike left open. Result: a `Seq`/`Set`-based
      Verus shadow of the kernel proves all four standing obligations, unbounded, by
      structural induction, in ~2s (mutation-tested, not vacuous) — exactly where Kani's
      bounded model checking cannot terminate at all on the same recursive shape. A
      differential test (4,000 generated trees, plain `cargo test`) binds that shadow's
      executable transcription to the real `ply-kernel` crate. **Open before M7 commits**:
      this proved a faithful shadow, not `tools/kernel/src/lib.rs`'s literal
      `Vec`/`String`-based source — whether Verus's own executable-collection support
      pays the same symbolic cost that stalled Kani on `Vec<String>` is untested and is
      the next spike, not a foregone conclusion of this one.
- [ ] **Revisit triggers** (a decision with a trigger, not a dismissal): an
      AutoVerus-equivalent for VeriFast reaching Verus-level proof-closure; VeriFast
      emitting machine-readable failure output an adapter can parse; a vetting scenario
      showing `fuzzed·spec-strong` is genuinely insufficient on recursive structures; or
      the arena-flattening experiment failing.
- [x] **Trusted claims had no staleness** — the evidence-lying hazard the fallback would
      have shipped: an entry outlived the code it attested and rendered identically fresh
      forever. §5.4d now fingerprints the attested item, marks it stale on change, and
      requires human re-attestation (`accept` does not clear it).
- [ ] **Separation-logic constructs: split the question.** Lemma functions and ghost
      open/close are proof steps, not specification — below the watermark, never
      spec-resident. Heap *predicates* are admissible in principle (the §7.1 gate admits
      them on the same mark-plus-tooltip precedent as contract clauses; separation logic
      is highly diagrammable), but largely unnecessary: §5.4a already admits calls to
      `pure` helpers, so a recursive `pure fn len(&self)` is legal in a contract TODAY.
      What is missing is an engine that can earn `proved` on it, not vocabulary.
- [ ] Smallest useful version for M7: a per-fn `proof:` field naming a proof artifact,
      drawn as a badge, fingerprinted under D14 so it goes stale with the body. No
      predicate sub-language until a vetting scenario forces one.

## From the external review (codex, 2026-08-23) — see docs/review-2026-08-23.md

- [x] **M0 spike done, ADR-0003 accepted** (0974f57). 8/9 mechanisms work; cross-crate
      stubbing works via caller-local re-proof. Fixtures + run.sh under tests/spike/.
- [x] **M0 fully discharged** — the cargo-mutants item is now exercised
      (tests/spike/mutants/). It found §5.4c asserting a mechanism that does not
      exist: there is no "custom test command" flag, and the claim "confirmed in the
      M0 spike" was false. Real mechanism verified and specced; `--gitignore false`
      must be pinned or the build fails; `W0502` now caveats equivalent mutants
      (strong spec killed 13/14, the survivor was provably equivalent, not a gap).
- [x] **Scale spike done; §5.4b rewritten around evidence** (tests/spike/scale/).
      Headline: recursive/self-referential types are NOT supported in v1 — a 3-node
      tree makes 64,147 verification conditions and doesn't finish in 180s, even with
      the unwind fix that makes flat `Vec` cheap. That is the shape of our own verdict
      tree. Also: `Vec` works ONLY if codegen emits `#[kani::unwind(N+1)]`; fixed
      arrays are cheap and become the preferred shape; BTreeSet/BTreeMap are out past
      one element; HashMap needs a codegen hasher swap or it won't compile.
- [x] **Self-hosting resolved**: the enumeration IS bounded-kind evidence (exhaustive
      within a stated bound, independent oracle, covering more than the Kani harness
      would have). CLAUDE.md reframed rather than apologised. Reshaping the kernel was
      rejected on evidence — the stall just moves to the next unbounded field.
- [x] **Enumeration REDUCTION ARGUMENT written 2026-08-25** (`docs/kernel-honesty-cleanups.md`
      part 2). One leg held (per-bit uniformity of StatusSet). The other — content-independence
      of the assumption merge — **did not**, and was measured not holding: six one-line
      breakages of the real kernel, four survived the corpus as it then stood. The corpus was
      repaired in the same change (period-2 payload cycles in `tools/kernel/tests/enumeration.rs`)
      and those four now die. See the KNOWN GAP below for the fifth.
- [x] **Kani harnesses DELETED 2026-08-25**, not gated (`docs/kernel-honesty-cleanups.md`
      part 1). They contradicted our own §5.4b rule — a recursive shape is one Ply refuses
      by name rather than routing to an engine that times out — and the role they filled is
      now filled better by `tests/spike/verus/`, which proves all four obligations unbounded
      by induction in ~2s. The investigation survives as a historical note in
      `crates/ply-core/src/kernel.rs`'s doc comment.

- [ ] **KNOWN GAP — one kernel mutant still survives the enumeration.** A node carrying
      BOTH a status flag AND a conditional at once is not in the enumerated corpus, so a
      breakage that treats the two as mutually exclusive (miscounting what is still owed)
      goes unnoticed. Left open deliberately: closing it costs 3,117,996 trees, roughly
      tripling the gate's runtime. Recorded 2026-08-28 — it was measured on 2026-08-25 and
      never written down, which is exactly the failure this list exists to prevent.

- [ ] **KNOWN GAP — is an empty assumption list representable, and should it be?** Raised
      by the same 2026-08-25 reduction work and never carried into this list. A spec
      question, not a bug: decide and write it into The-Ply-Spec.md either way.
- [ ] **Pull a thin M3 vertical slice ahead of M1/M2** (fable's sequencing call, and the
      external review's undone recommendation #2): one hand-written ply.yaml, one
      contracted fn, generated harness WITH the unwind emission, one real cex, the D7
      rendered red test, one JSON diagnostic. Seven sessions of engine-free scaffolding
      before re-contacting the layer that just falsified five spec claims is the
      reviewed failure mode, rescheduled.
- [ ] **Reweigh M4 above the bulk of M3**: M3 is 8-10 sessions for an engine covering a
      sliver of signatures; M4 is 4 sessions covering every signature, and its scariest
      mechanism is already proven end to end.
- [ ] **Generalise D13 beyond M0**: each milestone opens by spiking its riskiest
      external-tool claim, and no spec sentence may say "confirmed"/"verified" without
      naming the artifact that shows it. §5.4c carried a fabricated confirmation until
      an adversarial re-check caught it.
- [ ] Measure whether the unwind annotation rescues ITERATOR-CHAIN bodies (marked NOT RUN
      in the scale sweep). Until measured, §5.4b's gate still admits functions that hit
      the exact failure it was rewritten to prevent.
- [ ] **D7 correction: the generated playback test does not reproduce contract
      violations.** Adversarial re-verification found `cargo kani playback` never
      evaluates contract closures — only the real body runs — so an `ensures` violation
      replays and the generated test PASSES. Playback preserves the witness *input*
      exactly, but the rendered plain `#[test]` is the only artifact that can be a red
      reproduction, and that is most Ply counterexamples. §8/D7 wording needs a pass.
      It is a documented Kani limitation, not a defect, and the fix is ours: Ply's
      generated test must assert the postcondition explicitly so the failure is
      panic-shaped and therefore replayable. Kani reportedly warns when it declines to
      generate a test; no warning appeared in our run, so don't depend on it.
      → PLAN WRITTEN: docs/plans/d7-replayable-tests.md (worked example compiled and
      run red-for-the-right-reason). Decisions to review before implementing: the
      generated test lives in an in-crate `cfg(test)` module (private items are
      unreachable from `tests/`); contract expressions render through one shared
      overflow-safe assertion renderer, widening to i128 so the assertion states the
      broken promise instead of re-triggering the overflow; refusals reuse W0541 with
      a reason field; and `counterexample.kani_playback` is renamed `kani_witness` so
      the schema stops implying it reproduces anything. DECIDED: witnesses are
      generated for EVERY falsified claim (default `all`); §1 states the concrete
      failing input as a MUST, so a default cap would have violated the spec for
      findings past it. `--witnesses=N|none` is an explicit opt-out only, and when
      used it must announce the skip (W0541 reason `budget_exhausted`). The ~20s
      near-fixed cost is accepted, shown in progress output, and optimised by making
      it cheaper — never by skipping it.
- [x] **Callee-before-caller ordering got kernel-grade treatment** — new `ply-schedule`
      crate: SCC-condensation planning (cycles land in one batch, never deadlock) and a
      `may_stub` decision that returns Allowed ONLY when the callee's proof actually
      passed this run. Invariants enumerated exhaustively: all 65,536 four-node digraphs
      for planning, 4,096 graph+config combinations for the stub decision, both against
      oracles written from D5's text rather than from the production code. Mutation-
      checked by letting `NotRun` license a stub — the exact unsound shortcut Kani
      itself takes — and confirming it goes red.
      **RETRACTED as of 2026-08-30 (`4dd4d30`): the SCC-condensation planning
      described above never shipped** — `crates/ply-cli/src/verify.rs` carried its
      own, stricter, untested ordering the whole time, and the two disagreed about
      where a cycle's dependents go (see "The two schedulers order cycles
      differently" and its resolution, above). The planning half (`plan`/`Batch`) is
      now deleted; `may_stub` and its own enumeration, described accurately above,
      are untouched. Read this bullet as "the stub-decision half only" from here on.
- [ ] **D5 ambiguity surfaced by the scheduler**: cross-crate proof results are really
      scoped per (calling-crate, callee), since each consumer re-proves locally, but
      `ProofResults` models one global status per fn. Exact for same-crate; a
      simplification cross-crate. Decide before M3 whether the distinction matters.
- [x] **Real defect fixed: component-level `checks` inheritance** (merged from
      worktree): a fn's own list wins entirely; otherwise it inherits the nearest
      ancestor component's default. Resolution lives once in `ply-model` so the
      validator and renderer cannot drift. Tooltips now name the source —
      "inherited from component `pricing`: bounded(2) — …". E0504 evaluates the
      effective list. All five committed SVGs byte-identical (no vetting document
      uses component defaults — grep-confirmed, not assumed).
- [x] Engine-limit diagnostics specced (52222ab) — §8 now requires timeout/unsupported
      to name the cause and populate `fixes`, with the boundary written in: Ply
      proposes, never rewrites. IMPLEMENTATION still owed when the engines are wired.
- [x] `schema/ply.schema.json` was called normative in §5/D3 while not existing. BUILT
      (`c8528ce`) and load-bearing: the key vocabulary and required-field list are read from
      it at runtime. Also recorded at the top of this file under Phase 1a.
- [ ] Separate declared ceilings from earned verdicts in the type system (both are
      `Evidence` today; only convention keeps them apart).
- [ ] `trusted` claims are unrestricted prose — no identity, date, commit, scope, or
      expiry. The shield can read as approval.
- [ ] `conditional` assumptions are free-form strings, untied to the call graph.
- [ ] Kani harnesses do not terminate (e46e4a9): CBMC unwinds BTreeMap's generic clone
      on every recursive `aggregate_raw` call. Kani's docs confirm heap collections
      blow up the encoding AND that generic std methods cannot be stubbed — so the
      documented workaround does not apply directly. ATTEMPT 1 (done): statuses are
      now a `StatusSet` bitmask instead of a BTreeSet — behaviour identical (991k-tree
      enumeration green, untouched assertions) and 40% faster, and Kani moved from an
      indefinite hang to a deterministic timeout at 5 min. Still no verdict: CBMC now
      stalls one field over, sorting/dedup'ing `Option<Vec<String>>` assumptions.
      Deliberately NOT collapsing assumptions to a count — D5 and the newbie-bar rule
      need callers to read them verbatim, so that would narrow what a passing proof
      means. Untried: `-Z stubbing` of the String sort/compare for the harness only.
      **Bigger implication for the project:** Ply routes `bounded` checks to Kani; if
      Kani struggles this much with std collections in a 300-line pure module, the
      supported-signature story (§5.4b) is optimistic. This is exactly the kind of
      engine limit M0 exists to find — more evidence for doing M0 next.
- [x] Renderer CLI now covered — 11 tests over flags, exit codes, and error wording;
      two messages rewritten to the newbie bar (`--depth 0` and a non-numeric depth
      used to fail silently or with clap's raw error).

## M3 thin vertical slice — landed 2026-08-24 (7e6fc79)

- [x] First production code of `cargo ply` itself: `crates/ply-attrs` (the
      `#[ply::requires]`/`#[ply::ensures]` proc macros, D2), `crates/ply-core`
      (`config`, `harness`, `engines::kani`, `contract_rt`, `diag` — exactly the five
      modules authorized, nothing more), `crates/ply-cli` (`cargo-ply verify` +
      `--json`). Root `Cargo.toml` is now the product workspace
      (`members = ["crates/*", "tests/e2e"]`, `exclude = ["tools", "tests/spike",
      "tests/fixtures"]`), separate from `tools/Cargo.toml`.
- [x] Four fixtures under `tests/fixtures/` (`clamp`, `passing`, `vecbound`,
      `timeout`) plus 5 black-box e2e tests under `tests/e2e/` that build the real
      binary and run it — the §9 cex validity oracle, for real, on the `clamp`
      fixture: FAIL (stating the contract + "postcondition", never the overflow trap)
      before the fix, PASS (the same `ply_cex_clamp_01` test) after. `cargo test
      --workspace` green (17 unit tests + 5 e2e tests).
- [x] Measured (not copied from §5.4b's own number, which is for a different harness
      shape) the Vec unwind bound for this slice's own harness: `k+1` for a manual
      indexed-loop consumer of `any_vec::<u8,k>` — 9 at k=8, confirmed 8 fails
      ("unwinding assertion loop 0") and 9 succeeds, with an adversarial e2e test
      proving the emission is load-bearing (the identical harness minus the
      annotation does not verify within a bounded cap).
- [x] Timeout correctly distinguished from violation end to end (`K0601` vs `K0502`,
      `timeout` status carries no counterexample) — see docs/m3-slice-findings.md
      finding 3 for a real, load-bearing caveat: this environment shows CBMC/CaDiCaL
      SAT-solve wall-clock variance (~1s to ~107s on an *identical* harness), and one
      run's raw CBMC log showed a SATISFIABLE result reached before Kani's own
      "CBMC timed out" text was printed — meaning the timeout/violation textual
      distinction can, rarely, itself be racing the engine's own reporting. Routed
      around with generous timeouts here; not fixed. Flagged for the next session.
- [x] Spec amended: D2 (the `unexpected_cfgs` lint requirement, confirmed again);
      D7 + §0 + §1 + §8 + §9 + the M3 milestone bullet, applying
      `docs/plans/d7-replayable-tests.md`'s own pre-drafted deltas now that the D7
      renderer is actually built (the `kani_playback`→`kani_witness` rename is live in
      code, pinned by a unit test).
- [x] Two deliberate self-mutations (§ CLAUDE.md), each caught and reverted: disabling
      the "CBMC timed out" check in `parse_output` (caught by a unit test); making the
      rendered cex test's `Ok(false)` arm a no-op, i.e. "renderer skips the assertion"
      (caught by the real `clamp_oracle` e2e test going red, not a unit test).
- [ ] **KNOWN GAP, recorded not hidden**: `docs/m3-slice-findings.md` finding 6 — the
      witness-persistence mechanism that makes the D7 oracle's "same test transitions
      FAIL→PASS" promise hold across two `verify` runs (`target/ply/witness/<fn>.json`)
      is a real design decision this slice made ad hoc; it was not settled in the D7
      plan and duplicates a sliver of what full D14 staleness tracking will eventually
      own. Needs an explicit call, not silent acceptance, once `ply.lock` lands (M1).
- [ ] Witness-replay half of the §9 oracle (`cargo kani playback` reproducing a stored
      `kani_witness`) is implemented (`engines::kani::run_playback`) but NOT wired into
      `verify` or any e2e test — recorded as NOT RUN in §9, not silently skipped.
- [ ] Not attempted this session, all explicitly out of scope per the M3 brief:
      `impl`-method contracts (`&self`, `old()`), generic fns/`check_with`, cross-crate
      callees, `stub_verified`/`conditional` (D5), the `ply.yaml`
      `requires`/`ensures`-merge path (only inline attributes are read),
      `BTreeSet`/`HashMap` handling, the engine-timeout reliability fix above.
- [ ] TODO(M1), recorded in `crates/ply-core/src/config.rs`'s own doc comment:
      reconcile the hand-rolled ~4-struct `ply.yaml` model here with `tools/model`'s
      full model (promote one, delete the other).
