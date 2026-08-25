# The record of results, and what invalidates it

*2026-08-25. Written against the build in this repository; every output below is copied
from a real run rather than reconstructed.*

Ply used to re-pay full engine cost on every run. A crate whose proofs took four minutes
took four minutes again on a branch that touched a README, and a diff never showed that a
claim had been checked at all. The specification's answer to that was a committed record
of verdicts, a "stale" state for when the code drifted away from one, a command for a
human to re-bless the drifted claims, and an error for blessing something that had last
failed — four pieces of machinery that exist only to manage the consequences of
*remembering a verdict*.

This replaces all of it with something smaller. A result is stored beside a hash of
everything the answer depended on, and that hash is recomputed **before the stored result
is ever used or shown**. It matches: reuse it, and say so. It does not: run the check
again. There is nothing to re-bless, because there is no window in which a stored result
might have gone quietly wrong — the confirmation happens at every single use.

| Was | Is |
|---|---|
| A verdict recorded in a file, with a warning when the code moved under it | A verdict recorded beside a hash of what it stood on |
| A `stale` state on the node, and a warning code for it | No such state. A result whose hash no longer matches is not shown; it is re-earned |
| `cargo ply accept`, plus an error for blessing a failed claim | Deleted. Nothing to confirm |
| "Every run re-pays full engine cost" | 89s the first time, under a second the second |

---

## What is hashed, and why each input is in there

Every input below is one the answer genuinely depended on, and leaving any of them out
means reusing a result that is about something else.

| Input | Why it is in the hash |
|---|---|
| The checked function's own source, as tokens | The obvious one. Tokens, not text, so reformatting a function or editing the comment above it does not cost you a four-minute proof |
| Its contract — written on the function, or declared for it in the configuration file | The contract *is* the claim; a changed claim is a different question |
| Every promise the proof stood on instead of real code | When a proof crosses into old code, it reasons about a promise somebody wrote rather than the body. The result is *about that promise*, so editing the promise has to re-run every caller resting on it, even though no Rust changed |
| The checks that ran, and the seed the generated cases were drawn from | `bounded(3)` is not `bounded(2)`, and a replay of a different seed is a different run |
| Each engine's name, version and flags | A proof earned by one model checker at one version, under one set of flags, is not evidence about another |
| The compiler and the build target | An old success must not bless a different toolchain |
| The crate's declared features | Different features, different code |
| **Ply's own version** | The one that makes the scheme sound rather than merely fast — see below |

### Why Ply's own version belongs in there

In the hours before this change, four defects were fixed that each changed what a result
*means*:

- a test harness that failed to compile earned a confident pass;
- an ordinary import let a proof silently include code nobody had vouched for;
- an impossible declared promise made a proof succeed vacuously;
- a claim inside a nested group was skipped entirely, in silence.

Every result recorded by yesterday's build carries one of those risks. Every one of them
would hash-match perfectly today on the user's source alone, because the source did not
change — Ply did. Putting the version in the hash invalidates exactly the results a tool
fix should invalidate, automatically, with nobody having to remember which release had
which bug. It is demonstration 5 below, on a real crate.

### What is deliberately *not* hashed

**The time budget.** A proof that finished inside 300 seconds is not made false by a later
run that would only have allowed 60, and folding the budget in would re-pay every proof in
a CI job that sets a different one.

**Anything that failed.** A result is recorded only if it earned evidence. A violation, a
timeout, a missing engine, a shape Ply cannot build inputs for, a harness that would not
compile, a claim that asked for nothing — none of them is stored, so none of them can be
carried forward, and a check that times out re-pays its budget every run. The rule reads
one shared vocabulary of "this is an absence of evidence" — the same list the exit code
reads, in one place, so the next absence added cannot be missed by one of the two.

---

## What the record looks like

`ply.lock`, beside `ply.yaml`, committed. One entry per claim: the fingerprint, the
verdict, the statuses beside it, the evidence block naming the run where there was one,
and the diagnostics that came with it. Demonstration 1 below shows a real one in full.

The diagnostics are stored with the result on purpose. What gets carried forward is the
*whole* report: a reused result that stands on an unchecked promise still prints the
paragraph naming that promise, word for word. A reused verdict that printed its marks
without its explanation would be a worse report than no reuse at all.

