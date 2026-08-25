# Closing the adversarial review of the post-004 fixes — 2026-08-25

`docs/review-post-004-fixes.md` found two fail-open holes beside sound mechanisms, four
minor defects, five overstatements and three judged gaps. This is the disposition of every
one: **fixed**, **disputed** (with the evidence), or **deferred** (with the reason).

Method, per CLAUDE.md: for each fix the test that fails *because of that defect* was
written and watched fail first, and its message read to check it named the real defect.
Where the red is a compile error rather than an assertion — a status the kernel had no
variant for, a formula with no parameter to pass — that is said out loud rather than
dressed up as a failing assertion.

**One number first, because the review and the brief both quoted it wrong.** The tree held
**21** e2e `#[test]` functions before this session, not 22. `70 ply-core + 11 ply-cli + 21
e2e = 102` — `docs/post-004-fixes.md` was right and O4 is disputed below.

---

## D1 — MAJOR: an ordinary `use` import bypassed the boundary rule. **FIXED** (`e83ccb9`)

The resolver never read `use` declarations. A bare-name call it could not find among the
caller's own top-level `fn` items classified `Unresolved`, and `boundary_plan` treated
`Unresolved` exactly like `Contracted`: descend. So the most idiomatic spelling in Rust
bought a clean proof over a body nobody vouched for.

**Red first.** `tests/fixtures/useimport/` is the review's probe committed as a fixture:
the same unclaimed callees as `tests/fixtures/unclaimedcallee`, reached through
`use rates::{cap_bps as capped, legacy_rate};` — plain import, rename, and nested group in
one line. Its e2e failed, in 38.94s:

```
assertion `left == right` failed: an unclaimed callee reached through `use` is still an
unclaimed callee -- the boundary rule must not be bypassable by spelling the call
differently: {"command":"verify","diagnostics":[], ... "verdict":"bounded(2)" ...}
  left: "bounded(2)"
 right: "unclaimed"
```

Ten `ply-core` unit tests pin the individual spellings. Eight of them compile against the
pre-fix resolver (the other two name a `CalleeStatus` variant that did not exist yet);
spliced into HEAD's `callgraph.rs` and run there, six fail, and every failure reads the
same way — which is the defect in one word:

```
---- callgraph::tests::a_use_imported_callee_is_classified_exactly_like_a_qualified_one ----
assertion `left == right` failed: `use rates::legacy_rate;` plus a bare-name call is the
most ordinary spelling in Rust, and it must not buy a descent into an unclaimed body
  left: Unresolved
 right: Unclaimed
---- callgraph::tests::a_renamed_import_is_followed_to_the_function_it_names ----      left: Unresolved
---- callgraph::tests::a_nested_use_group_binds_every_name_it_lists ----               left: Unresolved
---- callgraph::tests::an_imported_module_prefix_is_followed_too ----                  left: Unresolved
---- callgraph::tests::a_glob_ply_can_see_through_is_resolved_exactly_like_a_named_import ---- left: Unresolved
---- callgraph::tests::a_file_module_on_disk_is_read_and_its_fn_classified ----        left: Unresolved
```

The other two passed against the old resolver too, deliberately: `use std::cmp::*;` plus
`max(a, b)` must stay `Unresolved`, and `Some(x)` must not become a boundary call. They
pin what the fix must *not* break.

**Literal before and after**, same fixture, same binary invocation
(`cargo-ply verify . --json --engine-timeout 120`), the "before" binary built from HEAD
`3adca0e` in a `git worktree`:

```
BEFORE   root verdict: bounded(2) | diagnostics: []
         real 0m40.562s        exit: 0

AFTER    root verdict: unclaimed | diagnostics: [W0512]
         "Ply did not check `tiered_fee`: proving it would mean descending into `capped`
          (called at line 30, column 15), `legacy_rate` (called at line 30, column 22),
          and no contract anywhere describes what that code promises ..."
         real 0m0.007s         exit: 1

CONTROL  tests/fixtures/unclaimedcallee, unchanged: unclaimed, W0512, 0m0.006s, exit 1
```

**What the fix is.** Resolution now follows the crate's own structure: `use` declarations
(renames, nested groups, globs), inline `mod`s, file modules (`mod foo;` → `foo.rs` /
`foo/mod.rs`), re-exports at each file's entry, and the same walk inside a path
dependency's `src/lib.rs`. The rule underneath is one sentence, and it is in §5.5:

