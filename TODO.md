# TODO

- [ ] **NEXT** `ply-render --depth N` / `--focus <component>` — collapse/expand per
      §7.1: collapsed box = solid border, contents line, worst-descendant fill;
      capability badges, pin count, and finding count never fold away. Acceptance: 003
      at --depth 1 shows ingest as one white box still wearing the subtree's `net` and
      `unsafe` badges. (Starts when the in-flight hollow/gutter agent lands — same
      crate.)
- [ ] Collapsed boxes draw as a stack (offset card edge behind — §7.1, specced):
      flat = fully shown, stacked = folded content, dashed = nothing inside. Also:
      swap 003's committed artifacts (collapsed view becomes 003-trading-system.svg,
      full depth moves to -full.svg; repoint the default-output regression guard).
- [ ] **Color SVG config** — make the renderer's palette configurable (the style
      constants: ceiling scale, finding red, ink, amber) instead of hardcoded; must
      keep the §7.1 channel discipline (a config can retune a hue, not repurpose a
      channel) and the style-rule invariant test.
- [ ] `ply-render --legend` — opt-in legend strip below the frame, generated from the
      live style constants (§7.1, specced 2026-08-23).
- [ ] `W0409` in ply-check — redundant parent-to-descendant edge lint (§5.3, specced;
      renderer draws nothing for such edges).
- [ ] Cross-container edge routing — edges into another component's descendant
      (`strategy -> ingest.book`) routed cleanly; 003 render findings 1, 3, 4
      (intra-container label collisions, same-rank deny overlap, bar striking except
      text) plus a collision-freedom invariant test.
- [ ] Gate debt (§7.1): DRAW the three assigned-but-undrawn forms — `strict` corner
      notch, `mode: synth` violet chip, `examples` e×N token.

## From the external review (codex, 2026-08-23) — see docs/review-2026-08-23.md

- [ ] **M0 spike + ADR-0003 first.** The review's central charge: the project sequence
      is backwards — substantial renderer/kernel work exists while the feasibility
      spike the spec itself gates everything on (§10 M0, D13) has never run. One
      vertical slice: inline contract → Kani run → parsed counterexample → replayable
      test → JSON diagnostic.
- [ ] **Real defect: component-level `checks` inheritance is ignored.** §5.1 says
      component `checks` are defaults for fns in scope; `ply-check` explicitly does not
      merge them and the ceiling computation reads only `fc.checks`, so a declared
      ceiling can be wrong today.
- [ ] **Engine-limit diagnostics (from the Kani investigation).** When Kani times out
      because the target manipulates a std collection, Ply must say so in plain words
      rather than reporting a bare timeout — name the cause and suggest a smaller
      bound or an array-shaped path. NOTE the boundary: Ply may never reshape the
      user's data structures to suit the verifier (we did that to our own kernel;
      doing it to user code would verify a program they don't run). Stubbing stays
      restricted to D5's sound case — a callee stubbed by a contract it actually
      proved; anything weaker is `conditional` with the assumption listed.
- [ ] `schema/ply.schema.json` is called normative in §5/D3 and does not exist —
      build it or cut the claim.
- [ ] Separate declared ceilings from earned verdicts in the type system (both are
      `Evidence` today; only convention keeps them apart).
- [ ] `trusted` claims are unrestricted prose — no identity, date, commit, scope, or
      expiry. The shield can read as approval.
- [ ] `conditional` assumptions are free-form strings, untied to the call graph.
- [ ] Kani harnesses do not terminate (investigated 2026-08-23, written up in
      tools/kernel/src/lib.rs): CBMC unwinds BTreeMap's generic clone on every
      recursive `aggregate_raw` call. Untried avenue: `-Z stubbing` to stub the
      collection ops — but that changes what is actually verified, so it needs its
      own review. Meanwhile the 991k-tree enumeration is the real proof.
      **Bigger implication for the project:** Ply routes `bounded` checks to Kani; if
      Kani struggles this much with std collections in a 300-line pure module, the
      supported-signature story (§5.4b) is optimistic. This is exactly the kind of
      engine limit M0 exists to find — more evidence for doing M0 next.
- [ ] Renderer CLI entry point has zero test coverage (main.rs; the library is
      covered).
