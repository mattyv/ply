# M4 adversarial review — closure

Closes `docs/review-m4-2026-08-24.md` (8 defects, 5 overstatements, 5 gaps, a NOT CHECKED
ledger). Method, per CLAUDE.md: for every finding, the test that fails *because of that
defect* was written and watched fail first, and the failure message was read to check it
named the actual defect. Anything not fixed is marked disputed or deferred with its
reason, never silently skipped.

**Headline**: D1 was real, is fixed, and is pinned by an e2e whose harness genuinely fails
to compile. Before the fix that fixture reported `fuzzed(64)` with **zero** diagnostics
and exit 0; it now reports `tool_error` with two `X0901`s naming the compiler's own error
and a non-zero exit. Eight of eight defects and five of five overstatements are closed;
nothing is disputed; three review *gaps* are deferred with their reasons recorded in
TODO.md.

**Suite**: `cargo test --workspace -- --test-threads=1` green, **405s (6m45s) wall clock,
measured**, zero warnings on a fresh `cargo check --workspace --tests`. 72 tests, up from
53: 50 `ply-core` unit (+7), 7 `ply-cli` unit (+2), 15 e2e (+5). The five new e2e fixtures
add ~55s; the pre-existing Kani-driven fixtures still dominate (one alone is 127.8s).