> Ply descends only into a callee it resolved, or one that lies outside the workspace
> entirely. First-party source Ply was pointed at and could not read is refused, never
> assumed harmless.

**What a glob means, deliberately.** Three cases, because they are three different facts:

1. A glob Ply **can see through** (`use rates::*;` over an inline module or a path
   dependency) resolves exactly like a named import — the name is either in there or it is
   not.
2. A glob into **first-party source Ply cannot read** leaves the bare name genuinely
   ambiguous. That is an absence of *resolution* inside the workspace, and it refuses:
   `CalleeStatus::Opaque`, diagnostic `W0513`, verdict `unclaimed`. A glob that cannot be
   resolved never silently means "descend".
3. A glob into a **crate outside the workspace** (`use std::cmp::*;`) is left alone, like
   every other call into one. §5.5 already states that gap; pretending to have closed it
   here would fire the rule on ordinary Rust and tell the reader nothing they could act on.

One convention rides along: a bare name beginning with a capital is a type or
enum-variant constructor (`Some(x)`, `Ok(v)`), not a free call, and never triggers case 2.

`W0513` is a separate code from `W0512` on purpose. "No contract describes this callee"
and "Ply could not read this callee" are different facts with different repairs, and a
`W0512` whose words claimed no contract existed for a callee Ply never opened would be
false in exactly the way the M4 review's D7 was.

**The false claims this defect created are retracted**, not patched over: §5.5's "just
never a first-party one" (rewritten, with the two gaps that really do remain), the same
sentence in `docs/post-004-fixes.md` (marked RETRACTED in place, twice), and TODO.md's
KNOWN GAP entry.

**What remains open, and is now stated rather than implied.** (a) The rule inspects the
claimed function's **own body**. Until D5's first branch lands, a contracted callee `g` is
inlined rather than stubbed, so an unclaimed callee one level below `g` still travels into
the caller's proof unnamed. (b) Calls the reader cannot see at all — macro-generated,
`#[path = "..."]`-routed, or made through a function pointer or trait method. Both are in
§5.5's limits and on TODO.md. (a) is a fourth-appearance candidate for the same pattern
and I did not build it: it is a different bypass from the one this task named, and
"nothing adjacent" applies. §5.5's honesty condition 2 ("Ply never inlines an unclaimed
body into a caller's proof") was **qualified** in the same pass to "at any call site in
the caller's own body", because as an unqualified sentence it is false for (a) — and the
same qualification is noted in `docs/post-004-fixes.md`, which repeats it.

## D2 — the fail-by-default enumeration missed absences encoded as statuses. **FIXED** (`a92e61f`)

**Reproduced first**, with the review's own shim (a `cargo` on `PATH` that fails only
`cargo mutants ...` and forwards everything else), on `tests/fixtures/weakspec`
(`checks: [fuzz(64), mutate]`):

```
BEFORE   verdict: fuzzed(64)  statuses: ['inconclusive']  diags: [('W0110','warning')]  exit: 0
AFTER    verdict: fuzzed(64)  statuses: ['engine-missing'] diags: [('W0110','warning')]  exit: 3
```

The e2e red, verbatim:

```
a `mutate` check with no cargo-mutants behind it is a missing engine, and must say so by
name -- `inconclusive` reads as "the engine ran and settled nothing", which is a different
fact and the one that used to exit 0:
{"evidence":{...},"id":"vacuous","statuses":["inconclusive"],"verdict":"fuzzed(64)"}
```

**How the rule was made general rather than special-cased.** The old rule was a list of
verdict *strings* checked against `node.verdict`. Adding `"inconclusive"` to that list
would have closed this case and left the next status-shaped absence open. What changed is
the shape of the rule, not its contents:

- **An absence is a name, not a slot.** One vocabulary — `timeout`, `unsupported`,
  `tool_error`, `unclaimed`, `engine-missing`, `inconclusive` — is read against **both**
  fields of every node, its verdict and its statuses, by one predicate. §1 says this in
  those words, and §6's exit row and `--fail-on` table say "as its verdict or as a status".
  A future absence recorded in a third field is a one-line change at the walker, not a new
  rule.
