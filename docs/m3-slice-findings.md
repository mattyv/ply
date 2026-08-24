# M3 thin vertical slice — findings

Run 2026-08-24. Toolchain pinned as observed: **cargo-kani 0.67.0, CBMC 6.8.0**
(matches ADR-0003's pin exactly; already installed in this container, nothing
installed by this session). Rust: `rustc 1.94.1`, edition 2024, `cargo 1.94.1`.

This is the first production code of `cargo ply` itself — everything under `tools/`
predates it and is spec-validation tooling, not the product. Scope, per the M3 brief:
the thinnest possible end-to-end path — `ply.yaml` + a contracted fn → generated Kani
proof harness (with the mandatory unwind emission) → run Kani as a subprocess → parse
its output → either a clean `bounded(k)` verdict or a `violation` carrying a witness or
a `timeout` → the D7 rendered contract-asserting `#[test]` → one §8 JSON envelope.

## What was built

- **`crates/ply-attrs`** — the `#[ply::requires(expr)]` / `#[ply::ensures(|result| expr)]`
  proc-macro attributes (D2). Each re-emits the original item unchanged plus
  `#[cfg_attr(kani, kani::requires(...))]` / `kani::ensures`. Verified both directions on
  all four fixtures: bare `cargo build`/`cargo test` compiles the function exactly as
  written (attributes inert), and `cargo kani` sees the real, instrumented function.
