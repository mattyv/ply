# ADR-0003 — M0 feasibility: Kani carries the plan, with two soundness caveats

Date: 2026-08-23. Status: accepted.
Evidence: `tests/spike/FINDINGS.md`, fixtures under `tests/spike/`, re-runnable via
`tests/spike/run.sh`. Toolchain pinned: cargo-kani 0.67.0, CBMC 6.8.0.

## Decision

Proceed with Kani as the `bounded` engine. Eight of the nine mechanisms M0 tested work;
the ninth (cross-crate stubbing) works with a documented workaround. Two findings change
what the spec may claim, and one open risk gates M3.

## What the experiment settled

Private functions, contracted methods with `old()`, user-defined inputs, same-crate
stubbing, real counterexamples with replay, the `cfg_attr` dual-compilation trick, and
the in-crate proof module all work as designed. The plan is physically possible.

## Caveat 1 — Ply's scheduler is the entire soundness guarantee for D5

Kani checks only that a `#[proof_for_contract]` harness *exists* for a stub target, never
that it ran or passed. A caller's proof verifies happily while assuming a callee contract
nobody proved — confirmed by running the caller's harness alone in a fresh process, and
again with the callee's contract deliberately falsified.

Therefore: **any implementation that does not strictly order callees before callers is
unsound, and nothing downstream will detect it.** This is now stated in D5 rather than
left implicit. It also makes the callee-ordering logic a prime candidate for the same
treatment the verdict kernel gets — a pure module with its own invariant tests.

## Caveat 2 — the sibling-crate fallback has a hard wall

D2 offered "verify `pub` items only from a sibling harness crate" as the fallback if
in-crate proof modules failed. They didn't fail — but the fallback is weaker than
described: a type with private fields and an invariant (the shape smart constructors
produce) needs a hand-written `Arbitrary` that can only exist where the fields are
visible. A sibling crate cannot construct witnesses for those types at all.

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