Two defects outside the review's list were found while fixing its neighbours and are
closed here too (N1, N2 below) — both are the same class of finding as D5 (a report no
engine's output justifies).

---

## Defects

### D1 — a harness that fails to compile was reported as a clean pass — **FIXED**

- **Red first**: `tests/fixtures/badexample` (an `examples` entry comparing a `u32` to a
  string literal — the review's own probe shape) + `tests/e2e/tests/badexample_fixture.rs`.
  First run failed with `a harness that never compiled must not earn evidence, got
  `fuzzed(64)`: {... "diagnostics":[] ...}` — the defect, in the tool's own output.
- **Fix**: `verify::run_fuzz_and_test_checks` now classifies "did not succeed, did not time
  out, named no failing test" as a run that executed zero cases, and returns `X0901` +
  verdict label `tool_error` for **every** check sharing that harness (both `fuzz` and
  `test`: one crate, so neither ran). The diagnostic carries the compiler's first specific
  error (`engines::fuzz::first_build_error`, which deliberately reaches past cargo's
  causeless `error: could not compile` summary), names the likeliest user-side cause, and
  populates two concrete `fixes` per §8's non-result rule. It is never a `violation`:
  there is no witness (§5.4c MUST).
- **Observable outcome** (`cargo-ply verify` on the fixture, human output):
  `workspace — tool_error` … `[X0901] badexample::add_small — `add_small`'s `fuzz(64)`
  check ran zero cases: the test harness Ply generates for it failed to compile, so
  nothing was checked at all. … The compiler's own first error was: error[E0308]:
  mismatched types. …`, exit 1.
- Spec amended (§5.4c) with the rule itself, so the classification is specified rather
  than folklore.

### D2 — mislabeled counterexample `inputs` — **already fixed by the reviewer**, not redone.

### D3 — the `>50%` rejection warning was arithmetically unreachable — **FIXED**

- **Red first**: `tests/fixtures/highreject` (`requires(x > 14)`, ~62% of draws rejected
  under the biased-small strategy) + its e2e. First run: `expected exactly one
  high-rejection warning … left: 0, right: 1` with an envelope carrying **no**
  diagnostics.
- **Fix**: the generated test compared `rejected / (total + rejected)`, but `total` already
  counted every rejected draw — i.e. `rejected > total`, i.e. `accepted < 0`. Now
  `rejected / total`, evaluated only on a run that finished.
- **Wording corrected too**: the old `W0503` said the check "ran … on far fewer real cases
  than 256 suggests", which is false — proptest draws until it has *n accepted* cases. The
  new text says the count is honest and the *spread* is narrow, and carries two fixes. The
  e2e asserts the wording exactly.

### D4 — an abandoned fuzz run still earned `fuzzed(n)` — **FIXED**

- **Red first**: `tests/fixtures/rejectabort` (`requires(x > 20 && x < 24)`, essentially
  unsatisfiable for the strategy) + its e2e. First run: `a run that checked approximately
  nothing must not claim fuzz evidence, got `fuzzed(256)``.
- **Fix**: the abort path now prints its own distinct marker with the real accepted and
  rejected counts (`PLY_FUZZ_ABORT|<fn>|<reason>|accepted=..|rejected=..`,
  `engines::fuzz::parse_abort_marker`), and the verdict becomes `unclaimed` with a `W0503`
  naming both counts: 1025 of 1025 draws rejected, 0 checked. A warning beside an
  overstated number is still an overstated number.
- Spec amended (§5.4c) to distinguish the two rejection outcomes: high-but-survivable
  keeps `fuzzed(n)` (the count is real, the spread is narrow); an abort earns no verdict.

### D5 — `M0601` was dead code and `mutate` had no wall-clock cap — **FIXED**

- **Red first**, two unit tests in `engines::mutants`:
  `the_whole_invocation_carries_a_wall_clock_cap_not_just_a_per_mutant_one` (red: `argv[0]`
  was `cargo`), and `a_killed_run_is_a_timeout_not_a_tool_error` (red, after temporarily
  reverting the classification, with the message `a run the wall-clock cap killed must be
  reported as `timeout`, never conflated with a tool error or a completed run`).
- **Fix**: `mutants_argv` now wraps the invocation in `timeout <wall>s` exactly as the fuzz
  and Kani adapters do, and `classify_run` (extracted, pure) maps exit code 124 to
  `Timeout`. The cap is `mutate_wall_clock_secs` = 10× the per-mutant budget, minimum 120s
  — measured M4 runs are 24–26s at the 60s default, ~4% of the resulting cap, so a healthy
  run cannot be turned into a spurious timeout. `M0601`'s wording no longer says "per
  mutant" (it is a whole-run cap), explains what a mutate run consists of, and carries two
  fixes.
- **NOT RUN**: no test drives a genuinely hung cargo-mutants. The cap and the
  classification are unit-tested; the end-to-end timeout path is not, because a fixture
  slow enough to hit it would be a minutes-long, timing-fragile test. Recorded in TODO.md.
- The review's side observation about `timeout.txt` mutants not blocking `all_caught()` is
  **deferred, documented**: it is cargo-mutants' own convention; recorded in TODO.md rather
  than changed under a review.

### D6 — a `violation` with no witness on the marker-parse-failure path — **FIXED**

- **Red first**: `tests/fixtures/panicbug` (a body that panics on odd inputs, so the
  harness fails without ever printing its marker) + its e2e. First run: verdict
  `violation` with a diagnostic carrying `"counterexample": null` — precisely §5.4c's
  forbidden report. This path was unreached by any previous fixture; it is now a real,
  plausible one (any panicking body).
- **Fix**: `render_fuzz_violation` returns `(label, diagnostic)`, and only the branch that
  recovered a real failing input may say `violation`. The no-marker branch is
  `tool_error`, with rewritten text naming the likeliest cause (the function panicking
  before its postcondition was evaluated) and two fixes.

### D7 — `W0541`'s wording was false for the case that triggers it — **FIXED**

- **Red first**: `tests/fixtures/btreesetbug` (the milestone's headline `BTreeSet<u8>`
  shape with a seeded bug) + its e2e. First run failed on the exact-string assertion,
  printing the old text: "a `Vec`/`BTreeSet` of anything but `u8` has no renderer yet" —
  told to a user whose type *is* `BTreeSet<u8>`.
- **Fix**: the diagnostic now says Ply "has no way yet to spell a `BTreeSet`, or a `Vec` of
  anything but `u8`, as a literal value", says shrinking happened, and says why there is no
  runnable test ("Ply never invents one"). The same false claim is corrected in three doc
  comments (`engines/fuzz.rs`, `fuzz_gen.rs`) and in docs/m4-findings.md.
- **Side observation fixed as well**: `harness::tidy_contract_text` was too narrow for
  method calls, so this diagnostic rendered `xs . len () as u32`. Red-first unit test
  (`contract_text_reads_like_the_line_the_user_wrote_even_with_method_calls`, red with
  `left: "|result|*result == xs . len () as u32"`), then two more replacements. The e2e
  asserts the contract text now reads `|result|*result == xs.len() as u32`.

### D8 — four (five) doc comments asserting claims the same commit falsified — **FIXED**

All five corrected, in the same commit as the behaviour they describe:
`harness_crate.rs` module doc item 3 (`--gitignore false` "must always pass it
explicitly" → must never pass it, with the reason); `engines/mutants.rs` module doc's
mechanism block (now shows the command as actually spawned, including the `timeout`
wrapper, and flags the old line as falsified); `HarnessTestRun::failed_tests`' field doc
(the `---- name stdout ----` claim, now stating what the parser really reads and why);
`fn_regex`'s doc ("Ply always anchors this" → unanchored, with the real reason and the
known over-match limitation); and `write_harness_cargo_toml`'s phantom
`depth_to_target_root` parameter. Doc comments carry no test — verified by reading, and
by `cargo doc`-free inspection of each claim against the code beside it.

## Overstatements

- **O1 — "derived, not guessed" overstates two fitted constants — FIXED (wording).**
  `verify.rs`'s doc comment no longer claims that solving `150 = base + rate·8` "gives
  base = 30, rate = 15"; it states plainly that the *split* is derived and the
  *coefficients* are fitted to a single working data point whose own variance (~1s–107s on
  an identical harness) dominates any k-linear model. §6 amended to match, and both now
  record that no e2e exercises the default at all.
- **O2 — the shrinking claim was not demonstrated by the fixture that made it — CLOSED by
  a new fixture.** `btreesetbug`'s bug fires for *any* set containing `3`, so an unshrunk
  witness would be a larger set; its e2e asserts `xs == "[3]"`. docs/m4-findings.md's own
  `fuzzbug` claim is corrected in place.
- **O3 — "every M4 non-result diagnostic carries at least one concrete Fix" — FIXED both
  ways.** The claim in docs/m4-findings.md is corrected (it was not true), and all five
  named paths now carry concrete fixes: the no-`ensures` `V0505`, `R0601`, both `W0110`
  skip paths, and the no-viable-mutants `W0502`. **NOT RUN**: no fixture exercises those
  five paths, so the new `fixes` are unobserved by any test; the newly reachable ones
  (`X0901` build failure, `W0503` both variants, `X0901` no-witness) *are* asserted
  non-empty by their e2e tests.
- **O4 — the `btreeset` acceptance is weaker than its name — CLOSED.** `btreesetbug` now
  demonstrates that the Kani-excluded shape can *catch* a bug, witness-only, shrunk, exit
  1 — the review ran this by hand; it is now in the suite. With D1 fixed, the original
  fixture's green run can also no longer be produced by a harness that never built.
  docs/m4-findings.md amended with both qualifications.
- **O5 — evidence-honesty MUSTs are coded but mostly untested for the new engines —
  PARTLY FIXED, remainder recorded.** Now tested end to end: no-violation-without-witness
  (D6's `panicbug`), the witness-only path (`btreesetbug`), and the
  never-report-evidence-you-do-not-have rule in two forms (`badexample`, `rejectabort`).
  Still untested: the proptest and `test`-check timeout paths (`P0601`/`R0601`) and
  mutate's `M0601`. §5.4c's "MUST carry the distinguishing engine output into the
  diagnostic" is now honoured by the new `X0901` (it carries the compiler's error line) and
  by `W0503` (real counts), but every other adapter still drops `raw_output` — an
  M3-inherited gap, recorded in TODO.md, not closed here.

## Found while fixing the above (not in the review)

- **N1 — an `examples` entry containing a `"` generated invalid Rust.** The entry is echoed
  into the assert's failure message, unescaped, so `f(0) == "zero"` closed that string
  literal early and the harness failed with a *syntax* error in Ply's own generated file —
  burying the user's real mistake under a compiler error pointing at a file they never
  wrote. Red-first unit test in `fuzz_gen`; fixed by escaping. This is why the D1 fixture's
  reported cause is now a clean `error[E0308]: mismatched types`.
- **N2 — a `mutate` run that produced no result was reported as `weak-spec`.** The caller
  had two states (suffix or no suffix), so a missing engine, a tool error, a killed run,
  *and* "no viable mutants" all landed on the `weak-spec` status — a finding no engine
  made. Red-first unit test (`a_mutate_run_that_produced_no_result_is_not_reported_as_a_weak_spec`,
  red with `["weak-spec"]`); `MutateOutcome` is now three-valued and inconclusive runs
  carry D6's own `inconclusive` status instead. D5's fix made this reachable, which is why
  it is fixed here rather than recorded.

## Spec changes

- **§5.4c** — three new rules, each naming the review finding it comes from: a harness that
  never ran is a tool error for every check in it (D1, including the no-recoverable-witness
  case, D6); an abandoned fuzz run earns no verdict, while a high-but-survivable rejection
  rate keeps `fuzzed(n)` (D4/D3); the engine cap is on the *whole* invocation, with
  cargo-mutants named as the multi-phase case, and a mutate run with no result carries
  `inconclusive`, never `weak-spec` (D5/N2).
- **§6** — the engine-timeout paragraph now separates the derived split from the fitted
  coefficients (O1) and records that no e2e exercises the default.
- **§10 M4 status** — records the one milestone sub-item never delivered ("weak-spec
  detection (W0502) wired into `worklist`" — there is no `worklist` command yet), and
  points at this closure document, per §10's generalised D13 (no "confirmed" without the
  artifact that shows it).
- Nothing in the spec was found wrong in a way that needed retracting: the review's own
  conclusion (§5.4c's mutate amendment and §10's M4 status are accurate) held up.

## Deferred (recorded in TODO.md, not fixed)

- **`ensure_workspace_member` bails on any crate whose `Cargo.toml` lacks a `[workspace]`
  table** — i.e. every ordinary crate inside a larger workspace. `fuzz`/`test`/`mutate`
  therefore work only on fixture-shaped crates today. This is a real M5-scale limitation
  (it needs a decision about where the harness crate lives — the same decision the
  `--copy-target true` cost forces), not a wording fix, so it is recorded rather than
  guessed at.
- **`mutants.out/` is left in the user's crate root** after every mutate run (removed at
  the *start* of the next one). Housekeeping, outside `target/ply/`; recorded.
- **A missing-engine label beats a passing check in `combine_fn_check_verdicts`** —
  `checks: [prove, fuzz(256)]` with fuzz passing yields `engine-missing`. Unreachable until
  M7 declares `prove` fixtures, and it belongs with the M5 verdict-kernel work the review
  itself suggests; recorded there.
- **`checks: [fuzz(n), test]` on a fn with no `ensures` silently drops the `test` check
  too** — the no-`ensures` `V0505` branch returns before the harness runs, examples
  included. Recorded; the fix is a routing change (examples do not need a postcondition),
  which deserves its own red-first fixture rather than a rushed edit here.
- **§6's exit-code table reserves 2 for a tool error; the CLI still returns 1** for every
  error-severity diagnostic (M3-inherited, and now visible on D1's new path). The new e2e
  tests deliberately assert *non-zero* rather than pinning 1, so neither behaviour is
  blessed by a test. Recorded.

## Disputed

None. Every finding reproduced as described. Two were already stronger than stated: D1's
consequence (c) — `tested` earned by examples that do not compile — is exactly what the
`badexample` fixture shows (both checks reported), and D6's path turned out to be reachable
by an ordinary panicking body, not just by a lost marker.

## NOT RUN / NOT CHECKED (this closure)

- The `M0601` timeout path against a genuinely hung cargo-mutants (see D5).
- `P0601`/`R0601` (proptest and `test`-check timeouts) against a genuinely slow harness.
- The `W0110` engine-missing paths (`prove`, and cargo-mutants absent) — no fixture masks
  an engine, so their new `fixes` are unobserved.
- A `Vec<i32>`-shaped unrenderable witness: the `W0541` path is now covered through its
  `BTreeSet` branch only.
- The review's own NOT CHECKED ledger (mutant identities, the ~13s/189MB `--copy-target`
  measurement, the two earlier self-mutations) was not re-checked here either.