`written_by` is informational — the version is already inside every fingerprint — and it
names the version that last wrote the file, so a reviewer can see which Ply produced these
results without decoding a hash.

If the file cannot be read, the run stops and says so rather than quietly re-proving
everything. A committed file gets merge conflicts, and the message names the file and the
fix:

> `…/ply.lock` holds the results of previous runs, and this one could not be read. If it
> has merge-conflict markers in it, or was hand-edited, delete it and run
> `cargo ply verify` again: nothing is lost except the engine time to earn those results
> back

---

## The failures watched first

Five end-to-end tests, written against the old binary, each failing with a message that
names the actual defect.

**1. Nothing is recorded at all.**

```
---- a_first_run_records_what_it_earned stdout ----
thread 'a_first_run_records_what_it_earned' panicked at tests/e2e/tests/resultreuse_fixture.rs:24:5:
a first run must leave a record beside ply.yaml, or nothing can ever be reused: {"command":"verify", …}
```

**2. A second run re-proves everything.**

```
---- a_second_run_reuses_what_the_first_one_earned_and_says_so stdout ----
assertion `left == right` failed: a node whose inputs still hash the same must be carried
forward, not re-proved: { … }
  left: Null
 right: true
```

**3. Editing one function re-pays for the other.**

```
---- editing_a_function_re_runs_that_claim_and_only_that_claim stdout ----
assertion `left == right` failed: the untouched claim must still be reused -- a per-crate
hash would re-pay every proof for one edit: { … }
  left: Null
 right: true
```

**4. Editing a promise does not disturb the claim that assumed nothing.**

```
---- editing_only_a_declared_promise_re_runs_the_claim_that_rests_on_it stdout ----
assertion `left == right` failed: the claim that assumed nothing is untouched by the
edit: { … }
  left: Null
 right: true
```

**5. The terminal never mentions it.**

```
---- the_terminal_says_which_results_were_carried_forward stdout ----
the node line must say the result was carried forward: workspace — bounded(2)  [assumed, evidence owed]
  resultreuse — bounded(2)  [assumed, evidence owed]
    safe_increment — bounded(2)
    total — bounded(2)  [assumed, evidence owed]
```

And one unit-level failure watched separately, by deleting the version from the hashed
inputs — the whole scheme in one assertion:

```
---- record::tests::a_new_ply_version_invalidates_a_result_whose_source_did_not_change stdout ----
assertion `left != right` failed: a result recorded by yesterday's build must not be
reused by today's -- the four defects fixed on 2026-08-25 would each have matched perfectly
  left: "ab0c930e0f2cf5d996861e93deab1336cae13257d2d382b874c934bc5bf38c81"
 right: "ab0c930e0f2cf5d996861e93deab1336cae13257d2d382b874c934bc5bf38c81"
```

The fingerprint tests are one loop over every hashed input rather than a pile of
spot-checks, for the reason the repository's own guidance gives: an input added later that
nothing feeds into the hash is exactly the defect worth catching, and a per-field
spot-check would not catch the field nobody remembered to add.

---

## Five demonstrations, on a real crate

The crate is `tests/fixtures/resultreuse`. Three claims: one standing on its own body
(`safe_increment`), one standing on a promise declared for the old code it calls
(`total` → `legacy_rate`), and one checked by sampling rather than by proof (`widen`).
Every block below is verbatim terminal output, captured in one sitting on a machine that
was also running the full test suite — so the wall-clock figures are if anything
pessimistic.

### 1. Verify once

```
$ cargo ply verify . --engine-timeout 300
workspace — fuzzed(64)  [assumed, evidence owed]
  resultreuse — fuzzed(64)  [assumed, evidence owed]
    safe_increment — bounded(2)
    total — bounded(2)  [assumed, evidence owed]
    widen — fuzzed(64)

  [assumed]        this result rests on a promise Ply was handed and did not check — if the promise is wrong, the result is wrong with it
  [evidence owed]  nothing has run the real code against that promise yet; the lines below name it and say what would settle it

[W0511] resultreuse::total — `total` earned bounded(2), but conditionally: the proof used the
contract declared in ply.yaml for each callee it crosses into, instead of that callee's real
body. Assumed: `legacy_rate`: ensures |result| *result <= 10_000. …

[wall clock: 89s]
```

