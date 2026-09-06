# Plan: self-verification concentrated on evidence integrity

Status: **draft for review**, 2026-09-06.

## Why this plan exists

Eight defects were found on 2026-09-05 across two external review rounds. Every one of them
sat in the machinery **between a promise and its check** — the parser that reads a contract,
the rewrite that turns it into an assertion, the counter that decides a case ran, the
fingerprint that decides a result still applies, the composition that draws it. None was a
false promise about a function.

Ply's own claims, measured rather than assumed: **56 claims earning 70 `fuzzed(256)` and 4
`tested`. Zero `bounded`. Zero `proved`.** Every declaration is `fuzz(256)` or
`fuzz(256), test`. The only genuinely proved thing in the repository — the verdict kernel,
exhaustive over 991,389 trees plus the Verus induction proof — is not a `ply.yaml` claim at
all. So the part that is proved is not counted, and the part counted is not proved.

Two consequences drive everything below:

1. **More claims would not have caught any of the eight.** They were in code Ply does not
   claim, and adding claims uses the machinery rather than checking it.
2. **A higher rung would not have caught them either.** `contract_rt` is shared by the fuzz
   and Kani paths — its own comment says so — so `bounded` on the float function would have
   exhaustively proved the *rewritten integer* comparison: a stronger, more confident wrong
   answer.

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

| # | Property | Layer | Status |
|---|---|---|---|
| P1 | Every declared clause is preserved, or explicitly rejected | L1 + L3 | **done** (repeated attributes conjoined; was keeping only the last) |
| P2 | The generated check preserves the original expression's meaning | L2 + L3 | **partial** — floats closed by exhaustion over all 32 `RustType` variants; the general property is unproven |
| P3 | Evidence requires at least one real check, not a passing wrapper | L1 + L3 | **done** (a precondition-rejected case no longer counts) |
| P4 | Changing a relevant input prevents reuse | L2 + L3 | **partial** — module-scoped resolution and type-declaration hashing closed; dependency identity still discards git revisions |
| P5 | Every drawn component/function has matching metadata; no invented evidence | L2 | **done** (envelope 9 elements → 74) |
| P6 | The rendered contract text is byte-stable for unchanged source | L2 | **open — new** |
| P7 | A claim of exhaustiveness is only made where the domain is bounded | review rule | **open — new** |

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

1. **P6** — cheapest, and it closes a hole that has already cost a red CI run.
2. **P4 remainder** — dependency identity: preserve Cargo's full package identities and
   dependency edges, including source revisions.
3. **P2 general** — a differential harness: evaluate the author's contract and Ply's
   generated assertion over the same inputs, compare outcomes including panics. This is the
   only one of these that would have caught the float bug *as a class* rather than as a case.
4. **L3 sweep** — the mutation job over the checking pipeline, which makes 1–3 measured
   rather than assumed.

## Terminology

`fuzzed(256)` is honest. "All 50 claims earn evidence" is not, and it is the current headline
on the published page and in several merged pull-request descriptions. Sampled over 256
inputs is useful testing, not proof. This wants correcting wherever it appears before any of
the above lands, because it is the same overclaim-by-omission the tool exists to refuse.
