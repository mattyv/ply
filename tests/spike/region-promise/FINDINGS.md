# Region-promise spike — findings

Run 2026-08-25, Linux x86_64, `cargo-kani 0.67.0` / CBMC 6.8.0 — the pin, untouched.

`tests/spike/havoc/FINDINGS.md` killed the *empty* region contract: replace a legacy
callee with an unconstrained return and 2 of 8 crossings still verify, none of them
written without the experiment in mind. But "write nothing" was never the maintainer's
ask. The ask was **"don't make me write a promise per function."** So there is a middle
this spike measures:

> **One promise, declared once for a whole region, applied to every function in it.**

Unlike havoc it says something, so the caller's verdict is honestly `conditional` rather
than a proof over an unconstrained value. Unlike a per-callee contract you write it once.
The question is whether it rescues the six crossings havoc failed.

**It rescues one.** The measurement is below, and the mechanism that limits it is
structural rather than a property of this sample.

**Nothing here implements `given:` or any region construct.** Every harness is hand-written
in the shape `crates/ply-core/src/harness.rs::generate_proof_module` already emits, with
the stub body carrying one clause written once and expanded into every stub for that
region (`natural/feature/src/ply_generated.rs`'s `catalog_promise!`). No change to
`crates/`, `tools/`, or the spec. Re-run every row with `./run.sh`. Every row below is a
literal observed run; the NOT RUN list at the end is part of the finding.

---

## The headline number

**3 of 8 crossings pass under a region-wide promise, against 2 of 8 under havoc.**

The one crossing gained is `remaining_limit_cents`. The five still failing fail for a
reason the promise cannot fix by being written better, and the pre-registered
tightest-possible variant confirms it: **B1 and B1-tight produce the identical verdict on
every row.** The ceiling was not the variable.

**Clauses per region, honestly counted:**

| region | functions in it | honest clauses writable | crossings it rescues |
|---|---|---|---|
| `ledger::fees` (`given004`) | 1 | 1 | 2 of 2 (both already passed under havoc) |
| `catalog` (`natural`) | 6 | **1** — and one is all there is | **1 of 6** |

So the "one line instead of six" claim is **true and it does not help.** One clause really
does cover all six `catalog` functions; it is also the *only* clause honestly writable
about all six, and it carries one of them.

## Sample, and how it was kept honest

**The eight crossings are byte-for-byte the havoc spike's.** Not rewritten, not made
friendlier. `run.sh`'s `hashes` row proves it on every run:

```
c0cf3136d6dcc095de2eb53d3417e31afad6c42764d104e83bf14fb2d25bbf4a  natural/feature/src/lib.rs
b109337d248e9c6c9c79cb6ba733fb2d6594f6c23826065970e3e26d99b95bac  natural/legacy/src/lib.rs
HASHES MATCH
```

Both match what `havoc/FINDINGS.md` recorded before *its* first run. `given004/` is
`diff -r` identical. The single manifest difference is a `[features] tight` stanza added to
`natural/feature/Cargo.toml` so the pre-registered tight variant runs without editing the
committed harness; `run.sh` prints that diff and names it as the one permitted delta.

**The promises were written first and did not move.** `PROMISES.md` was committed in
`dab312f`, before any Kani invocation, and hashes to
`e5f83887c931051d840bd7409a331e1ee23ea8eacd8c5a46684d20fa616e1edd` unchanged at the end of
the run. It was written from each module's own point of view — what its owner could say
about all of it — and it records two decisions taken in advance that turned out to matter:

* **The non-zero clause was refused up front.** An owner of `catalog` wants to add "and
  every count here is at least 1." It is false: `spend_limit_cents(0)` returns `0`, the
  body says so and the module's own test `a_new_account_has_no_limit` asserts it. Refusing
  it before running is the honest move; adding it afterwards because two callers failed on
  zero would have been reverse-engineering. **No promise was tuned after seeing a
  failure, and no row here is a retry.**
* **`ledger` as a whole admits no single clause, and that was recorded before running,**
  because it is a type-level fact and not a result. Its public surface returns `u64`,
  `i64` (a legitimately negative balance), `Vec<Entry>`, `Vec<AccountId>` and `u32`. The
  largest honest region is the `fees` submodule, which holds exactly one function — hence
  the 1:1 ratio in the table above.
* **The ceiling objection was answered in advance, not argued afterwards.** B1 is
  `<= 1_000_000` (a round number with headroom, which is what a region-wide promise is
  written with); B1-tight is `<= 500_000`, the module's real maximum, below which the
  promise becomes false. Both were pre-registered. Both were run. They agree on every row.
* **The per-callee comparison column was pre-registered too**, six clauses written from
  each callee's own meaning before any run, so it is not a column tuned to flatter or to
  lose.

**Where bias could still live, stated rather than denied.** The promises are mine, written
in one sitting, and a different owner would phrase them differently — though B1-tight
brackets the whole honest range of the only quantity a `catalog`-wide clause can bound.
The deeper limit is the sample: six functions by one author, inherited from the havoc
spike along with its own stated limits.

---

## Results

`--harness-timeout 300s` on every row (§6's stubbed floor). "VT" is CBMC's own Verification
Time; wall is the whole `cargo kani` invocation. `TIMEOUT@300s` is CBMC's own
`CBMC timed out` message, told apart from a real violation by reading past
`VERIFICATION:- FAILED`.

**Timing caveat, up front.** Another agent was running tests on this machine during the
Group A rows. The same harness shapes measured 133.74s and 212.63s in the havoc file and
measured 203.28s and 293.01s here. Group A timings are contended and should not be read as
a cost signal; `r2` finished 7s inside the 300s cap and could plausibly have timed out.
Group B rows ran in one window and compare with each other cleanly.

### Region A — `ledger::fees`, promise A1: `result <= 10_000` (1 clause, 1 function)

| row | crossing | havoc | **region promise** | VT | wall |
|---|---|---|---|---|---|
| **r1** | `tier_fee_cents` — the flagship crossing | SUCCESSFUL | **SUCCESSFUL** | 203.28s | 233s |
| **r2** | `approve_withdrawal` — transitive, never names `ledger` | SUCCESSFUL | **SUCCESSFUL** | 293.01s | 294s |

Both already passed under havoc. The region promise changes nothing here **because
`tier_fee_cents` clamps its own input** — it is correct for any value the callee returns,
so no contract was ever load-bearing. The mutation pair below is what turns that sentence
into a measurement.

### Region B — `catalog`, promise B1: `result <= 1_000_000` (1 clause, 6 functions)

| row | crossing | baseline (no stub) | havoc | **region B1** | B1-tight (500_000) | per-callee |
|---|---|---|---|---|---|---|
| r3 | `gross_cents` (havoc's n1f: the widened scratch copy) | SUCCESSFUL 49.04s | FAILED | **FAILED** 63.52s — `vat_bps = 955_611`, add overflow | FAILED 59.43s — `438_016` | **SUCCESSFUL** 48.03s |
| r4 | `line_total_cents` — units × a returned price | (havoc: SUCCESSFUL 17.76s) | FAILED | **FAILED** 40.58s — `unit_price_cents = 328_501`, postcondition | FAILED 40.71s — `156_152` | **SUCCESSFUL** 22.68s |
| r5 | `top_band_price_cents` — index from a returned length | (havoc: SUCCESSFUL 27.97s) | FAILED | **FAILED** 71.12s — `band_count = 0` (subtract overflow) **and** `= 5` (index out of bounds) | FAILED 65.65s — `0` and `5` | **FAILED** 48.15s — `band_count = 36` |
| r6 | `batches_needed` — ceiling-divide by a returned divisor | (havoc: SUCCESSFUL 38.05s) | FAILED | **FAILED** 84.79s — `batch_size = 0` (subtract overflow **and** divide by zero) | FAILED 85.02s — `0` | **SUCCESSFUL** 48.72s |
| r7 | `manifest_weight_grams` — loop over a returned count | (havoc: SUCCESSFUL 22.18s) | TIMEOUT@300s | **TIMEOUT@300s** (wall 301s, 1,909 loop unwinds, no witness) | TIMEOUT@300s (301s) | **TIMEOUT@300s** (311s, 1,971 unwinds) |
| **r8** | `remaining_limit_cents` — subtract against a returned limit | (havoc: SUCCESSFUL 17.01s) | FAILED | **SUCCESSFUL** 24.42s | SUCCESSFUL 22.28s | SUCCESSFUL 23.50s |

Baselines in brackets are the havoc file's own, on the identical source; only `r3`'s was
re-run here because its scratch copy is built fresh each time.

### Mutation rows

| row | what it mutates | expected | observed |
|---|---|---|---|
| **x1** | **mutation on a pass**: strip B1's `kani::assume` out of `r8`'s stub, leaving the empty contract | must flip to FAILED, or `r8`'s pass never depended on the promise | **FAILED** 34.66s — `spend_limit_cents = 183_558_144`, *the identical witness havoc's n6 reported*. The stub is applied and the clause is load-bearing. |
| **x2** | delete `tier_fee_cents`'s own `.min(10_000)`, **keep** A1 | havoc's m1 on this exact mutation FAILED; if A1 is applied it must now pass | **SUCCESSFUL** 152.40s. The promise supplies precisely what the deleted clamp did. |
| **x3** | the same mutated caller, promise removed (this is havoc's m1, re-run as x2's control) | must FAIL | **FAILED** 171.20s — `bps = 2_640_338_943`, `amount = 67_108_863`, `tier = 255`: **byte-identical to havoc's m1 witness.** |

x2 and x3 are a matched pair and together they are Region A's stub-applied check: the same
caller, the same mutation, passing with the promise and failing without it. x1 is the same
check for Region B, in the ordinary direction.

Counted as crossings for the headline: r1, r2, r3, r4, r5, r6, r7, r8 — the same eight
havoc counted. `withdraw` still has no harness (havoc g5: `ledger::Ledger` is not
`kani::Arbitrary`); nothing about a region promise changes that.

---

## What the numbers say

### 1. A region promise is only as tight as its loosest function — and that is structural

This is the mechanism, and it explains four of the five failures without appealing to the
sample at all.

A region-wide ceiling has to hold for **every** function in the region, so it is pinned by
the region's largest value. `catalog`'s largest is `spend_limit_cents`'s 500,000. But
`gross_cents` needs `vat_bps <= 10_000` and `line_total_cents` needs
`unit_price_cents <= 100_000`. Each caller of a *small* member of the region inherits the
ceiling set by the *biggest* member — 50× and 5× too loose respectively. The per-callee
column proves the gap is exactly that and nothing else: **r3 and r4 fail under B1 and pass
under a clause differing only in its number** (48.03s and 22.68s).

**That is why B1-tight changed no verdict.** Tightening 1,000,000 → 500,000 was the
largest honest move available and it was never going to be enough, because 500,000 is
already the region's floor for honesty. There is no third variant to try. The clause set
honestly writable about `catalog` is essentially `{result <= 500_000}`, and it is
maximal.

### 2. The clause the failing callers need is a **lower** bound, and a region can rarely make one

Of the five failures, two (`r5`, `r6`) fail on **zero**: `band_count = 0` overflows the
subtraction; `batch_size = 0` overflows the subtraction and then divides by zero. Both want
`>= 1`.

A region promise is naturally an *upper* bound — "nothing here is bigger than X" is the
thing a module owner can say about a mixed bag of config values. A *lower* bound has to
hold for every member, and **one function in the region that legitimately returns zero
kills it for all of them.** `catalog` has exactly such a function, and it is not contrived:
a brand-new account has no spend limit set. PROMISES.md refused that clause in advance for
this reason, and the two zero-witness failures are what the refusal costs.

So the two halves of the finding meet: the clause a region can write (an upper bound) is
too loose for the callers that need an upper bound, and the clause those other callers need
(a lower bound) is one a region usually cannot write.

### 3. A region promise can never beat the per-callee route. This is a subset property, not a measurement

Mechanically a region promise **is** a per-callee `ensures` with the same text repeated
across the region's functions. For any single crossing, the region clause is therefore
pointwise no stronger than the best per-callee clause for that callee. Its pass set is a
subset of the per-callee pass set, always.

The data lands exactly there: region passes `{r8}`, per-callee passes
`{r3, r4, r6, r8}` — a strict superset. There is no row where the region promise wins and
no row where it could.

What that means for the value case: the region form's only possible advantage is *typing
less*, never *proving more*. And on this sample typing less bought 1 crossing where typing
six times bought 4.

### 4. Cost, per clause — the one number that flatters the region form, and why it still loses

| route | clauses written | crossings rescued (of 6 in Group B) | crossings per clause |
|---|---|---|---|
| region promise B1 | 1 | 1 | 1.00 |
| per-callee | 6 | 4 | 0.67 |

Per clause the region form is *more* efficient. That is real and worth stating, because it
is the strongest version of the maintainer's argument. But it is capped, not merely
smaller: no second region clause exists to write. The per-callee route can go on buying
crossings by writing more clauses; the region route has spent its whole vocabulary after
one line.

### 5. The verdicts would be honest — and the debt is bigger than the declaration

**Structurally, a region promise can only ever yield `conditional`.** It is an assumed
contract, so §5.5's second branch applies verbatim: stub the callee, mark the caller
`conditional` with `owed-evidence`, list the assumption (`W0511`). Nothing in this route
produces a clean `bounded`, and the mutation rows confirm nothing goes green that should
not — x1 and x3 both fail loudly the moment the assumption is withdrawn.

But there is an honesty wrinkle the "one line" pitch hides, and the spec would have to
carry it: **writing is 1:6, discharging is still 1:1.** The `owed-evidence` debt of a
region promise is one obligation *per function in the region*, because the clause has to be
checked against six real bodies before it stops being green paint. §5.5's third honesty
condition says fuzzing can discharge a boundary contract against the real legacy body —
that is six fuzz targets, from one declaration. One line to write, six to pay off.

Worse for staleness: a function added to `catalog` next quarter is **silently** covered by
B1. If it returns 2,000,000 the declaration does not change, no diagnostic fires, and every
caller's `conditional` verdict now rests on a false assumption. Per-callee, the new callee
raises a fresh `W0512` at its first crossing. That is the same argument the havoc findings
made against silent transitive coverage, and it applies with more force here because the
region promise is a *substantive* claim that can become false, not merely an empty one.

### 6. Two findings that are about §5.5 rather than about regions

* **A per-callee contract has to be tight enough for the caller's *use*, not merely true of
  the callee** (`p5`). PROMISES.md's clause for `band_count` — `>= 1 && <= 64` — is an
  honest statement about the callee written before any run, and the caller still fails:
  `band_count = 36`, index out of bounds. The reason is a real defect in the caller,
  exposed here for the first time: `top_band_price_cents` indexes `card[band_count() - 1]`
  into a fixed `[u32; 4]` with no bound check, so it needs `band_count() <= 4` —
  a fact about the *caller's array*, which no honest author of `catalog` would think to
  write. This is a live gap in §5.5's second branch and it has nothing to do with regions:
  the contract a boundary needs is not always a contract the callee's owner would author.
* **The loop-bound shape is not fixable by any contract at this floor** (`r7`, `p7`).
  Havoc's finding 3 reported `manifest_weight_grams` unwinding 2,387 times to a bare
  timeout and asked whether a contract would rescue it. It does not: under B1 it unwinds
  1,909 times, and under a per-callee `manifest_lines() in 1..=1_000` it unwinds 1,971 —
  **both time out at 300s with no witness**, where the unstubbed baseline verifies in
  22.18s. That is now measured under all three routes, so havoc's "fourth outcome" is a
  property of stubbing a loop bound, not of havoc. §6's absence vocabulary still needs a
  word for it.

---

## Recommendation: **a region-wide promise does not earn a place in the grammar**

It is honest, it is cheap, it is checkable, and on the sample this project has it converts
**one** additional crossing out of eight. Against that:

1. **It cannot beat the per-callee route on any crossing, ever** (finding 3). Its pass set
   is a subset by construction. A construct whose ceiling is "no better than the feature we
   already have" has to justify itself purely on typing, and one clause here bought one
   crossing where six clauses bought four.
2. **Its one available clause is pinned loose by the region's biggest member** (finding 1),
   which is why the pre-registered tight variant moved nothing. This is structural: any
   region holding values of different magnitudes has this property, and a legacy config
   module is exactly such a region.
3. **The clause the failures need is usually a lower bound, and one honest zero anywhere in
   the region forbids it** (finding 2). Legacy config modules reliably contain that zero.
4. **The declaration is 1:6 but the debt and the staleness risk stay 1:1** (finding 5), and
   the staleness is silent in the direction that matters.

**If it were built anyway, here is exactly what it should be allowed to say**, because the
run does bound that precisely:

* **Upper bounds on scalar returns only.** That is the only clause shape a region can
  honestly carry, for the reason in finding 2.
* **Never a lower bound, unless every function in the region is checked against it** — at
  which point it is per-callee work wearing a region's clothes.
* **Only over a region whose functions return one type and one kind of quantity.** `ledger`
  cannot carry a region clause at all (recorded before running, finding in PROMISES.md);
  `catalog` can carry one because its six functions all return small non-negative counts and
  amounts. The spec would have to state that limit, and stating it honestly reveals how
  narrow the construct is: a region of one kind of quantity is close to a callee.
* **The declaration must be closed, not open.** A region promise that silently absorbs
  functions added later is the failure mode in finding 5; it would have to name its members
  and raise a diagnostic when the region gains one.

**The positive recommendation is the havoc file's, unchanged:** take open question 6's
fallback — per-callee entries in `ply.yaml`, no new grammar. This run strengthens it rather
than qualifying it, because the per-callee column here rescued 4 of 6 crossings using
clauses that were pre-registered from the callee's own meaning, at 22–49s each.

**Weakest part of this recommendation, stated so it can be argued with.** Only two regions
were measured and one of them holds a single function, so the clause-per-function ratio
rests on `catalog` alone. A legacy region that really is homogeneous — a table of six rates
all in basis points, say — would carry a tight region clause and rescue every caller of it.
That region exists in real codebases. The counter is finding 3: those callers pass under
the per-callee route too, and a region of one quantity is a region whose per-callee
contracts are all the same line anyway, which is a copy-paste cost, not a design problem.

---

## Deltas the maintainer may want (not applied here — `TODO.md` and the spec untouched)

1. **`docs/plans/trusted-boundary.md`'s superseded banner can name this file too.** The
   region idea is now refused in both its forms — empty (havoc) and substantive (this run) —
   and the second refusal is the stronger one because it is structural rather than
   sample-dependent.
2. **§5.5 second branch: a boundary contract must be tight enough for the caller, and the
   spec does not say so** (finding 6, first bullet). `top_band_price_cents` is a worked
   example where an honest callee-side contract still fails the caller, and the fix is in
   the caller. Today a user reading §5.5 would reasonably expect "declare a contract for
   the callee" to be the whole story.
3. **§6's absence vocabulary still needs a word for a stub that made the harness
   unsolvable** (finding 6, second bullet) — now measured under havoc, a region promise, and
   a per-callee contract, all three timing out at 300s where the unstubbed proof takes 22s.
   This was already havoc's recommendation item 4(b); it is no longer a one-route
   observation.
4. **`extract_witness_bytes` still takes the first playback block.** `r5` and `r6` each emit
   two, and in `r5` the informative one (`band_count = 0`) is first while in `r6` the
   informative one is second. Unchanged from havoc's item 4(a), reconfirmed.
5. **`natural/feature/src/lib.rs::top_band_price_cents` has a real out-of-bounds index**
   against `[u32; 4]` whenever `band_count()` exceeds 4. It is a fixture, so nothing is
   broken in the product, but if that function is ever reused as an example it should carry
   the bound check.

## What was NOT RUN

* **Any run of the Ply product itself** (`cargo ply verify`). Every row drove `cargo kani`
  directly against hand-written harnesses. The claim in finding 5 that a region promise
  yields `conditional` + `owed-evidence` + `W0511` is read off §5.5's second branch and the
  structural fact that a region clause is an assumed contract; **it is not an observed
  product verdict, and no tooltip or diagnostic wording was rendered or read.**
* **A third region.** Only `ledger::fees` and `catalog` were measured, which is the whole
  basis of the clauses-per-function ratio and the stated weak point of the recommendation.
* **A homogeneous region** — six functions all returning the same quantity in the same
  units — which is the shape the region promise would most flatter. Not constructed,
  because constructing one after seeing these results is exactly the reverse-engineering
  PROMISES.md forbids.
* **Discharging a region promise by fuzzing it against the six real bodies** (finding 5's
  1:6 debt claim). Reasoned from §5.5's third honesty condition, not measured.
* **`withdraw` / `&mut` crossings.** Still no harness to write (havoc g5).
* **A per-callee clause for `band_count` tight enough to pass** (`<= 4`). The pre-registered
  clause failed and was deliberately **not** retried with a tuned number, since the
  per-callee column's job here is comparison, not maximisation. The failure is reported as
  observed.
* **macOS/aarch64.** Everything is Linux x86_64. Group A timings are contended (see the
  caveat above the results) and should not be compared across files.