The record it leaves, complete for the two claims whose entries are short (the third
carries the assumption paragraph shown above, elided here):

```json
{
  "format": 1,
  "written_by": "0.1.0",
  "results": {
    "resultreuse::safe_increment": {
      "fingerprint": "f4c12c511c1ffdef0a4a6be4167ea2980b21e404582bc17a94747899e6de4091",
      "verdict": "bounded(2)",
      "statuses": []
    },
    "resultreuse::total": {
      "fingerprint": "c28a8fad98bbc3b859ad4a889c00a305ba223eb991dcb4f467ce5a785a38dcff",
      "verdict": "bounded(2)",
      "statuses": ["conditional", "owed-evidence"],
      "diagnostics": [ … the paragraph naming the promise this result rests on … ]
    },
    "resultreuse::widen": {
      "fingerprint": "a96cfb04d8c7814d521f8885293fb5a153626f7a7b34dca9b507c1828ccdc469",
      "verdict": "fuzzed(64)",
      "statuses": [],
      "evidence": {
        "engine": "proptest",
        "seed": "fd465ecffe482b0396c2aab6ad3483640359c833e139c5e40c7ad21350d99a0c",
        "cases": 64
      }
    }
  }
}
```

The sampled claim keeps the seed and the case count of the run that actually happened, so
a reused `fuzzed(64)` still names a run somebody can repeat.

### 2. Verify again — the results are reused

Nothing touched, same command:

```
workspace — fuzzed(64)  [assumed, evidence owed]
  resultreuse — fuzzed(64)  [assumed, evidence owed]
    safe_increment — bounded(2)  [reused]
    total — bounded(2)  [assumed, evidence owed, reused]
    widen — fuzzed(64)  [reused]

  [assumed]        this result rests on a promise Ply was handed and did not check — if the promise is wrong, the result is wrong with it
  [evidence owed]  nothing has run the real code against that promise yet; the lines below name it and say what would settle it
  [reused]         this result was not re-run: an earlier run recorded it, and everything it depended on — the code, the promises it assumes, the checks, the engines, Ply's own version — hashes the same today

[W0511] resultreuse::total — … Assumed: `legacy_rate`: ensures |result| *result <= 10_000. …

[wall clock: 0s]
```

**89 seconds to under one.** (Measured to 0.06s elsewhere in the same session, on a quiet
machine.) Both marks qualifying the middle result are still there, and so is the paragraph
naming the promise it stands on: what is carried forward is the report, not just a word.

### 3. Change the function — that claim is re-run, and only that claim

`safe_increment`'s body becomes `let step = 1; x + step`. Nothing else moves:

```
workspace — fuzzed(64)  [assumed, evidence owed]
  resultreuse — fuzzed(64)  [assumed, evidence owed]
    safe_increment — bounded(2)
    total — bounded(2)  [assumed, evidence owed, reused]
    widen — fuzzed(64)  [reused]

[wall clock: 35s]
```

`safe_increment` lost its `[reused]` mark — its recorded result was about code that is no
longer there, so it was proved again. The other two kept theirs. A hash over the whole
crate would have re-paid all three.

### 4. Change *only* a declared promise — the claim resting on it is re-run

No Rust is touched at all. The promise declared for `legacy_rate` in `ply.yaml` goes from
`*result <= 10_000` to `*result <= 9_000`, and the source checksum is identical before and
after the run (`9de025a63bcae61e7abaee8cb1ee2cda` both times):

```
workspace — fuzzed(64)  [assumed, evidence owed]
  resultreuse — fuzzed(64)  [assumed, evidence owed]
    safe_increment — bounded(2)  [reused]
    total — bounded(2)  [assumed, evidence owed]
    widen — fuzzed(64)  [reused]

[wall clock: 30s]
```

`total` was proved again — its proof was *about* that promise — while the two claims that
assume nothing were reused. This is the demonstration that the record is not a cache of
function bodies.

### 5. Change Ply's own version — every stored result is invalidated

The only edit is Ply's version number, `0.1.0` to `0.1.1`; the binary is rebuilt and the
same crate verified again. The crate's own files are byte-identical either side of it:

