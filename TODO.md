# TODO

- [x] `ply-render --depth N` / `--focus` / `--collapse <component>` (8d8910f) —
      collapsed box shows contents line, rolled-up capability badges, pin and finding
      counts; edges reattach; default output byte-identical without flags.
- [x] Collapsed boxes draw as a stack (dc1ad4b, repaired in 26cdeb6); 003's canonical
      artifact is now the collapsed system view, full depth moved to -full.svg.
- [ ] **Color SVG config** — make the renderer's palette configurable (the style
      constants: ceiling scale, finding red, ink, amber) instead of hardcoded; must
      keep the §7.1 channel discipline (a config can retune a hue, not repurpose a
      channel) and the style-rule invariant test.
- [ ] `ply-render --legend` — opt-in legend strip below the frame, generated from the
      live style constants (§7.1, specced 2026-08-23).
- [x] `W0409` redundant parent-to-descendant edge lint (7d4c6fc) — both directions,
      both edge kinds; brought a W-warns/E-fails severity model with it.
- [x] Edge and deny routing + collision-freedom invariant (b3da43c, 2b07bd0) — 003
      render findings 1, 3, 4 closed. KNOWN GAP left open deliberately: deny lines in
      *different* margin columns can still cross (repro:
      tools/render/tests/fixtures/deny_stress.ply.yaml). Needs a routing policy
      decision (§7.1), not a guess.
- [ ] Gate debt (§7.1): DRAW the three assigned-but-undrawn forms — `strict` corner
      notch, `mode: synth` violet chip, `examples` e×N token.

## From the external review (codex, 2026-08-23) — see docs/review-2026-08-23.md

- [ ] **M0 spike + ADR-0003 first.** The review's central charge: the project sequence
      is backwards — substantial renderer/kernel work exists while the feasibility
      spike the spec itself gates everything on (§10 M0, D13) has never run. One
      vertical slice: inline contract → Kani run → parsed counterexample → replayable
      test → JSON diagnostic.
- [x] **Real defect fixed: component-level `checks` inheritance** (merged from
      worktree): a fn's own list wins entirely; otherwise it inherits the nearest
      ancestor component's default. Resolution lives once in `ply-model` so the
      validator and renderer cannot drift. Tooltips now name the source —
      "inherited from component `pricing`: bounded(2) — …". E0504 evaluates the
      effective list. All five committed SVGs byte-identical (no vetting document
      uses component defaults — grep-confirmed, not assumed).
- [x] Engine-limit diagnostics specced (52222ab) — §8 now requires timeout/unsupported
      to name the cause and populate `fixes`, with the boundary written in: Ply
      proposes, never rewrites. IMPLEMENTATION still owed when the engines are wired.
- [ ] `schema/ply.schema.json` is called normative in §5/D3 and does not exist —
      build it or cut the claim.
- [ ] Separate declared ceilings from earned verdicts in the type system (both are
      `Evidence` today; only convention keeps them apart).
- [ ] `trusted` claims are unrestricted prose — no identity, date, commit, scope, or
      expiry. The shield can read as approval.
- [ ] `conditional` assumptions are free-form strings, untied to the call graph.
- [ ] Kani harnesses do not terminate (e46e4a9): CBMC unwinds BTreeMap's generic clone
      on every recursive `aggregate_raw` call. Kani's docs confirm heap collections
      blow up the encoding AND that generic std methods cannot be stubbed — so the
      documented workaround does not apply directly. ATTEMPT 1 (done): statuses are
      now a `StatusSet` bitmask instead of a BTreeSet — behaviour identical (991k-tree
      enumeration green, untouched assertions) and 40% faster, and Kani moved from an
      indefinite hang to a deterministic timeout at 5 min. Still no verdict: CBMC now
      stalls one field over, sorting/dedup'ing `Option<Vec<String>>` assumptions.
      Deliberately NOT collapsing assumptions to a count — D5 and the newbie-bar rule
      need callers to read them verbatim, so that would narrow what a passing proof
      means. Untried: `-Z stubbing` of the String sort/compare for the harness only.
      **Bigger implication for the project:** Ply routes `bounded` checks to Kani; if
      Kani struggles this much with std collections in a 300-line pure module, the
      supported-signature story (§5.4b) is optimistic. This is exactly the kind of
      engine limit M0 exists to find — more evidence for doing M0 next.
- [ ] Renderer CLI entry point has zero test coverage (main.rs; the library is
      covered).
