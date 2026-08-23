# Vetting 004 — top-down authoring (sketch first, solidify later)

Scenario: work the way an architect actually starts — draw the boxes before any code
or claims exist, commit to structure, then descend and fill in. This vets whether the
grammar supports *sketch-first* authoring, and whether the diagram tells the truth at
every stage of solidifying. Domain: a settlement service (fresh ground — first use of
the `fs` capability).

Two canonical YAMLs, two stages of the same system:

- [004-topdown-sketch.ply.yaml](004-topdown-sketch.ply.yaml) — the pure sketch: four
  components with anchors only (all **hollow** — no fns, no nesting), the full edge
  and deny structure, and the open decisions as `unresolved` registry entries. This is
  a complete, valid Ply document that declares *architecture without any claims*.
- [004-topdown-filled.ply.yaml](004-topdown-filled.ply.yaml) — one step later:
  `ledger` (the riskiest component — it owns the money) solidifies first, with `owns`,
  `strict`, a double-entry contract using `old()` (posting preserves the balance total
  and appends exactly one entry), and the heaviest checks. Everything else stays
  hollow; decision #11 (who owns the fs question) is resolved by filling ledger and
  drops out.

## What this probes (and 001–003 could not)

1. **Hollow as a first-class stage.** Every box in the sketch should draw dashed with
   an unclaimed-white fill — the picture of an honest sketch: all structure, zero
   promises. The §7.1 hollow form gets its first full-document workout.
2. **Deny rules before any code.** Can the safety story be committed before any fn
   exists? The sketch already forbids `* -> ledger` (except matcher and reporting) and
   `ledger -> gateway` (the money never talks to the network).
3. **The solidify diff.** Stage 2 minus stage 1 should read as pure addition — fill
   one component, delete one resolved decision — with no structural rewrites. If
   solidifying forces edits elsewhere, that's a finding.
4. **Ceilings across stages.** Sketch: everything unclaimed. Filled: `ledger` alone
   turns deep green while its neighbors stay dashed white — the diagram should make
   "we hardened the money first" legible at a squint.

## Runs

Pending — the render and check crates are mid-work (collapse flags, W0409). To run:
`ply-check` both YAMLs (both should pass clean; hollow is legal), render both, and
record findings here. Nothing below this line is verified yet.
