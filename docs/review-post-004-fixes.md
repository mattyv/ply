# Adversarial review — the five post-004 fixes, 2026-08-25

Scope: commits `2cf09c2`, `d73558f`, `c8e231b`, `23e8f67`, `593cf9a` on
`claude/project-concept-eval-6soxfl`, reviewed against The-Ply-Spec.md (§1, §5.1a,
§5.4b/c, §5.5, §6, §7, §8, D5, D6), `docs/post-004-fixes.md`,
`docs/review-post-004-strategy.md`, `vetting/004-legacy-extension.md`, and TODO.md.
Method: full read of all five diffs and every touched module; two live adversarial
probes through the real `cargo-ply verify` binary (details inline); one spot re-run of
the `unclaimedcallee` refusal; `ply-core`/`ply-cli` unit suites, `cargo fmt --check`
and clippy re-run clean. The reviewer's own earlier recommendations are treated as
context, not as something to defend — one of them turns out to have been optimistic
(the "about a minute" estimate) and one under-specified (see G1).

**Bottom line first**: four of the five findings these fixes target (004's 1, 4, 5, 7)
are genuinely closed, red-first, with honest write-ups — and the interlock the two big
items form (a refusal verdict that the new exit default turns into a failing run) is
exactly the right design. But finding 2 is only **half** closed, and the unclosed half
is worse than what 004 measured: the boundary rule keys on how a call is *spelled*, so
an unclaimed first-party callee reached through an ordinary `use` import is silently
inlined and earns a clean `bounded(2)`, zero diagnostics, exit 0 — live-reproduced
below in 42 seconds. That is the exact defect item 1's own red test quoted as the
"before", surviving the fix behind the most idiomatic Rust spelling there is. And it is
the pattern this review was asked to hunt: `CalleeStatus::Unresolved => {}` is a
fail-open — absence of *resolution* read as license to descend. Item 2 has a matching,
smaller hole: a `mutate` check whose engine is missing exits 0 (live-reproduced),
against §6's own exit-3 row. Neither hole invalidates the work; both must be closed or
retracted in the spec before this branch's claims are true as written.

---

## Defects (wrong)

### D1 — MAJOR: an ordinary `use` import bypasses the boundary rule — silent clean `bounded(2)` over an unclaimed first-party body

**Claimed** (§5.5, new limits paragraph; repeated in `docs/post-004-fixes.md` and
TODO.md): the rule fires for callees Ply resolves, and calls into `std`/`core`/registry
crates are the stated gap — "a `bounded` verdict can still include a body Ply never
examined, **just never a first-party one**."

**Actually true**: the resolver (`crates/ply-core/src/callgraph.rs`) never reads `use`
declarations. A single-segment call path is looked up only among top-level `fn` items
in the caller's own file; not found there, it classifies `Unresolved`, and
`boundary_plan` treats `Unresolved` identically to `Contracted`: descend. So the
moment a caller writes `use rates::legacy_rate;` (or `use ledger::fees::bps_for_tier;`)
and calls the bare name — the way most Rust is written — the unclaimed first-party
body is silently inlined into the proof. The same blind spot covers re-exports and any
path-dependency fn living in a file-based `mod foo;` rather than an inline `mod` in
`src/lib.rs`.

**How checked**: live probe. Copied `tests/fixtures/unclaimedcallee`, moved
`legacy_rate` into an inline module, and imported it with `use rates::legacy_rate;` —
same unclaimed callee, same contracted caller, different spelling. Real run
(`--engine-timeout 120 --json`):

```
"root": { "verdict": "bounded(2)", ... "tiered_fee": "bounded(2)" }, "diagnostics": []
real 0m42.343s        exit: 0
```

The pristine fixture, for contrast, refuses in 0.029s with `W0512`, verdict
`unclaimed`, exit 1 — re-verified in the same session. One `use` line converts the
loud refusal into the exact envelope item 1's red test quoted as the defect: "Kani
inlined the unclaimed body, the caller banked the result, and the envelope carried
**zero** diagnostics."

**Why this is the pattern, again**: 004's boundary at least failed *visibly* (11
minutes of timeout looks wrong). This fails invisibly — a clean verdict that looks
right. Absence of evidence reading as success has now appeared a fourth time, this
time inside the very rule built to close its third appearance: the resolver's "I
cannot see this callee" is silently treated as "this callee is fine to descend into".
The honest default for an unresolvable *single-segment* name in the caller's own file
is refusal or at minimum a diagnostic, because unlike `x.min(..)` it is not
overwhelmingly `std`.

