# M4 — fuzz + test + mutate tier — findings

Run 2026-08-24. Toolchain pinned as observed: **rustc/cargo 1.94.1** (edition 2024,
unchanged from M3), **cargo-mutants 27.1.0** (installed fresh at session start via
`cargo install cargo-mutants --locked`, matching the earlier spike's exact pinned
version — nothing else needed installing). Kani 0.67.0 remains pinned and untouched;
this session's own changes never touch `crates/ply-core/src/engines/kani.rs`'s behavior,
only its callers.

This session extends the M3 thin slice (`docs/m3-slice-findings.md`) with the `fuzz`,
`test`, and `mutate` checks, the shape-aware default-check routing D12/§5.4c require, and
the engine-timeout fix Task 0 of the M4 brief asked for first. **Every M3 acceptance test
stays green** — re-run in full as part of this session's own final `cargo test
--workspace`, not assumed.

## What was built

- **`crates/ply-core/src/harness.rs`** extended, not replaced: `RustType` gained two
  fuzz-only variants, `Vec(Box<RustType>)` (a general `Vec<T>` for scalar `T != u8` — the
  Kani path here only ever builds `VecU8`) and `BTreeSet(Box<RustType>)` (§5.4b's own
  measured Kani exclusion, and the shape this session's Kani-excluded acceptance fixture
  uses). The old `RustType::is_supported`/`ContractFn::is_supported` are renamed to
  `is_bounded_supported` (every M3 call site updated, behaviour unchanged for every type
  M3 knew about) and joined by a new, strictly broader `is_fuzz_supported` — the two-gate
  split that makes shape-aware routing possible. `ContractFn::has_contract()` is new (used
  to decide whether "no checks declared" means "nothing to default").
- **`crates/ply-core/src/fuzz_gen.rs`** (new) — codegen for all three M4 checks' generated
  tests, sharing one type-directed strategy builder:
  - `generate_fuzz_test`: one `#[test]` per fn, driving `proptest::test_runner::TestRunner`
    directly (not the `proptest!` macro — needed for the manual reject-counting and
    marker-printing this check does; the macro form gives no hook for either). Ints are
    biased small via `prop_oneof![3 => 0..=16, 1 => any::<T>()]`; `Vec`/`BTreeSet` use
    proptest's own `collection::{vec,btree_set}` combinators at length 0–8; `requires`
    becomes a `TestCaseError::reject`, counted so a >50% rejection rate can be flagged.
    `ensures` is checked via the *same* `contract_rt::widen` the Kani cex renderer uses
    (exposed as `pub(crate)` for this reuse) inside `catch_unwind`, exactly mirroring
    `contract_rt::render_cex_test`'s own overflow discipline. On a shrunk failure (proptest
    shrinks internally before `TestRunner::run` returns `TestError::Fail`), the concrete
    values are printed as a `PLY_FUZZED_CEX|<fn>|k=v;k=v` marker line.
  - `generate_example_test`: one `ply.yaml` `examples` entry (§5.4a: exempt from the
    contract subset, "arbitrary Rust `==` expressions") parsed as a `syn::Expr` and
    compiled as a plain `assert!`.
  - `generate_direct_contract_cases`: a small, fixed, diagonally-zipped battery of
    boundary literals per parameter (0/1/MAX for unsigned, 0/MIN/MAX for signed,
    true/false, and a few `Vec`/`BTreeSet` lengths) run through the real function with
    `ensures` asserted directly — §5.4c's "generated direct contract cases."
  - Deliberately **not built**: struct-parameter ("field-by-field") fuzzing — see "Scope
    cuts" below.
