# M0 feasibility spike — findings

Run 2026-08-23. Toolchain pinned as observed: **cargo-kani 0.67.0, CBMC 6.8.0**,
Kani's own `nightly-2025-11-21` toolchain, macOS aarch64. Every invocation used
`--harness-timeout 300s`; the slowest single harness was ~21s (playback generation).

Fixtures: `fixture/` (items 1–4, 6–9) and `fixture_callee/` (item 5), each with its own
`[workspace]` table so neither joins `tools/`. Re-run everything with `./run.sh`.

| # | Item | Verdict | Flags needed |
|---|---|---|---|
| 1 | Private free fn, `proof_for_contract` | works | `-Z function-contracts` |
| 2 | Contracted method (`&self`, `old()`) | works | `-Z function-contracts` |
| 3 | User-defined struct input | works for public fields; private-field invariant types need a hand-written `Arbitrary` | `-Z function-contracts` |
| 4 | Same-crate `stub_verified` | **works, but unenforced** | `-Z function-contracts -Z stubbing` |
| 5 | Cross-crate callee | fails naively; **sound workaround exists** | `-Z function-contracts -Z stubbing` |
| 6 | Real violation + playback | works (detect ~0.05s, witness 18–23s) | `-Z concrete-playback` |
| 7 | Cex as a plain `#[test]` | possible; needs real per-case judgment | — |
| 8 | `cfg_attr` emission, both directions | works | — |
| 9 | In-crate `#[cfg(kani)]` proof module | works | — |

## The three that matter

### 1. `stub_verified` has no soundness backstop (item 4)

Kani's only enforcement is a **compile-time check that a `#[proof_for_contract]` harness
exists** for the stub target. It never checks that the harness ran, or passed.

Demonstrated: `check_f_stubs_g` verifies in a fresh process with `check_g` never run —
confirmed independently by hand, not just by the spike. Then, with `g`'s `ensures`
deliberately falsified, the caller's proof still compiled and ran; it failed only because
that particular false value happened to break `f`'s own contract too. Had `f` tolerated
it, Kani would have reported SUCCESS on a false premise, silently.

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

### 3. The toy-type caveat that undercuts the clean scoreboard

Every fixture here used scalars and small structs. On the same day, Ply's own verdict
kernel — 300 lines, pure, no I/O — could not be proved at all: CBMC unwinds `BTreeMap`'s
clone algorithm without bound on its `BTreeSet`/`Vec` fields, and no unwind bound, solver,
or object-bits setting changed that (see `tools/kernel/src/lib.rs`).

So this spike shows the **mechanisms** work. It does not show Kani scales to
collection-shaped code, which is what real targets look like. That gap is the open risk
in routing `bounded` checks to Kani, and it deserves its own spike before M3 commits.

## Smaller findings, each with a cost

- **Private-field invariant types** (item 3b) need a hand-written `kani::Arbitrary` that
  can only be written where the field is visible — inside the crate. D2's "verify `pub`
  items from a sibling harness crate" fallback cannot construct witnesses for these at
  all. That is precisely the shape smart constructors produce.
- **Playback needs `--lib`**; without it, doc-tests run and fail on a sysroot mismatch
  introduced by playback's own toolchain swap.
- **Witness generation is not free**: 18–23s for a single-`u8` harness, versus 0.05s to
  detect the violation. A "generate playback on every failure" policy needs a budget.
- **Counterexample → readable test is not mechanical** (items 6–7). The raw witness for
  `saturating_bump(255)` reproduces an *overflow inside the ensures closure*; stating the
  actual defect ("the contract claims 256, the function returns 255") required widening
  to `u16` by hand. Generated tests will often say what panicked, not what was wrong.

## Spec amendments this forces

1. **D5** — state plainly that Kani provides no enforcement of callee-before-caller
   ordering; Ply's scheduler is the whole guarantee.
2. **D2** — the sibling-crate fallback is a hard wall for private-field invariant types,
   not merely reduced coverage.
3. **D2/D5** — cross-crate callees are supported via caller-local re-proof; decide
   whether M3 generates those automatically, and record the no-caching cost.
4. **M3** — playback runner always passes `--lib`; witness generation needs a stated
   budget or a lazy policy.
5. **M3/M4** — counterexample-to-test rendering needs per-case judgment; the spec should
   not imply it is mechanical.
6. **§10 M0** — cargo-mutants with a custom test command was in M0's own list and remains
   untested. M0 is not fully discharged until it is.