**Cost to fix**: small and contained. syn exposes `ItemUse`; walking the caller's
file's `use` declarations (including renames) and mapping imported names to their full
paths before classification closes the import half; glob imports and file-based dep
modules can conservatively classify `Unclaimed`-with-diagnostic rather than
`Unresolved`. Until that lands, §5.5's "never a first-party one" sentence and the
matching claims in `docs/post-004-fixes.md`/TODO.md are overclaims and should be
retracted to "never a first-party one *called by qualified path*". Not fixed here: the
resolver change deserves its own red fixture (this probe is the fixture), and the
conservative-refusal scope is a design call.

### D2 — item 2's fail-by-default has a hole: a missing engine for `mutate` exits 0

**Claimed** (§1, §6, `docs/post-004-fixes.md` item 2): `timeout`, `unsupported`,
`tool_error`, `unclaimed` and `engine-missing` are absences and fail the run by
default; §6's row "3 — missing engine for an explicitly requested check" is "now
actually returned".

**Actually true**: the enumeration is complete over node **verdict strings** — every
verdict the tool can emit was traced (`proved`/`bounded`/`fuzzed`/`tested`/`violation`
pass or fail correctly; the five absence strings all fail; violations fail via
error-severity diagnostics on every path including witness-only `W0541`). But two
absences are encoded as a *status/outcome* instead of a verdict, and
`exit_code_for` never looks at statuses:

- `mutate` with cargo-mutants missing → `MutateOutcome::Inconclusive` → status
  `inconclusive`, `W0110` warning, verdict stays `fuzzed(n)` → **exit 0**.
- The same `Inconclusive` covers a mutate run killed by the wall-clock cap or with
  unreadable output — a declared check that established nothing, exit 0.

Meanwhile the `prove` path pushes an `engine-missing` *label*, so `checks: [prove]`
exits 3. Two missing-engine paths, opposite exit behaviour, and the `mutate` one
contradicts both §6's exit-3 row and §1's "every claim earned its declared evidence".

**How checked**: live probe. `tests/fixtures/weakspec` (`checks: [fuzz(64), mutate]`)
run with a shim `cargo` that fails only `cargo mutants --version`:

```
verdict: fuzzed(64)  statuses: ['inconclusive']  diags: [('W0110','warning')]  exit: 0
```

**Cost to fix**: small — either `Inconclusive` contributes an `engine-missing` /
absence label to the fn's verdict combination, or `exit_code_for` also treats the
`inconclusive` status as an absence; plus one masked-engine e2e (the shim above is
three lines). Requires one design call first: whether an inconclusive-but-not-missing
mutate should fail the run (§1 says yes; D9's "a missing engine downgrades the check,
never fails the run" now reads as contradicting §6's exit-3 row and should be
reconciled in the same pass).

### D3 — minor, FIXED HERE: `W0510`'s user-facing message carried runs of literal spaces

