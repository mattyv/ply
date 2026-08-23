# M0 feasibility spike — findings

Run 2026-08-23. Toolchain pinned as observed: **cargo-kani 0.67.0, CBMC 6.8.0**,
Kani's own `nightly-2025-11-21` toolchain, macOS aarch64. Every invocation used
`--harness-timeout 300s`; the slowest single harness was ~21s (playback generation).

Fixtures: `fixture/` (all harnesses, including item 5's caller-side ones) and
`fixture_callee/` (item 5's remote callee only), each with its own `[workspace]` table so
neither joins `tools/`. Re-run everything with `./run.sh` (verified idempotent: two
consecutive runs exit 0 with identical verdicts and no source modification —
`--concrete-playback inplace` detects the already-present generated test and skips).

Adversarially re-verified 2026-08-23 (same toolchain): all recorded verdicts reproduce;
every "works" harness was also mutation-tested (body broken → harness fails), so none
passes vacuously. Two claims needed correction: item 3b's "sibling-crate hard wall" is
actually reduced coverage (a pub-constructor harness works), and item 6's playback test
does not reproduce contract violations (finding 3 below).

| # | Item | Verdict | Flags needed |
|---|---|---|---|
| 1 | Private free fn, `proof_for_contract` | works | `-Z function-contracts` |
| 2 | Contracted method (`&self`, `old()`) | works | `-Z function-contracts` |
| 3 | User-defined struct input | works for public fields; private-field invariant types need a hand-written `Arbitrary` or a pub-constructor + `assume` harness | `-Z function-contracts` |
| 4 | Same-crate `stub_verified` | **works, but unenforced** | `-Z function-contracts -Z stubbing` |
| 5 | Cross-crate callee | fails naively; **sound workaround exists** | `-Z function-contracts -Z stubbing` |
| 6 | Real violation + playback | detection works (~0.05s; witness 18–23s), but the generated playback test **passes** — see below | `-Z concrete-playback` |
| 7 | Cex as a plain `#[test]` | possible; needs real per-case judgment | — |
| 8 | `cfg_attr` emission, both directions | works | — |
| 9 | In-crate `#[cfg(kani)]` proof module | works | — |

## The four that matter

### 1. `stub_verified` has no soundness backstop (item 4)

Kani's only enforcement is a **compile-time check that a `#[proof_for_contract]` harness
exists** for the stub target. It never checks that the harness ran, or passed.

Demonstrated: `check_f_stubs_g` verifies in a fresh process with `check_g` never run —
confirmed independently by hand, not just by the spike. Then, with `g`'s `ensures`
deliberately falsified, the caller's proof still compiled and ran; it failed only because
that particular false value happened to break `f`'s own contract too.

The silent-success case is no longer a counterfactual: re-verified adversarially with a
*tolerant* caller (`g` body `x + 1`, `ensures x + 5` falsified; `f` `ensures x + 10`,
which follows only from the false premise). `check_f_stubs_g` reported **VERIFICATION
SUCCESSFUL, 0 of 92 failed** while `check_g` fails in the same crate.

Whole-crate `cargo kani` (no `--harness` filter) is **not** a backstop either, despite
Kani's RFC-0009 promising exactly that ("Kani verifies all regular harnesses *if* their
`stub_verified` contracts passed"): observed in 0.67.0, harnesses run in arbitrary order
— the stubbing caller ran *before* `check_g` and kept its SUCCESS after `check_g` failed
later in the same invocation. Nothing links or retracts the caller's verdict. The one
mitigation whole-crate mode does give: the callee's own harness failure is reported as an
independent failure and fails the overall exit code — provided that harness is present,
isn't filtered out, and doesn't time out. For Ply, which reports per-function verdicts,
that global exit code is not the product, so it changes nothing.

**Consequence for Ply:** D5's rule — a caller may assume a callee's contract only when
that callee itself passed a proof — is guaranteed *entirely by Ply's own scheduler*. Any
implementation that does not strictly order callees before callers is unsound by
construction, and nothing downstream will notice.

### 2. Cross-crate stubbing is possible after all (item 5)

The naive form fails exactly as the spec predicted, with a precise error: the callee
crate's `#[cfg(kani)]` proof module is invisible to the caller's build. But declaring a
second `#[kani::proof_for_contract]` harness *in the caller's own crate*, naming the
remote `pub` function by qualified path, verifies — and CBMC's trace confirms it checks
the real linked function body, so it is genuinely sound rather than a trick.

Costs: the target must be `pub`, and there is no cross-crate proof caching — every
consuming crate re-declares and re-proves the same callee.

Soundness re-verified adversarially: with `g_remote`'s *body* mutated to violate its
contract (`x + 2` against `ensures x + 1`), the caller-local
`check_g_remote_locally` **fails**, pointing at the callee crate's source line — so CBMC
really does check the real linked body, not a local shadow. (The stubbing caller
`check_f_remote_stubs_g_remote` still passed against the broken body — the same
no-backstop behaviour as item 4, now confirmed cross-crate too.)

### 3. The generated playback test passes — it does not reproduce a contract violation

`cargo kani playback` compiles with `cfg(kani)` but **never evaluates the contract
closures** — only the real function body runs. For item 6 the failing check lives *inside
the ensures closure* (`x + 1` overflows u8 at 255), so the generated
`kani_concrete_playback_*` test replays `saturating_bump(255)`, nothing panics, and the
test **passes**. Verified both ways: moving the overflow into the body (`x + 1` instead
of `saturating_add`) makes the very same generated test fail with the overflow panic.

This is a **documented limitation, not a defect**: Kani generates playback tests only
for failures that would trigger a runtime error, and captures only `kani::any()`
initialisations — contract instrumentation (havoc, `old()` snapshots, stubbed callees)
is not in the recorded value stream, so a concrete replay can legitimately diverge from
the verification trace. Noted for accuracy: the docs say Kani warns when it declines to
generate a test for such a check; no such warning appeared in our run (checked
explicitly), so Ply cannot rely on the warning to detect this case.

**The mitigation Ply must implement**, straight from Kani's own guidance: turn the
postcondition into an explicit `assert!` in the generated artifact, so the failure
becomes panic-shaped and therefore replayable. Ply generates its harnesses anyway, so
this is squarely within its control — and it is the difference between D7's promised
red test and a green one.

So playback reproduces body-level panics/UB, not contract-check failures. The witness
*input* is preserved exactly (D7's storage claim holds), but the generated test is not a
red reproduction of an `ensures` violation and cannot serve as one — which is most Ply
counterexamples. The separately *rendered* failing `#[test]` (D7's second artifact, item
7's shape) has to carry that weight alone.

### 4. The toy-type caveat that undercuts the clean scoreboard

Every fixture here used scalars and small structs. On the same day, Ply's own verdict
kernel — 300 lines, pure, no I/O — could not be proved at all: CBMC unwinds `BTreeMap`'s
clone algorithm without bound on its `BTreeSet`/`Vec` fields, and no unwind bound, solver,
or object-bits setting changed that (see `tools/kernel/src/lib.rs`).

So this spike shows the **mechanisms** work. It does not show Kani scales to
collection-shaped code, which is what real targets look like. That gap is the open risk
in routing `bounded` checks to Kani, and it deserves its own spike before M3 commits.

## Smaller findings, each with a cost

- **Private-field invariant types** (item 3b): `derive(kani::Arbitrary)` and hand-written
  `Arbitrary` impls are both crate-local (field visibility; orphan rule for a foreign
  trait on a foreign type), so D2's sibling-crate fallback gets neither. It is **not**,
  however, a hard wall: a harness using only the `pub` smart constructor
  (`NonZero::new(kani::any())` + `kani::assume(n.is_some())`) verifies the same contract
  — re-verified here, VERIFICATION SUCCESSFUL. The honest costs of the fallback are: no
  derive, per-type hand-written harness code, and witness coverage capped at what `pub`
  constructors can reach — provably narrower than what in-crate code can produce (this
  very fixture's `bump_nonzero(NonZero(-1))` yields `NonZero(0)`, a value `new` refuses,
  so a constructor-based harness never explores it). Reduced coverage, not impossibility.
- **Playback needs `--lib`**; without it, doc-tests run and fail on a sysroot mismatch
  introduced by playback's own toolchain swap.
- **Bare `cargo build`/`cargo test` is not warning-clean**: every `#[cfg_attr(kani, ...)]`
  and `#[cfg(kani)]` triggers an `unexpected_cfgs` lint (24 warnings in this small
  fixture). D1/D2's "everything works with bare cargo" needs each instrumented crate to
  carry `[lints.rust] unexpected_cfgs = { level = "warn", check-cfg = ['cfg(kani)'] }`
  (or the ply macros to emit the equivalent) — a real, if small, footprint per crate.
- **Witness generation is not free**: 18–23s for a single-`u8` harness, versus 0.05s to
  detect the violation. A "generate playback on every failure" policy needs a budget.
- **Counterexample → readable test is not mechanical** (items 6–7). The raw witness for
  `saturating_bump(255)` reproduces an *overflow inside the ensures closure*; stating the
  actual defect ("the contract claims 256, the function returns 255") required widening
  to `u16` by hand. Generated tests will often say what panicked, not what was wrong.

## Spec amendments this forces

1. **D5** — state plainly that Kani provides no enforcement of callee-before-caller
   ordering; Ply's scheduler is the whole guarantee.
2. **D2** — for private-field invariant types the sibling-crate fallback loses the
   `Arbitrary` route entirely (visibility + orphan rule) but keeps a pub-constructor +
   `assume` route (verified). It is reduced coverage after all — capped at pub-reachable
   states — not a hard wall; a wall remains only where no `pub` construction path exists.
3. **D2/D5** — cross-crate callees are supported via caller-local re-proof; decide
   whether M3 generates those automatically, and record the no-caching cost.
4. **M3** — playback runner always passes `--lib`; witness generation needs a stated
   budget or a lazy policy.
4b. **D7** — the generated playback test must not be treated as a failing reproduction:
   for `ensures` violations it passes (finding 3 above). D7's playback artifact is input
   storage only; the rendered plain `#[test]` is the only red-test artifact.
5. **M3/M4** — counterexample-to-test rendering needs per-case judgment; the spec should
   not imply it is mechanical.
6. **§10 M0** — cargo-mutants with a custom test command was in M0's own list and remains
   untested. M0 is not fully discharged until it is.
