# TODO

## Agreed with the maintainer, not yet started

- [ ] **D7 for stubbed crossings — `W0541 stub_substituted`** (planned
      2026-08-25, `docs/plans/d7-stub-failures.md`). The Kani-pin spike proved a
      stub-caused failure has no faithful plain-Rust reproduction, on any engine version:
      the rendered test calls the real callee, which never returns the stub's invented
      value, so it is emitted **green** (that test is in `tests/fixtures/boundarycontract`
      right now and passes). D7's unqualified red-test promise is corrected in the spec
      today. Build: a third `W0541` reason, the fabricated value + admitting clause in the
      diagnostic, a `fixes` entry proposing the tightening, and *stop emitting the passing
      `ply_cex_*` test* — a green reproduction that reproduces nothing is worse than none.
      Refused by name: rendering the test against a rewritten body, which would go red for
      a program the user does not run.

- [ ] **Trusted boundary declared in `ply.yaml`** (maintainer's idea, 2026-08-25) — the
      coarse-grained sibling of §5.5's per-callee rule: declare a region taken as given,
      rather than writing a contract per legacy function a new feature happens to touch.
      Fills a real hole in §7.2's taxonomy — *our code, checkable in principle,
      deliberately not checked* — which is distinct from an `external` (someone else's
      system, permanently). Three conditions agreed up front, all learned the hard way
      here: it must never read as evidence (crossing one marks the caller `conditional`,
      never clean, or trusting the whole tree goes green); it must be counted on the
      audit surface so the trusted region is under pressure to shrink; and it must draw,
      per §7.1's gate. Carry `trusted`'s own lesson: it shipped with no staleness and an
      attestation would have outlived the code it vouched for. Proposal first, gate,
      then adopt — the sequence that worked for external elements.

      **GATE RUN 2026-08-25 — the answer is no; take the fallback.**
      `docs/plans/trusted-boundary.md` bound itself to one empirical claim: that real
      callers are defensive enough to verify with the callee replaced by an
      unconstrained symbolic return, and that "if most crossings fail under havoc, this
      is a hint generator wearing a grammar construct." **Most crossings fail: 2 of 8
      pass (25%), and both passes are 004's own functions.** Zero of six callers written
      without the experiment in mind passed; six of six failed, five with a
      counterexample and one by timing out at the 300s floor with no witness at all.
      The falsifiable prediction on record **held** — `tier_fee_cents` passes under
      havoc (133.74s, inside the floor) because of its own `.min(10_000)`, and so does
      `approve_withdrawal` above it (212.63s) — but it held only on the two functions
      the plan already knew about. Cost is not the objection: havoc costs the same as a
      declared contract stub (133.74s vs 148.62s on the same function) and lands inside
      §6's floor. Three findings the plan has no row for: a havoc'd loop bound turns a
      22s proof into a 300s timeout with no diagnostic; the breaking value names the
      callee and the direction but never the threshold (`2_813_465` where the contract
      needed is `<= 100_000`); and Ply would print the *least* useful witness where
      several exist. **Recommendation: do not build `given:` as a grammar construct;
      adopt the plan's own open question 6 fallback** — let a clause-free boundary entry
      mean havoc, per callee, no new grammar (the codegen already emits it; only
      `verify.rs`'s `if claim.requires.is_empty() && claim.ensures.is_empty() { continue; }`
      stands in the way). Evidence, fixtures and a reproducing `run.sh`:
      `tests/spike/havoc/FINDINGS.md`. Commit `ff15b23`.

## Kani pin — spiked 2026-08-25; recommendation: stay put, two gaps left open

- [x] **Bump the Kani pin — a D13-shaped spike, not a fork.** Ran, against Kani `main`
      built from source (`245709373965fcb78209135822cbafb59c08d036`, 2026-08-25, CBMC
      6.10.0, `nightly-2026-04-01`) beside the untouched 0.67.0. **Recommendation: do
      not move the pin, and do not fork.** Four measured reasons.
      (1) There is nothing to bump *to* — crates.io's newest `kani-verifier` is still
      0.67.0, so a bump means pinning a commit of an unreleased branch that still
      reports itself as `0.67.0`, which would stamp two different engines with one D14
      fingerprint.
      (2) Blocker 2 is **not fixed**: `#[kani::stub]` over a contracted target still
      fails with `Failed to find contract closure __kani_recursion_check_<fn>` on
      today's `main` (Kani #4591, open).
      (3) Blocker 1 as recorded here was **never true** — at 0.67.0 a stubbed harness
      that fails *does* print a concrete-playback witness, and Ply's own
      `extract_witness_bytes` would accept it. The real limit, identical on both
      toolchains and stated in Kani's own generated doc comment, is that the playback
      test **does not apply the stub**: replaying a stub-caused failure panics on
      leftover concrete values instead of reproducing anything. That is worse than the
      documented blocker because a naive "the test is red" check passes.
      (4) Ply's real §5.5 shape already works at the pin — `boundarycontract`'s stubbed
      proof verifies (94.6s, 85 checks) and a violation in the same configuration yields
      a witness — at ~12-14% *lower* cost than the candidate (107.7s, 110 checks).
      Evidence, fixtures and a reproducing `run.sh`: `tests/spike/kani-pin/FINDINGS.md`.
      Commit `82555a9`.
- [ ] **KNOWN GAP, raised by the spike, no Kani version fixes it.** §5.5 can produce a
      violation that no test of the real code can reproduce: the counterexample's third
      value is the *stub's* invented return, and the real callee never returns it. Written
      out D7-style at the two real inputs, the test is green
      (`tests/spike/kani-pin/boundary/src/lib.rs::witness_replay`, observed passing).
      §8 forbids a witness-free `violation`; here the witness exists but is not
      replayable. A spec conversation about §5.5/§8, not an engine upgrade.
- [ ] **`boundarycontract`'s clean proof does not exercise its own assumption.** Delete
      the generated stub's `kani::assume` so the callee is unconstrained and
      `ply_proof_tiered_fee` still verifies (86.4s at the pin, 107.1s on the candidate):
      `legacy_rate(tier).min(10_000)` clamps whatever comes back, so the proof holds for
      *any* callee. The `conditional` verdict is still honest, but the fixture does not
      show the assumption doing work. Consider adding a harness that does (the spike's
      `tiered_fee_halfclaim` is one).

## Post-004 review closure — landed 2026-08-25

Disposition of every finding in `docs/review-post-004-fixes.md`, with the red-first
failure message and literal before/after for each: `docs/post-004-review-closure.md`.
Six commits, one per finding.

- [x] **Review D1 (MAJOR) — an ordinary `use` import bypassed the boundary rule.** The
      resolver never read `use` declarations, so a bare-name call classified `Unresolved`
      and `Unresolved` meant descend: `bounded(2)`, zero diagnostics, **exit 0** in
      40.562s over an unclaimed body, against `unclaimed`/`W0512`/**exit 1** in 0.007s for
      the identical claim spelled with a qualified path. Resolution now follows `use`
      (renames, groups, globs), inline and file modules, re-exports, and a path
      dependency's `src/lib.rs`; first-party source Ply cannot read is refused (`W0513`,
      new) rather than descended into. New fixture `useimport` + e2e, nine `ply-core` unit
      tests. §5.5's "just never a first-party one" retracted here, in
      `docs/post-004-fixes.md` and in this file. Commit `e83ccb9`.
- [x] **Review D2 — the fail-by-default rule missed absences encoded as statuses.**
      `mutate` with cargo-mutants masked: `fuzzed(64)` + status `inconclusive` + exit 0,
      against §6's own exit-3 row. The rule is now over **names, not slots** — one
      absence vocabulary read against a node's verdict *and* its statuses — and `mutate`'s
      non-results name which absence they are, so exit 3/2/1 follow the fact. Masked-engine
      e2e (§9's matrix, first entry built) + unit tests, including that `conditional`,
      `owed-evidence`, `weak-spec` and `stale` still exit 0. §3's "It never fails the run"
      reconciled with §6. Commit `a92e61f`.
- [x] **Review D4 — `W0541`'s wording was false for the shapes it now fires on.** It named
      `BTreeSet`/`Vec` to users whose parameter is a `[u32; 4]`. It now names the
      parameters and types that blocked the rendering; `RustType::display_name` fixes the
      same omission in `X0901` ("`xs: `", type missing). Commit `681fc75`.
- [x] **Review D5 — `evidence` described runs that never happened.** `cases: n` was
      attached whenever `fuzz(n)` was *declared*: a harness that never compiled reported
      `cases: 64`. Now built where the run happens, with `cases` only when the count is
      real. Commit `283cd83`.
- [x] **Review D6 — `owed-evidence` was emitted but defined nowhere.** Defined in §0's
      glossary and D6's status list as the debt half of `conditional`; §5.5 calls it a
      status; the verdict kernel gains the variant and a round-trip test over the whole
      vocabulary. Enumeration unchanged and green. Commit `9c730dd`.
- [x] **Review G1 — the `conditional` path was dead at the tool's own defaults.** 004's
      `tier_fee_cents` is scalar-signature, so plain `cargo ply verify` gave it 60s and
      reported `timeout` in 1m6.776s, saying nothing about the assumption. A **stubbed**
      `bounded` harness now gets a 300s floor — derived split (a stub is knowable before
      the run and always trades concrete values for a symbolic one), fitted constant
      (201.77s measured, plus the ~107s CBMC variance the M3 findings recorded), with
      9.72s and ~110s as the second and third measurements showing the cost is the body's
      as much as the stub's. `K0601` explains the premium when there was one. The
      `boundarycontract` fixture now carries 004's body shape and its e2e passes **no**
      `--engine-timeout`: the only test that observes §6's default end to end.
      Commit `182e9e1`.
- [x] **Review O2, O3, O5 — overstatements corrected in place** ("Nothing was lost" on
      seeding; §5.5's present-tense `audit`/`worklist`; "s1/s2 behaviour unchanged", which
      item 2 falsified by flipping their exit codes).
- [x] **Review O4 — DISPUTED, with evidence.** The tree holds **21** e2e `#[test]`
      functions at `3adca0e`, counted file by file, so `70 + 11 + 21 = 102` was right and
      the review's 22 was wrong. Nothing changed.
- [ ] **KNOWN GAP — the boundary rule inspects the claimed function's own body only.**
      Until D5's first branch lands, a contracted callee `g` is inlined rather than
      stubbed, so an unclaimed callee one level below `g` still travels into the caller's
      proof unnamed. Same pattern as review D1, a different bypass; stated in §5.5's
      limits. Not started deliberately (out of that task's scope).
- [ ] **KNOWN GAP — calls Ply's reader cannot see are not call sites for the rule**:
      macro-generated calls, `#[path = "..."]` module attributes, function pointers and
      trait methods.
- [ ] **KNOWN GAP (review G2) — the assumed-contract enforcement loop, as ONE item**,
      because the three parts are one loop and their conjunction is the risk: (1) no
      vacuity check — a declared `ensures: ["|result| false"]` makes the stub's
      `kani::assume` unsatisfiable and the caller's proof vacuously green, and a
      `kani::cover!` after the stubbed call would catch it cheaply; (2) no staleness — D14
      fingerprints trusted claims, nothing fingerprints a declared boundary contract
      against the callee's body, so legacy code can change under a standing assumption
      (the hazard §5.4d closed for `trusted`, reopened one mechanism over); (3) no
      accumulating surface — `audit`/`worklist` are not built, so the debt lives only in
      per-run output that scrolls away, and the run is CI-green by default.
- [ ] **KNOWN GAP (review G3) — declared-contract keying assumes the anchor equals the
      Cargo.toml dependency key.** `ledger = { package = "real-name", path = ... }` with
      `anchor: real-name` would not match the path a caller writes, and the callee would
      classify `Unclaimed`. It fails **closed** (a loud `W0512` naming a callee whose
      contract the user just wrote), so this is a usability gap, not an honesty one. Fix:
      resolve the anchor through the same rename logic the resolver already has.
- [ ] **Recorded-entropy fuzz mode** (the review's complement to the seeding decision):
      vary the seed by default in some contexts and *always* record it, so cross-run
      detection accumulation comes back without reopening the re-roll-until-green channel
      that determinism closed.

## Post-004 fixes — landed 2026-08-25

Closes the five items `docs/review-post-004-strategy.md` sequenced after vetting 004.
Full write-up with literal before/after output, red-first failure messages and measured
costs: `docs/post-004-fixes.md`. Four commits, one per item plus item 1's spec-and-code.

- [x] **Finding 2 / D5's third branch — the boundary rule.** §5.5 rewritten (three-way
      split, three honesty conditions, and its own stated limits); §2's D5 row amended.
      Built: a `bounded` check whose fn calls a callee no contract describes refuses to
      descend and names it (`W0512`), 004's `run.sh s3` going from `timeout` after
      **11m23.094s** to a named refusal in **0m0.005s**. With a contract declared in
      `ply.yaml`, the callee is stubbed (`#[kani::stub]`, cross-crate, real) and the
      caller earns `bounded(2)` + statuses `["conditional", "owed-evidence"]` + `W0511`
      listing the assumption — 004's `run.sh s5`, **3m15.9s** wall at a 600s budget.
      Commit `2cf09c2`.
- [x] **Finding 7, `anchor:` half.** A component anchored at another crate is a boundary
      component: contracts read, `checks` not run here (`W0303`), no node. A fn entry
      declaring only `requires`/`ensures` is a boundary contract declaration, not a claim.
      Commit `2cf09c2`.
- [x] **Finding 1 — a run that checked nothing exits 0.** §1 gains the
      absence-of-evidence principle; §6's exit table gains the missing row and
      `--fail-on=warn|evidence|error` (default `evidence`, `error` the documented
      opt-out). Exit codes 2 and 3 are returned for the first time. Commit `d73558f`.
- [x] **Finding 4 — `fuzzed(n)` is not reproducible**, and the escalation the review
      added to it. Seed derived per fn, recorded in the §8 envelope as
      `evidence: { engine, seed, cases }`, `--seed <hex>` replays; proptest's own
      persisted-failure replay switched off. `run.sh s8`: six identical `fuzzed(256)`,
      where it used to split 3/3. And a panicking body now earns a `violation` carrying
      proptest's own shrunk input instead of `X0901` — the class of real bug that could
      not be reported at any seed. Commit `c8e231b`.
- [x] **Finding 7, `ensures:` half.** `config::validate_keys` enforces §5.1a rule 1 on the
      verify path (`E0204`, location, nearest key) against the **whole** §5 key
      vocabulary, so the keys `verify` ignores are still accepted. §5.1a amended to say
      the rule binds every reader of the file, and so does its converse. Commit `23e8f67`.
- [x] **Finding 5 — the implemented fragment is narrower than §5.4b.** `char`,
      `Option<T>`, `Result<T,E>`, `[T; N]` and top-level type aliases are in. Measured
      first (Kani `Verification Time`, trivial bodies): 0.028s `u32`, 0.064s `char`,
      0.036s `Option<u32>`, 0.040s `Result<u32,u8>`, 0.036s `[u32; 4]`, 0.041s
      `[u32; 16]`, 0.028s alias. No unwind annotation for an array — its bound is a
      compile-time constant. Commit `593cf9a`.
- [ ] **KNOWN GAP — D5's *first* branch is still not implemented.** A callee that passed
      its own Kani proof this run is inlined, not `stub_verified`, because callees-first
      scheduling (ADR-0003's "entire soundness guarantee", living unlinked in
      `tools/schedule`) is not promoted into the product. Concretely: 004's
      `total_debit_cents` still times out at 120s with `fee_cents` inlined. The review
      sequences this as the next tranche.
- [ ] **KNOWN GAP — §5.5's rule does not reach `std`/`core`/registry callees.** A call
      into a crate whose source Ply cannot read is left alone, so a `bounded` verdict can
      still include a body Ply never examined. Stated in §5.5, not left to be discovered.
      (The clause "just never a first-party one" was **retracted 2026-08-25**: an
      ordinary `use` import bypassed the rule entirely — see the closure of the review's
      D1 below.)
- [ ] **KNOWN GAP — a boundary assumption is reported as owed, and nothing exercises it.**
      §5.5 says an unexercised assumption is owed evidence and that `audit`/`worklist`
      list it. The `owed-evidence` status and `W0511` are built; `cargo ply audit` and
      `cargo ply worklist` are **not built**, and fuzz-checking a declared contract
      against the real legacy body is not built either.
- [ ] **KNOWN GAP — `ply.yaml` `requires`/`ensures` are still not ANDed into the fn's own
      check** (§5.4 says they are). They are read, and used for §5.5's boundary
      assumption; `W0510` now says out loud which of the two a user is getting.
- [ ] **KNOWN GAP — no witness decoder for the newly admitted shapes.** `char`,
      `Option`, `Result` and `[T; N]` reach the engines, but `WitnessValue` cannot spell
      them, so a Kani violation on one is reported `X0901`/`tool_error` naming the
      parameter (never a witness-free `violation`) and a fuzz violation lands on the
      existing `W0541` witness-only path.
- [ ] **NOT DONE, deliberately deferred**: cross-crate type-alias resolution (004's
      `withdraw` takes `ledger::AccountId`; resolving it changes nothing, because the
      `&mut ledger::Ledger` beside it keeps the fn `unsupported` either way), structs of
      scalars, `--only-changed`, `cargo ply check`, `schema/ply.schema.json`, and the
      renderer's earned-vs-declared split (finding 8).
- [ ] **Rendered cex test for a panicking body fails with the function's own panic**, not
      the contract message, because the call sits outside the test's `catch_unwind`.
      §9's cex-oracle clause "failure output states the contract" therefore does not hold
      for that shape; the contract is named in the generated test's comment. The oracle
      test itself (`clamp_oracle.rs`) is unaffected and green.
- [x] **`run.sh` budgets raised, annotated in place**: s5 120s → 600s (the stubbed proof
      needs ~202s of Kani time), s7 120s → 600s (arrays are cheap, this fn's body is not).
      Both original runs are quoted in `docs/post-004-fixes.md`. (A record of a done thing;
      the unchecked box was a bookkeeping slip the 2026-08-25 review caught.)

## Vetting 004 — legacy boundary, fragment-first — landed 2026-08-24

The first vetting scenario designed inside §5.4b's fragment from line one, and the first
run against the real `cargo ply verify` (Kani 0.67.0) rather than reasoned about on
paper. Write-up: `vetting/004-legacy-extension.md`; two crates + `ply.yaml` + `run.sh`
under `vetting/004-legacy-extension/`; SVG committed. Nothing in `crates/`, `tools/` or
`The-Ply-Spec.md` was touched — this scenario finds, a later session decides.

- [x] `legacy/` (ordinary `BTreeMap`/`Vec`/generic-helper module, no `ply::` anywhere) +
      `feature/` (five fns, all claimed, contracts inline) + one `ply.yaml` read by
      `verify`, `ply-check` (clean, exit 0) and `ply-render`.
- [x] Twelve `verify` invocations across `run.sh s1..s8`, all reproducible; every verdict
      quoted in the write-up is literal tool output (two long envelopes are cut to their
      verdict spine, and say so).
- [x] **The boundary's answer is `timeout`.** `tier_fee_cents` (fragment-clean signature,
      body calling one unclaimed `BTreeMap`-backed legacy fn) never finished: `timeout` at
      120s and again at **600s** (11m23s wall). Control: the identical fn with the legacy
      call replaced by a `match` earns `bounded(2)` in 1m20s total. `conditional`/D5 never
      fired — none of D5 (`stub_verified`, `W0511`, `ply-schedule`) is linked into
      `crates/` at all.
- [x] `--only-changed` and `cargo ply check` confirmed **absent** (§6 specifies both);
      recorded as findings, not built.
- [x] **Finding 1 — a run that checked nothing exits 0.** CLOSED 2026-08-25 (`d73558f`). `K0601 timeout` is warning
      severity, `--fail-on` is unimplemented, so a run whose root verdict is `timeout` is
      CI-green. Proposal in the write-up: absence of evidence fails by default.
- [x] **Finding 2 — D5 has no branch for an *unclaimed* callee.** CLOSED 2026-08-25 (`2cf09c2`). Both its branches assume
      the callee has a contract. Needs an explicit third rule, and the diagnostic must name
      the callee that was descended into (K0601 today names only the caller).
- [ ] **Finding 3 — checkability is about bodies, and §5.4b gates on types.**
      `total_debit_cents` (no legacy contact at all) also timed out at 120s in the same run
      where `fee_cents` passed.
- [x] **Finding 4 — `fuzzed(n)` is not reproducible.** CLOSED 2026-08-25 (`c8e231b`), with the panic-witness escalation. Six fresh runs of the *same*
      unfixed source: 3 × `fuzzed(256)`, 3 × `tool_error` (X0901, the real panic). Seed is
      entropy-derived (`Config { cases, ..default() }`) and recorded nowhere; exit code
      flips with it. The §8 envelope needs the seed, and a `--seed`/lockfile replay.
- [x] **Finding 5 — the implemented fragment is narrower than §5.4b.** CLOSED 2026-08-25 for arrays, aliases, `char`, `Option`, `Result`; structs of scalars still open. `[u32; 4]` (the
      spec's own *preferred* bounded shape) is `Unsupported`; so is a `type X = u64` alias.
      No `Type::Array` arm and no alias resolution in `rust_type_from_syn`.
- [ ] **Finding 6 — V0505's fix names a mechanism that does not exist** ("add a
      `pure`-marked generator hook"): no `#[ply::pure]` macro, no ply.yaml key.
- [x] **Finding 7 — `verify` is single-crate** — CLOSED 2026-08-25 for both halves (`anchor:` consumed in `2cf09c2`, `E0204` parity in `23e8f67`); multi-crate *verification* is still out of scope.: `anchor:` is parsed and never used, every
      component's fns are looked for in one `src/lib.rs`, and ply.yaml `requires`/`ensures`
      are silently dropped (unknown serde fields) while `ply-check` on the same file
      enforces `additionalProperties: false`.
- [ ] **Finding 8 — the render draws declared ceilings as earned.** `tier_fee_cents B2`
      and `withdraw B2` (unsupported!) draw exactly like the fn that really earned
      `bounded(2)`. Already on this list as "separate declared ceilings from earned
      verdicts"; 004 is the first live instance.
- [ ] **Finding 9 — `--only-changed` is the delta thesis's mechanism**, not a convenience.
- [ ] **Finding 10 — `verify` writes into the crate under test** (generated modules in
      `src/`, harness member appended to `[workspace]`), which is why `run.sh` copies to a
      scratch tree. Second vote for the "where the harness crate should live" item below.
- [ ] **NOT COVERED: 004's document is outside the renderer's invariant sweep.**
      `tools/render/tests/render.rs` walks a hardcoded list of 001/002/003 plus its own
      fixtures. Adding 004 means editing `tools/`, which this session was not permitted to
      do. The committed SVG was rasterised (CairoSVG) and checked by eye instead.
- [ ] NOT RUN, recorded: `mutate`/`prove` in this scenario; a boundary callee with a
      non-scalar signature; any bound other than `bounded(2)`; any budget above 600s.

## External systems and actors — landed 2026-08-24 (31a669d)

Full detail in `docs/external-elements-adoption.md`; the gate this was conditioned
on (vetting re-run before any spec amendment) is recorded as a numbered finding
plus an "external-elements gate" section in `vetting/003-trading-system.md`.

- [x] `tools/model`: `externals:` (`External { note }`, required field) and
      per-fn `entry: Vec<String>` on `FnClaim`.
- [x] `tools/check`: five new document-local rules — `E0202` (name collides with a
      component), `E0207` (external in a `->`/`deny`), `E0208`
      (`external ~> external`), `E0209` (`entry:` names an undeclared external),
      `W0410` (external declared, never referenced) — all fixture-tested,
      `tools/check/tests/externals.rs`.
- [x] `tools/render`: external box outside the frame, `~>`/derived `entry:` edges
      routed around intervening components, frame border weight bumped to read as
      a boundary. New invariant `frame_boundary::no_external_box_intersects_
      the_frame_deny_wildcards_stay_inside_and_external_edges_cross_once`
      (`tools/render/tests/render.rs`) — written red first (confirmed: it failed
      on its own vacuous-pass guard before the renderer had any external support,
      not a compile error), green after, mutation-tested (two real mutations,
      each reverted). Fixed two pre-existing routing-algorithm limitations the
      real 003 picture exposed (wrong rail-side heuristic, obstruction filter too
      narrow) in a new dedicated function, without touching the existing
      (already-tested) deny-line routing at all.
- [x] **Correction, same session, coordinator review**: the committed
      `003-trading-system.svg` (`--collapse ingest`) drew `venue ~> ingest.feed`
      straight through `strategy`/`signals` — a real crossing
      `no_drawn_element_intersects_a_box_it_is_not_inside` did not catch, because
      that test never rendered `--collapse <name>` for any single component, only
      "default" and "--depth 1" (collapse-everything). Root cause and full fix in
      `docs/external-elements-adoption.md`; short version: the test now sweeps
      one `--collapse` per top-level component per fixture (watched go red on
      the exact defect, plus a second, previously-unknown one on
      `--collapse gateway` crossing `pnl`, before the routing fix landed), and
      the router's first-leg sweep — sound for deny's always-off-to-the-side
      `from`, unsound for an external edge's `from` (an ordinary component
      border, which can sit inside another component's column) — now tries a
      straight vertical run first and only detours sideways when that specific
      run is blocked. Both 003 SVGs regenerated again and rasterised with
      headless Chromium; confirmed by eye, no line crosses a box it shouldn't.
- [x] **Second correction, same session, coordinator review**: the committed
      `003-trading-system.svg`'s `RawFrame` edge label was struck by a drawn
      line (the derived `entry` edge's, not even its own path) — same shape of
      gap as the correction above, this time in text, not boxes. Extended
      `no_drawn_element_intersects_a_box_it_is_not_inside` to check every
      `edge-label` against every drawn line (tried and rejected two narrower
      forms first — all-text-vs-all-lines produced false positives on `any`
      `*`/deny `except` labels, own-path-only produced a false negative on this
      exact bug, since the striking line belonged to a *different* edge);
      watched red first, naming the exact label and line, before any placement
      code changed. Fixed by splitting external-edge rendering into a
      route-then-draw two-pass structure so each label can be checked against
      every sibling line (regular edges, deny lines, and other external
      routes), not just its own, plus widening the label-placement escalation
      to vary the anchor point along the segment as well as the perpendicular
      offset. Mutation-tested (line-avoidance clause disabled, confirmed red,
      reverted, confirmed green). 13 pre-existing, out-of-scope violations on
      edges that predate this feature (`BookUpdate`/`OrderIntent`/`Order`/
      `Fill`) are now surfaced by the general check but not failed on —
      recorded, not fixed, per `docs/external-elements-adoption.md`. Both 003
      SVGs regenerated a third time and rasterised again; confirmed by eye, no
      label is struck by a line in either image.
- [x] `vetting/003-trading-system.ply.yaml`: `venue` external, three flow edges,
      `entry: [venue]` on `Oms::submit`; `ply-check` clean. Both committed SVGs
      regenerated and diffed line-by-line before accepting; `vetting/001-*.svg`,
      `vetting/002-*.svg`, and the disruptor insta snapshot regenerated too (the
      only diff in each: the frame stroke-width bump, confirmed by diffing).
- [x] `The-Ply-Spec.md` amended: §5.1 (structure + example), §5.1a rule 6, §5.3
      (external edges), §7.1 (two table rows + the dash-channel restatement),
      §7.2 (the fourth kind of unspecified — "out of scope by ownership").
- [x] Gate passed — no fallback to the flag-only form was needed.
- [x] `cd tools && cargo test`, `cargo fmt --check`, and
      `cargo clippy --release --all-targets -- -D warnings` all green/clean.
- [ ] **Left for the maintainer, not attempted**: the *holistic* squint test —
      does this read well, beyond "nothing overlaps" — is explicitly the
      maintainer's own call, per the task brief. The specific correctness
      property (no line crosses a box it shouldn't) is now confirmed two ways,
      not just judged: the extended invariant, and a direct-eye check of both
      committed SVGs rasterised with headless Chromium.
- [ ] NOT RUN: a document with more than one external, or with two external
      edges to the same external from different components sharing no lane —
      the layout code has a defensive width-overflow guard but no fixture
      exercises multi-external layout or that specific lane-fan gap.
- [ ] Out of scope by the task brief, not attempted: `crates/` (the
      `entry:`/audit surface lands there at M5); `tools/kernel` untouched (and
      correctly so — externals never enter the verdict tree).
- [ ] NOT FIXED, recorded: 13 pre-existing `edge-label`-vs-line violations on
      edges that predate this feature (`BookUpdate`, `OrderIntent`, `Order`,
      `Fill` between `gateway`/`oms`/`pnl`), now surfaced by the general
      check added above but not failed on. Fixing them means extending this
      session's two-pass restructure and multi-anchor escalation to the
      regular-edge and deny-edge label placement code too — a larger, riskier
      change to well-tested code, not attempted here. Full list printed by the
      test itself; see `docs/external-elements-adoption.md`.

## M4 — fuzz + test + mutate tier — landed 2026-08-24 (2520f8b)

Note on provenance: 2520f8b's own message flags that the full-suite result was "NOT
yet independently confirmed" at commit time (a session-salvage commit, written before
verification finished). It now is: `cargo test --workspace` (single-threaded) is green,
5m31.8s wall clock, zero warnings on a fresh `cargo check --workspace --tests` — recorded
in docs/m4-findings.md along with two deliberate self-mutations, each caught and reverted.

- [x] Task 0: engine-timeout default made shape-aware
      (`verify::default_engine_timeout_secs`) — a `Vec`-typed `bounded(k)` harness now
      gets `30 + 15·k` seconds (reproduces the M3-measured 150s for `bounded(8)` exactly);
      scalar-only stays at 60s. §6 amended with the reasoning.
- [x] `fuzz(n)` check: proptest harness generation (ints biased small, `Vec`/`BTreeSet`
      length 0-8, `requires` as a reject filter with a >50%-rejection `W0503` warning),
      shrink-on-failure rendered through the *same* `contract_rt` renderer the Kani path
      uses. Struct-parameter fuzzing NOT implemented -- deliberate scope cut, recorded in
      docs/m4-findings.md, not silently skipped.
- [x] `test` check: `examples` entries (parsed as arbitrary `==` Rust exprs, §5.4a) +
      generated direct-contract boundary cases.
- [x] `mutate` check: cargo-mutants wired via the spike's verified mechanism, with a real
      correction the spike didn't know about -- `--copy-target true`, not `--gitignore
      false` (see below). `E0504` (mutate with no test/fuzz kill signal) implemented and
      fixture-tested.
- [x] Shape-aware default-check routing (§5.4c's own MUST, unimplemented in M3):
      `default_checks_for` -- `[bounded(2)]` only when Kani-supported, `[fuzz(256)]` when
      the shape is fuzz-supported but Kani-excluded, `[]` otherwise.
- [x] 5 new fixtures + e2e tests (`fuzzbug`, `weakspec`, `strongspec`, `mutatetier`,
      `btreeset`) -- all 6 of the M4 brief's acceptance criteria pass, including the
      milestone's own headline case: a `BTreeSet<u8>` fixture (Kani-excluded per §5.4b)
      earning an honest `fuzzed(256)` verdict via the default route, no `checks:` declared
      by hand.
- [x] **Falsified spec claim, real cost**: §5.4c's mutate mechanism said `--gitignore
      false` was the fix for the harness crate's git-ignored `target/ply/fuzz/`
      placement. Real runs found this wrong on two counts -- `--gitignore`'s own default
      is already off, and there is a *separate*, gitignore-independent skip
      (cargo-mutants prunes any directory literally named `target` at the copy root,
      unconditionally) that hit every real `mutate` run. Fixed with `--copy-target true`
      (which cannot even be passed alongside `--gitignore` -- confirmed, they share a
      clap argument group). Honest cost: this copies the crate's entire `target/` build
      cache into every scratch tree cargo-mutants builds (~13s against a 189MB `target/`
      in this session's fixtures) -- a real, size-dependent tax flagged for M5, not a free
      fix. §5.4c amended.
- [x] Falsified: `engines::fuzz`'s failed-test-name parser looked for libtest's per-test
      `---- name stdout ----` header, which never appears under `--nocapture` (which this
      adapter always passes, for the high-rejection marker) -- caught because it silently
      reported a real seeded bug as a clean pass on the first real run against a fixture,
      not by a unit test in isolation. Fixed; a regression test pins the real output shape.
- [x] Two deliberate self-mutations, each caught and reverted (docs/m4-findings.md):
      suppressing the `·spec-strong` suffix append (caught by `strongspec_fixture`);
      removing `--copy-target true` again (caught by `weakspec_fixture`, reproducing the
      exact tool-error this session hit for real before the fix).
- [ ] **KNOWN GAP, recorded not hidden**: fuzz-found witnesses are not persisted across
      `verify` runs the way Kani's are (M3 finding 6's `target/ply/witness/<fn>.json`
      convention has no fuzz-path equivalent yet) -- a fix that narrows a bug to a
      *different* input than the one already rendered would leave a stale red test behind.
      Needs its own `<fn>_fuzz.json` path (never the same file Kani writes to, since one
      fn could in principle declare both `bounded` and `fuzz`).
- [x] `W0541` (unrenderable fuzz witness) was implemented but NOT exercised against a real
      failing case. **Now run** (2026-08-24 review closure): `tests/fixtures/btreesetbug` --
      a `BTreeSet<u8>` violation reported witness-only, shrunk to `[3]`, no `cargo_test`
      artifact, exit 1. The item's original wording ("`Vec`/`BTreeSet` of non-`u8`") was
      itself wrong: the path fires for every `BTreeSet`. Still not run: a `Vec<i32>`-shaped
      witness.
- [ ] `mutate`'s `--re <fn>` is an unanchored substring match on cargo-mutants' own
      descriptive mutant names (anchoring with `^fn$` matched *zero* mutants in a real
      run -- confirmed and fixed to the unanchored form). This means a fn whose name is a
      substring of another's in the same crate could see cross-fn mutate scope leak; no
      fixture here exercises more than one fn per crate, so this was not reproduced, only
      named.
- [ ] TODO(M1, carried from M3): reconcile the hand-rolled `ply.yaml` model in
      `crates/ply-core/src/config.rs` with `tools/model`'s full model.

## M4 adversarial review — closed 2026-08-24 (see docs/review-m4-2026-08-24.md and docs/m4-review-closure.md)

Every item below was fixed red-first: the test that fails *because of that defect* was
written and watched fail before the fix, and its failure message read to check it named
the defect. `cargo test --workspace -- --test-threads=1` green afterwards: 405s (6m45s)
wall clock, 72 tests (was 53), zero warnings on `cargo check --workspace --tests`.

- [x] **D1 (SEVERE) — the fuzz/test adapter failed open on a harness that would not
      compile**: an ill-typed `examples` entry earned `fuzzed(64)`/`tested` with zero
      diagnostics and exit 0. A run that did not succeed, did not time out and named no
      failing test ran *zero* cases: now `X0901` + verdict `tool_error` for every check in
      that harness, carrying the compiler's own first error and two concrete fixes. Pinned
      by `tests/fixtures/badexample` + `tests/e2e/tests/badexample_fixture.rs`. §5.4c
      amended with the rule.
- [x] D2 — counterexample `inputs` mislabeled for non-alphabetical parameter order: fixed
      by the reviewer in `94e0a2d`, not redone here.
- [x] D3 — the `>50%` rejection `W0503` was arithmetically unreachable (rejected draws
      counted on both sides of the ratio, i.e. `accepted < 0`). Now `rejected/total`;
      `tests/fixtures/highreject` (~62% rejection) pins it, and the wording no longer
      claims fewer cases ran than the verdict says.
- [x] D4 — a fuzz run proptest *abandoned* (global-reject abort) still earned
      `fuzzed(256)`. Now `unclaimed` + a `W0503` naming the real accepted/rejected counts,
      via a distinct `PLY_FUZZ_ABORT` marker. `tests/fixtures/rejectabort`. §5.4c amended.
- [x] D5 — `M0601` was dead code and cargo-mutants ran with no wall-clock cap (`-t` caps
      only each mutant's test phase, so a hung copy or baseline build hung `verify`
      silently, which §5.4c forbids). The invocation is now wrapped in `timeout` like the
      fuzz and Kani adapters, exit 124 classifies as `Timeout`, and the cap is 10x the
      per-mutant budget (min 120s; measured runs use ~4% of it). M0601's wording no longer
      says "per mutant".
- [x] D6 — a `violation` could be emitted with no witness (marker-parse-failure path),
      breaching §5.4c's MUST. The label now comes from what the renderer could establish;
      `tests/fixtures/panicbug` (a panicking body — an ordinary case, not a contrived one)
      pins `tool_error` with rewritten text and fixes.
- [x] D7 — `W0541`'s wording was false for the exact shape that triggers it (it fires for
      every `BTreeSet`, `BTreeSet<u8>` included). Reworded and exact-string tested; the
      same false claim corrected in three doc comments and docs/m4-findings.md.
      `harness::tidy_contract_text` also widened for method calls, so the quoted contract
      reads `xs.len() as u32` instead of `xs . len () as u32`.
- [x] D8 — five in-tree doc comments asserting claims the M4 commit itself falsified
      (`--gitignore false` "must always pass it explicitly", the mutants mechanism block,
      `failed_tests`' `---- name stdout ----` claim, "Ply always anchors this",
      `write_harness_cargo_toml`'s phantom parameter) all corrected.
- [x] O1 — "derived, not guessed" overstated two fitted constants: `verify.rs`'s doc
      comment and §6 now separate the derived shape split from the fitted coefficients,
      and both record that no e2e exercises the default.
- [x] O2/O4 — the shrinking claim and the `btreeset` acceptance were weaker than their
      names. `tests/fixtures/btreesetbug` (the Kani-excluded shape with a real bug) closes
      both *and* docs/m4-findings.md's own NOT RUN item: witness-only `W0541`, shrunk to
      `[3]`, no `cargo_test` artifact, exit 1.
- [x] O3 — "every M4 non-result diagnostic carries a concrete `Fix`" was not true; the
      claim is corrected in docs/m4-findings.md and all five named paths now carry fixes.
- [x] O5 — partly closed (no-violation-without-witness, witness-only, and
      never-claim-evidence-you-lack are now all tested end to end); the remainder is
      recorded below.
- [x] Found while fixing the above: an `examples` entry containing a `"` generated invalid
      Rust (the entry is echoed into the assert message unescaped), and a `mutate` run that
      produced no result at all was reported as `weak-spec` — a finding no engine made. Both
      fixed red-first; inconclusive mutate runs now carry D6's own `inconclusive` status.
- [ ] **NOT RUN, recorded not hidden**: `M0601` against a genuinely hung cargo-mutants, and
      `P0601`/`R0601` against a genuinely slow harness. The caps and classifications are
      unit-tested; no fixture is slow enough to trip them without making the suite
      timing-fragile.
- [ ] **NOT RUN**: the `W0110` engine-missing paths (`prove`, cargo-mutants absent) — no
      fixture masks an engine, so their newly populated `fixes` are unobserved. §9's own
      engine-absence matrix is the right home for this.
- [ ] §5.4c's "MUST carry the distinguishing engine output into the diagnostic" is now met
      by the new `X0901` (carries the compiler's error line) and `W0503` (real counts), but
      every other adapter still drops `raw_output` (`let _ = raw_output;`) — M3-inherited,
      unchanged.
- [ ] The shape-aware engine-timeout default is exercised by a unit test only: every e2e
      passes `--engine-timeout` explicitly, so no test observes the default in real use.
- [ ] `ensure_workspace_member` bails on any crate whose `Cargo.toml` lacks a `[workspace]`
      table — i.e. every ordinary crate inside a larger workspace, and every standalone
      crate without the marker. `fuzz`/`test`/`mutate` therefore work only on
      fixture-shaped crates today. Needs the same decision as the `--copy-target true`
      cost: where the harness crate should live.
- [ ] `mutants.out/` is left in the user's crate root after every mutate run (removed at the
      *start* of the next one) — outside the `target/ply/` housekeeping convention.
- [ ] A missing-engine label beats a passing check in `combine_fn_check_verdicts`
      (`checks: [prove, fuzz(256)]` with fuzz passing yields `engine-missing`), contradicting
      D9 and D6's status-vs-order split. Unreachable until M7 declares `prove` fixtures —
      fold into the M5 verdict-kernel work.
- [ ] `checks: [fuzz(n), test]` on a fn with no `ensures` silently drops the `test` check
      too (the no-`ensures` `V0505` branch returns before the harness runs, examples
      included). Examples need no postcondition, so this is a routing fix with its own
      fixture, not a wording change.
- [ ] Mutants whose tests time out land in cargo-mutants' `timeout.txt` and do not block
      `all_caught()`, so a fn can earn `·spec-strong` with timed-out (uncaught) mutants.
      Defensible as cargo-mutants' own convention; undocumented until now.
- [ ] §6's exit-code table reserves 2 for a tool error, but `main::exit_code_for` returns 1
      for every error-severity diagnostic (M3-inherited; now visible on D1's new path). The
      new e2e tests assert *non-zero* rather than pinning 1, so no test blesses either
      behaviour.


- [x] `ply-render --depth N` / `--focus` / `--collapse <component>` (8d8910f) —
      collapsed box shows contents line, rolled-up capability badges, pin and finding
      counts; edges reattach; default output byte-identical without flags.
- [x] Collapsed boxes draw as a stack (dc1ad4b, repaired in 26cdeb6); 003's canonical
      artifact is now the collapsed system view, full depth moved to -full.svg.
- [ ] **Color SVG config** — make the renderer's palette configurable (the style
      constants: ceiling scale, finding red, ink, amber) instead of hardcoded; must
      keep the §7.1 channel discipline (a config can retune a hue, not repurpose a
      channel) and the style-rule invariant test.
- [ ] `ply-render --legend` — opt-in legend strip below the frame, generated from the
      live style constants (§7.1, specced 2026-08-23).
- [x] `W0409` redundant parent-to-descendant edge lint (7d4c6fc) — both directions,
      both edge kinds; brought a W-warns/E-fails severity model with it.
- [x] Edge and deny routing + collision-freedom invariant (b3da43c, 2b07bd0) — 003
      render findings 1, 3, 4 closed. KNOWN GAP left open deliberately: deny lines in
      *different* margin columns can still cross (repro:
      tools/render/tests/fixtures/deny_stress.ply.yaml). Needs a routing policy
      decision (§7.1), not a guess.
- [x] Gate debt closed for real — `strict` notch, `mode: synth` violet chip, `examples`
      e×N token all drawn and test-pinned (worktree merge).

## Engine strategy — settled 2026-08-24 (fable review)

- [x] **No pivot to VeriFast, and no additional engine now.** It is a category error:
      Ply is multi-engine by design (D9, §5.4c), so there is no "primary engine" to
      swap — only check kinds and adapters. Three independent reasons VeriFast is the
      wrong first tenant: (1) it emits a symbolic-execution trace, never a concrete
      counterexample, and a failure means *unproved*, not *false* — so a VeriFast-primary
      Ply could never emit a `violation` from its main engine, deleting §1's core
      mechanism rather than weakening it; (2) LLM proof-closure measures 31.4%
      (arXiv:2606.26490, C) against Verus's 44% / AutoVerus ~90%, and our users are
      agents; (3) measured cost on the real verify-rust-std proofs: linked_list
      2,254 → 4,390 lines (+95%; 39 lemmas, 166 `open`, 229 `close`), raw_vec
      854 → 3,246 (+280%).
- [x] **Today's answer for external proofs**: they enter as `trusted` claims — no new
      grammar, and safe now that attestations go stale with the code.
- [ ] **When we reach M7**, the `prove` slot takes a deductive engine, with **Verus as
      first tenant** (Rust-shaped, better agent proof-closure), not VeriFast. Adding any
      engine is milestone-sized, and we are one milestone of seven in with no working
      `cargo ply` — so the next step stays the thin end-to-end slice, not a second engine.
- [x] **Verus feasibility spike done** (`tests/spike/verus/`, FINDINGS.md) — the
      deductive-vs-bounded question the scale spike left open. Result: a `Seq`/`Set`-based
      Verus shadow of the kernel proves all four standing obligations, unbounded, by
      structural induction, in ~2s (mutation-tested, not vacuous) — exactly where Kani's
      bounded model checking cannot terminate at all on the same recursive shape. A
      differential test (4,000 generated trees, plain `cargo test`) binds that shadow's
      executable transcription to the real `ply-kernel` crate. **Open before M7 commits**:
      this proved a faithful shadow, not `tools/kernel/src/lib.rs`'s literal
      `Vec`/`String`-based source — whether Verus's own executable-collection support
      pays the same symbolic cost that stalled Kani on `Vec<String>` is untested and is
      the next spike, not a foregone conclusion of this one.
- [ ] **Revisit triggers** (a decision with a trigger, not a dismissal): an
      AutoVerus-equivalent for VeriFast reaching Verus-level proof-closure; VeriFast
      emitting machine-readable failure output an adapter can parse; a vetting scenario
      showing `fuzzed·spec-strong` is genuinely insufficient on recursive structures; or
      the arena-flattening experiment failing.
- [x] **Trusted claims had no staleness** — the evidence-lying hazard the fallback would
      have shipped: an entry outlived the code it attested and rendered identically fresh
      forever. §5.4d now fingerprints the attested item, marks it stale on change, and
      requires human re-attestation (`accept` does not clear it).
- [ ] **Separation-logic constructs: split the question.** Lemma functions and ghost
      open/close are proof steps, not specification — below the watermark, never
      spec-resident. Heap *predicates* are admissible in principle (the §7.1 gate admits
      them on the same mark-plus-tooltip precedent as contract clauses; separation logic
      is highly diagrammable), but largely unnecessary: §5.4a already admits calls to
      `pure` helpers, so a recursive `pure fn len(&self)` is legal in a contract TODAY.
      What is missing is an engine that can earn `proved` on it, not vocabulary.
- [ ] Smallest useful version for M7: a per-fn `proof:` field naming a proof artifact,
      drawn as a badge, fingerprinted under D14 so it goes stale with the body. No
      predicate sub-language until a vetting scenario forces one.

## From the external review (codex, 2026-08-23) — see docs/review-2026-08-23.md

- [x] **M0 spike done, ADR-0003 accepted** (0974f57). 8/9 mechanisms work; cross-crate
      stubbing works via caller-local re-proof. Fixtures + run.sh under tests/spike/.
- [x] **M0 fully discharged** — the cargo-mutants item is now exercised
      (tests/spike/mutants/). It found §5.4c asserting a mechanism that does not
      exist: there is no "custom test command" flag, and the claim "confirmed in the
      M0 spike" was false. Real mechanism verified and specced; `--gitignore false`
      must be pinned or the build fails; `W0502` now caveats equivalent mutants
      (strong spec killed 13/14, the survivor was provably equivalent, not a gap).
- [x] **Scale spike done; §5.4b rewritten around evidence** (tests/spike/scale/).
      Headline: recursive/self-referential types are NOT supported in v1 — a 3-node
      tree makes 64,147 verification conditions and doesn't finish in 180s, even with
      the unwind fix that makes flat `Vec` cheap. That is the shape of our own verdict
      tree. Also: `Vec` works ONLY if codegen emits `#[kani::unwind(N+1)]`; fixed
      arrays are cheap and become the preferred shape; BTreeSet/BTreeMap are out past
      one element; HashMap needs a codegen hasher swap or it won't compile.
- [x] **Self-hosting resolved**: the enumeration IS bounded-kind evidence (exhaustive
      within a stated bound, independent oracle, covering more than the Kani harness
      would have). CLAUDE.md reframed rather than apologised. Reshaping the kernel was
      rejected on evidence — the stall just moves to the next unbounded field.
- [ ] Write down the enumeration's REDUCTION ARGUMENT (per-bit uniformity of StatusSet,
      content-independence of the assumption merge) — without it, "exhaustive" is
      overclaiming by quotient.
- [ ] Decide whether the three non-terminating Kani harnesses should stay invocable by
      default: `cargo kani` on the workspace now costs ~15 min of guaranteed timeouts,
      and they contradict our own rule about not routing recursive shapes to Kani.
- [ ] **Pull a thin M3 vertical slice ahead of M1/M2** (fable's sequencing call, and the
      external review's undone recommendation #2): one hand-written ply.yaml, one
      contracted fn, generated harness WITH the unwind emission, one real cex, the D7
      rendered red test, one JSON diagnostic. Seven sessions of engine-free scaffolding
      before re-contacting the layer that just falsified five spec claims is the
      reviewed failure mode, rescheduled.
- [ ] **Reweigh M4 above the bulk of M3**: M3 is 8-10 sessions for an engine covering a
      sliver of signatures; M4 is 4 sessions covering every signature, and its scariest
      mechanism is already proven end to end.
- [ ] **Generalise D13 beyond M0**: each milestone opens by spiking its riskiest
      external-tool claim, and no spec sentence may say "confirmed"/"verified" without
      naming the artifact that shows it. §5.4c carried a fabricated confirmation until
      an adversarial re-check caught it.
- [ ] Measure whether the unwind annotation rescues ITERATOR-CHAIN bodies (marked NOT RUN
      in the scale sweep). Until measured, §5.4b's gate still admits functions that hit
      the exact failure it was rewritten to prevent.
- [ ] **D7 correction: the generated playback test does not reproduce contract
      violations.** Adversarial re-verification found `cargo kani playback` never
      evaluates contract closures — only the real body runs — so an `ensures` violation
      replays and the generated test PASSES. Playback preserves the witness *input*
      exactly, but the rendered plain `#[test]` is the only artifact that can be a red
      reproduction, and that is most Ply counterexamples. §8/D7 wording needs a pass.
      It is a documented Kani limitation, not a defect, and the fix is ours: Ply's
      generated test must assert the postcondition explicitly so the failure is
      panic-shaped and therefore replayable. Kani reportedly warns when it declines to
      generate a test; no warning appeared in our run, so don't depend on it.
      → PLAN WRITTEN: docs/plans/d7-replayable-tests.md (worked example compiled and
      run red-for-the-right-reason). Decisions to review before implementing: the
      generated test lives in an in-crate `cfg(test)` module (private items are
      unreachable from `tests/`); contract expressions render through one shared
      overflow-safe assertion renderer, widening to i128 so the assertion states the
      broken promise instead of re-triggering the overflow; refusals reuse W0541 with
      a reason field; and `counterexample.kani_playback` is renamed `kani_witness` so
      the schema stops implying it reproduces anything. DECIDED: witnesses are
      generated for EVERY falsified claim (default `all`); §1 states the concrete
      failing input as a MUST, so a default cap would have violated the spec for
      findings past it. `--witnesses=N|none` is an explicit opt-out only, and when
      used it must announce the skip (W0541 reason `budget_exhausted`). The ~20s
      near-fixed cost is accepted, shown in progress output, and optimised by making
      it cheaper — never by skipping it.
- [x] **Callee-before-caller ordering got kernel-grade treatment** — new `ply-schedule`
      crate: SCC-condensation planning (cycles land in one batch, never deadlock) and a
      `may_stub` decision that returns Allowed ONLY when the callee's proof actually
      passed this run. Invariants enumerated exhaustively: all 65,536 four-node digraphs
      for planning, 4,096 graph+config combinations for the stub decision, both against
      oracles written from D5's text rather than from the production code. Mutation-
      checked by letting `NotRun` license a stub — the exact unsound shortcut Kani
      itself takes — and confirming it goes red.
- [ ] **D5 ambiguity surfaced by the scheduler**: cross-crate proof results are really
      scoped per (calling-crate, callee), since each consumer re-proves locally, but
      `ProofResults` models one global status per fn. Exact for same-crate; a
      simplification cross-crate. Decide before M3 whether the distinction matters.
- [x] **Real defect fixed: component-level `checks` inheritance** (merged from
      worktree): a fn's own list wins entirely; otherwise it inherits the nearest
      ancestor component's default. Resolution lives once in `ply-model` so the
      validator and renderer cannot drift. Tooltips now name the source —
      "inherited from component `pricing`: bounded(2) — …". E0504 evaluates the
      effective list. All five committed SVGs byte-identical (no vetting document
      uses component defaults — grep-confirmed, not assumed).
- [x] Engine-limit diagnostics specced (52222ab) — §8 now requires timeout/unsupported
      to name the cause and populate `fixes`, with the boundary written in: Ply
      proposes, never rewrites. IMPLEMENTATION still owed when the engines are wired.
- [ ] `schema/ply.schema.json` is called normative in §5/D3 and does not exist —
      build it or cut the claim.
- [ ] Separate declared ceilings from earned verdicts in the type system (both are
      `Evidence` today; only convention keeps them apart).
- [ ] `trusted` claims are unrestricted prose — no identity, date, commit, scope, or
      expiry. The shield can read as approval.
- [ ] `conditional` assumptions are free-form strings, untied to the call graph.
- [ ] Kani harnesses do not terminate (e46e4a9): CBMC unwinds BTreeMap's generic clone
      on every recursive `aggregate_raw` call. Kani's docs confirm heap collections
      blow up the encoding AND that generic std methods cannot be stubbed — so the
      documented workaround does not apply directly. ATTEMPT 1 (done): statuses are
      now a `StatusSet` bitmask instead of a BTreeSet — behaviour identical (991k-tree
      enumeration green, untouched assertions) and 40% faster, and Kani moved from an
      indefinite hang to a deterministic timeout at 5 min. Still no verdict: CBMC now
      stalls one field over, sorting/dedup'ing `Option<Vec<String>>` assumptions.
      Deliberately NOT collapsing assumptions to a count — D5 and the newbie-bar rule
      need callers to read them verbatim, so that would narrow what a passing proof
      means. Untried: `-Z stubbing` of the String sort/compare for the harness only.
      **Bigger implication for the project:** Ply routes `bounded` checks to Kani; if
      Kani struggles this much with std collections in a 300-line pure module, the
      supported-signature story (§5.4b) is optimistic. This is exactly the kind of
      engine limit M0 exists to find — more evidence for doing M0 next.
- [x] Renderer CLI now covered — 11 tests over flags, exit codes, and error wording;
      two messages rewritten to the newbie bar (`--depth 0` and a non-numeric depth
      used to fail silently or with clap's raw error).

## M3 thin vertical slice — landed 2026-08-24 (7e6fc79)

- [x] First production code of `cargo ply` itself: `crates/ply-attrs` (the
      `#[ply::requires]`/`#[ply::ensures]` proc macros, D2), `crates/ply-core`
      (`config`, `harness`, `engines::kani`, `contract_rt`, `diag` — exactly the five
      modules authorized, nothing more), `crates/ply-cli` (`cargo-ply verify` +
      `--json`). Root `Cargo.toml` is now the product workspace
      (`members = ["crates/*", "tests/e2e"]`, `exclude = ["tools", "tests/spike",
      "tests/fixtures"]`), separate from `tools/Cargo.toml`.
- [x] Four fixtures under `tests/fixtures/` (`clamp`, `passing`, `vecbound`,
      `timeout`) plus 5 black-box e2e tests under `tests/e2e/` that build the real
      binary and run it — the §9 cex validity oracle, for real, on the `clamp`
      fixture: FAIL (stating the contract + "postcondition", never the overflow trap)
      before the fix, PASS (the same `ply_cex_clamp_01` test) after. `cargo test
      --workspace` green (17 unit tests + 5 e2e tests).
- [x] Measured (not copied from §5.4b's own number, which is for a different harness
      shape) the Vec unwind bound for this slice's own harness: `k+1` for a manual
      indexed-loop consumer of `any_vec::<u8,k>` — 9 at k=8, confirmed 8 fails
      ("unwinding assertion loop 0") and 9 succeeds, with an adversarial e2e test
      proving the emission is load-bearing (the identical harness minus the
      annotation does not verify within a bounded cap).
- [x] Timeout correctly distinguished from violation end to end (`K0601` vs `K0502`,
      `timeout` status carries no counterexample) — see docs/m3-slice-findings.md
      finding 3 for a real, load-bearing caveat: this environment shows CBMC/CaDiCaL
      SAT-solve wall-clock variance (~1s to ~107s on an *identical* harness), and one
      run's raw CBMC log showed a SATISFIABLE result reached before Kani's own
      "CBMC timed out" text was printed — meaning the timeout/violation textual
      distinction can, rarely, itself be racing the engine's own reporting. Routed
      around with generous timeouts here; not fixed. Flagged for the next session.
- [x] Spec amended: D2 (the `unexpected_cfgs` lint requirement, confirmed again);
      D7 + §0 + §1 + §8 + §9 + the M3 milestone bullet, applying
      `docs/plans/d7-replayable-tests.md`'s own pre-drafted deltas now that the D7
      renderer is actually built (the `kani_playback`→`kani_witness` rename is live in
      code, pinned by a unit test).
- [x] Two deliberate self-mutations (§ CLAUDE.md), each caught and reverted: disabling
      the "CBMC timed out" check in `parse_output` (caught by a unit test); making the
      rendered cex test's `Ok(false)` arm a no-op, i.e. "renderer skips the assertion"
      (caught by the real `clamp_oracle` e2e test going red, not a unit test).
- [ ] **KNOWN GAP, recorded not hidden**: `docs/m3-slice-findings.md` finding 6 — the
      witness-persistence mechanism that makes the D7 oracle's "same test transitions
      FAIL→PASS" promise hold across two `verify` runs (`target/ply/witness/<fn>.json`)
      is a real design decision this slice made ad hoc; it was not settled in the D7
      plan and duplicates a sliver of what full D14 staleness tracking will eventually
      own. Needs an explicit call, not silent acceptance, once `ply.lock` lands (M1).
- [ ] Witness-replay half of the §9 oracle (`cargo kani playback` reproducing a stored
      `kani_witness`) is implemented (`engines::kani::run_playback`) but NOT wired into
      `verify` or any e2e test — recorded as NOT RUN in §9, not silently skipped.
- [ ] Not attempted this session, all explicitly out of scope per the M3 brief:
      `impl`-method contracts (`&self`, `old()`), generic fns/`check_with`, cross-crate
      callees, `stub_verified`/`conditional` (D5), the `ply.yaml`
      `requires`/`ensures`-merge path (only inline attributes are read),
      `BTreeSet`/`HashMap` handling, the engine-timeout reliability fix above.
- [ ] TODO(M1), recorded in `crates/ply-core/src/config.rs`'s own doc comment:
      reconcile the hand-rolled ~4-struct `ply.yaml` model here with `tools/model`'s
      full model (promote one, delete the other).
