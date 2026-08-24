# TODO

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
