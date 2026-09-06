# Plan: self-verification concentrated on evidence integrity

Status: **live**, 2026-09-06. Reviewed twice; the corrections from the second round are
folded in, and the status table below was rebuilt after that review pointed out it was
marking work finished that was not.

## Why this plan exists

Eight defects were found on 2026-09-05 across two external review rounds. Every one of them
sat in the machinery **between a promise and its check** — the parser that reads a contract,
the rewrite that turns it into an assertion, the counter that decides a case ran, the
fingerprint that decides a result still applies, the composition that draws it. None was a
false promise about a function.

Ply's own claims, counted from the two documents rather than assumed: **56 function claims
— 50 in the library, 6 in the command-line tool. 29 declare `fuzz(256)`, 26 declare
`fuzz(256), test`, one declares `test` alone. Zero declare `bounded`. Zero declare
`proved`.**

(An earlier draft said "56 claims earning 70 `fuzzed(256)` and 4 `tested`". Those were node
counts from the report, and the report has a node per component box as well as per function,
so the 70 counted containers alongside claims. Corrected here rather than left to be found:
a document about not overclaiming should not inflate its own denominator.)

The only genuinely proved thing in the repository — the verdict kernel, exhaustive over
991,389 trees plus the Verus induction proof — is not a `ply.yaml` claim at all. So the part
that is proved is not counted, and the part counted is not proved.

Two consequences drive everything below:

1. **More claims would not have caught any of the eight.** They were in code Ply does not
   claim, and adding claims uses the machinery rather than checking it.
2. **A higher rung would not have caught them either.** `contract_rt` is shared by the fuzz
   and Kani paths — its own comment says so — so `bounded` on the float function would have
   exhaustively proved the *rewritten integer* comparison: a stronger, more confident wrong
   answer.

A ninth and a tenth arrived on 2026-09-06, and both were in fixes made the day before —
which is the strongest argument this document makes for its own existence. The zero-case
guard closed "green with no evidence" by opening "red with no evidence", reporting a
correct function as a reproduced violation and offering advice that could not be followed.
And the exhaustive classification proof covered one of the two classifiers that decide the
same question, leaving `u128` on the other one's list with the float defect's exact shape.
Neither was found by any test here; both were found by a reviewer reading the diff.

## The circularity constraint

Ply checking itself cannot be the only check on the machinery that produces its verdicts. If
the transformation is broken, a claim verified *through* that transformation returns a
reassuring green. The same holds for a parser that silently drops one of its own clauses.

**Rule for this plan: every property below names its layer, and at least one layer verifying
it must be independent of the pipeline under test.** The exhaustive `RustType` classification
landed on 2026-09-05 is the model — it is a plain Rust test with an independently written
oracle, deliberately *not* a Ply claim, because a Ply claim would have run through the very
function it was checking.

## Layers

- **L1 — Ply contracts on small pure decision functions.** The rules that admit evidence,
  combine clauses, or permit reuse. Cheap, and honest about what it is: sampling.
- **L2 — Independent differential tests.** Evaluate the original contract and its generated
  equivalent on the same inputs and compare outcomes, panics included. Compare fresh
  verification against cached after a controlled change. Independent of the pipeline.
- **L3 — Mutation tests on those checks.** Drop a clause, truncate a float, count a skipped
  case, omit a dependency; require the check to catch each. This is `kernel-mutants` pointed
  at the checking pipeline rather than at the kernel.

## Properties

**Four columns, not one, because the review was right that they were being conflated.**
"Defect fixed" is not "regression added", and neither is "general property tested", and none
of the three is "mutation resistance measured". A single **done** hid which of the four had
actually happened. `cargo mutants` in CI still targets only the verdict kernel, so the last
column is empty everywhere below and will stay empty until the sweep in step 4 runs.

