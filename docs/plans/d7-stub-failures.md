# Plan — D7 and stubbed crossings: the repair target is a contract, not a test

Status: **planned, not started.** Origin: the Kani-pin spike
(`tests/spike/kani-pin/FINDINGS.md`, 2026-08-25), which established that this gap is not
a version problem and cannot be closed by bumping the engine.

## 1. Problem

§5.5's boundary rule stubs a callee with its declared contract. When the caller then
fails, the failure is *caused by a value the stub invented* — a value the real callee
never returns.

D7 promises that whenever a witness's inputs render as stable Rust source, Ply emits a
plain `#[test]` that fails under `cargo test`. **For a stubbed crossing that promise is
false, and the spike proved it**: the inputs render fine, the test is emitted, and it
**passes** — because it calls the real callee, which never produces the stub's value.
That test is in `tests/fixtures/boundarycontract` today and it is green.

This is not fixable by version. `cargo kani playback` does not re-apply stubs (Kani's own
`test_generator.rs` and generated doc comment say so), and the behaviour is identical on
Kani `main`, whose docs claim the opposite and are contradicted by their own source.

## 2. Why no red test is possible, and why we must not fake one

To reproduce a stub-caused failure in ordinary Rust, the caller would have to call
something other than the real callee. Rust offers no seam for that without editing the
user's source.

Rendering the test against a *modified copy* of the caller's body — the call replaced by
the constant — would go red, and would be dishonest: it would fail for a program the user
does not run, while looking exactly like D7's genuine red tests. That is the failure mode
this project has now caught four times, and it crosses §8's stated boundary ("Ply
proposes, never rewrites"). **Refused.**

## 3. The reframe: two kinds of falsification, two repair targets

A stubbed failure is a different *kind* of finding from a body failure, and wants a
different artifact:

| falsification | what is wrong | repair target | artifact |
|---|---|---|---|
| body violates its own contract | the code | the code | a red `#[test]` (D7 as it stands) |
| caller fails under a stubbed callee | the **declared contract is too weak** — it permits a value the caller cannot survive | the **contract** | the fabricated value + a proposed tightening |

The caller is not necessarily buggy: it is correct for every value the real callee
produces, and incorrect for some value the *declaration* permits. Handing back a test
would be answering a question nobody asked. What the reader needs is: *this declared
`ensures` admits X; at X the caller breaks its own postcondition.*

The witness already carries X — the spike confirmed the fabricated callee return is
present in the concrete-playback output and that `extract_witness_bytes` accepts it. The
information is in hand; only its presentation is missing.

**Precedent.** D7 made exactly this move once already: Kani's playback does not evaluate
contract closures, so an `ensures` violation replayed green, and the answer was for Ply to
render its own assertion rather than fight the engine. Same instinct here; different
conclusion, because this time the thing to repair lives upstream of the function tested.

## 4. What to build

Small, and the plumbing exists.

1. **A third `W0541` reason.** §8 already defines a structured `reason`
   (`inputs_unrenderable`, with `expression_unrenderable` reserved). Add
   `stub_substituted`: the inputs rendered, but no red test is possible because the
   failure depends on a stubbed callee's return. "No red test here" becomes a stated,
   distinguishable fact rather than a silent absence.
2. **Carry the fabricated value in the diagnostic** — which callee, what it was made to
   return, and which declared clause admitted that value.
3. **Populate `fixes`** with the proposed tightening, as §8 already requires of
   non-results: narrow the declared `ensures` to what the callee really guarantees, or
   make the caller defensive at the boundary.
4. **Do not emit a passing `ply_cex_*` test.** Today one is written and stays green,
   which is worse than none: it reads as a reproduction that reproduces nothing. Suppress
   it and say why.

## 5. The neighbouring honesty gap (recorded, not solved here)

The spike also found that `boundarycontract`'s *clean* proof verifies with the assumption
deleted entirely, because the body's own `.min` clamp carries it. So a passing
`conditional` verdict does not establish that the assumption was load-bearing.

That is the same lesson from the other side — what matters at a boundary is whether the
caller survives everything the contract *permits* — but it is a distinct defect
(a `conditional` that overstates its own dependence) and belongs with the vacuity work
already recorded as a KNOWN GAP, not here.

## 6. Spec deltas

1. **D7** — qualify the red-test promise: it holds for failures arising in the function's
   own body; a failure that depends on a stubbed callee's invented return has no faithful
   plain-Rust reproduction, and reports `W0541 stub_substituted` with the fabricated value
   and a proposed contract fix instead. (Applied now — the unqualified claim is false
   today.)
2. **§8** — add `stub_substituted` to the `reason` enum and state the artifact it implies.
3. **§9** — the cex validity oracle must not require a red test for this class; it must
   require the diagnostic to name the callee, the value, and the admitting clause.
