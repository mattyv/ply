# External systems and actors — adoption

Session 2026-08-24, implementing `docs/plans/external-elements.md` (reduced form,
9b324ad) against its own gate: vetting 003 re-run first, spec amended only if the
frame-crossing invariant and the real picture both held. Toolchain unchanged from
the M4 session (rustc/cargo 1.94.1, edition 2024) — this work touches `tools/` and
`vetting/` only, never `crates/`, and never `tools/kernel` (externals are not nodes
of the verdict tree, so the enumeration gate is untouched by construction).

## Verdict: gate passed

The frame-crossing invariant is green and mutation-tested (below); the rendered
picture reads correctly on inspection (both committed 003 SVGs, reviewed as PNG
renders). `The-Ply-Spec.md` is amended: §5.1 (structure + example), §5.1a rule 6,
§5.3 (external edges), §7.1 (two table rows + the dash-channel restatement), §7.2
(the fourth kind of unspecified). No fallback to the flag-only form was needed.

## What landed

**`tools/model`** — `External { note: String }` (required field, so a bare name is
a parse error) and an `externals: IndexMap<String, External>` map on `Document`;
`entry: Vec<String>` on `FnClaim`. No new micro-syntax: `~>`/`->` edge strings
already parsed identically: whether a token names a component or an external is a
resolution question, not a grammar one.

**`tools/check`** — five new document-local rules, all document-local (no anchors,
so no interaction with E0301 staleness):

| Code | Severity | Fires on |
|---|---|---|
| `E0202` | error | an external's name collides with a top-level component's |
| `E0207` | error | a `->` call edge or a `deny` pattern names an external |
| `E0208` | error | `external ~> external` (no workspace endpoint) |
| `E0209` | error | `entry:` names an external that was never declared |
| `W0410` | advisory | an external named by no `~>` edge and no `entry:` list |

**`tools/render`** — the bulk of the session:

- An external draws as a solid-bordered, unfilled box (`.external-box`, a fresh
  CSS class — deliberately *not* riding on any `ceiling-*` fill, since externals
  are never on the verdict scale) in a band below the workspace frame, laid out
  left to right in `externals:` declaration order. A document with none renders
  byte-identical to before (checked: the whole band/edge computation collapses to
  `0.0`/empty on that path, and `collapse::default_output_is_unchanged_without_
  flags` pins it against 001 and 002's committed SVGs, which changed for the
  frame-stroke reason below and nothing else).
- Every `~>` edge with exactly one external endpoint, and every derived `entry:`
  edge, is routed around whatever real components stand between its two ends —
  reusing the deny wildcard's existing "step around a combined obstruction span"
  algorithm, but through a new function (`route_around_to_external`, `svg.rs`) and
  a wider obstacle set (every component's rect, nested ones included, not just
  top-level — see "Two renderer bugs" below for why). Two edges sharing a
  (workspace component, external) pair fan into parallel lanes the same way two
  ordinary edges between the same pair already do.
- The workspace frame's own border went from a plain 1px line to `stroke-width:
  2.5` (`.workspace-frame`). This is the one place the proposal explicitly left
  as a live renderer judgment (§4 point 3: "the frame border must survive the
  squint test as a boundary, not a decoration... that is a renderer judgment the
  implementation makes"). Made here, because with real content now sitting
  outside it, the frame stops being merely the canvas edge and starts being a
  drawn boundary — and a hairline didn't read as one under a squint test on the
  real picture.
- Tooltips: the external box uses the proposal's own wording verbatim (§4):
  `"⟨name⟩ — a system or person outside this codebase: ⟨note⟩. Ply draws it so
  the boundary is visible, but checks nothing about it — every arrow touching it
  is a declaration, not a verified fact."` An explicit `~>` edge touching one
  keeps the existing flow-edge sentence and appends one newbie-bar clause naming
  which side is external and why it matters. The derived `entry:` edge's tooltip
  names the fn and the external, then either lists each `requires` clause as an
  environmental assumption or says plainly there is nothing to assume yet.

## The invariant, red first

`tools/render/tests/render.rs`, module `frame_boundary`, single test
`no_external_box_intersects_the_frame_deny_wildcards_stay_inside_and_external_
edges_cross_once` — the three clauses the task specified, walked against real
rendered output (not spot-checked), same family as the existing
`no_drawn_element_intersects_a_box_it_is_not_inside`.

Watched fail before any renderer code existed for the feature: with the fixture
(`tools/render/tests/fixtures/externals.ply.yaml`) written and the model/check
layers already accepting `externals:`/`entry:`, the test's own vacuous-pass guards
caught the *absence* of the feature correctly —

```
no external box intersects the frame — this test would pass vacuously
```

not a compile error, confirming the fixture was right and the renderer genuinely
had nothing yet. After the renderer changes landed, the same run went green on
its own without further changes to the test.

**Mutation-tested**, three separate mutations, each reverted after confirming red:

