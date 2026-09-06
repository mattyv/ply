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
actually happened. `cargo mutants` on the pull-request path still targets only the verdict kernel. The sweep in
step 4 now runs nightly over the four files this plan is about, so the last column stops
being structurally empty the first night it runs — but it is empty *today*, because it has
not run yet, and writing anything else in it would be the exact thing this table was rebuilt
to stop.

| # | Property | Defect fixed | Regression pinned | General property tested | Mutation-measured |
|---|---|---|---|---|---|
| P1 | Every declared clause is preserved, or explicitly rejected | yes — was keeping only the last of repeated attributes | yes | no — "every clause survives, for every contract shape" is untested | nightly, unmeasured until it first runs |
| P2 | The generated check preserves the original expression's meaning | yes, twice — floats, then `u128` in the *second* classifier the first proof never looked at | yes | yes, in the reachable form — both classifiers exhaustive over their own domains, plus a walking invariant that the rewrite never widens a leaf a classifier refuses. Not evaluated-equivalence; see step 3 | no |
| P3 | Evidence requires at least one real check, not a passing wrapper | yes — and the first fix was itself a defect, reporting a correct function as a violation; fixed 2026-09-06 | yes — a fixture with both cases, checked to bite both ways | no | no |
| P4 | Changing a relevant input prevents reuse | yes — module-scoped resolution, type declarations hashed, git revisions kept, two versions of one crate no longer collapsed | yes | no — "every relevant input is in the hash" is not tested as a property | no |
| P5 | Every drawn component/function has matching metadata; no invented evidence | yes — the declaration path (9 elements → 74) and, on 2026-09-06, the verification path, which that fix had not touched | yes — a linked pair where the run checks nothing and the drawing shows three items | yes — the envelope is completed from the same links-aware walk the renderer draws from, not a second copy | no |
| P6 | The rendered contract text is byte-stable for unchanged source | n/a — a hole, not a defect | yes — 223 contracts pinned, checked to catch the exact 2026-09-05 reseed | yes — every fixture contract, not a chosen few | no |
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
builds, produces the same bytes.** Landed 2026-09-06 as `contract_text_is_stable`: it renders
every fixture's contracts through the real pipeline and compares 223 lines against a committed
list, so the next such change arrives as a reviewable diff rather than as a silent reseed.
Checked against the regression it exists for — putting that bracket back makes it fail in
seconds and print the pair, where the original took hours and three apparently unrelated
end-to-end failures.

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
invariant is what catches tomorrow's variation.

An earlier draft said all eight fixes followed this. Review pointed out that they did not,
and it was right. Two owed their invariant when that was written (P2 general, P4 dependency
identity) — both landed on 2026-09-06. Two more were *not* fixed at all when the draft called
them done: the filesystem-effect scanner still failed open, and P5's verification path still
collects metadata from the unexpanded tree. Both were fixed on 2026-09-06. The lesson is the one the four-column table exists for: a
plan that grades itself is a claim like any other, and this one was overclaiming in exactly
the way the tool it plans for refuses.

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
3. ~~**P2 general** — a differential harness~~ — **done, in the form the code actually
   admits, which is not the form first proposed.** Evaluating both expressions over the same
   inputs means compiling both, per contract, per input: an end-to-end harness of its own.
   What the rewrite *does* is narrower than that, and so is the property it needs. It wraps
   numeric leaves in `as i128` and nothing else, so "does the rewrite preserve meaning"
   factors into two questions that are each closable:

   - *Is the cast lossless for this leaf's type?* Both classifiers, exhaustively, against
     independently written oracles. Landed.
   - *Does the rewrite only ever widen a leaf a classifier admitted?* A walking invariant
     over the rewrite's real output, across a corpus reaching every arm — the shape
     `every_painted_element_resolves_a_style_rule` has, so an arm added later cannot quietly
     skip it. Landed.

   The honesty condition, and it is a real one: **the walking invariant is blind to a wrong
   classifier by construction**, because it asks the classifier whether each widened leaf is
   admissible. Measured rather than assumed — putting the floats back on the admitted list
   leaves it green. What catches that is a separate test asserting the output directly. The
   two together close the class; neither alone does, and the module says so where a reader
   will find it.
4. **L3 sweep** — **standing, as a nightly rather than a gate.** 274 planted bugs across
   the four files the machinery actually lives in — the contract rewrite, the reachability
   walk, the effect scan, the record — sharded eight ways so a run finishes inside the hour.
   Off the pull-request path entirely: it is about four hours of machine time, and CI wait
   was already the thing people complained about.

   It **reports** rather than fails, and that is a deliberate difference from
   `kernel-mutants`, which sets the bar at zero survivors with no excused list. That bar is
   right there and would be wrong here: the kernel is proved and its first run found three
   real gaps that were fixed, whereas this has never run and nobody knows what it will say.
   A job red from its first day is a job people stop reading, which is how four hours of
   machine time ends up worth nothing. Turning it into a gate is the follow-up, once the
   number is known and the real gaps among it are closed — and a survivor is never an
   ignore-list entry: it is a gap in the tests, dead code, or a change with no observable
   effect, and all three are worth acting on.

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