The title string in `declared_contract_not_anded_diag` was written as one line with
~14-space gaps mid-sentence ("…is used where §5.5              needs it…") — a
mis-joined string literal. User-facing wording, pinned by no test, so it slipped the
"words are reviewed like code" rule. Fixed in this commit (whitespace only, proper
`\` continuations); `fmt`/clippy/unit suites re-run clean. Called out per the brief.

### D4 — minor: `W0541`'s wording is false for the new shapes it now fires on

The fuzz witness-only path now also catches `char`/`Option`/`Result`/`[T; N]`
violations (their `WitnessValue` decoders don't exist), but the diagnostic still says
"it has no way yet to spell a `BTreeSet`, or a `Vec` of anything but `u8`, as a
literal value". This is the exact defect class the M4 review's D7 fixed ("W0541's
wording was false for the exact shape that triggers it"), reintroduced for the widened
fragment. The Kani side got a correct new `X0901` naming the parameter and its type;
the fuzz side reuses the stale words. Exact-string pinned only for the `BTreeSet` case
(`btreesetbug_fixture`), where it is true. Cost: shape-aware wording plus a pinning
test; not fixed here because the wording is exact-string territory and deserves its
own red test.

### D5 — minor: the `evidence` field claims a run that never happened

`run_fn_checks` attaches `evidence: { engine, seed, cases: n }` whenever `fuzz(n)` was
*declared*, not when it ran: a fuzz check refused as `unsupported`, aborted by
proptest's global-reject limit (verdict `unclaimed`), timed out, or dead in a harness
compile failure (`tool_error`) still carries `cases: n`. The field's own doc comment
says "cases the engine was asked for and reached" — "reached" is unverified on every
one of those paths. §1's new sentence is about naming the evidence that produced a
verdict; these nodes have none. Cost: attach evidence only on a label the harness
actually earned, or record accepted-case counts (the abort path already parses them).

### D6 — minor: `owed-evidence` is a status the spec never defined

The implementation and §10's M5 bullet call `owed-evidence` a status propagating
upward; §5.5 calls it an "open item"; D6's status list and §0's glossary — the two
normative homes for statuses — name neither. The envelope now carries a status that no
spec sentence defines, which is the CLAUDE.md failure mode ("when behaviour changes,
the artifact describing it changes in the same commit") in miniature. Cost: one D6/§0
amendment plus a decision (status vs open-item count — §7's tree spec says statuses
become corner markers, so the choice has a rendering consequence).

---

## Overstatements

### O1 — §5.5's stated limits understate the real narrowing (disclosure 4, under-disclosed)

The spec says the gap is `std`/`core`/registry crates. D1 shows the gap also contains
first-party code: `use`-imported callees, re-exports, and any path-dependency fn not
in `src/lib.rs`'s inline-module tree. The disclosed limit ("never a first-party one")
is the load-bearing half of the honesty claim, and it is false. The agent's disclosure
4 asked the right question and materially under-answered it.

### O2 — "Nothing was lost" (the seeding write-up) is not quite true

`docs/post-004-fixes.md` on s8: "Nothing was lost — that half was never reliable —
but nothing was gained in *detection* either." Per-run, correct. Across runs it is
not: entropy gave each CI run an independent ~50% draw at this bug, so ten runs of
history had a ~99.9% chance of surfacing it once; the derived seed fixes that
probability at zero, forever, for every fn whose derived seed happens to miss. The
honest sentence is "per-run power unchanged; cross-run accumulation deliberately
traded for determinism". See the seeding judgment below for whether the trade is
right (I judge yes, with a condition).

### O3 — §5.5 speaks of `audit`/`worklist` in the present tense

"…the caller's node carries the `owed-evidence` open item, and `audit` and `worklist`
list it as owed." Neither command exists (§10's M5 bullet and TODO.md say so
honestly). A spec sentence describing unbuilt behaviour as current is the pattern the
2026-08-23 review's D13 generalisation exists to prevent. One-line fix: mark the
clause "(M5)".

### O4 — the test-count arithmetic doesn't add up

The gates section claims "102 passed … 70 `ply-core` unit, 11 `ply-cli` unit, 21
e2e". The tree holds **22** e2e `#[test]` functions (none ignored), and 70 + 11 + 22
= 103. One of the numbers is wrong; the coordinator's in-flight full run will settle
which. Trivial, but a totals claim in a gates section should survive `grep -c`.

### O5 — "s1/s2 … behaviour is unchanged except where s3's rule applies" is not true

Item 2 changes s1's and s2's **exit codes** regardless of the boundary rule: both
runs contain `withdraw` (`unsupported`) and, in s2, two `timeout`s — absences that now
exit 1 where the vetting doc records exit 0. The vetting doc's new follow-up note
("the outputs here are the ones measured on 2026-08-24") covers this honestly at the
document level, but the NOT RUN justification in `docs/post-004-fixes.md` claims a
behavioural invariance that does not hold.

---

## Gaps (open, judged)

### G1 — the `conditional` path does not work at the tool's own defaults (the serious half of disclosure 1)

Raising `run.sh s5`'s stage budget from 120s to 600s was **legitimate**: the structure
changed (stub instead of descent), the ~202s cost is measured and explained (a
symbolic return constrained only by `<= 10_000` is weaker than four concrete values,
and costs more to reason about), the original 120s run is preserved, and the change is
annotated in place. This is not the "bigger number, same structure" move the strategy
review warned against — that warning was about budget as a substitute for a rule, and
the rule now exists. My "about a minute" estimate was simply wrong, and the write-up
was right to correct it against measurement.

What the disclosure does *not* say: `default_engine_timeout_secs` returns **60s** for
a scalar-signature fn, and `tier_fee_cents` is scalar-signature. So D5's second branch
— the headline capability of this whole tranche — **times out at the tool's own
default budget** on its own flagship case (202s needed vs 60s given), and the same is
true of item 5's `arraycard` (~139s of body vs 60s). Every e2e passes
`--engine-timeout` explicitly, so no test observes this. §6's shape-aware default
knows about `Vec` and nothing about stubs or body cost; it was not amended. Until it
is, a user who declares a boundary contract and runs plain `cargo ply verify` gets
`timeout`, and the diagnostic that should have carried the assumption never appears.
Cost: a default bump for stubbed harnesses (the one measured data point is 202s), or
at minimum a `K0601` fix-suggestion that names the stub cost; either way §6 needs the
sentence.

### G2 — the assumed-contract enforcement loop is entirely IOU (disclosure 3, judged)

The question was whether `owed-evidence` recorded nowhere a user looks makes the
honesty claim hollow. Judgment: not hollow, but **thin** — the assumption is visible
in every run (root-level `conditional` status, `W0511` with the contract text
verbatim, the §8 `assumptions` array), which is more than "nowhere". What makes it
thin is that visibility is the *only* mechanism, and three compounding gaps sit behind
it, none individually secret but nowhere stated together:

1. **No vacuity check.** A declared `ensures: ["|result| false"]` — or any
   unsatisfiable or simply wrong contract — makes the stub's `kani::assume`
   unsatisfiable and the caller's proof vacuous: clean `conditional bounded(2)` from a
   contract that describes nothing. A `kani::cover!` after the stubbed call would
   detect this cheaply and is not emitted. (NOT RUN as a live probe; read from the
   generated-stub code path.)
2. **No staleness.** D14 fingerprints trusted claims; nothing fingerprints a declared
   boundary contract against the callee's body, so the legacy fn can change under a
   standing assumption with no `stale` flag — the exact hazard §5.4d closed for
   `trusted`, reopened one mechanism over.
3. **No accumulating surface.** `audit`/`worklist` are honestly NOT BUILT (TODO.md),
   so the debt lives only in per-run output that scrolls away, and the run is
   CI-green by default (`W0511` is a warning; `--fail-on evidence` ignores warnings).

Each is disclosed somewhere; their conjunction is the actual laundering risk and
belongs in TODO.md as one item, because they are one loop.

### G3 — declared-contract keying depends on the anchor matching the Cargo.toml dependency name

`declared` is keyed as `{anchor}::{fn_key}` for a non-local component, and matched
against the path *as the caller spells it*. 004 works because `anchor: ledger`
happens to equal the renamed dependency key. §5.1 says `anchor` is a crate name; a
`ledger = { package = "real-name", path = ... }` rename with `anchor: real-name`
would silently fail to match, and the callee would classify `Unclaimed`. This fails
**closed** (loud `W0512` refusal naming the callee), so it is a usability gap, not an
honesty one — recorded here because the first user with a renamed dep will hit it and
the diagnostic will say "no contract anywhere describes" a callee whose contract they
just wrote. Cost: resolve the anchor through the same `path_dependency` rename logic
the resolver already has.

### TODO.md's six KNOWN GAP entries — judged honestly open

All six are honestly open, each with a concrete anchor (a fn that still times out, a
command that does not exist, a shape with no decoder), none quietly abandoned, and the
follow-up note added to `vetting/004-legacy-extension.md` is exactly the right
preservation move. Two bookkeeping nits: the finding-5 `[x]` entry is the only closed
item with no commit hash (`593cf9a`), against the TODO rule; and the "run.sh budgets
raised" entry is a record of a done thing living under an unchecked box. D1 above
means the "§5.5 does not reach std/core/registry" KNOWN GAP needs its wording widened
(it currently repeats the "just never a first-party one" overclaim).

---

## What was verified, what was trusted, NOT CHECKED

**Verified first-hand**: the five diffs in full; the D1 and D2 live probes (fresh
fixture copies through the real binary, envelopes quoted verbatim above); the
`unclaimedcallee` refusal re-run (0.029s, `W0512`, exit 1 — matching the coordinator's
own s3 verification); `ply-core` (70) and `ply-cli` (11) unit suites, `cargo fmt
--check`, clippy `-D warnings` equivalent count 0, all after the D3 fix; the
`validate_keys` vocabulary against `tools/model`'s serde structs field-by-field (they
match, including `entry` and nested components); `tools/` untouched by these commits
(diff-confirmed); the composite-leaf gate test (`[BTreeSet<u8>; 2]` pinned
Unsupported); the exit-code table unit tests read against §6.