- **The absence names itself, so the exit code follows it.** `mutate`'s three non-results
  used to collapse into one `inconclusive`. They now say which absence they are —
  `engine-missing`, `timeout`, `tool_error`, or plain `inconclusive` for a run that
  completed and found nothing to mutate — so §6's table can tell "no engine" (3) from "the
  tool broke" (2) from "it ran and settled nothing" (1) without a special case per engine.
- **The other direction is pinned too**, because a rule that fails on any status would fail
  every legacy codebase on its first `conditional` run: a unit test asserts that
  `conditional`, `owed-evidence`, `weak-spec` and `stale` beside a real verdict still exit
  0.

**The design call the review asked for**, made explicitly: an inconclusive-but-not-missing
`mutate` run **does** fail the run. §1's "every claim earned its declared evidence" is the
whole principle; a declared check that established nothing did not earn any. §3's "It never
fails the run" is reconciled with §6's exit-3 row in the same commit: a missing engine is
never reported as a failure *of the check* — nothing about an absent cargo-mutants says a
spec is weak — and is never a passing *run* either. `--fail-on error` remains the
documented opt-out.

**Observed by**: `tests/e2e/tests/mutate_engine_missing.rs` (§9's engine-absence matrix,
first entry actually built) plus two `ply-cli` unit tests.

## D3 — `W0510`'s literal-space runs. **ALREADY FIXED by the reviewer** (`3adca0e`)

Verified present in HEAD and left alone.

## D4 — `W0541`'s wording was false for the new shapes. **FIXED** (`681fc75`)

The message said Ply "has no way yet to spell a `BTreeSet`, or a `Vec` of anything but
`u8`, as a literal value" — true for the only shape that could reach it when written,
false from the moment `char`/`Option`/`Result`/`[T; N]` entered the fragment without
witness decoders. The red, verbatim, for a fn with a `[u32; 4]` parameter and no
`BTreeSet` anywhere:

```
the array is what Ply cannot spell, and the message must say so: `carded_fee` fails its
own contract ... it has no way yet to spell a `BTreeSet`, or a `Vec` of anything but
`u8`, as a literal value. ...
```

It now names the parameters and types that actually blocked the rendering
(`parameter(s) `card_bps: [u32; 4]``), the way the Kani side's `X0901` does. Two unit
tests pin the wording for both shapes; the `btreesetbug` e2e's exact-string assertion moved
with it, and says in its own message why.

`RustType::display_name` is total where `rust_name` is not, which fixes the same omission
one diagnostic over: `X0901` printed "`xs: `" — a parameter named, its type missing — for
exactly the shapes that reach it.

## D5 — `evidence` claimed runs that never happened. **FIXED** (`283cd83`)

Two reds, both literal:

```
badexample   (harness never compiled)   {"cases":64,"engine":"proptest","seed":"9edcb136..."}  verdict tool_error
rejectabort  (proptest gave up)         {"cases":256,"engine":"proptest","seed":"825b11d9..."} verdict unclaimed
```

`evidence` is now built where the run happens: absent entirely when nothing ran; `cases`
present only when the count is real — the full `n` on a completed run, the accepted count
on a run proptest abandoned — and absent when the run was cut short by its budget or
stopped at its first failing case. The seed stays wherever a run happened, because that is
what replays it. §8 amended.

One golden moved deliberately: `panicbug` asserted `cases == 256` on a **violation**.
proptest stops at the first failing case and shrinks from there, so it never reached 256 —
the same overstatement, pinned by a test. The declared count remains on the diagnostic's
`check` field.

## D6 — `owed-evidence` was a status nothing defined. **FIXED** (`9c730dd`)

Defined in both normative homes — §0's glossary row and D6's status list — as the debt half
of `conditional`: `conditional` says the verdict rests on an assumed contract,
`owed-evidence` says nothing has checked that contract against the real body. §5.5 now
calls it a status (it said "open item"), and §7 counts it among the open items.

The verdict kernel's `StatusKind` set mirrors §0's row, so it gained the variant and a test
that walks every status through the bitset — a name in the vocabulary without a bit now
fails there instead of vanishing from an aggregation. **The red here is a compile error**
(`StatusKind::OwedEvidence` did not exist), not a failing assertion; that is weaker
evidence than the other reds and is recorded as such. The 991,389-tree enumeration is
unchanged in what it asserts and still green in 2.04s; its one representative status now
stands in for six others rather than five.

## G1 — the `conditional` path was dead at the tool's own defaults. **FIXED** (`182e9e1`)

