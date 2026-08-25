# Havoc spike — findings

Run 2026-08-25, Linux x86_64, `cargo-kani 0.67.0` / CBMC 6.8.0 — the pin, untouched.

This is the gate experiment `docs/plans/trusted-boundary.md` binds itself to. That plan
recommends a `given:` region whose crossings are **havoc-stubbed**: at a `bounded`
crossing the callee is replaced by an unconstrained symbolic return of its resolved type
— the empty contract — so a caller that verifies anyway has earned `conditional`
evidence by its own defensiveness, and one that fails is told which value broke it. The
plan names its own unanswered question:

> its usefulness rests on the empirical claim that real callers are defensive enough at
> boundaries to pass under an unconstrained return, measured on exactly one function and
> predicted on one more. If most crossings fail under havoc, the feature is a hint
> generator wearing a grammar construct.

**Most crossings fail under havoc.** The measurement is below.

**Nothing here implements `given:`.** Every harness is hand-written in the shape
`crates/ply-core/src/harness.rs::generate_proof_module` already emits, with one
deliberate difference: the `ply_stub_*` body is a bare `kani::any()` and no
`kani::assume`. Re-run every row with `./run.sh`. Every row below is a literal observed
run; the NOT RUN list at the end is part of the finding.

---

## The headline number

**2 of 8 crossings pass under havoc — 25%.**

**Both passes are `vetting/004`'s own functions**, written by someone deliberately
designing inside Ply's fragment. **Zero of the six callers written without this
experiment in mind passed.** Six out of six failed: five with a counterexample, one by
timing out at the 300s floor with nothing to show for it.

The plan's own disjunction resolves against it. On this sample a `given:` region is a
hint generator: it almost never produces the `conditional bounded(k)` verdict that is
its entire reason to exist, and what it produces instead is a list of contracts the user
must write anyway — which is the per-callee route the region was meant to relieve.

## Sample, and how it was kept honest

Sample selection is this experiment's whole methodological risk, because 004's `feature/`
crate and `tests/fixtures/boundarycontract` were both written by people thinking about
verification. Measuring only those gives a flattering answer.

**Group A — 004's real crossings** (`given004/`, a transcription of
`vetting/004-legacy-extension`'s two crates; see the header of `given004/feature/src/lib.rs`
for the two changes made and why). Three functions in `feature/` reach `legacy/`:
`tier_fee_cents` directly, `approve_withdrawal` transitively through it, and `withdraw`
through `&mut ledger::Ledger`.

**Group B — six naturally-written callers** (`natural/feature/src/lib.rs`). A small
`billing` feature beside a `catalog` module, covering the shapes the brief named:
arithmetic trusting a returned rate; a quantity times a returned price; an index derived
from a returned length; a division by a returned divisor; an accumulation over a returned
count; a subtraction against a returned limit.

How bias was resisted, and where it could still have crept in:

* Each function was written from a one-line task description, in 004's contract idiom,
  **before any Kani run**. `natural/feature/src/lib.rs` hashes to
  `c0cf3136d6dcc095de2eb53d3417e31afad6c42764d104e83bf14fb2d25bbf4a` and
  `natural/legacy/src/lib.rs` to
  `b109337d248e9c6c9c79cb6ba733fb2d6594f6c23826065970e3e26d99b95bac`; those hashes were
  recorded before the first run and the committed files still match them. No body was
  edited after seeing a result. Every mutation and every fix runs on a scratch copy.
* The six carry a `#[test]` (`natural/feature/src/lib.rs::tests`) asserting what each
  returns on real data, so they are working code, not sketches.
* **Where bias could still live, stated rather than denied.** (1) The `ensures` ceilings
  are mine. A looser ceiling passes more often; every ceiling here is the module's own
  stated domain bound, the same idiom as 004's `MAX_MOVEMENT_CENTS`, but a different
  author would pick differently. (2) Six functions by one author on one afternoon is a
  sample, not a population. (3) The `catalog` callee bodies are `match` arms and
  constants rather than `BTreeMap`-behind-`OnceLock`, **on purpose**: every Group B row is
  also run with no stub at all, and that baseline has to terminate. 004's `ledger`
  carries the expensive-callee case and is measured separately (row g2).