**Taken on trust** (consistent with quoted literal output, not re-run): the s3/s5/s7/s8
before/after outputs — the coordinator independently verified s3, s8 and the panic
witness; the item-5 measurement table (0.028–0.064s per shape) and the 60.3s
widened-body measurement; the 201.77s stub verification time; the seed-hunt replay
triple.

**NOT CHECKED**: the full 102/103-test suite (coordinator's run in flight — the count
discrepancy in O4 will resolve there); `boundarycontract`/`arraycard` e2e end-to-end
(each is a multi-minute Kani run; their harness-content assertions were read instead);
a vacuous declared contract through the real stub path (G2.1 is from code reading); a
boundary callee with a struct return (`unstubbable` `-> ()` refusal is code-read only,
matching the write-up's own NOT RUN); rendering of any of this (the renderer consumes
none of the new envelope fields yet — `conditional` has no visual form in practice,
which §5.5's own "must read as routine and legible" condition will eventually demand).

---

## Do the five fixes close what 004 opened?

**Findings 1, 4, 5 and 7: closed, genuinely.** The exit-code default is the single
highest-leverage honesty fix available and it landed with the right opt-out semantics
(`--fail-on error` reproduces the old behaviour exactly, tested). The panic-witness
recovery turns a class of real bug from "tool error" into "violation with input" while
keeping §5.4c's MUST intact. The fragment widening is measured-first and gates
composites at the leaves the engines really build. E0204 parity validates the whole §5
grammar, not verify's subset. Each fix was red-first with a failure message naming the
real defect — the discipline held.

**Finding 2: half closed, and the open half is quieter than the original defect.** The
refusal mechanism, the declared-contract stub, and the refusal-fails-run interlock are
all real and all correct *for qualified call spellings*. For `use`-imported callees
the pre-004 behaviour survives — and where 004's version of it announced itself with
an 11-minute timeout, the surviving version hands back a clean `bounded(2)` in 42
seconds. The problem was not moved; it was closed on a subset, and outside that subset
it got harder to see. Aggregation, for what it's worth, is honest about `unclaimed`
once it exists: it drags every ancestor down (worst-of rank 4, below all real
evidence) and fails the run — nothing launders it. The failure is upstream: the
resolver's blind spot prevents `unclaimed` from being produced at all.

## The four questions

**Does `conditional` risk becoming the new silent pass?** Not silent — but quiet, and
trending quieter. Today it is visible three ways (root-level status, `W0511` with the
contract verbatim, the `assumptions` array), it cannot be laundered by aggregation
(`union_statuses` carries it to the root unconditionally; the verdict rank is
unchanged, per D6's design), and the run is green — which §5.5 explicitly intends,
since `conditional` is to be the normal state of legacy-extension code. The risk is
G2's loop: an assumed contract that is wrong or vacuous produces the same green
`conditional bounded(2)` as a right one, nothing ever exercises it, nothing goes
stale, and no command accumulates the debt. The honest summary: `conditional` is
currently an IOU with a visible sticky note; it becomes the silent pass on the day
users learn to skim `W0511` — which §5.5 itself predicts. Before M5 closes, the loop
needs at least one of: the fuzz-exercise of declared contracts, `audit`/`worklist`, or
a vacuity cover. All three are already on TODO.md except the vacuity check, which this
review adds to the pile.

**Is item 2's enumeration complete?** Over node-verdict strings, yes — checked against
every verdict the tool can emit, including suffixed forms. Over *absences*, no, two
ways: (1) absences encoded as statuses/outcomes escape it — a missing or inconclusive
`mutate` engine exits 0 against §6's own exit-3 row (D2, live-reproduced); (2) D1
prevents the `unclaimed` absence from being produced at all for `use`-imported
callees, so the enumeration never sees it. Also noted: `--fail-on evidence` fails a
run whose *verdict* is absent but not one whose verdict rests on an unexercised
assumption — correct per spec, worth saying out loud.

**Which of the six disclosures are serious?** Disclosure 4 (resolution limits) is the
serious one, because it is materially *under*-disclosed — D1 is its missing half.
Disclosure 1 (budget) is serious for what it omits, not what it says: the raise
itself is legitimate, but the default-timeout implication means the feature is dead at
defaults (G1). Disclosure 3 (owed-evidence homeless) is serious in conjunction with
the unchecked-vacuity and no-staleness gaps (G2) — individually each is tolerable
debt. Disclosure 2 (seed misses the bug) is a defensible design honestly framed, with
one overstated sentence (O2): determinism is the right call because it kills the
re-roll-until-green laundering channel, and `--seed` sweeps remain available — but a
recorded-entropy mode (vary by default in some contexts, always record) deserves a
TODO entry as the complement, not a rejection. Disclosures 5 (widening buys
reachability, not speed) and 6 (spec text in the wrong commit) are honest and minor —
5 is exactly §5.4c's own claim measured again and undermines nothing.

**Does anything need redoing before this branch merges?** No — nothing here is built
wrong; the two majors are holes beside sound mechanisms, not rot inside them. But two
things need **doing**, because until then the branch's own spec text is false: (1)
D1 — either extend the resolver through `use` declarations (small, and this review's
probe is the red fixture) or retract §5.5's "never a first-party one" sentence and
the two documents that repeat it; (2) D2 — route the mutate missing-engine path into
the absence machinery or amend §6's exit-3 row. O3/O4/O5 and the D6 status-list
omission are same-session wording fixes. Everything else is correctly parked on
TODO.md. If (1) and (2) land — or their claims are honestly retracted — this is good
work: the interlock of refusal + fail-by-default is the first time the project's
"absence of evidence" principle exists as machinery rather than as a lesson learned
three times.