- **`crates/ply-core`** — exactly the five modules the brief authorized:
  - `config` — ~4 serde structs for the `ply.yaml` subset this slice needs
    (`ply: 1`, `components.<name>.{anchor, fns}`, `fns.<name>.checks`), plus the
    `test | fuzz(N) | bounded(K) | prove | mutate` micro-syntax parser with the §5.1a
    range checks. **TODO(M1)**, recorded in the module doc comment: reconcile with
    `tools/model`'s full model (promote one, delete the other) — not attempted here,
    per the brief's explicit scope line.
  - `harness` — syn-based discovery of one contracted fn from source (params, types,
    `requires`/`ensures` AST + text) over a deliberately narrow §5.4a/§5.4b-lite
    vocabulary (`u8..i64`, `bool`, `Vec<u8>`, `&T`/`&[T]` of those); codegen of the
    `#[kani::proof_for_contract]` harness, including the mandatory
    `#[kani::unwind(k+1)]` emission whenever a `Vec` parameter is present; the in-crate
    "generated file + one `mod` declaration" writer (D2's own described mechanism),
    used for both the proof harness (`ply_generated.rs`) and the D7 cex test
    (`ply_generated_cex.rs`).
  - `engines::kani` — runs `cargo kani` as a subprocess and classifies its output into
    `Verified` / `Violation{witness_bytes}` / `Timeout` / `ToolError` — structurally
    incapable of returning `Violation` without a witness (there is no code path that
    constructs one without extracting witness bytes first) and never conflating a
    timeout with a violation (both are separate enum variants reached by disjoint
    conditions in `parse_output`, unit-tested directly against captured real Kani
    output). Also decodes concrete-playback witness bytes into typed values
    (little-endian scalars; length-prefixed `Vec<u8>` — see "Witness decoding" below).
  - `contract_rt` — the D7 renderer: §5.4a `ensures` AST + a decoded witness → an
    overflow-safe `#[test]` that asserts the postcondition explicitly (arithmetic
    widened to `i128`, the whole check wrapped in `catch_unwind`, a newbie-bar message
    naming both sides of the comparison).
  - `diag` — the §8 Diagnostic + envelope types, with the D7 rename already applied:
    the field is `kani_witness`, and a unit test pins that `kani_playback` never
    reappears.
- **`crates/ply-cli`** — `cargo-ply` binary, `verify` subcommand + global `--json` only,
  per the brief. The verify→harness→engine→render→envelope orchestration lives here
  (not in ply-core), since the brief restricted ply-core to the five modules above.
- **Four fixtures** under `tests/fixtures/`, each its own cargo project (`[workspace]`
  root marker, matching `tests/spike`'s convention), each carrying
  `[lints.rust] unexpected_cfgs = { level = "warn", check-cfg = ['cfg(kani)'] }` (M0's
  finding, confirmed again here: without it, bare `cargo build` is not warning-clean).
  Each fixture is checked in at its pristine "before" state — the annotated function and
  nothing else; `cargo-ply verify` generates everything else at run time. The e2e tests
  exercise this against scratch copies, never against the checked-in fixtures directly.
- **`tests/e2e/`** (`ply-e2e`, an explicit workspace member alongside `crates/*`) — 5
  black-box acceptance tests that build the real `cargo-ply` binary and run it against
  fixture copies, asserting file-system and process outcomes exactly as the brief
  demands ("not by hand").

## The acceptance criteria, one by one

- **`rendered_cex_test_fails_for_noncrashing_ensures_violation`**: implemented as
  `tests/e2e/tests/clamp_oracle.rs`. Green. Runs `cargo-ply verify` on a scratch copy of
  `clamp`; asserts the envelope reports `violation`/`K0502` with a `cargo_test` path and
  a `kani_witness` field (never `kani_playback`); asserts `ply_generated_cex.rs` exists
  and its test name starts with `ply_cex_`; runs `cargo test --lib` in the copy and
  asserts it **fails**, with output containing both `postcondition` and the literal
  contract text `result == x`, and explicitly asserts the failure does **not** mention
  `attempt to add with overflow` (the exact spike trap, pinned forever). Then rewrites
  the source (`*result == x` → `*result == x.min(100)`), re-runs `verify`, asserts a
  clean `bounded(2)` with zero diagnostics, and re-runs `cargo test`, asserting the
  **same** `ply_cex_clamp_01` test now **passes**. FAIL-then-PASS, confirmed end to end
  on the pinned Kani 0.67.0.
- **Passing fixture**: `tests/e2e/tests/passing_fixture.rs`. `safe_increment` earns a
  clean `bounded(2)` with zero diagnostics and no generated cex test (there is nothing
  to reproduce).
- **`Vec<u8>` fixture verifies with the emitted unwind**: `vecbound_fixture.rs`. The real
  tool's generated harness for `vec_sum` (declared `bounded(8)`) carries
  `#[kani::unwind(9)]` and verifies cleanly. **Adversarial check**, exactly as asked:
  a second test mutates the tool's own output — the identical harness with the
  `#[kani::unwind(9)]` line stripped — and confirms it does **not** report
  `VERIFICATION:- SUCCESSFUL` within a 25s cap, proving the emission is load-bearing.
- **Timeout fixture reports `timeout`, never `violation`**: `timeout_fixture.rs`. The
  scale spike's own iterator-chain confound (`v.iter().map(|&x| x as u32).sum()`)
  reproduced here, times out reliably within a 30s cap, and the envelope reports
  `timeout`/`K0601` with `counterexample: null` and severity `warning`, never `error`.

All five e2e tests plus all 17 `ply-core` unit tests are green under
`cargo test --workspace` (verified as one run, not five separate invocations).

## Measured: the Vec unwind bound

Per the M3 brief's explicit instruction ("derive it from the actual construction,
measure what bound actually works, and record the real number" — not copy the number
already in §5.4b, which was measured for a *different* harness shape):

For `pub fn vec_sum(v: &Vec<u8>) -> u32` (a manual indexed loop, `for i in 0..v.len()`,
summing into `u32`) over `kani::vec::any_vec::<u8, 8>()`, wrapped in
`#[kani::proof_for_contract(vec_sum)]`:

| `#[kani::unwind(N)]` | Result |
|---|---|
| (none) | `CBMC timed out` at 60s (confirmed 3× — this is the adversarial control) |
| 5 | `VERIFICATION:- FAILED` — `Failed Checks: unwinding assertion loop 0` (insufficient unwind, not a real violation) |
| 8 (== declared bound) | same — `unwinding assertion loop 0` |
| **9 (== bound + 1)** | **`VERIFICATION:- SUCCESSFUL`** |
| 16, 22, 24 | all `VERIFICATION:- SUCCESSFUL` (headroom, not required) |

**Measured minimal bound: `k + 1` = 9, for this exact harness shape** (a manual
indexed-loop consumer of `any_vec::<u8, k>`, wrapped in `proof_for_contract`). This *is*
the formula Ply's codegen uses (`harness::generate_proof_module`).

This measured number is smaller than §5.4b's own quoted figure ("`kani::vec::any_vec::<u8,
8>` needs 22") for what is, on its face, the same `N=8` case. We did not chase down the
discrepancy to a root cause (out of this slice's scope), but the two most likely
explanations, both consistent with everything observed: (a) §5.4b's "22" describes a
different consuming shape — the scale spike's own item-1 sweep measured `#[kani::unwind]`
against a *bare* `vec_sum_loop` (no `proof_for_contract` wrapper, no `ensures` check),
where our harness additionally has CBMC's own contracts-checking machinery
(`__CPROVER_contracts_write_set_check_assignment` etc., observed directly in the
`unwind=8` failure's check list) sharing the same single unwind bound, which could cut
either way depending on which loop dominates; (b) different element/computation shape.
**This is exactly the kind of thing §5.4b warns about ("the bound is not N+1") cutting
both directions**: it is also not always 22, or any other single constant independent of
harness shape — the only honest policy is what M3 already committed to, measure per
shape, never assume. Recorded as an open question for whoever next touches Vec codegen,
not smoothed over.

## Falsified / confirmed against the spec

1. **CONFIRMED, D2's `unexpected_cfgs` lint requirement** (already an M0 finding,
   re-confirmed here on the real tool's generated code, not just a hand-written
   fixture). Spec amended: D2's row now states the requirement plainly and notes that
   automating the one-line `Cargo.toml` insertion is a natural near-term enhancement,
   not yet built (this slice's fixtures all set it by hand).
2. **CONFIRMED, D7/ADR-0003 caveat 3**: Kani's own concrete-playback test for the clamp
   violation reports `WARNING: Kani could not produce a concrete playback ... because
   there were no failing panic checks` whenever the contract holds — consistent with the
   earlier finding that playback never re-checks contract closures. This slice never
   depends on Kani's own playback test as a red artifact; only the separately rendered
   `ply_cex_*` test is asserted to fail. The D7/§8/§9 spec text has been updated with the
   plan's own pre-drafted wording (`docs/plans/d7-replayable-tests.md` §7), including the
   `kani_playback` → `kani_witness` rename, now landed in code (`ply-core::diag`) and
   pinned by a unit test.
3. **NEW — engine-timeout reliability is genuinely fragile in this sandboxed
   environment, in a way that threatens the timeout/violation distinction itself.**
   The exact same harness (`vec_sum`, `bounded(8)`, `#[kani::unwind(9)]`), run
   back-to-back with no code change, was observed to take anywhere from **~1s to
   ~107s** to reach `VERIFICATION:- SUCCESSFUL`. Worse: with a 60s `--harness-timeout`,
   one run produced `CBMC failed / VERIFICATION:- FAILED / CBMC timed out` — Kani's own
   textual signal for engine exhaustion — **while CBMC's own log, read in full, showed
   `SAT checker: instance is SATISFIABLE` moments earlier**: a genuine result had
   already been found internally, but the harness-timeout watchdog fired before Kani
   finished *reporting* it, and Kani's summary line does not distinguish "exhausted
   before any verdict" from "found a verdict, then ran out of time formatting/printing
   it." No stray competing processes, no cgroup CPU quota, low load average — this
   reads as inherent CBMC/CaDiCaL SAT-solve wall-clock variance (plausibly
   memory-layout/ASLR-sensitive internal ordering), not resource contention. **This
   means §5.4c's MUST — "never conflate timeout with violation" — can be genuinely
   undecidable from Kani's own output alone in an environment with this much run-to-run
   variance**: reading past "VERIFICATION:- FAILED" to "CBMC timed out" is necessary but
   was observed, at least once, to not be sufficient, because the "CBMC timed out"
   text itself appeared over a result that had, in fact, already been reached. This
   slice's e2e tests route around it by using generous timeouts (150s for the Vec
   fixture) rather than fixing it — the real fix (retry-once-on-timeout-with-larger-cap,
   or trusting an internal SAT-result marker over the wall-clock summary line) is left
   as an open, named risk for the next session, not silently absorbed into "just use a
   bigger number."
4. **NEW, confirmed**: the file-based "generated file + one `mod` declaration" mechanism
   D2 describes as one of two options works exactly as written, for both the proof
   harness and, by the same mechanism, the D7 cex test: `mod ply_generated;` /
   `mod ply_generated_cex;` declared in the crate's `lib.rs`, each backed by a
   `ply_generated*.rs` file in the same `src/` directory, each idempotently inserted
   (a second `verify` run does not duplicate the `mod` line — unit-tested directly in
   `harness::tests::write_generated_module_is_idempotent`).
5. **NEW, confirmed**: quote's `TokenStream::to_string()` (used to recover contract text
   for diagnostics from a parsed `syn::ExprClosure`/`syn::Expr`) inserts a space around
   every token (`|result| *result == x` becomes `| result | * result == x`), which is a
   real, if purely cosmetic, newbie-bar problem for any user-facing string built from
   parsed-then-restringified Rust. Not previously named in the spec or FINDINGS.md.
   Worked around here with a narrow, explicitly-scoped cleanup
   (`harness::tidy_contract_text`) rather than a general pretty-printer — good enough for
   this slice's contracts, explicitly not claimed to be general.
