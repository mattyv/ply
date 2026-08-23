# TODO

- [ ] **NEXT** `ply-render --depth N` / `--focus <component>` — collapse/expand per
      §7.1: collapsed box = solid border, contents line, worst-descendant fill;
      capability badges, pin count, and finding count never fold away. Acceptance: 003
      at --depth 1 shows ingest as one white box still wearing the subtree's `net` and
      `unsafe` badges. (Starts when the in-flight hollow/gutter agent lands — same
      crate.)
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
- [ ] Gate debt (§7.1): visual forms or removal for `strict`, `mode: synth`,
      `examples`.
- [ ] Vetting 004 — top-down authoring scenario (all-hollow sketch first, then fill
      in; see memory/roadmap).
- [ ] Run the kernel's Kani harnesses once Kani is installed (`cargo kani` in
      tools/kernel).
- [ ] Renderer CLI entry point has zero test coverage (main.rs; the library is
      covered).