- **`crates/ply-core/src/harness_crate.rs`** (new) — scaffolding for the generated harness
  crate the mutants spike's verified mechanism needs: reads a target crate's package name
  and `[lib]` identifier out of its `Cargo.toml` (plain line-scanning, same deliberately
  narrow convention as `harness::tidy_contract_text`); idempotently registers the harness
  crate as a workspace member of the target crate's own root `Cargo.toml` (`members = [".",
  "target/ply/fuzz/<name>"]`) — this is the exact mechanism the mutants spike identified as
  load-bearing (`cargo metadata`-visible package resolution for `-p`/`--test-package`);
  writes the harness crate's own `Cargo.toml` (a `[dev-dependencies]` path dependency on
  the target crate plus `proptest = "1"`) and its `src/lib.rs` (one `#[cfg(test)] mod
  {fn}_harness { ... }` per fn needing `fuzz`/`test`, concatenated, regenerated wholesale
  every run — Ply owns this file entirely, there is no user code in this crate to
  preserve).
- **`crates/ply-core/src/engines/fuzz.rs`** (new) — the proptest engine adapter:
  `run_harness_tests` runs `cargo test -p <harness> --lib <filter> -- --nocapture` (wrapped
  in the `timeout` command for a hard cap, same pattern as `engines::kani::run_playback`),
  and parses libtest's own final `failures:\n    <name>\n` summary block for failing test
  names (**not** the per-test `---- <name> stdout ----` detail header — see "Falsified"
  below, this was a real bug caught by testing against a real fixture, not assumed
  correct). `parse_fuzz_marker`/`decode_marker_fields` turn a `PLY_FUZZED_CEX` line back
  into the *same* `WitnessValue` type Kani witnesses decode into (the D7 plan's "two
  consumers, one renderer," now both wired) — returning `None`, never a fabricated value,
  for a `Vec`/`BTreeSet` of anything but `u8` (no renderer exists for that yet; see W0541
  below).
- **`crates/ply-core/src/engines/mutants.rs`** (new) — the cargo-mutants adapter, using
  the mechanism `tests/spike/mutants/MUTANTS-FINDINGS.md` verified: `cargo mutants -p
  <target> --test-package <harness> --re <fn> --copy-target true --no-times -t <secs> --
  <fn>_harness::`. Classifies by reading cargo-mutants' own structured
  `mutants.out/{missed,caught,unviable,timeout}.txt` (one description per line) rather than
  scraping the human-readable summary line — more robust, and directly what a `--leak-dirs`
  inspection confirms those files contain. **Falsifies the mutants spike's own
  `--gitignore false` recommendation** — see "Falsified" below, this is the session's
  single most consequential finding.
- **`crates/ply-core/src/config.rs`** — `FnClaim` gained `examples: Vec<String>` (raw,
  parsed at codegen time); `validate_mutate_has_kill_signal` implements D12's own MUST
  (`mutate` with no `test`/`fuzz` in the same list is `E0504`).
- **`crates/ply-core/src/diag.rs`** — `Diagnostic` gained a `fixes: Vec<Fix>` field (`Fix {
  title, edits }`) — §8's own schema already specified this; M3 never populated it. Every
  M4 non-result diagnostic (`K0601` timeout, `E0504`, `V0505` unsupported, `W0502` weak
  spec) now carries at least one concrete `Fix`.
- **`crates/ply-cli/src/verify.rs`** — substantially restructured (three passes: discover
  + resolve effective checks + validate D12 for every fn; write the shared harness crate
  once if any fn needs it; run each fn's checks and combine). New: `default_checks_for`
  (the shape-aware routing MUST), `default_engine_timeout_secs` (Task 0), and
  `combine_fn_check_verdicts` — the *opposite* combinator from the existing `worst_of`
  (which aggregates *across* fns/components, weakest-child-wins, D6): a single fn's own
  checks list combines to its *strongest* passing verdict when nothing failed, and to the
  worst outcome when anything did (§5.4c: "a function's verdict is the strongest evidence
  its passing checks earned; a failing check is a violation regardless of what else
  passed"). New diagnostic codes (recorded in the diag-registry sense, not yet in a
  generated schema file): `P0502`/`P0601` (proptest violation/timeout), `R0502`/`R0601`
  (the `test` check's own violation/timeout — example/direct-case tests), `M0601` (mutate
  timeout). `mutate` only runs against a genuinely passing base verdict; it is skipped
  (with a `W0110`-coded note, not silently) when the fn's own `test`/`fuzz` check itself
  failed, matching cargo-mutants' own refusal to proceed past a failing baseline.
- **5 new fixtures** under `tests/fixtures/` (`fuzzbug`, `weakspec`, `strongspec`,
  `mutatetier`, `btreeset`), each pristine "before" state, same convention as M3's four.
- **5 new e2e tests** under `tests/e2e/tests/`, one per acceptance fixture (see
  "Acceptance" below).

## Acceptance, one by one

- **Seeded-bug fixture, shrunk and rendered** (`tests/fixtures/fuzzbug`,
  `fuzzbug_fixture.rs`): `seeded_bug(x) = if x == 7 { x + 1 } else { x }` against
  `ensures(|result| *result == x)`. `fuzz(256)`'s biased-small strategy
  (`prop_oneof![3 => 0..=16, 1 => any::<u32>()]`) finds and shrinks to exactly `x = 7`
  reliably (deliberately chosen inside the small-range arm's support, not the rare
  full-range one). Rendered through the *same* `contract_rt::render_cex_test` the Kani
  path uses, verified FAIL (states `postcondition` and the literal contract text
  `result == x`) then, after removing the seeded bug, the *same* test PASSES. Green.
- **Vacuous-ensures fixture flagged weak** (`tests/fixtures/weakspec`,
  `weakspec_fixture.rs`): `#[ply::ensures(|_result| true)]` — the fuzz check itself passes
  (nothing can ever violate `true`), and `mutate` finds **2/2 mutants MISSED** (`replace
  vacuous -> u32 with 0`/`with 1`), producing `W0502` with the exact wording asked for
  ("weak spec (2 surviving mutants)") plus the equivalent-mutant caveat every occurrence of
  this diagnostic now carries. The fn node's own verdict stays `fuzzed(64)` (the check
  itself genuinely passed) with a `weak-spec` *status* alongside it (D6: statuses
  propagate alongside the evidence order, they do not replace it). Green.