6. **NEW — the D7 oracle needed a persistence decision the plan didn't fully specify.**
   §9 says a rendered `cargo_test` "must FAIL ... before the fix ... and PASS after" —
   read literally, the *same* test artifact must transition, not a fresh one appear and
   an old one linger stale. But Kani only produces a witness on a *failing* run; a
   second `verify` after the fix finds no new violation and so has no fresh witness to
   render from. This slice resolves it by persisting the witness bytes (`Vec<Vec<u8>>`,
   the same shape Kani prints) under `target/ply/witness/<fn>.json` — Ply already owns
   everything under `target/ply/` per §6's housekeeping note — and re-rendering the
   *same* test's assertion against the *current* contract text on every `verify` run
   whenever a stored witness exists, regardless of that run's own Kani outcome. This
   turns a prior counterexample into a permanent regression test once found, which
   seems like the right general behavior (not just a test-suite convenience), but it is
   a real design decision this slice made that the spec did not settle in
   `docs/plans/d7-replayable-tests.md` — flagged here for review rather than silently
   assumed correct.
7. **NOT RUN**: the witness-*replay* half of the §9 oracle (`cargo kani playback` on the
   stored `kani_witness`, asserting its decoded inputs equal `counterexample.inputs`).
   The adapter function exists (`engines::kani::run_playback`, `--lib` per FINDINGS.md's
   own "playback needs `--lib`" cost) but is not wired into `verify` or exercised by any
   e2e test. Recorded honestly as not attempted, not silently skipped — §9 amended to
   say so explicitly.