```
$ md5sum src/lib.rs ply.yaml
9de025a63bcae61e7abaee8cb1ee2cda  src/lib.rs
fd5adcc5a6ef091a09fe7c76fb7b78f5  ply.yaml

$ grep fingerprint ply.lock
      "fingerprint": "a88affb052747079137d57ce897f482a4b59942bf9f5dcd7a3a34e6b0356649d",
      "fingerprint": "95a73be0572283243665e71f4fa2f16bd9a123591c9fd13ec89d9af309aefd24",
      "fingerprint": "a96cfb04d8c7814d521f8885293fb5a153626f7a7b34dca9b507c1828ccdc469",

$ cargo ply verify . --engine-timeout 300          # under Ply 0.1.1
workspace — fuzzed(64)  [assumed, evidence owed]
  resultreuse — fuzzed(64)  [assumed, evidence owed]
    safe_increment — bounded(2)
    total — bounded(2)  [assumed, evidence owed]
    widen — fuzzed(64)

[wall clock: 67s]

$ grep -E '"fingerprint"|"written_by"' ply.lock
  "written_by": "0.1.1",
      "fingerprint": "ef4e5d7563849dc2e91bb3a9afddd7279abf31acfe84af9ad69d0cae7560a321",
      "fingerprint": "8676e92d6a83a6beda8597875be429181cfd310768fcc0f0043b2ec559807323",
      "fingerprint": "107148a787f45906371f071c0bd330c543e3ed1fe02dd6f0d4b5c44604ef5619",

$ md5sum src/lib.rs ply.yaml
9de025a63bcae61e7abaee8cb1ee2cda  src/lib.rs
fd5adcc5a6ef091a09fe7c76fb7b78f5  ply.yaml
```

Not one `[reused]` mark, 67 seconds of real engine work, three new fingerprints — for a
crate in which not a single character changed. That is the whole argument: had this been a
release that fixed a defect in how proofs are built, every result earned under the old one
would have been thrown away and re-earned, with nobody deciding which results the fix
touched.

The invalidation is one-shot, not a standing penalty. The next run under 0.1.1:

```
    safe_increment — bounded(2)  [reused]
    total — bounded(2)  [assumed, evidence owed, reused]
    widen — fuzzed(64)  [reused]

[wall clock: 0s]
```

### And the case committing the record exists for

A clone with the file and none of the build output: no `target/`, no generated proof
module, not even the line declaring it. Nothing in a fingerprint is keyed on any of that:

```
$ ls src/
lib.rs

$ time cargo ply verify . --engine-timeout 300
workspace — bounded(2)  [assumed, evidence owed]
  resultreuse — bounded(2)  [assumed, evidence owed]
    safe_increment — bounded(2)  [reused]
    total — bounded(2)  [assumed, evidence owed, reused]
…
real	0m0.064s

$ ls src/
lib.rs
```

Same verdicts, 64 milliseconds, nothing compiled and nothing written back. That is the CI
run and the colleague's first checkout. It is pinned by a test
(`a_checkout_with_the_record_and_no_build_output_still_reuses`).

---

## Everything else stays green

Both suites were run in a clean checkout of this branch, in a worktree of its own, because
a second session was editing the shared crate alongside this work and a half-finished edit
there had already broken one test run that had nothing to do with it.

| Suite | Result |
|---|---|
| product workspace (`cargo test --workspace`) | **284 passed, 0 failed**, exit 0 |
| specification tooling (`cd tools && cargo test --release`) | **118 passed, 0 failed**, exit 0 |

Formatting and lint are clean in both workspaces (`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets`). The product count was 257 before this work: the
difference is this change's own tests — eleven on the record and the fingerprint, one table
covering what may be stored, one pinning the model checker's flag set, and seven end to end
on the fixture above.

---

## What was removed

- **The re-blessing command** (`cargo ply accept`) and the error for blessing a claim
  whose last run failed — from the command list, the design decision that introduced them,
  and the milestone that scheduled them.
- **The "code changed since the evidence was recorded" state**: the status name, its
  warning code, its row in the user-facing verdict table, and its place in the status
  vocabulary that travels up the tree.
- **The staleness tier of `cargo ply check`** — it reported "NOT CHECKED, this needs a
  file Ply does not write yet" on every clean run. There is nothing there to check now:
  the confirmation happens inside `verify`, at the moment a result is used.
