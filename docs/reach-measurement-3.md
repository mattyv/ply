# What stops `bounded` on Ply's own claims

*Measurement, 2026-09-07. The method of `docs/invariant-reachability.md` and
`docs/reach-measurement-2.md`, turned on this repository instead of an outside library.*

*Ran: every one of Ply's own 60 declarations flipped from `fuzz(256)` to `bounded(2)`, then
`cargo-ply verify crates/ply-core --json --engine-timeout 20` — so the blockers are Ply's
own refusals, not a hand classification of types. The edit was scratch and is reverted; no
product code changed.*

## Result

**Zero of 56 claims earn `bounded`.**

| outcome | claims |
|---|---|
| refused at the parameter gate (`V0508`) | 50 |
| reached Kani, adapter saw no verdict line (`X0901`) | 4 |
| earned `tested` from the `test` check beside it | 2 |
| **earned `bounded`** | **0** |

## The ranking

| capability | claims it blocks | unblocks alone |
|---|---|---|
| **`String` parameters** | **42** | **39** |
| `[String]` | 4 | 0 |
| `Level` (a crate enum) | 2 | 1 |
| `[HarnessModule]`, `[(String, String)]`, `[(String, u32)]`, `BTreeMap<usize, BTreeSet<usize>>`, `BTreeSet<usize>`, `f64`, `FieldShape` | 1 each | 1 (`[HarnessModule]`) |

Chain depth is **1 for 44 of the 50** refusals, 2 for four, 3 for two.

**This is the first of the three measurements where any single capability unblocks anything
alone**, and the reason is the subject rather than the tool: `semver`'s properties chained
two to four blockers deep because its types are built from each other, while Ply is a text
processor whose functions almost all take one or two strings and nothing else. The previous
two rankings put floats first and `&str` first; on this codebase floats block one claim and
`&str` appears nowhere, because Ply's own signatures take owned `String`.

## Why shipping the top entry would still be wrong

A `String` Kani can build is a **length-bounded** one. The claims it would unblock parse
`Cargo.toml`, walk Rust source, and classify type names — so the proposition earned would be
"holds for every string up to four bytes", stated in a word that sounds stronger than the
`fuzzed(256)` it replaced while asking a far narrower question. That is the failure already
recorded in this repository under the float/`i128` bug: `bounded` there "would have
exhaustively proved the *rewritten integer* comparison and reported a stronger, more
confident wrong answer".

**The four claims that got past the gate say the same thing independently.** All four return
a heap string built with `format!`, and all four produced no verdict at all. Kani itself is
healthy here — `kani 0.67.0`, symbolic `u32` arithmetic verified in **0.70s**. The same
crate, with the body changed to a single `format!` over a symbolic `u32` and an assertion
that the result is non-empty, was **killed at 900s with no verdict**. Unblocking the
parameter would hand those claims to an engine that does not finish on their bodies.

## What this says about proving components

The same wall, one level up. A component proof needs a backend that can reason about the
component's real operations; Verus can, but only over the subset it supports, and
string-shaped code is outside what the bounded tier finishes today. A component-invariant
proof is therefore honest for components *written to be provable* and refuses otherwise —
it does not retroactively make this codebase provable.

## Honesty conditions on this measurement

- It measures **declared claims**, not properties an independent author wrote down. The two
  earlier measurements deliberately used outside libraries to avoid grading our own homework;
  this one grades our own homework on purpose, because the question asked was about Ply's own
  document.
- `bounded(2)` was the depth used throughout. A different depth changes runtime, not which
  shapes are refused: the 50 refusals never reach the solver.
- The three claims whose refusal text this analysis could not parse are counted as refused
  and excluded from the ranking.