**Reproduced first**, vetting 004's `run.sh s5` run verbatim except that no
`--engine-timeout` is passed, so the run uses §6's own default:

```
BEFORE   verdict: timeout   statuses: []   diags: [('K0601','warning')]
         "Kani could not finish checking `tier_fee_cents` within its 60s time budget ..."
         real 1m6.776s      verify exit: 1
```

The tranche's headline capability, at the tool's defaults, on its own flagship case: a
timeout, and not one word about the assumption the user declared. After, same command,
same scenario:

```
AFTER    verdict: bounded(2)   statuses: ['conditional', 'owed-evidence']
         diags: [('W0511','warning')]
         assumptions: [('ledger::fees::bps_for_tier',
                        '`ledger::fees::bps_for_tier`: ensures |result| *result <= 10_000')]
         real 3m5.487s      verify exit: 0
```

(The reproduction script is `run.sh`'s `s5` with the `--engine-timeout 600` argument
removed and nothing else changed; `run.sh` itself still passes the explicit budget, so the
measurement it preserves is untouched.)

**Three measurements, because one would have been a number without a reason** (all Kani
`Verification Time` or wall clock as marked):

| stubbed proof | cost |
|---|---|
| `boundarycontract` fixture, trivial body (`cargo kani` on the generated harness) | **9.72s** |
| the same fixture with vetting 004's body shape (wall clock, whole `verify`) | **1m59s** |
| vetting 004's `tier_fee_cents`, cross-crate stub (measured 2026-08-25, `docs/post-004-fixes.md`) | **201.77s** |

So the stub is not the whole cost — the body is as much of it. What the stub *is*, is the
part Ply knows **before the run**: the harness either carries one or it does not, which is
the same standard the `Vec` split already meets. A stubbed `bounded` harness therefore gets
a floor of **300s** = the 201.77s measurement plus room for the run-to-run CBMC variance
`docs/m3-slice-findings.md` measured on an identical harness (~1s–107s). The split is
derived; the constant is fitted to one data point, and §6 and the code both say so.

**Body cost is deliberately *not* in the default.** `arraycard`'s array parameter costs
0.036s to construct and its body ~139s, and no signature-shaped rule can know that (§5.4c:
"checkability is a property of the body, not just the signature"). That case gets `timeout`
plus `K0601`, whose first fix is to raise the budget — and `K0601` now explains the stub
premium when there was one, naming the callee stood in for, why a symbolic return costs
more than a concrete one, and both measurements so a reader can tell a stub premium from a
heavy body.

**A test observes the real cost.** `tests/fixtures/boundarycontract`'s caller now carries
004's body shape (the defensive `.min`, the widened product, the full `100_000_000`
ceiling) instead of a trivial one, and its e2e passes **no** `--engine-timeout` at all — the
only e2e that exercises §6's default end to end, which is how the default went unobserved
for a milestone. Red at the old default, verbatim:

```
assertion `left == right` failed: the proof is real -- it is the assumption that is
conditional, not the evidence: ... "verdict":"timeout" ... "Kani could not finish checking
`tiered_fee` within its 60s time budget ..."
  left: String("timeout")
 right: "bounded(2)"
```

Green after, in 119.68s (the fixture was 53.68s before; the suite pays ~66s for observing
this, deliberately).

---

## Overstatements

- **O1 — §5.5's limits understated the narrowing. FIXED** with D1: the paragraph is
  rewritten around what resolution actually reaches, the three outcomes it can produce, and
  the two first-party gaps that remain.
- **O2 — "Nothing was lost" on seeding. FIXED**: corrected in place to "per-run detection
  power is unchanged; cross-run accumulation was deliberately traded for determinism", with
  the ~50%-per-run arithmetic and the reason the trade is still right.
- **O3 — §5.5 described unbuilt `audit`/`worklist` in the present tense. FIXED**: "will
  list it as owed **(M5 — neither command is built yet, §10)**".
- **O4 — the test-count arithmetic. DISPUTED.** `docs/post-004-fixes.md` claimed "102
  passed … 70 `ply-core` unit, 11 `ply-cli` unit, 21 e2e", and the review called the e2e
  count 22 and the total inconsistent. Counted at HEAD `3adca0e`, one file at a time:

  ```
  2 vecbound_fixture.rs  2 unknown_key_fixture.rs  2 timeout_fixture.rs
  1 × 15 others                                    TOTAL: 21
  ```

  and the full suite at that commit reports `70 + 11 + 21 = 102`. The document was right;
  the review's 22 (and this task's brief, which inherited it) was not. Nothing changed.
