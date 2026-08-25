# Strategic review after vetting 004 — 2026-08-24

Scope: what vetting 004 (`vetting/004-legacy-extension.md`, first fragment-first scenario,
first executed against the real `cargo ply verify`) means for the sharpened thesis — *Ply
is for new and modified code, written inside §5.4b's fragment, beside code that is not.*
Reviewed against The-Ply-Spec.md (§1, §5.1a, §5.4a/b/c, D5, D6, §6, §8, §9, §10),
docs/m4-findings.md, docs/review-m4-2026-08-24.md, docs/m4-review-closure.md,
docs/m3-slice-findings.md, tests/spike/scale/SCALE-FINDINGS.md, docs/adr/0003, TODO.md.
Method: full read of 004's write-up, crates and `run.sh`; the load-bearing findings
checked against the product source and, where cheap, re-run live. Judgment work only —
nothing in `crates/`, `tools/`, the spec, or the vetting files was changed.

**Bottom line first**: the thesis survives **with conditions**, and 004's findings are
neither overstated nor alarmist — one is actually *under*stated (see "worse than
reported" below). But the report's own severity ordering needs one correction: the
nondeterministic fuzz tier (finding 4) is the most *visible* defect; the most *important*
fix is finding 1's class — absence of evidence reading as success — because it is a
missing spec principle, not an implementation slip, and it is the one failure mode that
makes every other investment worthless in CI. And the most important *decision* is D5's
third branch, because it is the linchpin that either defuses or confirms the worrying
chain in question 2.

---

## What was verified, what was trusted, what was wrong

**Verified first-hand (code read + live re-run):**

- **Finding 4 (fuzz nondeterminism) — confirmed, mechanism and behaviour.** The generated
  harness builds `proptest::test_runner::TestRunner::new(Config { cases, ..Config::default() })`
  (`crates/ply-core/src/fuzz_gen.rs`) — seed from entropy; no field anywhere in the §8
  envelope, the diagnostics, or `ply.lock` records it. Re-ran `run.sh s8` in this session:
  six fresh runs of the identical unfixed source split **4× `fuzzed(256)` / 2×
  `tool_error`** (the original run split 3/3). Two consecutive `verify` runs on the *same*
  scratch tree flipped pass→fail. The run that finds the panic cannot be replayed; the run
  that misses it is indistinguishable from a real pass.
- **The silent `ensures:` drop — confirmed, and it is two defects.** `FnClaim` in
  `crates/ply-core/src/config.rs` has only `checks` and `examples`; there is no
  `deny_unknown_fields` anywhere in `crates/`, so serde eats `ensures:` (and any typo)
  silently — while `ply-check` on the same file enforces §5.1a rule 1. Separately,
  `anchor:` is parsed and never consumed (`.anchor` appears only in a config unit test):
  `verify_crate` resolves every component's fns against `<crate_dir>/src/lib.rs`, which is
  why s5's legacy claim died with E0301 in the *feature* crate. Both halves of 004's
  finding 7 are accurate.
- **Findings 2+3 — confirmed.** `stub_verified`, `conditional`, `W0511` appear nowhere in
  `crates/` (grep); `tools/schedule` and `tools/kernel` are separate-workspace tools linked
  into nothing the product runs. The boundary behaviour is therefore whatever Kani does
  with the inlined `BTreeMap`/`OnceLock` body. The stage logs from the original run
  survive in `/tmp/ply-004/` and match the write-up verbatim: `tier_fee_cents` `timeout`
  at 120s and at 600s (11m23s wall, exit 0), the s4 control `bounded(2)` in 1m20s, and
  `total_debit_cents` — no legacy code at all — timing out at 120s in the run where
  `fee_cents` passed. I did not re-run the 7–11-minute Kani stages; the logs are
  contemporaneous, complete, and internally consistent.
- **Finding 5 (the implemented fragment) — confirmed, and 004 under-claims it.**
  `rust_type_from_syn` has no `Type::Array` arm and no alias resolution;
  `is_bounded_supported` is exactly the eight integer types + `bool` + `Vec<u8>`. But
  `char`, `Option<T>`, and `Result<T,E>` — the shapes §5.4b lists as "cheap
  unconditionally (~0.1s)", measured so by the scale spike — are *also* missing from the
  implementation. The implemented fragment is narrower than even 004 says.
- **Finding 1 (exit 0) — confirmed.** `exit_code_for` returns non-zero only for an
  `error`-severity diagnostic; `K0601` is constructed with `severity: "warning"`
  (`crates/ply-cli/src/verify.rs`). `--fail-on`, `--only-changed`, and `cargo ply check`
  are absent from the clap surface (s6's literal output, re-checked against the code). The
  s3 log — one claim, `timeout`, nothing checked — ends `verify exit: 0`.
- **Finding 6 — confirmed.** `crates/ply-attrs` exports `requires` and `ensures` only;
  `verify.rs:617` emits the "add a `pure`-marked generator hook" fix for a mechanism that
  does not exist anywhere.
- **Finding 10 — confirmed.** `ensure_workspace_member` bails on any `Cargo.toml` without
  a `[workspace]` table, and `verify` writes generated modules into the target's `src/` —
  already in TODO.md's deferred list from the M4 closure; 004 is a second, live vote.
- **Finding 8 — confirmed at the artifact level.** The committed SVG carries four
  identical `B2` badges; nothing in it distinguishes `fee_cents` (earned `bounded(2)`)
  from `tier_fee_cents` (never finished) or `withdraw` (`unsupported`).
- `schema/ply.schema.json` does not exist; §5/D3 call it normative.

**Taken on trust:** the wall-clock numbers (backed by `time` output in the surviving
logs); the CairoSVG raster-and-look check; "600s is the largest budget tried"; the exact
Kani/cargo-mutants versions; "roughly a third of v1 by milestone weight" (consistent with
§10 — M0–M4 landed as thin slices, M5–M7 not started — though by §6 command surface it is
one command of ten, and by soundness machinery less: the two components ADR-0003 calls
load-bearing, the scheduler and the kernel, are wired into nothing).

**Found wrong or miscalibrated in 004 — three items, none load-bearing:**

1. Finding 1's headline, "a run in which nothing was checked exits 0", literally
   describes s3, not s2: in s2 two of five claims *did* earn evidence. The finding's body
   states this correctly; the headline rounds up.
2. Finding 5 understates the gap (`char`/`Option`/`Result` also missing, above).
3. Finding 4 is **worse than reported**, in a way that changes what "fix the seed" buys.
   In the failing half of the coin flip, the real bug — an overflow panic inside the
   declared precondition — is reported as verdict `tool_error` with an `X0901` reading
   "proptest reported a failing case … but Ply could not find the line its own generated
   harness prints … The commonest cause is `approve_withdrawal` itself panicking"
   (captured live this session). That text is honest and even names the right cause — but
   the *classification* means a body-panic bug can **never** earn `violation` under the
   current fuzz adapter at any seed: the call sits outside the harness's `catch_unwind`,
   the panic escapes before the witness marker prints, and the M4-review D6 fix (rightly)
   forbids witness-free violations. So the boundary-crossing tier's two possible answers
   for this real bug are "all green" and "Ply's harness had a problem" — never "your code
   is wrong, here is the input", even though proptest's own failure output contains the
   shrunk failing input and the adapter discards `raw_output` (the M4 review's O5 gap).
   Seeding fixes reproducibility; it does not fix this.

---

## Question 1 — does the fragment-first thesis survive 004?

004's split verdict — natural for the arithmetic core, tool-dictated for everything else
— is fair as a description of *today's tool*. But the strategic reading has to separate
three different gaps that 004's "the fragment forced this" list runs together:

- **Implementation debt** (most of it): no arrays, no aliases, no `Option`/`Result`, no
  structs of scalars, no methods, one file, single-crate. Every one of these is *inside*
  §5.4b's specified, measured fragment. The spec's fragment is a plausible SPARK-like
  subset; the implemented fragment is a calculator. This gap is closable, mostly
  mechanically, and 004 itself says so (finding 5: "mostly cheap to close").
- **A genuine spec gap** (the boundary): D5 has no rule for an unclaimed callee, so the
  thesis's defining situation — checked code calling unchecked code — is governed by no
  rule at all. Question 4.
- **A permanent v1 wall** (the shell): two-state contracts over method calls
  (`old(accounts.balance(account))`) are out of scope by declared design. `withdraw`-shaped
  functions stay outside the evidence in any version of this thesis, and the honest
  posture is what 004 did: claim it, get `unsupported`, and make the picture say so
  (finding 8).

Is that good enough to build a product on? **Not as it stands — and yes, conditionally.**
The conditions, in order of what has to be true:

1. The implemented fragment reaches the specified one (at minimum: arrays, aliases,
   `Option`/`Result`, structs of scalars). Until then "design inside the fragment" is
   advice about a fragment nobody can write against, and every future vetting scenario
   measures the implementation gap instead of the thesis.
2. The boundary gets a D5 rule, so `bounded` at a legacy call is a decision Ply makes in
   milliseconds, not an engine burn that says nothing after 11 minutes.
3. Absence of evidence fails the run by default (question 3), so the delta pitch — "CI
   green means the new code is checked" — is actually true of a green run.

What 004 *proved* for the thesis is worth stating plainly, because it is the part that
cannot be retrofitted: the fragment paid for itself on first contact (a real overflow,
concrete counterexample, one-line fix, `bounded(2)` after — §1's promised loop, working),
and every verdict on every run was defensible. Honesty held under conditions the code was
never designed for. That is the hard half of the product; the missing half is breadth,
which is ordinary engineering.

## Question 2 — the chain (bounded can't cross → fuzz is the workhorse → fuzz is a coin flip)

The chain is real end to end — every link verified. But two corrections change what it
forces:

**First, `bounded` was never going to carry the boundary — the spec already says so.**
§1's "What v1 optimises for" designates `fuzzed(n)·spec-strong` the workhorse tier and
`bounded(k)` reinforcement for the shapes it can reach. The scale spike measured why. 004
did not *push* the thesis onto fuzz; it removed the illusion that `bounded` would
gracefully degrade at a boundary instead of burning its budget in silence. And
`total_debit_cents` — pure fragment arithmetic, no legacy call — timing out in the same
run is the same lesson from §5.4c ("checkability is a property of the body") measured
again: `bounded` is fragile even inside the fragment. The thesis was always going to run
on fuzz + mutate, with bounded as a bonus where it lands.

**Second, the bounded end of the chain is breakable, and cheaply in principle.** The s4
control is the decisive measurement: the identical function proves in ~80s total once the
legacy body is out of the equation. `bounded` doesn't need the legacy code to be
*checkable*; it needs permission not to *descend into it* — an assumed contract for the
callee and a stub, which is exactly D5's missing third branch plus the `ply.yaml`
contract-merge that finding 7 shows is silently dropped today. Do that and the boundary
function earns `conditional bounded(2)` in about a minute, listing its assumption, and
the chain's first link dissolves.

So what the chain forces is **both** fixes, not a choice: make fuzz reproducible and
auditable (seed in the envelope, replay path), *and* give bounded the assumed-contract
crossing. One honesty caveat on the first: a recorded seed buys replay and auditability,
not detection power — 256 samples missing an overflow that begins at ~29% of the input
range is a sampling property (§1 already records that proptest at 256 missed boundary
mutants `bounded` kills by construction). The reliability story for the fuzz tier at the
boundary is seed + `mutate`'s kill signal + the finding-4 witness fix above, not seed
alone. A seeded coin flip is still a coin flip; it is merely an honest, replayable one.

## Question 3 — the pattern: absence of evidence reading as success

The three instances are real but they are two classes, and the distinction matters
because one class is *already against the spec* and one is *faithfully implementing it*:

- The M4 fail-open (compile-failed harness → `fuzzed(256)`) violated coded rules; it was
  found by review and fixed. An implementation slip.
- 004's exit-0-on-timeout is **not a slip**. `exit_code_for` implements §6's table
  exactly as written; the table has no row for "nothing was checked", and every absence
  Ply knows about — `K0601` timeout, `V0505` unsupported, `W0503` — is warning severity
  by design. The spec painstakingly polices what a *verdict* may claim (§5.4c's MUSTs,
  the M4-review amendments) and says nothing about what a *run* may conclude.
- The nondeterministic pass is a third, adjacent class: evidence that existed but is
  unattributable — nothing records what would reproduce it.

So this is a missing principle, and it deserves to be stated once, where the evidence
rules live, rather than patched per-code. Proposed wording, for §1 (with the operative
rule in §6's exit-code table):

> **A run succeeds only if every claim earned its declared evidence.** `timeout`,
> `unsupported`, `tool_error`, `unclaimed`, and `engine-missing` are absences of
> evidence, and absence of evidence fails the run by default — `--fail-on` exists to
> relax that, never to enable it. And every verdict, passing or failing, must name the
> evidence that produced it concretely enough to reproduce it: a fuzz verdict carries its
> seed and case count the way a violation carries its witness.

The second sentence is the D14 fingerprint philosophy extended to passes; the seed
belongs in the envelope and the fingerprint alongside engine version and flags. With the
first sentence in force, 004's s2 run exits non-zero, the M4 fail-open class becomes a
CI-visible event even if a future adapter reintroduces it, and the "green nothing" —
which the project's own §1 calls the one unforgivable failure — is structurally
impossible rather than reviewed for.

## Question 4 — D5's third branch

Recommendation: **both proposed options, as one two-part rule keyed on whether the
unclaimed callee has a declared contract.**

- **No declared contract → refuse to descend, before the engine runs.** A distinct
  outcome (`unclaimed-callee`, its own diagnostic code) naming the callee and the call
  site, delivered in milliseconds from the syn call graph (D11's extractor), not after
  600s of CBMC. The verdict for the caller's `bounded` check is an absence — and under
  question 3's principle it fails the run, with the fix text offering the two real
  options: declare a contract for the callee (below), or drop the check to `fuzz`.
- **Declared contract (in `ply.yaml`, §5.4's external-spec route) → assume it, stub the
  callee, verdict `conditional`, W0511 listing the assumption.** This is SPARK's boundary
  model exactly — contracts on the boundary of the subset, trusted and *named* — and it
  is what D5's existing "anything else" branch already does for weaker-verified callees;
  the third branch extends it to callees with no verification at all, which is honest
  because `conditional`'s whole meaning is "true if these listed assumptions are".

The honesty argument for admitting the assumption at all: the alternative that 004
measured — descend into the real body — is not more honest, it is just slower; it either
times out (evidence-free) or, worse, would *prove against a body the user never claimed*,
producing a `bounded` verdict whose meaning silently includes two years of legacy code
nobody vouched for. A declared assumption is visible, auditable (`cargo ply audit`'s
trust surface), stale-able (D14), and — the part that makes this better than trust —
**checkable by the cheap tier**: the fuzz engine has no trouble crossing the boundary, so
a declared contract on `bps_for_tier` can itself be fuzz-checked against the real legacy
body, turning the assumption into measured evidence without asking Kani to do the
impossible. That closes the laundering loop that is this option's real risk: an assumed
contract nobody ever exercises is green paint. The rule should be that an unexercised
boundary assumption is listed by `audit` and `worklist` as owed evidence.

The costs, stated honestly: it is the largest work item in this review — contract merge
(finding 7's fix, done properly against the schema), stub generation, and the
callees-first scheduling that ADR-0003 says is the *entire* soundness guarantee, meaning
`tools/schedule`'s logic finally gets promoted into the product and tested there. And it
makes `conditional` the *normal* state of the target workload — nearly every green in a
legacy-extension codebase will carry assumptions — so the visual grammar and the CLI
must render `conditional` as legible and routine rather than as an alarm, or users will
learn to ignore exactly the annotation that carries the trust story.

## Question 5 — sequencing

The coordinator's proposed order — (1) seed the fuzz run, (2) fix the `ensures:` drop,
(3) absence-of-evidence fails by default, (4) fixed arrays, (5) decide D5's third branch
— contains the right five items and inverts the dependency. Item 5 is the cheapest (a
decision and a spec amendment, no implementation) and three of the other four are shaped
by it: what the `ensures:` merge is *for*, what the boundary diagnostic must say, and
whether "pass the rate card as a fixed array" is the recommended boundary idiom or just a
nice shape. Deciding it last means building around the gap and then rebuilding.

Recommended order:

1. **Decide D5's third branch and amend the spec** (the two-part rule above, including
   "the diagnostic names the callee" and "an unexercised boundary assumption is owed
   evidence"). No code. Everything below implements toward it.
2. **State the absence-of-evidence principle (§1) and flip the exit-code default (§6)**,
   with `--fail-on` as the opt-out. One classification branch plus a small e2e; the
   largest honesty return per line of code available anywhere in this list.
3. **Seed and record the fuzz run** — seed in the §8 envelope and the D14 fingerprint, a
   replay path (`--seed` or lockfile). Fold in the finding-4 corollary if cheap: on a
   body panic, recover proptest's own shrunk failing input from the output the adapter
   currently discards, so a real bug stops presenting as a tool error. Sell this as
   reproducibility, not power — the power story is `mutate`, already built.
4. **Make the verify path strict** — reject unknown `ply.yaml` keys (E0204 parity with
   `ply-check`), which turns the silent `ensures:` drop into a loud error *now*, in one
   serde attribute. The actual contract-*merge* lands as part of implementing 1, where
   its semantics are defined; `schema/ply.schema.json` finally exists as part of the same
   reconciliation (it is normative and absent, and the M3 module doc already records the
   two-model debt).
5. **Widen the implemented fragment to the specified one**: arrays (the spec's own
   preferred shape, and the fragment-first boundary idiom), alias resolution,
   `Option`/`Result`/`char`, then structs of scalars. Mechanical, delegable per
   CLAUDE.md's delegation rule, and each shape lands with its fixture.

Then, as the next tranche rather than this one: implement D5's branch (merge + stub +
callees-first scheduling — the ADR-0003 soundness item), `--only-changed` and `cargo ply
check` (finding 9 is right that this is the delta thesis's mechanism, but scoping is
worthless until the fragment is wide enough for a delta to land inside it), and the
renderer's earned-vs-declared split (finding 8) when the envelope reaches the renderer.

**What I would not do:** raise engine budgets or tune Kani flags at the boundary (600s
vs 80s is a structural result, not a tuning problem — the scale spike already walked this
road); start M7/Verus; run a vetting 005 before conditions 1–3 from question 1 hold (it
would re-measure the same implementation gap); build multi-crate `verify` generality
beyond what D5's branch needs; implement `--fail-on` as an opt-in warning gate (that
blesses today's default); or reshape the kernel or the scenario code to suit any engine —
the standing CLAUDE.md position, which 004's evidence supports rather than strains.

## The third-of-v1 weight, under the new framing

The unbuilt two-thirds is not uniformly distributed against this thesis. Three items move
from "later milestone" to "core of the product" under the new framing: **the D5/scheduler
machinery** (the boundary is now the product, and ADR-0003 established that Ply's
scheduling is the entire soundness guarantee there — it currently exists only as an
unlinked tool); **`--only-changed` + `check`** (the delta is the pitch, and today the only
scoping mechanism is which directory you point at); and **the schema** (the strictness
story is currently split between two tools that disagree, which is how the `ensures:`
drop happened). Conversely, M6 synth and M7 Verus move further out — nothing in 004
touches them. The kernel work keeps its place: aggregation honesty is what makes
`conditional`-everywhere legible, which question 4 makes load-bearing. Net: the milestone
*order* survives, but M5's contents should be re-cut around D5-third-branch + delta
scoping rather than around tree polish.

---

## Summary answers

**Does the thesis survive?** **Yes, with conditions.** Honesty survived contact with
reality — every verdict in twelve runs was defensible, and the fragment caught a real bug
with a concrete counterexample on its first outing. Usefulness did not survive *as
implemented*: the checkable set today is a calculator's, the boundary has no rule, and
the workhorse tier is unreproducible. The conditions are: the implemented fragment
reaches the specified one; D5 gets its third branch (decided first, built next); and
absence of evidence fails by default. None of these is research; all are engineering the
spec already points at. If, with all three landed, a future scenario still finds the
fragment tool-dictated for ordinary feature work, *that* would falsify the thesis — 004
does not.

**The single most important fix**: make absence of evidence fail the run by default
(finding 1, generalized as the question-3 principle). Not because it is the most
dramatic finding, but because it is a missing principle rather than a bug — the current
exit behaviour faithfully implements a spec table with a hole in it — and because it is
the failure mode that silently devalues everything else: a tool whose green CI run can
contain zero evidence cannot be trusted about anything, including its own future fixes.
It is also the cheapest of the five items. (The most important *decision* is D5's third
branch; the most *visible* defect is the unseeded fuzz tier. All three are in the first
three steps of the sequence.)

**Overstated or wrong in 004**: nothing load-bearing. Finding 1's headline rounds s2 up
to "nothing was checked" (true literally of s3); finding 5 *under*states the fragment gap
(`char`/`Option`/`Result` are also missing); and finding 4 is more severe than written —
in the failing half of the coin flip the real bug is classified `tool_error`, so the
boundary tier can never call a body-panic bug a violation at any seed, and the fix must
include witness recovery, not just seeding. The report's literal-output discipline held:
every quoted verdict matches the surviving stage logs exactly, and its own severity
judgment ("finding 4 is the most serious") is the only call this review adjusts.

**Recommended order**: (1) decide D5's third branch, spec-first; (2) absence of evidence
fails by default; (3) seed + record fuzz, with the panic-witness recovery; (4) strict
verify-path parsing (E0204 parity — makes the `ensures:` drop loud now, merge lands with
D5); (5) widen the fragment to §5.4b as specified, arrays first. Then D5's
implementation with the scheduler promotion, then `--only-changed`/`check`, then the
renderer's earned-vs-declared split.