- **The backlog line in `cargo ply worklist`** for claims waiting to be re-confirmed.
- Two sentences elsewhere that explained a gap by naming a file Ply never wrote. They now
  say what is actually true: those commands start no engines and do not read what an
  earlier run recorded.

What stayed, reworded: an **attestation** — a human's word that something outside Ply's
reach holds — is still not checked against the item it vouches for, and `cargo ply audit`
still says so on every run. That is a different thing from a recorded result, and the
wording now says why: Ply keeps a hash of what it checked itself, and nothing runs for a
person's word.

---

## Honest gaps

1. **The fuzz tier's engine version is a requirement, not a resolved version.** Ply
   records the property-testing library at the version *requirement* it writes into the
   generated test crate (`1`). A `1.x` release that changed how a strategy draws values
   would keep that string, so a result recorded before it can be reused after it. Closing
   this means reading the generated crate's resolved lock file, which does not exist on a
   first run — the fix is real but not free, and it is recorded here rather than papered
   over.
2. **A promise still does not go stale.** If the old code changes underneath a standing
   promise, nothing notices: the caller's result is hashed against the *promise*, because
   the promise is what its proof used, not against the body it was never allowed to look
   at. This is unchanged, and it is already documented as one of the four things to know
   before trusting a boundary promise.
3. **`worklist` and `audit` do not read the record.** Both could now answer questions they
   currently list as "not checked" — a weak spec found by an earlier run, an assumption
   some later run discharged. Left alone deliberately: the brief was reuse, not a new
   reader.
4. **The record is trusted like the source it sits beside.** The hash guards a stored
   result against *drift* — the inputs moving under it — not against somebody editing the
   file. A hand-edited `ply.lock` claiming `proved` is believed, exactly as a hand-edited
   source file is. That is the same trust boundary the repository already has, and worth
   stating rather than leaving to be discovered.
5. **A claim scoped to part of a crate would need a narrower prune rule.** Entries for
   claims the document no longer contains are dropped at the end of a run, which is
   correct today because `verify` always reads the whole document. If `verify [fn]` ever
   lands, that rule needs to shrink to the claims in scope. The same rule means a machine
   that cannot reproduce a result — no model checker installed, a different toolchain —
   drops it from the file rather than leaving a verdict its own run did not stand behind.
   Two machines with different toolchains will therefore take turns rewriting the record;
   that is inherent in one entry per claim, and the alternative is a file that shows
   results the last run did not produce.

---

## One thing worth arguing about later

Putting Ply's version in the hash is right, and the granularity is blunt: *any* version
change throws away *every* recorded result in every repository, including the ones no
change could possibly have affected. That is the conservative direction and it is the
correct default — a fix whose blast radius nobody can enumerate should invalidate
everything — but it means an ordinary tool upgrade costs a full re-verification of every
codebase that uses it, which on a large one is measured in hours of CI. The obvious
follow-up is a separate number that only moves when something about *how a result is
earned* changes, so that a release fixing a diagram colour does not re-prove a workspace.
That is a decision for whoever owns releases, and it should be made deliberately rather
than discovered the first time an upgrade lands.

---

## TODO deltas

Not applied — `TODO.md` was out of bounds for this change. These are the lines it wants:

- **Done, this change**: result reuse keyed by a content fingerprint, including Ply's own
  version; the record committed as `ply.lock`; the re-blessing command, the stale status,
  its warning and its error removed from the specification and from every command that
  anticipated them.
- **KNOWN GAP (new)**: the fuzz engine's version in a fingerprint is the requirement Ply
  writes, not the resolved version (gap 1 above).
- **KNOWN GAP (new)**: the record is guarded against its inputs moving, not against being
  hand-edited (gap 4 above).
- **Open question (new)**: whether every Ply version bump should invalidate every stored
  result, or only one that changes how a result is earned — see the section above.
- **Found while working, not fixed, not mine**: `docs/SCHEMA.md` still says in two places
  that a promise which cannot be satisfied is undetected ("Nothing detects this yet",
  section 6; and the not-built list in section 14). The commit that added that detection
  did not update the page. Both statements are false as of that commit.