8. **NOT ATTEMPTED, out of scope per the brief**: `impl`-method contracts, generic fns /
   `check_with`, cross-crate callees, `stub_verified`/`conditional` (D5), the `ply.yaml`
   `requires`/`ensures` merge path (only inline attributes are read in this slice),
   `BTreeSet`/`HashMap` handling, and anything under `tools/` (untouched).

## Mutation-tested, per CLAUDE.md

Two deliberate self-mutations, each confirmed to make the relevant acceptance test go
red, then reverted (diffs shown here, not left in the tree):

- `engines::kani::parse_output`: disabled the `"CBMC timed out"` check
  (`if false && combined.contains(...)`). `engines::kani::tests::recognizes_timeout_never_as_violation`
  failed immediately, for the right reason (asserted a `Timeout`, got something else).
  Reverted.
- `contract_rt::render_cex_test`: changed the rendered test's `Ok(false)` arm from
  `panic!(...)` to a no-op (the renderer silently accepting a false postcondition — the
  brief's own example mutation, "make the renderer skip the assertion"). Ran the real
  `clamp_oracle` e2e test (not just a unit test) against the mutated renderer:
  `ply_generated_cex::ply_cex_clamp_01` now **passed** where it must fail, and the
  e2e test's own `assert!(!test_run.success, ...)` caught it and failed for exactly that
  reason. Reverted; `cargo test --workspace` green again afterward.

## Honest costs

- The full `cargo test --workspace` run took **~4.5 minutes** in this environment, almost
  entirely Kani wall-clock time (four `cargo kani` invocations across the e2e suite, one
  of which — the Vec fixture — cost up to ~110s on its own per finding 3 above). This is
  not a fast test suite and will not become one while it shells out to a real model
  checker; `--engine-timeout` and D14's fingerprint-keyed skip-if-fresh caching (out of
  scope here, since `ply.lock`/staleness are explicitly excluded from this slice) are
  the spec's own answer to this, not yet built.
- Witness decoding (`engines::kani::decode_witness`) is deliberately narrow: little-endian
  scalars up to 8 bytes and one `Vec<u8>` shape (length-prefixed, `N` single-byte
  entries). This covers exactly this slice's four fixtures and no more; it is not a
  general Kani-witness decoder and does not claim to be.
- `contract_rt`'s widening/message rendering is exercised end-to-end only for a
  top-level comparison (`==`, `<=`, etc.) over scalar arithmetic — the shape all four
  fixtures' contracts use. The D7 plan's fuller case table (compound `&&`/`||`
  decomposition, `u128`/`i128` `checked_*` fallback, non-`Debug` result values) is
  implemented only where it happened to be free (the `catch_unwind` backstop, the
  fallback generic message for a non-comparison top-level expr) and is otherwise
  untested — recorded as scope, not silently claimed complete.

## What the next M3 session should pick up

1. Decide finding 3 (engine-timeout reliability) properly: either a retry-on-timeout
   policy, a way to trust CBMC's internal SAT-result line over Kani's summary
   verdict line, or an explicit documented floor under which `--engine-timeout` should
   never be set on this class of hardware. Left as an open, named risk, not a guess.
2. Decide finding 6 (witness persistence for the regression-test oracle) as a real
   design choice, not an implementation accident — it currently lives entirely inside
   `ply-cli`'s orchestration and duplicates a sliver of what full D14 staleness
   tracking will eventually own; worth an explicit call on whether it should be
   subsumed by `ply.lock` once that lands (M1) or stay a separate, simpler mechanism.
3. Wire the witness-replay half of the §9 oracle (finding 7) so `cargo kani playback`
   is actually exercised, not just implemented and left unused.
4. `impl`-method support (`&self`, `old()`) is the most spec-visible gap versus M0's own
   fixture (which covered it by hand); this slice's `harness::discover_fn` only handles
   top-level free functions.