* **The baseline row is what makes a havoc row mean anything.** A caller that fails with
  the real callee tells you nothing about havoc. One row (n1) failed its own baseline on a
  genuine `u32` overflow — 004's s1 finding, verbatim — so it is reported twice: once as
  found, and once (n1f) with 004's own one-line widening applied, which is the only row in
  this file where the source under test differs from what was pre-registered.

---

## Results

`--harness-timeout 300s` on every row (§6's stubbed floor). "Verification Time" is
CBMC's own; "wall" is the whole `cargo kani` invocation including compilation.
`TIMEOUT@300s` is CBMC's own `CBMC timed out` message — Kani prints `VERIFICATION:-
FAILED` for it exactly as it does for a real violation, so the two are told apart here by
reading past that line.

### Group A — 004's real crossings

| row | what it is | verdict under havoc | Verification Time | wall |
|---|---|---|---|---|
| **g1** | `tier_fee_cents` — the flagship crossing, `ledger::fees::bps_for_tier -> u32` havoc-stubbed | **SUCCESSFUL** | 133.74s | 134s |
| g1′ | the identical invocation, run once more | SUCCESSFUL | 146.41s | 147s |
| g2 | same claim, **no stub**: Kani descends into the real `BTreeMap`-behind-`OnceLock` lookup | TIMEOUT@300s | — | 340s |
| g3 | same stub with the declared contract's one `kani::assume` put back (§5.5's second branch) | SUCCESSFUL | 148.62s | 150s |
| **g4** | `approve_withdrawal` — the **transitive** crossing; it never names `ledger` | **SUCCESSFUL** | 212.63s | 215s |
| g5 | `withdraw` — `&mut ledger::Ledger` | **no harness exists**: `error[E0277]: the trait bound ledger::Ledger: kani::Arbitrary is not satisfied` | — | 0s |

### Group B — the six naturally-written callers

| row | what it does | baseline (no stub) | **havoc** | first witness's invented return |
|---|---|---|---|---|
| n1 | `gross_cents` — net plus VAT at a returned rate | **FAILED** 54.31s (`multiply with overflow`, real callee) | FAILED 48.63s | `vat_bps = 2_020_030_208` |
| n1f | ″, with the overflow its own baseline reported fixed | SUCCESSFUL 34.66s | **FAILED** 43.15s | `vat_bps = 2_725_117_952` |
| n2 | `line_total_cents` — units × a returned price | SUCCESSFUL 17.76s | **FAILED** 42.05s | `unit_price_cents = 2_813_465` |
| n3 | `top_band_price_cents` — index from a returned length | SUCCESSFUL 27.97s | **FAILED** 46.51s | `band_count = 0` |
| n4 | `batches_needed` — ceiling-divide by a returned divisor | SUCCESSFUL 38.05s | **FAILED** 67.25s | `batch_size = 4_294_574_072` |
| n5 | `manifest_weight_grams` — accumulate over a returned count | SUCCESSFUL 22.18s | **TIMEOUT@300s** | *none — no witness at all* |
| n6 | `remaining_limit_cents` — subtract against a returned limit | SUCCESSFUL 17.01s | **FAILED** 27.60s | `spend_limit_cents = 183_558_144` |

### Mutation and sensitivity rows

| row | what it mutates | expected | observed |
|---|---|---|---|
| **m1** | g1's caller: delete `tier_fee_cents`'s `.min(10_000)` | must flip to FAILED, or g1's pass never depended on the stub | **FAILED** 133.24s, on `fee_cents`'s own precondition `bps <= 10_000`; witness `amount = 67_108_863, tier = 255, stubbed bps = 2_640_338_943` |
| **m3** | the same deleted clamp, under g4's transitive crossing | must flip to FAILED | **FAILED** 181.98s, same precondition, witness `amount = 92_315_442, balance = 1, tier = 255, stubbed bps = 4_086_202_368` |
| m2 | tighten n6's `ensures` ceiling to the real table's `500_000` | — | FAILED 25.47s, witness `spend_limit_cents = 176_218_112`. **Written as a mutation-on-a-pass and it is not one**: n6 does not pass under havoc, so the premise was void. Reported as run. |

Counted as crossings for the headline: g1, g4, n1f, n2, n3, n4, n5, n6. g2 and g3 are
cost baselines for g1; n1 is superseded by n1f; g5 has no harness to run; m1/m2/m3 are
mutations.

---

## What the numbers say

### 1. The prediction on record **held** — and it is the exception, not the rule

`docs/plans/trusted-boundary.md` §7 predicted `tier_fee_cents` would pass under havoc at
the 300s floor because of its own `.min(10_000)` clamp. **It does** (g1), and the
transitive caller above it does too (g4). The mechanism is exactly the one predicted: the
clamp makes the caller correct for *any* value the callee returns, so the proof never
needed the callee's contract. `tests/spike/kani-pin/FINDINGS.md` row 3f found the same
thing from the other direction — delete `boundarycontract`'s `kani::assume` and its clean
proof still verifies.

The mutation rows are what make that a measurement rather than a coincidence. Delete the
clamp and the identical harness fails (m1, m3). The stub is applied and it is
load-bearing.

But the prediction held on **the two functions the plan already knew about**, and on
nothing else. The plan wrote the prediction as evidence that "real callers are defensive
at boundaries." What this run shows is narrower and less useful: *the caller Ply's own
vetting scenario was built around is defensive at its boundary.* Generalising from it was
the plan's own stated risk, and the generalisation does not survive.

### 2. Why the naturally-written callers fail, and why that is not a fixable sample

They fail on **arithmetic safety, not on their postconditions**. Of the six, four fail on
a panic check Rust itself inserts — multiply/add/subtract overflow, divide-by-zero, index
out of bounds — before any declared `ensures` is reached. That is the pattern:

> An ordinary caller is defensive about *meaning* and trusting about *magnitude*. It
> checks whether the number is sensible for the business (n6 guards its subtraction, n4
> ceiling-divides correctly for every plausible batch size); it does not guard against
> the returned value being 2.7 billion, because in the real system it never is.

Havoc removes exactly the magnitude assumption, and Rust's own overflow checks turn it
into a failing proof. A `.min(clamp)` like `tier_fee_cents`'s is the one idiom that
survives, and it is rare in ordinary code because it is redundant against the real callee
— 004's own comment calls it "the ordinary defensive clamp any caller of a table lookup
writes," and the six functions here suggest that is more optimistic than true.

This is not a sample that better selection rescues. Picking more clamping callers to raise
the number would be measuring the clamp, not the boundary.

### 3. A third outcome the plan's taxonomy does not have: **havoc that never returns**

`manifest_weight_grams` loops `0..catalog::manifest_lines()`. With the real callee that
bound is 24 and the proof verifies in 22s. Havoc-stubbed, the bound becomes a symbolic
`usize` and CBMC unwinds the loop **2,387 times** before hitting the 300s cap and printing
`CBMC timed out`. No verdict, no witness, no diagnostic — and it burns the entire stubbed
floor to produce nothing.

§4's precedence table has three outcomes (Given/pass, Given/fail-with-value,
refusal-stands). This is a fourth, it is not rare (any loop or collection sized by a
returned count reaches it), and it is strictly worse than the `W0512` refusal it would
replace: today that crossing costs 0.015s and prints an actionable message; under a
`given:` region it costs 300s and prints a timeout. **A construct that converts a fast
refusal into a slow timeout is a regression at that crossing**, and the plan has no row
for it.

### 4. Cost: havoc lands inside the floor, and is indistinguishable from a contract stub

| | Verification Time |
|---|---|
| `tier_fee_cents`, **havoc** (g1) | 133.74s / 146.41s (two runs) |
| `tier_fee_cents`, **declared contract** (g3) | 148.62s |
| `tier_fee_cents`, no stub at all (g2) | TIMEOUT@300s |
| `approve_withdrawal`, havoc (g4) | 212.63s |
| `boundarycontract`, declared contract (`tests/spike/kani-pin`, 3a) | 94.61s |

Every passing havoc row lands inside §6's 300s stubbed floor, and the one function
measured both ways costs **the same** stubbed by the empty contract as by the declared
one — within CBMC's own run-to-run variance. Removing the `kani::assume` neither helps nor
hurts the solver here. So **cost is not an argument against the feature**; on the pass
side the plan's cost estimate holds.

The failing rows are cheaper still (27–67s), because a counterexample is found early. The
exception is the one that matters: n5's timeout consumes the whole floor.

### 5. The fail-side claim — "precisely the contract the proof needs" — is **overstated**

The plan's compensating story is that a havoc failure is a *useful* absence: the
diagnostic names the callee and the breaking return value, "which is precisely the
contract the proof needs, with a witness for why." Measured against the six witnesses:

* **The direction is always right.** Every witness names the callee and shows which way
  the missing bound runs. That much delivers.
* **The threshold is never right.** CBMC prints an arbitrary satisfying assignment, not a
  tight or minimal one. n2's `2_813_465` when the contract needed is `<= 100_000`; n6's
  `183_558_144` when the ceiling is `100_000_000`; n1f's `2_725_117_952`. A user reading
  "the callee returned 2,813,465" learns *that a bound is needed*, never *which bound*.
  Writing the contract still requires knowing the real system.
* **One witness is genuinely excellent**, and it is the structural one: n3's
  `band_count = 0`. Zero is the natural boundary, and "`band_count()` must be at least 1"
  is exactly the contract to write. The pattern is that structural preconditions
  (non-zero, non-empty, in-range) get sharp witnesses; magnitude bounds get arbitrary ones.
* **Ply would report the least useful witness where several exist.** Kani emits one
  playback block per failing check — n4 emits **three**: `batch_size = 4_294_574_072`
  (add overflow), then `0` (subtract overflow), then `0` (divide by zero). The two `0`s
  are the actionable ones. `extract_witness_bytes`
  (`crates/ply-core/src/engines/kani.rs`) takes `combined.find(marker)` — **the first
  block only** — so Ply would print the near-`u32::MAX` extremum and silently drop both
  zeros. That is a real defect in the fail-side story as currently designed, and it is
  independent of whether `given:` is ever built: §5.5's existing declared-contract failures
  have the same shape.
* **And n5 produces no witness at all.** The fail-side story presumes a counterexample
  exists to report. Sometimes there is only a timeout.

So the honest summary of the fail side: *a hint generator that names the callee and the
direction of the missing bound, usually with an unusable number, occasionally with a
sharp one, and sometimes with nothing.* That is worth something — it is more than
`W0512`'s "declare a contract or drop to fuzz" says — but it is not "precisely the
contract the proof needs," and the plan should not be adopted on that sentence.

### 6. What survived unscathed

* **Cross-crate stubbing works at the pin.** `#[kani::stub(ledger::fees::bps_for_tier, …)]`
  from the `withdrawal` crate against the `ledger` crate compiles and applies. The plan
  assumed this; it is now observed.
* **The transitive crossing works** (g4). `approve_withdrawal` never mentions `ledger`,
  and the region-shaped stub still reaches it. That is the plan's §2 argument for a named
  region over the per-callee fallback — "call sites nobody anticipated are covered" — and
  it is the one advantage of the region form this run confirms.
* **The `&mut` refusal row is real, and it is a compile error, not a judgement call**
  (g5). `ledger::Ledger` is not `kani::Arbitrary`; there is no harness to write. §4's
  "refusal stands" row needs no defending.
* **Nothing goes green that should not.** Every failing crossing fails loudly. The plan's
  structural claim — a `given:` region can never make a non-defensive caller pass — holds
  in every row here. The construct is honest. It is just not useful very often.

---

## Recommendation on the gate: **do not proceed with the region. Take the fallback.**

The plan asked to be judged on one empirical claim and offered its own verdict in advance
if the claim failed: *"if most crossings fail under havoc, this is a hint generator wearing
a grammar construct."* Most crossings fail. On the sample that was not written with this
feature in mind, **all** of them do.

Concretely, against the plan's own open question 6:

1. **Do not build `given:` as a grammar construct.** 3.5–5 sessions buys a `ply.yaml` key,
   a schema field, an amber fill with a §7.1 channel restatement, a `ply.lock` inventory
   fingerprint, `audit` counting, and a §7.2 taxonomy slot — all in service of a verdict
   that arrived in 2 of 8 measured crossings, both of them functions the project already
   knew were defensive. The picture, the lock file and the audit counter are machinery for
   a state most crossings never reach.
2. **Adopt open question 6's fallback instead, and price it as the small thing it is.**
   Let an explicit clause-free boundary entry mean havoc, per callee, no new grammar. The
   codegen already emits the unconstrained stub (`StubSpec::render` with empty clause
   lists); only `crates/ply-cli/src/verify.rs`'s `if claim.requires.is_empty() &&
   claim.ensures.is_empty() { continue; }` stands in the way. That is roughly a
   line-and-a-test, and it delivers everything this run showed havoc actually doing:
   the occasional earned `conditional` where a caller really is clamped, and a
   named-callee hint where it is not — **per callee, chosen deliberately, at a crossing
   the user has already looked at.** No taxonomy slot, no fill-channel restatement, no
   inventory staleness, no new refusal rows.
3. **What the fallback loses, named honestly.** The transitive coverage g4 demonstrates:
   with a per-callee entry the user must name `fees::bps_for_tier`, and a *new* call into
   `ledger` next month raises a fresh `W0512` rather than being covered silently. On this
   evidence that is a feature, not a loss — silent coverage of an unanticipated crossing
   is precisely how a 25% pass rate becomes a wall of timeouts and arbitrary-extremum
   hints nobody asked for.
4. **Two defects to fix regardless of the gate**, both independent of `given:`:
   * `extract_witness_bytes` takes the first playback block; Kani emits one per failing
     check and the first is routinely the least informative (n4). Prefer the block for the
     check the user is most likely to act on, or report all of them.
   * A stubbed crossing can time out with no witness (n5, g2). §6's absence vocabulary
     needs to say what a `bounded` claim reports when the stub itself made the harness
     unsolvable — today that reads as `timeout`, which blames the caller for the boundary.
5. **The §7.2 taxonomy hole is real and is not addressed by this refusal.** *Our code,
   checkable in principle, deliberately not checked* still renders as a dashed hollow box
   that reads "unfinished." That was the plan's second motivation and it stands on its own;
   it deserves its own smaller proposal, one that does not have to carry a verification
   semantics to earn a drawing.

**Weakest part of this recommendation, stated so it can be argued with:** six functions,
one author, one sitting. A team that writes clamping callers by habit — as 004's author
did — would measure a much higher pass rate, and for them the region would pay. The
counter is that a codebase whose callers all clamp does not need the region either: those
callers pass under the per-callee fallback just as well, one entry at a time.

---

## What was NOT RUN

* **Any run of the Ply product itself** (`cargo ply verify`). Every row here drove
  `cargo kani` directly against hand-written harnesses. The claim about
  `extract_witness_bytes` picking the first playback block is read off
  `crates/ply-core/src/engines/kani.rs` and the observed multi-block output; it is not an
  end-to-end product run.
* **`vetting/004-legacy-extension/run.sh` with given-region stages.** The plan's §7 gate
  asks for an extension of 004's own `run.sh`, a render, a squint test and a cold reading
  of the tooltips. None of that was done: this spike answers the empirical question the
  vetting re-run was gated on, and on this result the re-run has nothing to gate.
* **The fill-channel restatement, the amber wash, the tooltip wording** (plan §6, open
  question 1). Not drawn, not read, not squint-tested.
* **`&mut` havoc** — havocking the referent as well as the return (open question 5).
  Refused by the plan; not attempted here.
* **A larger or second-author sample.** The generalisation limit above is the honest
  bound on this file.
* **`fuzz`/`test` tiers across a given region.** Unchanged by the proposal, unmeasured here.
* **macOS/aarch64.** Everything is Linux x86_64; timings compare only within this file
  and with `tests/spike/kani-pin/FINDINGS.md`, taken on the same machine.
