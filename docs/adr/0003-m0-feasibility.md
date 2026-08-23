# ADR-0003 — M0 feasibility: Kani carries the plan, with three caveats

Date: 2026-08-23. Status: accepted. Adversarially re-verified 2026-08-23 (all claims
re-run from scratch; every "works" fixture mutation-tested; caveat 1 upgraded from
counterfactual to demonstrated; caveat 3 added — playback replay was overstated).
Evidence: `tests/spike/FINDINGS.md`, fixtures under `tests/spike/`, re-runnable via
`tests/spike/run.sh`. Toolchain pinned: cargo-kani 0.67.0, CBMC 6.8.0.

## Decision

Proceed with Kani as the `bounded` engine. Eight of the nine mechanisms M0 tested work;
the ninth (cross-crate stubbing) works with a documented workaround. Three findings
change what the spec may claim, and one open risk gates M3.

## What the experiment settled

Private functions, contracted methods with `old()`, user-defined inputs, same-crate
stubbing, real counterexample *detection*, the `cfg_attr` dual-compilation trick, and
the in-crate proof module all work as designed. Counterexample *replay* only partly
works — caveat 3. The plan is physically possible.

## Caveat 1 — Ply's scheduler is the entire soundness guarantee for D5

Kani checks only that a `#[proof_for_contract]` harness *exists* for a stub target, never
that it ran or passed. A caller's proof verifies happily while assuming a callee contract
nobody proved — confirmed by running the caller's harness alone in a fresh process, and
again with the callee's contract deliberately falsified. The silent-success case is
demonstrated, not inferred: with a falsified callee contract the caller *tolerates*, the
caller reports VERIFICATION SUCCESSFUL while the callee's own harness fails.

This contradicts Kani's own RFC-0009, which promises regular harnesses are verified only
"*if* their `stub_verified` contracts passed" — 0.67.0 observably does not implement that
gating: in one whole-crate `cargo kani` run, the stubbing caller ran *before* the callee's
harness and kept its SUCCESS after that harness failed. The only mitigation whole-crate
mode offers is that the callee's failure fails the overall exit code — useless for Ply,
whose product is per-function verdicts, and absent whenever the callee harness is
filtered, missing, or timed out.

Therefore: **a caller may be credited `bounded` only after its callees' contract proofs
ran and passed this run — ordering callees first is how Ply gets that, and an
implementation that relaxes it is unsound with nothing downstream to detect it.** This is
now stated in D5 rather than left implicit. It also makes the callee-ordering logic a
prime candidate for the same treatment the verdict kernel gets — a pure module with its
own invariant tests.

## Caveat 2 — the sibling-crate fallback loses `Arbitrary`, keeps a narrower route

D2 offered "verify `pub` items only from a sibling harness crate" as the fallback if
in-crate proof modules failed. They didn't fail — and the fallback's weakness needed
correcting in both directions. A type with private fields and an invariant (the shape
smart constructors produce) cannot get a sibling-crate `Arbitrary`: the fields are
invisible and the orphan rule forbids the impl anyway. But witnesses are still
constructible through the `pub` smart constructor plus `kani::assume` — verified, the
same contract proves. The real costs are no derive, per-type hand-written harness code,
and coverage capped at pub-reachable states (in-crate code can produce values the
constructor refuses — the fixture's own `bump_nonzero` yields `NonZero(0)`). A hard wall
exists only for types with no `pub` construction path.

(The cross-crate workaround itself held up under attack: with the remote callee's body
mutated to violate its contract, the caller-local re-proof fails and points at the
callee's source line — CBMC checks the real linked body. Stubbing the broken remote
callee still verifies, though: caveat 1 applies across crates too.)

## Caveat 3 — the generated playback test does not reproduce contract violations

`cargo kani playback` never evaluates the contract closures; only the real body runs. An
`ensures`-violation witness (item 6, x = 255) therefore replays through the harness and
the generated test **passes** — it is not a red reproduction, and 18–23s of witness
generation buys input storage, not a failing test. Verified both ways: moving the same
overflow into the function body makes the identical generated test fail. D7's playback
artifact must be understood as exact input storage; only the separately *rendered* plain
`#[test]` can serve as the failing repair target, and rendering it takes per-case
judgment (items 6–7).

## The open risk that gates M3

Every M0 fixture used scalars and small structs. The same day, Ply's own verdict kernel —
pure, 300 lines — could not be proved: CBMC unwinds `BTreeMap`'s clone algorithm without
bound on `BTreeSet`/`Vec` fields. Replacing the status set with a bitmask moved it from an
indefinite hang to a deterministic timeout, but produced no verdict.

So M0 proves the *mechanisms*, not the *scale*. Before M3 commits to Kani for real
targets, a second spike must establish what collection-shaped code Kani can actually
handle, and §5.4b's supported-signature story must be rewritten around that evidence
rather than around optimism.

## Not yet discharged

M0's own list includes cargo-mutants driven by a custom test command running a generated
harness. That was outside this spike and remains untested; M0 is complete only when it is.