- **O5 — "s1/s2 behaviour unchanged". FIXED**: corrected in place. Item 2 flips their exit
  codes regardless of the boundary rule — both runs contain `withdraw` (`unsupported`), s2
  two `timeout`s — so the claim of behavioural invariance on stages nobody re-ran is
  withdrawn. The verdict trees are unchanged; the exit codes are not.

## Gaps: deferred, with reasons

- **G2 — the assumed-contract enforcement loop (vacuity, staleness, no accumulating
  surface). DEFERRED**, explicitly out of scope for this task. Recorded on TODO.md as **one**
  item rather than three, which is the review's own point: they are one loop, and their
  conjunction — not any one of them — is the laundering risk.
- **G3 — declared-contract keying depends on the anchor matching the `Cargo.toml`
  dependency name. DEFERRED**, recorded. It fails *closed* (a loud `W0512` naming the
  callee), so it is a usability gap rather than an honesty one, and the fix (resolve the
  anchor through the same rename logic the resolver already has) is a change to contract
  keying that deserves its own red fixture.
- **Explicitly not started**, per the task: the vacuity check for declared `ensures`, D14
  staleness on assumed contracts, and building `audit`/`worklist`.
- **Recorded-entropy fuzz mode** (the review's complement to disclosure 2 — vary the seed by
  default in some contexts, always record it): added to TODO.md as the review suggested.

## Gates

- `cargo test --workspace -- --test-threads=1`: **120 passed, 0 failed**, `real
  12m12.127s` — 80 `ply-core` unit, 17 `ply-cli` unit, 23 e2e. Up from 102 (70/11/21):
  +10 resolver unit tests, +6 `ply-cli` unit tests (absence-as-status ×2, `W0541` wording
  ×2, the stub budget, the stubbed-timeout wording), +2 e2e (`useimport`,
  `mutate_engine_missing`). The suite is ~1m40s slower than before, almost all of it the
  `boundarycontract` fixture growing from 53.68s to 119.68s so that a test observes the
  real cost of a `conditional` proof.
- `cd tools && cargo test --release`: **145 passed, 0 failed**, including the 991,389-tree
  kernel enumeration (2.04s) and the new status round-trip.
- `cargo fmt --all --check` and `cargo clippy --all-targets -- -D warnings`: clean in both
  workspaces (`--release` for `tools/`).

## Vetting 004 re-runs

- **`s5`, at the default budget**, before and after: quoted under G1 above. `run.sh`'s own
  `s5` (explicit 600s) is unchanged and its measurement preserved.
- **`s3`, the boundary stage whose resolver was rewritten underneath it** — unchanged, as
  it must be, and re-run to prove it:

  ```
  verdict: unclaimed | diags: [('W0512', 'withdrawal::tier_fee_cents')]
  "Ply did not check `tier_fee_cents`: proving it would mean descending into
   `ledger::fees::bps_for_tier` (called at line 44, column 15), and no contract ..."
  real 0m0.015s      verify exit: 1
  ```

  The call there is written as a qualified path, so it was never the spelling D1 was about;
  what this re-run establishes is that rewriting resolution around `use` declarations,
  file modules and globs did not move the answer for the case that already worked.

## NOT RUN

- **`run.sh` stages s0, s1, s2, s4, s6, s7, s8.** s1/s2/s7 are multi-minute Kani stages
  whose *verdicts* nothing in this session changes: the only fns they contain whose
  classification could have moved are the ones s3 and s5 isolate, and both were re-run.
  Their exit codes were already corrected — and that correction already recorded — by the
  fixes this review examined.
- **A masked-engine run for any engine other than cargo-mutants.** §9's matrix asks for one
  per engine; one exists.
- **The `Opaque` refusal against a real multi-file crate.** It is covered by unit tests over
  a resolver pointed at a directory with no such module, and by the glob case; no fixture
  crate with a deliberately missing module file was built, because a crate that does not
  compile is not a fixture the rest of the suite could run.
- **Rendering.** The renderer consumes none of the new envelope fields; `W0513` and
  `owed-evidence` have no visual form yet (§7.1's obligation, unchanged and still open).