1. `EXTERNAL_BAND_GAP` set to a large negative number (band drifts off-canvas
   entirely) — did **not** trip the invariant (the band moved somewhere the frame
   rect doesn't reach either, a reminder that this mutation was too blunt) — the
   real regression test was mutation 2.
2. `external_band_y` hard-set to `frame_content_h / 2.0` (band shoved inside the
   frame) — **red**, both the box-intersection clause and the zero-crossing
   clause, on `tests/fixtures/externals.ply.yaml`.
3. `any_x_from` hard-set to `-500.0` (deny `*` node pushed outside the frame) —
   **red**, the "stays inside" clause, on `../../vetting/003-trading-system.ply.yaml`
   (three separate wildcard nodes, all correctly flagged).

Both real mutations reverted; the test returned to green (confirmed by re-running
after each revert, not assumed).

## Two renderer bugs the real picture found, not designed in ahead of time

Both were caught by the *existing* invariant
(`no_drawn_element_intersects_a_box_it_is_not_inside`) going red against the real
003 fixture once external routing was wired in — exactly the role that invariant
exists to play, extended to a new construct without any change to the invariant
itself.

1. **Wrong rail side.** The deny-line routing this work reused
   (`route_deny_line`) picks whichever rail (top or bottom of the combined
   obstruction span) is closer to the *midpoint* of the two endpoints — correct
   for a deny wildcard, which can plausibly sit on either side of its target, but
   wrong for an external edge, whose external end is *always* below the frame. On
   `venue ~> ingest.feed` (the longest edge in the scenario — `ingest.feed` sits
   at the very top of the frame, `venue` below the bottom), the naive midpoint sat
   far enough up that the router picked the top rail, and the final leg cut back
   down through `risk`'s box on the way to `venue`. Fixed with a dedicated
   function, `route_around_to_external`, that always picks the rail nearer `to`
   (always the external side by construction) rather than the pair's midpoint —
   `route_deny_line` itself is untouched, so the existing deny-rendering tests
   needed no changes and none regressed.
2. **Obstruction filter too narrow.** The same function's obstruction filter
   originally matched `route_deny_line`'s: only components overlapping the
   *original* two endpoints' own narrow bounding box. That cleared `ring` (near
   `ingest.feed`) by widening the routing corridor just far enough for `ring`
   alone, then reused that same corridor X for the *entire* vertical run — far
   enough down to clip `pnl`, a box the narrow filter never considered because
   `pnl` doesn't overlap `ring`'s rank at all. Fixed by filtering on Y-overlap
   against the *full* vertical travel range instead of X-overlap against the
   endpoints' own span: any component the route's corridor merely passes *by*
   now counts, not only the ones near its two ends.

Both fixes are scoped to the new `route_around_to_external` function; `render_deny`
and `route_deny_line` are byte-for-byte unchanged, and the full existing render
test suite (including every deny-geometry test) stayed green throughout.

## Applying it to vetting 003

`vetting/003-trading-system.ply.yaml`: added `externals: { venue: { note: ... } }`;
`gateway ~> venue : FixMessage` (egress), `venue ~> gateway : Fill` (return),
`venue ~> ingest.feed : RawFrame` (ingress — closes the previously-dangling start
of the left column); `entry: [venue]` on `Oms::submit`. `ply-check` on the result:
zero diagnostics (verified — `cargo run -p ply-check -- vetting/003-trading-
system.ply.yaml` exits 0, and `clean_render_fixtures_produce_no_diagnostics` now
covers the fixture too).