- **Strong-spec fixture earning `·spec-strong`** (`tests/fixtures/strongspec`,
  `strongspec_fixture.rs`): `add_small` with a `requires`-bounded domain (no overflow) and
  an exact `ensures`, `fuzz(256)` + `test` (two `examples` entries) + `mutate` together.
  4/4 mutants caught (`replace add_small -> u32 with 0`/`1`, `replace + with -`/`*`), root
  verdict **`fuzzed(256)·spec-strong`** — the literal string §1 uses as its own headline
  example. Green.
- **Mutate-without-tier fixture producing E0504** (`tests/fixtures/mutatetier`,
  `mutatetier_fixture.rs`): `checks: [mutate]` alone. Caught in config validation, before
  any engine (Kani, proptest, cargo-mutants) is invoked at all — verified directly: no
  harness crate is ever created for this fn. `E0504`'s wording names both remedies
  (`test`/`fuzz(n)`) and populates `fixes` with both as concrete suggestions. Green.
- **Kani-excluded shape earning an honest `fuzzed(n)` verdict** (`tests/fixtures/btreeset`,
  `btreeset_fixture.rs`): `count_unique(xs: &BTreeSet<u8>)`, **no `checks:` declared at
  all** — this fixture doubles as the shape-aware-default acceptance case. `BTreeSet` fails
  `is_bounded_supported` (§5.4b's own measured Kani exclusion) but passes
  `is_fuzz_supported`, so the default lands on `[fuzz(256)]`, never `[bounded(2)]` and
  never silently nothing. Verified no Kani harness (`ply_generated.rs`) was ever written
  for this fn. Root verdict `fuzzed(256)`, zero diagnostics. Green. **This is the point of
  the whole milestone**, per the M4 brief, and it holds.
- **§8 JSON envelope carries the new verdicts and diagnostics**: every fixture above is
  asserted against its real `--json` output (`serde_json::Value` field access), not
  invented — `fuzzed(n)`, the `·spec-strong` suffix (literal `\u{00b7}`, not a hyphen or
  ASCII substitute), the `weak-spec` status array entry, and the new diagnostic codes
  (`P0502`, `E0504`, `W0502`) all appear in real tool output, captured by the e2e suite.

**All six acceptance items pass.**

## Falsified / confirmed against the spec

1. **FALSIFIED — §5.4c's `--gitignore false` mutate guidance is wrong; the real
   requirement is `--copy-target true`, and the two flags cannot even be passed
   together.** This is the session's single most consequential finding, and the most
   decision-relevant one for M5. Read directly from cargo-mutants 27.1.0's own installed
   source (`copy_tree.rs::copy_tree`'s `filter_entry` closure):
   ```text
   let is_top_level_target = name == "target"
       && entry.path().parent().is_some_and(|p| p == from_path);
   ... && (copy_target || !is_top_level_target) ...
   ```
   A directory literally named `target` sitting *directly at the copy root* is pruned
   before the walk even considers `.gitignore` — unconditional on it entirely. Ply's
   harness crate lives at `<crate_dir>/target/ply/fuzz/<name>`, exactly one level inside
   the target crate's own top-level `target/`, so **every real `mutate` run hit this**:
   `cargo build failed in an unmutated tree` / the harness crate's own `Cargo.toml`
   reported missing, confirmed directly (`--leak-dirs` showed the leaked scratch tree's
   `target/` entirely absent). `--gitignore false` does nothing for this case — and
   separately, its own *default* is already off (confirmed against cargo-mutants' own test
   suite, `options.rs::gitignore_off_by_default`), so passing it was always a no-op, not
   evidence the earlier spike's placement was safe. The earlier spike's own
   `harness-genloc` fixture (`tests/spike/mutants/scoped/lib/target/ply/fuzz/`) never
   actually exercised this path: its harness sat one level *deeper* (`lib/target/...`,
   `lib` itself a subdirectory of that spike's own copy root `scoped/`), so the
   top-level-target prune never matched — an accident of that spike's fixture depth, not
   evidence of general safety at the depth Ply actually generates (one level under the
   *target crate's own* root). The fix, `--copy-target true`, cannot be combined with
   `--gitignore` at all: both are members of the same clap `ArgGroup` in cargo-mutants' own
   CLI (`error: the argument '--gitignore <GITIGNORE>' cannot be used with '--copy-target
   <COPY_TARGET>'`), confirmed directly. Since `--gitignore`'s default already matches what
   Ply wants, the adapter passes `--copy-target true` alone. **Honest cost**: this copies
   the target crate's *entire* `target/` build cache into every scratch tree cargo-mutants
   builds — measured at ~13s total (baseline + 2 trivial mutants) against a 189MB
   `target/` in the `weakspec` fixture. For a real crate with gigabytes of build cache,
   this is a real, size-dependent tax on every `mutate` run, not a rounding error — and it
   is the opposite of what the earlier spike's own `--gitignore` guidance was trying to
   avoid. §5.4c and the mutants adapter doc comment are both amended with the exact
   mechanism and this caveat.
2. **FALSIFIED — §6's flat 60s `--engine-timeout` default.** Confirmed exactly as the M4
   brief described it (docs/m3-slice-findings.md already measured this in review): a
   `bounded(8)` proof over an 8-element `Vec` needs the M3 e2e suite's own explicit `150`
   to pass reliably, not the 60s default. Fixed with a real, derived formula
   (`default_engine_timeout_secs`), not a bigger constant — see §6's amendment and the
   module doc comment for the exact reasoning and the arithmetic that reproduces 150 from
   the M3-observed floor. Confirmed working: `weakspec`/`strongspec`'s own mutate runs
   complete in 13–26s well inside the (unscaled, scalar-only) 60s secondary default; no
   fixture in this session needed the flag raised by hand.
3. **NEW — a real parsing bug in the fuzz engine adapter, caught by testing against the
   real fixture, not assumed correct.** `engines::fuzz::parse_failed_test_names`
   originally looked for libtest's per-test `---- <name> stdout ----` failure-detail
   header. Under `--nocapture` (which `run_harness_tests` always passes — load-bearing for
   the `PLY_FUZZ_HIGH_REJECT` marker to be visible on a *passing* test, since libtest
   suppresses a passing test's output without it), libtest **never emits that header at
   all** — only the final `failures:\n    <name>\n` summary block, with no detail section
   preceding it. The bug's actual observed effect: running `cargo-ply verify` against the
   `fuzzbug` fixture's buggy body reported a clean `fuzz(256)` pass — the tool silently
   reported a real, reproducible, deliberately-seeded bug as verified, the exact failure
   mode §1 exists to prevent. Caught by running the real fixture end to end (not a unit
   test in isolation) before writing the acceptance e2e test, exactly as CLAUDE.md's
   test-driven rule asks ("watch it fail, and read the failure message") — the failure
   here was silent, which is itself the finding: a parser that fails open (reports zero
   failures when it can't find what it's looking for) is a defect Ply's own house rule
   about never emitting a violation without a witness does not, by itself, catch on the
   *success* side. Fixed (parse the final summary block, unconditional on `--nocapture`);
   a regression unit test now pins the exact real output shape that broke it.
4. **NEW, confirmed** — `cargo-mutants --re <fn>` must **not** be anchored (`^fn$`): `--re`
   matches against the whole descriptive mutant name (e.g. `"src/lib.rs:8:5: replace
   vacuous -> u32 with 0"`), not the bare fn name. An anchored regex matched zero mutants
   in a real run (`Found 0 mutants to test / WARN No mutants found under the active
   filters`) before this was caught and fixed to a plain unanchored fn-name regex, matching
   the earlier spike's own usage. Known limitation, not chased further given this
   session's fixtures: an unanchored fn name can over-match a fn whose name is a substring
   of another's in the same crate (e.g. `add` inside `add_small`) — every fixture here has
   exactly one fn under test, so this never actually collided, but a real multi-fn crate
   could see `mutate` scope leak across functions. Left for whoever next touches this path
   to tighten (e.g. a word-boundary regex over the `replace <fn>` shape cargo-mutants'
   mutant names use).
5. **CONFIRMED** — the D7 renderer really does serve two consumers with zero new code in
   `contract_rt.rs` itself beyond exposing `widen` as `pub(crate)`: the fuzz engine
   produces the *same* `WitnessValue` enum Kani witnesses decode into, and
   `contract_rt::render_cex_test` renders both without caring which engine found the
   input. Verified end to end on the `fuzzbug` fixture (FAIL then PASS), not merely by
   type-checking.
6. **CONFIRMED, with a real scope cut recorded rather than hidden** — the shape-aware
   default-check routing (§5.4c's own MUST, unimplemented in M3 despite being requested
   there) is now real: `default_checks_for` picks `[bounded(2)]` only when
   `is_bounded_supported`, `[fuzz(256)]` when the shape is excluded from bounded but
   fuzz-supported, and `[]` otherwise. The `btreeset` fixture exercises this directly with
   no `checks:` key at all. M3's own flat-default risk (a check list defaulting to
   `bounded(2)` regardless of shape) is gone from the code path this session touched.

## Scope cuts, named rather than silently skipped

- **Struct-parameter ("field-by-field") fuzzing was not implemented.** The M4 brief's own
  text lists it as part of the generation shape; the *acceptance* list does not require a
  struct fixture, and Kani's harness codegen here never supported struct parameters either
  (no `kani::Arbitrary`-deriving codegen exists in `harness.rs`), so adding fuzz-only
  struct support would have created an asymmetry the shape-aware routing can't express
  cleanly without a matching Kani story. The Kani-excluded acceptance shape uses
  `BTreeSet<u8>` instead — the spec's own explicitly offered alternative ("recursive, or a
  `BTreeSet`"). This is a real, deliberate scope cut, recorded here per CLAUDE.md's scope
  rule ("say so in one line and let the user decide"), not a gap discovered later.
- **Fuzz-found witnesses are not persisted the way Kani's are.** M3's D7 witness
  persistence (`target/ply/witness/<fn>.json`, re-rendered every run regardless of that
  run's own outcome — docs/m3-slice-findings.md finding 6) has no fuzz-path equivalent
  here: a fuzz violation's rendered cex test is written only on the run that finds it. The
  `fuzzbug` e2e test's own fix (removing the seeded-bug branch entirely) happens to make
  the stale rendered test start passing anyway, because the fix makes the postcondition
  hold *unconditionally*, including for the previously-witnessed input — this is a property
  of that specific fixture's fix, not evidence the general FAIL→PASS transition holds for
  every fix shape on the fuzz path. A fix that only narrows the bug to a *different* input
  than the one already rendered would leave a stale, now-irrelevant red test behind. Left
  as a named gap for the next session: extend the same `target/ply/witness/` persistence
  convention to fuzz-found witnesses (a `<fn>_fuzz.json`, distinct path, since a fn could in
  principle declare both `bounded` and `fuzz` and must not have them clobber one shared
  file).
- **`kani_witness` is reused, unrenamed, for a proptest-found witness.** The §8 schema
  field is literally named `kani_witness`; a fuzz-found violation still populates it (with
  descriptive text noting the real source), rather than forcing a second stability-breaking
  rename so soon after the M3 one. Flagged here as real naming debt for whoever next
  revisits §8's stability rule, not silently accepted as correct.
- **`Vec`/`BTreeSet` of anything but `u8` cannot be rendered as a cex test.** `W0541` (§8's
  own escape hatch, never previously exercised in code before this session) now fires for
  real: a fuzz-found violation on such a type is reported with its raw values and no
  `cargo_test` artifact. Not exercised by any fixture here (the `btreeset` acceptance
  fixture is a clean pass, not a violation) — recorded as implemented but **NOT RUN**
  against a real failing case.
- **Direct-contract-case generation is diagonal, not a full cross product**, to keep the
  generated file small for functions with several parameters — case `i` uses each
  parameter's `i mod (that parameter's own boundary-literal count)`'th value, rather than
  every combination. This means a bug that only manifests at a *specific joint* boundary
  (e.g. `x = MAX` *and* `y = MAX` simultaneously) may not be covered by the generated
  battery alone — `fuzz`'s own random sampling is the check's actual safety net for that
  case, not the direct cases.

## NOT RUN

- Struct-parameter fuzzing (see Scope cuts).
- Fuzz-witness persistence across separate `verify` runs (see Scope cuts).
- A real fixture exercising `W0541` for an unrenderable fuzz witness (`Vec`/`BTreeSet` of a
  non-`u8` scalar) actually violating its contract.
- `mutate`'s `--re` collision risk on a multi-fn crate with substring-overlapping fn names
  (noted, not reproduced or fixed).
- The `prove` check's `engine-missing`/`W0110` path is implemented but only exercised by
  code reading (no fixture declares `checks: [prove]` in this session's suite).

## Measured costs

- **Full `cargo test --workspace`, single-threaded (`-- --test-threads=1`): 5m31.8s wall
  clock** (measured, not estimated) — 43 `ply-core` unit tests, 5 `ply-cli` unit tests, and
  10 e2e tests (5 from M3 + 5 new from M4) all green, zero warnings on a fresh `cargo check
  --workspace --tests`. The bulk of the time is still Kani wall-clock (the `clamp`/`passing`/
  `timeout`/`vecbound` fixtures alone account for ~256s of the ~332s of e2e time); the five
  new M4 fixtures added ~85s combined (proptest + cargo-mutants), well within the same
  order of magnitude as the existing Kani-driven suite, not a new dominant cost.
- `weakspec` mutate run: 24.15s (2 trivial mutants, `--copy-target true` copying the
  fixture's own ~189MB `target/`). `strongspec` mutate run: 25.49s (4 mutants, same
  `--copy-target` cost). `fuzzbug` (fuzz-only, one shrunk failure + fix + re-verify): 11.79s
  total for the whole e2e test (two full `verify` invocations). `btreeset` (fuzz-only, clean
  pass): 11.32s. `mutatetier` (E0504, no engine invoked at all): 0.07s.

## What the next M4/M5 session should pick up

1. Decide the fuzz-witness persistence gap (Scope cuts item 2) as a real design choice —
   probably a `<fn>_fuzz.json` witness file alongside Kani's `<fn>.json`, re-rendered every
   run the same way, once D14's fuller `ply.lock` staleness story exists to potentially
   subsume both.
2. Tighten `mutate`'s `--re` regex past a bare substring match once a multi-fn crate is a
   real target (word-boundary match on the `replace <fn>` shape, or pass `--file` to scope
   to the exact source file too).
3. Struct-parameter fuzzing, if a vetting scenario ever needs it — would also motivate
   giving Kani's `bounded` path the matching struct-Arbitrary codegen it currently lacks,
   so the two gates stay honestly comparable.
4. Exercise `W0541` against a real `Vec<i32>`/`BTreeSet<i32>`-shaped violation to confirm
   the witness-only-no-cargo_test path end to end, not just by code reading.
5. The `--copy-target true` cost (finding 1 above) is the one item most worth act ing on
   before `mutate` sees a real, large target crate: either accept the per-run `target/`
   copy cost as the price of this placement, or revisit the harness crate's location
   (outside `target/` entirely, with its own `.gitignore` entry) to avoid it altogether.