| # | Property | Defect fixed | Regression pinned | General property tested | Mutation-measured |
|---|---|---|---|---|---|
| P1 | Every declared clause is preserved, or explicitly rejected | yes — was keeping only the last of repeated attributes | yes | no — "every clause survives, for every contract shape" is untested | no |
| P2 | The generated check preserves the original expression's meaning | yes, twice — floats, then `u128` in the *second* classifier the first proof never looked at | yes | partial — exhaustive over both classifiers' own domains; the general property (author's contract ≡ generated assertion) is untested | no |
| P3 | Evidence requires at least one real check, not a passing wrapper | yes — and the first fix was itself a defect, reporting a correct function as a violation; fixed 2026-09-06 | yes — a fixture with both cases, checked to bite both ways | no | no |
| P4 | Changing a relevant input prevents reuse | yes — module-scoped resolution, type declarations hashed, git revisions kept, two versions of one crate no longer collapsed | yes | no — "every relevant input is in the hash" is not tested as a property | no |
| P5 | Every drawn component/function has matching metadata; no invented evidence | partial — the declaration-only path (9 elements → 74); the verification publisher still collects metadata from the unexpanded tree | partial | no | no |
| P6 | The rendered contract text is byte-stable for unchanged source | no | no | no | no |
| P7 | A claim of exhaustiveness is only made where the domain is bounded | n/a — a review rule | n/a | n/a | n/a |
| P8 | The aggregation a user's verdict comes from is the one that was proved | yes — the shared six rungs now carry `kernel::Evidence` itself instead of a second ladder | yes — a bounded differential folds every small tree both ways | partial — bounded to trees of ≤3 leaves and depth 2, stated inline | no |

### P6, and why it is not covered by P4

Cached-vs-fresh agreement would **not** have caught the regression CI found on 2026-09-05.
Both sides re-rendered the contract identically; the text simply changed from what earlier
runs had recorded. `|result| *result >= 0` became `|result|(*result >= 0)` — cosmetic, and
it is a hashed fingerprint input *and* the case-generation seed, so it silently invalidated
every recorded result and made every function draw different inputs. What caught it was
end-to-end tests pinning observable output, by accident.

The property is idempotence, not agreement: **rendering an unchanged contract twice, across
builds, produces the same bytes.** Proposed as a test that renders every fixture's contracts
and compares against a committed list, so the next such change arrives as a reviewable diff
rather than as a silent reseed.

### P7, and why it is a review rule rather than a test

The float bug was itself a proof of the wrong thing: the old rule established that
`f64 as i128` **compiles**, when the question was whether it **preserved the comparison**.
Reaching for exhaustiveness over a domain that does not support it produces exactly that.

So: enumerate only where a bounded domain genuinely exists (a classification over an enum's
variants), and where it does not — a call walk over arbitrary source, attribute parsing,
generated-code shape, a fingerprint over whole files — use L2 and L3 instead. Any claim of
"every case" must state its honesty condition inline, the way the kernel's enumeration and
the new `RustType` proof both do.

## Order of work

Per review: **a regression that exposes the wrong observable result, then the fix, then a
general invariant that covers the class.** A regression catches yesterday's example; the
invariant is what catches tomorrow's variation. All eight fixes so far followed this except
the two where the invariant is still owed (P2 general, P4 dependency identity).

Proposed sequence:

Revised 2026-09-06, per review: semantic differential testing and correct execution
accounting come before stable contract-text formatting, because they protect what a green
result *means* rather than what it is spelled like. Execution accounting is now done (P3's
second fix). So:

1. ~~**P6** — contract-text stability~~ — **demoted**, not dropped. It closes a hole that
   cost a red CI run, and it is still cheaper than everything below it, but it guards
   spelling rather than meaning.
2. ~~**P4 remainder** — dependency identity~~ — **done**. Both halves: the `source =` line is
   kept whole (appended only for non-crates.io sources, since a published crates.io version is
   immutable and appending a constant would have reseeded the world), and packages are keyed by
   name *and* version so two copies of one crate no longer overwrite each other.
3. **P2 general** — a differential harness: evaluate the author's contract and Ply's
   generated assertion over the same inputs, compare outcomes including panics. This is the
   only one of these that would have caught the float bug *as a class* rather than as a case.
4. **L3 sweep** — the mutation job over the checking pipeline, which makes 1–3 measured
   rather than assumed.

## Terminology

`fuzzed(256)` is honest. "All 50 claims earn evidence" is not. Sampled over 256 inputs is
useful testing, not proof, and a promise that fails on one value in a billion sits among
them looking identical.

**Corrected 2026-09-06.** The published page now says what "checked" means on it — 256
generated inputs, a handful of hand-written examples, nothing proved — and says plainly that
nothing there has earned a proof yet. `docs/handoff-2026-09-04.md` carries the same
correction against its own headline. Whatever remains in a merged pull-request description
stands as the record of what was written that day; these two are the places a reader
actually arrives at.