Numbered finding 5 and an "external-elements gate" section added to
`vetting/003-trading-system.md`, recording the gap this addresses (the four
constructs that used to orbit venue with no referent: the `trusted` claim, the
deny wall, and open decisions #8/#9) and the gate's own outcome.

### The two committed SVGs — why each changed

Both regenerated with `ply-render` (no other tool touched them), diffed against
the prior committed files, and the diff reviewed line by line before accepting —
not blind-accepted.

- **`vetting/003-trading-system-full.svg`** (`ply-render vetting/003-trading-
  system.ply.yaml`, no flags): canvas height grows (1684 → 1772) for the new
  external band; four new edge groups (`FixMessage`, the return `Fill`, the
  `RawFrame` ingress — the long one, routed up the right margin past every
  intervening rank — and the derived `entry` edge from `Oms::submit`); one new
  `external` box for `venue`; the frame's `stroke-width:2.5`. Nothing else in the
  diff — every existing box, edge, and label is byte-identical.
- **`vetting/003-trading-system.svg`** (`ply-render --collapse ingest ...`, the
  scenario's canonical "system view"): same four additions, plus the `RawFrame`
  edge's workspace end correctly reattaches to the *collapsed* `ingest` box
  (§7.1's existing collapse-reattachment rule, which needed no change to support
  this — the fallback `positions_of_fns` lookup already falls through to the
  collapsed ancestor's rect when a folded fn's own chip isn't drawn).
- **`vetting/001-spsc-disruptor.svg`**, **`vetting/002-ingest-pipeline.svg`**: the
  *only* diff in each is the frame `stroke-width` change above — confirmed by
  diffing before copying the regenerated file into place. Neither scenario
  declares any external, so nothing else about them could change (the band/edge
  computation is a no-op on an empty `externals:` map, by construction, and this
  is exactly what `collapse::default_output_is_unchanged_without_flags` pins).
- **`tools/render/tests/snapshots/render__disruptor_fixture_golden_snapshot.snap`**
  (insta): same single-line stroke-width diff, reviewed the same way before
  accepting via `INSTA_UPDATE=always`.

## Exact new diagnostic and tooltip wording

Pinned by tests (`tools/check/tests/externals.rs`, `tools/render/tests/render.rs`);
reproduced here for review. All pass the newbie bar: name the construct, say what's
wrong, say what to do.

- `E0207` (call edge): `"gateway -> venue" is not allowed: venue is external
  (declared under \`externals:\`), and Ply can never verify a call into code it
  cannot see — use a data-flow edge ("venue ~> other : Type") to show data
  crossing this boundary, or "entry: [venue]" on the function venue can reach`
- `E0207` (deny): same shape, `"Ply cannot enforce a ban on a system it cannot
  observe"` in place of the call-edge clause.
- `E0208`: `"venue ~> clock : TimeSync" connects two externals with nothing of
  this codebase between them: a data-flow edge needs at least one real component
  as an endpoint — Ply draws externals to show where this codebase meets the
  outside world, not to describe the outside world talking to itself`
- `E0209`: `entry: names "venue", but no external called "venue" is declared —
  add it under \`externals:\`, or check the spelling against the names declared
  there (fn Oms::submit)`
- `W0410`: `external "venue" is declared but never used: it is not named by any
  \`~>\` edge or any function's \`entry:\` list, so nothing in this document says
  how it connects — add an edge or an entry:, or remove it if it is no longer
  needed`
- `E0202` (external/component collision): `"venue" is declared twice: both as a
  component and as an external — externals share the component reference
  namespace, so every name must be unique across both`
- External box tooltip (exact, from the proposal's own §4 wording): `venue — a
  system or person outside this codebase: the exchange: accepts orders, returns
  fills; market data source. Ply draws it so the boundary is visible, but checks
  nothing about it — every arrow touching it is a declaration, not a verified
  fact.`
- Explicit `~>` edge touching an external, appended clause: `... venue is
  outside this codebase, so this arrow is a declaration, never a verified fact.`
- Derived `entry:` edge tooltip: `entry — venue can reach Oms::submit from
  outside this codebase (declared via "entry: [venue]" on Oms::submit).` followed
  by either `no requires are declared on this function, so it makes no
  environmental assumption yet.` or, for a fn with a contract, `its requires
  clauses now stand as environmental assumptions: nothing inside this codebase
  calls it, so no caller here can discharge them:` plus each clause verbatim —
  and always closing with `Ply never checks this edge — it is declared, not
  verified.`

## Test results

`cd tools && cargo test` (whole workspace): **green** — every crate, every test
file, including the new `frame_boundary` invariant, the six new `ply-check`
externals tests, the mutation-tested invariant (reverted before the final run),
and every pre-existing test (render, check, model, kernel, schedule). Re-run after
`cargo fmt` to confirm formatting changed nothing behaviorally.
`cargo fmt --check`: clean. `cargo clippy --release --all-targets -- -D warnings`:
clean (one `#[allow(clippy::too_many_arguments)]` added to `walk_component`,
matching the existing precedent on `render_deny`/`render_external_edge` for
functions whose argument count is inherent to what they thread through, not
accidental).

## NOT RUN / left for the maintainer

- **The squint test on the real picture is the maintainer's own**, per the task
  brief — I looked hard at both rendered PNGs (converted locally via `cairosvg`
  for inspection, not part of the toolchain) and judged the frame reads as a
  boundary and `venue` reads as clearly outside it, but that judgment is
  ultimately the maintainer's to make, not mine to certify.
- **`cargo ply check`/`cargo ply audit` on this repo's own `ply.yaml`** (§11's
  session-end check) — NOT RUN. This repo does not yet have a self-describing
  `ply.yaml` (the spec says that lands "from M2 onward"); nothing in `crates/`
  was touched by this session, so this is unaffected by this change, but it was
  not exercised.
- **The `entry:`/audit surface in `cargo ply` itself** — out of scope by the
  task brief (M5), not attempted, not stubbed.
- **Rendering with more than one external, or externals whose band would exceed
  the frame's own width** — the layout code has a defensive `.max()` for the
  width-overflow case, but no fixture exercises it; only vetting 003's single
  `venue` and the small dedicated test fixture (also one external) were checked
  against the frame-crossing invariant. Untested, not unimplemented.
- **Visual overlap between two or more external-touching edges that don't share
  a (component, external) pair** (so the existing lane-fanning doesn't apply) —
  not a case that arises in vetting 003 (only one external, and its three
  workspace counterparts are all distinct pairs), and not covered by any
  invariant; a real second external, or a second edge to the same external from
  a *different* component, could plausibly need this and hasn't been checked.

## TODO.md

Ticked in the same session, per CLAUDE.md; see that file for the entry and its
commit hash.
