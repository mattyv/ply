# ADR-0004 — C++ support: intended, out of v1, and the seams that keep it cheap

Date: 2026-08-24. Status: accepted (a scoping decision).

**Evidence status, stated first because this project's own rules require it (§10's
generalised D13): this ADR is REASONING, not measurement.** No C++ spike has run. Nothing
below may be cited as "confirmed" until one does and names its artifact. The unmeasured
list is enumerated explicitly further down rather than left for a reader to infer.

## Context

Ply's author has a C++ codebase at work, and asked whether C++ becomes a target. The
question is worth answering now — not because the work should start now, but because the
answer constrains design choices that are cheap to honour today and expensive to retrofit
later.

## Decision

1. **C++ is not in v1.** It joins the §10 out-of-scope list.
2. **Everything above the `Extractor`/`Engine` seams stays language-neutral.** No
   Rust-specific type or assumption enters `ply-core`'s model, the verdict kernel,
   `ply-schedule`, or the §8 envelope without a recorded reason. §4 already assigns this
   shape, so the cost is close to zero — the discipline is to keep it.
3. **Revisit through a spike, not a milestone commitment** — after Rust M3 and M4 land.

## What ports cleanly (reasoned)

More of Ply is language-neutral than it looks. The `ply.yaml` grammar, the verdict kernel
and its Verus proof, `ply-schedule`, the §8 envelope, and the whole §7.1 visual grammar
never touch Rust — the renderer's only input is the envelope. C++ even shares `::` path
syntax, so anchors barely change.

Two facts actively favour C++ at the engine layer:

- **CBMC is natively a C/C++ tool.** Kani is a CBMC *frontend*; the bounded tier's real
  engine already speaks C++ directly, and `__CPROVER_requires`/`__CPROVER_ensures` are
  more mature there than Kani's `-Z function-contracts` is for Rust.
- **Extraction could be *better*, not worse.** libclang performs real name resolution,
  which syn cannot (D4's whole reason for making the item tier advisory). The tier that
  is approximate-in-principle for Rust could be closer to sound for C++.

D2's dual-compilation trick also has a direct analogue: a `PLY_REQUIRES(...)` macro
expanding to nothing in an ordinary build and to CBMC annotations under verification is
the moral equivalent of `cfg_attr`. C++26 contracts may eventually give it a native home.

## What breaks — the three that matter

1. **There is no sound build-graph tier.** D4's crate tier is the one layer sound enough
   to justify default-deny, and it rests entirely on `cargo metadata`. C++ has no
   equivalent: the dependency graph lives in CMake/Bazel/meson. The best available input
   is `compile_commands.json` plus convention, which is weaker. D4's two-tier
   errors-vs-warnings split has to be re-derived for C++ and may not land in the same
   place.

2. **Undefined behaviour undermines the assumption list, which is where Ply's honesty
   lives.** Every `bounded` verdict in C++ is conditional on no UB in what it touches, on
   aliasing rules the type system does not enforce, and on initialization the analysis
   cannot see. In Rust, `conditional` carries a short, enumerable assumptions list (D5).
   In C++, enumerating them honestly is a research problem. A tool that prints
   `bounded(3)` while silently assuming away UB commits precisely the sin this project
   exists to prevent. The answer is either a new status or a standing per-verdict caveat
   in the shape of §5.4b's existing aliasing/invariant ceilings — decided at spike time,
   on evidence, not now.

3. **The supported-signature wall sits lower.** §5.4b's Rust exclusions were *measured*
   (recursive types, `BTreeSet` past one element, default-hasher `HashMap`). C++'s
   prevailing idiom — raw pointers, hand-rolled invariants, non-owning views, templates —
   is more hostile still. `check_with` covers templates the way it covers generics, but
   the fraction of real code inside the gate is likely smaller, not larger.

## What is NOT measured

Specifically unmeasured, and not to be asserted until a spike says otherwise: CBMC's
contract support on real C++ at a pinned version; whether libclang extraction resolves
cross-TU calls at useful rates; the practical cost of the UB caveat; whether
`compile_commands.json` yields a usable component graph; and the real supported-signature
fraction on a work-shaped C++ codebase.

## Consequences

Immediate, and cheap: the `Extractor` and `Engine` traits remain the only language-aware
seam, and anything Rust-specific attempting to enter the model, kernel, scheduler, or
envelope is a design smell to raise rather than route around — the same rule the kernel
already lives under.

## Revisit triggers

A decision with triggers, per the VeriFast precedent, not a dismissal:

- Rust M3 and M4 land and the generate→check→repair pattern is proven end to end.
- A C++ M0-shaped spike returns a verdict: one fixture, a CBMC contract proof, a witness,
  a libclang call graph from `compile_commands.json`, and the `PLY_REQUIRES` macro —
  roughly two sessions, held to `tests/spike/`'s discipline.
- Work adoption actually requires it. This is the strongest trigger and the reason the
  ADR exists at all.

## Alternatives considered

**Start C++ now, in parallel.** Rejected: one milestone of seven in, with `cargo ply`
only just running end to end. Widening the target before the first one is proven is the
exact failure mode the project's own sequencing rules exist to block.

**Rule it out permanently.** Rejected: the verification *budget* lives in C++-heavy
industries — trading, automotive (ISO 26262), aerospace (DO-178C), medical — which are
already obliged to produce correctness evidence and assemble it by hand today. Ply's
verdict tree, with assumption chains and a `trusted` audit surface, is closer to what
those evidence packages want to be than anything the Rust community currently needs.
