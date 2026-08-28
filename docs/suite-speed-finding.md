# Why caching cannot make the test suite faster, measured

**The premise was wrong, and the measurement says so.** The suite proves the same fixture
up to eight times per run, and the obvious fix — let the first proof serve the rest —
cannot work, because **tests that share a fixture run concurrently, not in sequence.**
There is no first-then-rest ordering to exploit; they all race into a cache miss together.

Timed, on a clean tree, detached, to completion:

| run | wall clock | result |
|---|---|---|
| before | **2533s** (42m) | 445 passed, 0 failed |
| with the cache in place | **2569s** (43m) | 446 passed, 0 failed |

No speed-up. Slightly slower, within noise. The cache only ever hits on a *second*
invocation against an already-populated store, which is not the thing anyone waits on.

The mechanism was built and worked as designed — it seeds a *candidate* result and lets
Ply's own fingerprint decide whether to use it, so a stale seed can only cost a miss and
never earn an unchecked claim. Its safety test went genuinely red first: a deliberately
naive version (name-only key, whole-envelope cache, never re-invoking the tool on a hit)
reported `fuzzed(64)` where the truth was `violation`. The design was sound; the premise
underneath it was not. The code is removed rather than left dead, and this page is what
survives.

## What would actually work

The cost is that N tests each prove the same fixture from scratch, in parallel. So either:

1. **Fewer duplicate proofs** — several tests that each copy `clamp` and assert on
   different parts of one run could be one test making several assertions about a single
   run. That is a test-design change, needs no infrastructure, and removes the cost
   rather than caching around it.
2. **Serialise per fixture** — a lock per fixture key so the first test proves and the
   others wait and hit. Real, but it trades wall clock for contention and only pays where
   the proof is slower than the wait.
3. **Accept it** and stop having agents run the full suite at all: run targeted tests
   during development, one full run at the end. Costs nothing to adopt.

Option 3 is free and was adopted immediately. Option 1 is the real fix.

## Three product-level bugs found while attempting this

Recorded because they outlive the attempt:

1. **A zero-parameter function checked with `fuzz` earns `tested`** — a documented special
   case — but the record layer's own notion of which verdicts a check can earn does not
   know about it. The two disagree.
2. **A reuse hit skips code generation entirely**, so any test that reads a generated
   artifact finds nothing to read. That is correct behaviour, but it means "reused" and
   "the artifacts from this run exist" cannot both be assumed, and nothing states it.
3. The generated sampling harness lives in a second location that a freshness check
   looking only for the first one misses.

## The honest cost

Roughly two hours, four abandoned full-suite runs, and no speed-up. What was bought: the
knowledge that this approach cannot work, three product bugs, and a measurement that
stops anyone trying it again. Recorded rather than quietly dropped, because a failed
approach with a number attached is cheaper than the same idea occurring to someone next
month.
