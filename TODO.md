# TODO

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
