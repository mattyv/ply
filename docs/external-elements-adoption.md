# External systems and actors — adoption

Session 2026-08-24, implementing `docs/plans/external-elements.md` (reduced form,
9b324ad) against its own gate: vetting 003 re-run first, spec amended only if the
frame-crossing invariant and the real picture both held. Toolchain unchanged from
the M4 session (rustc/cargo 1.94.1, edition 2024) — this work touches `tools/` and
`vetting/` only, never `crates/`, and never `tools/kernel` (externals are not nodes
of the verdict tree, so the enumeration gate is untouched by construction).

## Verdict: gate passed

The frame-crossing invariant is green and mutation-tested (below); the rendered
picture reads correctly on inspection — both committed 003 SVGs, first reviewed
as locally-converted PNGs, then a real regression was found on exactly this
picture by the coordinator's own review (a routed edge crossing `strategy`/
`signals`, §"Three renderer bugs" below), fixed, and re-confirmed by rasterising
both files with headless Chromium and inspecting the result by eye. `The-Ply-
Spec.md` is amended: §5.1 (structure + example), §5.1a rule 6, §5.3 (external
edges), §7.1 (two table rows + the dash-channel restatement), §7.2 (the fourth
kind of unspecified). No fallback to the flag-only form was needed.

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

## Three renderer bugs the real picture found, not designed in ahead of time

All three were caught by the *existing* invariant
(`no_drawn_element_intersects_a_box_it_is_not_inside`) going red against the real
003 fixture once external routing was wired in — exactly the role that invariant
exists to play, extended to a new construct without any change to the invariant
itself needed for bugs 1 and 2. Bug 3 is different in kind (below): the invariant
did *not* catch it on its own, because it never rendered the configuration the bug
lived in — a coverage gap in the *test*, not a blind spot in the *check* the test
performs, and it is documented in its own section further down because the
coordinator's review asked for that distinction made explicit.

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
3. **First-leg sweep, found by the coordinator's own review of the committed
   `003-trading-system.svg`** (`--collapse ingest`, the scenario's canonical
   view) — `venue ~> ingest.feed`'s routed line ran horizontally through
   `strategy` and its nested `signals` chip (visibly striking through the text
   "momentum B2 F2048" in the rasterised PNG), and `--collapse gateway`'s own
   `entry`/flow edges independently hit `pnl` the same way. Root cause: after
   bugs 1–2's fixes, the route still did "move horizontally at `from`'s own
   height to clear the combined obstruction span, *then* rise to the rail" —
   sound for `route_deny_line`, whose `from` is always a wildcard margin node
   genuinely off to the side, so that first horizontal leg never crosses
   anything on its way *to* the span. An external edge's `from` is an ordinary
   component's own border, which can sit *inside* the horizontal range other
   components occupy (`ingest`'s collapsed box, x≈564, sits squarely between
   `strategy` at x≈691 and the rest of the diagram) — so the first leg swept
   straight through whatever was in between on its way out. Fixed by trying a
   straight vertical run at `from`'s own X first (`vertical_run_is_clear`,
   `svg.rs`) and only detouring sideways when that specific run would actually
   cross something; `to`'s side needs no symmetric check because once any point
   of the route reaches the rail — below every obstacle's bottom edge by
   construction — everything further down is safe regardless of X (this is the
   same fact bug 1's fix already established, applied to the other end).

All three fixes are scoped to the new `route_around_to_external` function (and its
new helper `vertical_run_is_clear`); `render_deny` and `route_deny_line` are
byte-for-byte unchanged throughout this whole session, and the full existing
render test suite (including every deny-geometry test) stayed green at every
step.

## The coverage gap: why the invariant reported green over a real crossing

The coordinator's review found that `no_drawn_element_intersects_a_box_it_is_not_
inside` passed even though `vetting/003-trading-system.svg` — the *committed*
file, produced by `ply-render --collapse ingest` — visibly drew `venue ~>
ingest.feed` straight through `strategy`/`signals`. This was flagged as more
serious than the routing bug itself, correctly: a check that is right in
principle but never runs against the shape that breaks it is worse than an
missing check, because it *looks* covered.

**Root cause, found and confirmed, not guessed:** the invariant test rendered
every fixture exactly two ways — `render_svg(&doc)` (fully expanded) and
`render_svg_with_options(&doc, RenderOptions { depth: Some(1), .. })` (every
top-level component collapsed at once). Neither of those is the configuration
vetting 003's own canonical committed SVG uses. `--collapse ingest` folds *one*
named top-level component while every other stays fully expanded — a third,
distinct layout shape, not a point on the line between "default" and "--depth
1": collapsing `ingest` alone changes the routing geometry around `strategy` and
`signals` (which stay expanded, at their normal full-depth positions) in a way
that collapsing *every* top-level component together, or collapsing *none*,
never exercises. The box-collection and "is this box one the line is allowed to
touch" logic in the test (`all_component_boxes`, `check_line_item`) were never
the problem — both already walk the real rendered DOM by tag and class,
transform-accumulated, nesting-depth-agnostic; the coordinator's own steer
("likely in how boxes are collected... or the allowed-to-be-inside logic") was a
reasonable place to look first, but the actual defect was one level up: the test
simply never asked the renderer to *produce* the one layout its own fixture's
committed artifact ships.

**Fix:** `no_drawn_element_intersects_a_box_it_is_not_inside` now also renders,
for every fixture, one `--collapse <name>` pass per top-level component the
document declares (`tests/render.rs`, the loop over `doc.components.keys()`
added alongside the existing "default" and "--depth 1" passes) — the general
form of "test the configuration you actually ship," not a one-off special case
for `ingest`. Run against the pre-fix routing code, this correctly went red on
exactly the two real defects (`--collapse ingest` crossing `strategy`/`signals`,
and the previously-unknown `--collapse gateway` crossing `pnl`), naming the
offending edge and box in both cases, before any routing code for bug 3 was
touched — confirming the *test* fix was the right one, independent of whatever
the routing fix would turn out to be.

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

### Second regeneration, after the coordinator's review (bug 3 above)

Both `003-trading-system-full.svg` and `003-trading-system.svg` were regenerated
again once bug 3's fix landed. Diffed against the versions above; in both files
the *only* change is the `d=` attribute (and, consequently, the label `x`/`y`) of
the two long-haul edges — `venue ~> ingest.feed`'s `RawFrame` and the derived
`entry` edge from `Oms::submit` — plus the same two lines' now-adjusted label
positions. Every box, every other edge, every tooltip, and the frame itself are
byte-identical to the first regeneration. In the collapsed file
(`003-trading-system.svg`), both edges now run a clean straight line down `from`'s
own X (`563.7`) with no sideways detour at all, since that column turned out to be
genuinely clear; in the full file, both detour left (to the shared margin at
`x=320`) instead of the first round's detour right (to `x=1056.8`), because
`from`'s own X in the fully-expanded layout (`feed`'s real border, not `ingest`'s
collapsed one) sits inside `gateway`'s column, so the straight-down check
correctly rejects it and the algorithm falls back to the nearer clear edge of the
combined span — which happens to be the left one this time. Neither file's
canvas dimensions changed (routing, not layout, was the fix).

## The second coverage gap: labels struck by lines

Two rounds of coordinator review found two coverage gaps of the same kind, both
in the same invariant, both in the same file's canonical committed picture:
round 2 was a box the check never rendered the shape to catch; round 3 was a
*label* — the check covered boxes but never covered text at all. Two gaps of
the same shape in one feature is the finding worth recording on its own: the
"walk the real rendered output" discipline (CLAUDE.md) has to be applied
per-drawable-kind, not once for the invariant as a whole — proving it covers
boxes says nothing about whether it covers text.

**The defect:** `venue ~> ingest.feed`'s own label ("RawFrame") sat at a point
its own edge's long vertical run passed straight through — in the committed
`003-trading-system.svg` (`--collapse ingest`), the path runs `x=563.7` from
`y=160` to `y=678`, and the label's midpoint-offset placement landed inside
that run at `y=419`. It rendered as a strikethrough. Of the document's 8 edge
labels, exactly one was struck; not a general convention, a real bug.

**Extending the invariant — three attempts, two rejected, one kept:**

1. *Every text node against every line.* The most general form: any `<text>`
   in the document, checked against every drawn line segment. Rejected —
   false positives on the `any`-node `*` glyph and the deny wall's `except`
   label, both of which sit deliberately close to (in the `*` glyph's case,
   touching) their own connector by design. Widening the check to every kind
   of text conflated "struck because nobody checked" with "adjacent because
   that's how the glyph is drawn."
2. *A label struck only by a segment of its own edge's path* — the
   coordinator's offered fallback, for exactly this reason. Tried, and
   rejected on evidence, not preference: rendering the fixture and cropping
   the actual raster around the RawFrame label (`cairosvg` + a zoomed PIL
   crop, not a coordinate calculation) showed the line striking it was the
   *derived `entry` edge's* line, not RawFrame's own path. The two edges run
   near-parallel down the same left margin. A same-edge-only check would have
   passed green over the exact defect the coordinator reported — a narrower
   property that is unsound for this specific bug is worse than an expensive
   general one, so this was dropped once the false negative was confirmed,
   not assumed.
3. **What shipped:** every `<text class="edge-label">` — scoped by CSS class,
   not by "which edge owns it" — checked against every drawn line in the
   document (`edge_labels_struck_by_any_line`, `tools/render/tests/render.rs`).
   Scoping by class rather than by "only my own path" avoided attempt 1's
   false positives (`*` uses `any-label`, `except` uses `deny-except` — neither
   matches `edge-label`) while staying sound against attempt 2's false
   negative (any line, not just the label's own). The bounding-box estimate
   for a label reuses the existing worst-case monospace character width
   (already erring wide, per CLAUDE.md's note that vetting 001 finding 4 once
   shipped a bound that was too narrow) plus a small vertical pad.

**Watched red first, against the currently-committed SVG**, before any
label-placement code changed: the extended invariant failed naming the exact
label, box, and striking segment — `edge label (516.7, 408.0, 56.0, 14.0)
["RawFrame"] is struck by a drawn line segment [(563.7, 160.0), (563.7,
678.0)]` — confirming the check finds the coordinator's exact defect before
the fix existed.

**The fix, and its actual root cause:** the coordinator's hypothesis (midpoint
logic assumes a simple two-point line; on a long routed path the anchor lands
on a run) was right, but the placement code also never had visibility into
*sibling* edges' geometry — `render_external_edge` computed a route and drew
its label inline, one edge at a time, so an edge's label could only avoid its
*own* path, never a nearby edge's. Fixed by splitting the function into
`compute_external_edge_route` (pure geometry) and `draw_external_edge`
(drawing) and restructuring the render loop into two passes: pass 1 computes
every external edge's route; pass 2 draws all of them, each label checked
against a `lines_drawn_so_far` collector that includes every regular edge,
every deny line, and every external edge's route computed so far — not just
its own. The label-placement escalation search was also widened: it used to
vary only the perpendicular offset from a fixed midpoint, which cannot escape
a case (this one) where a *different* edge's line crosses near the midpoint
broadly enough that no offset distance clears it; it now also varies the
anchor point along the segment (five points from 0.15 to 0.85 of the way
along), crossed with the existing offset/side search, 50 candidates total
before falling back to the unescalated midpoint.

**Mutation-tested:** disabling the new line-avoidance clause in the escalation
predicate (leaving only the pre-existing box-avoidance clause) reproduced the
invariant failure exactly, confirming the clause is load-bearing, not
redundant with the box check. Reverted immediately after confirming red; the
invariant returned to green.

**What's still an open, recorded gap:** the general `edge-label`-vs-any-line
check surfaces 13 more violations across the fixture sweep — all pre-existing,
none touching `venue`/`RawFrame`/`entry` or any other construct this feature
added. Every one belongs to an edge declared before this session
(`BookUpdate`, `OrderIntent`, `Order`, `Fill` between `gateway`/`oms`/`pnl` —
confirmed against `git show 31a669d -- vetting/003-trading-system.ply.yaml`,
which shows only the three `venue` edges as additions). These are real defects
of the same shape, just outside this feature's boundary: fixing them would
mean extending the two-pass restructure and multi-anchor escalation to the
regular-edge and deny-edge label placement code too — both delicate, both
well-tested by existing invariants for box-avoidance already, and neither
touched by this task. Rather than risk that refactor un-asked, the invariant
now classifies each violation by checking whether its edge's tooltip contains
the phrase "outside this codebase" (present on every external-touching edge
and only those): a hit is a hard failure (`violations`), a miss is reported at
`eprintln!` volume as a `known_pre_existing_gap` and does not fail the test.
This keeps the invariant honest — it still runs the full general check, so
regressing any `edge-label`'s clearance space (including these 13) would be
visible in the test's own output — without silently blocking on, or silently
absorbing, defects this task did not create and was not asked to fix. The 13
are named in full in the test's own `eprintln!` output (edge label bounding
box, offending line segment, per fixture/mode) so the next session that picks
this up starts from an exact list, not a re-discovery.

### Third regeneration, after the coordinator's second review (this gap)

Both `003-trading-system-full.svg` and `003-trading-system.svg` were
regenerated again once this fix landed. Diffed against the second-regeneration
versions above:

- **`003-trading-system.svg`** (`--collapse ingest`): two labels moved —
  `RawFrame`'s `y` from `419.0` to `315.4` (off the vertical run entirely,
  now sitting above `entry`'s own line rather than beside it), and the
  `venue → gateway` `Fill` label's `x`/`y` from `(548.7, 670.5)` to
  `(593.3, 713.5)` (it was also, independently, close enough to a line to
  need the wider escalation once line-avoidance was turned on generally — not
  struck as visibly as `RawFrame`'s, but the check is stricter now and the
  escalation naturally picked a clearer spot). No path `d=` attribute changed
  in this file — routing was untouched this round, only label placement.
- **`003-trading-system-full.svg`**: one label moved — the same `venue →
  gateway` `Fill` label, `y` from `1664.5` to `1707.5`. `RawFrame`'s label in
  this fully-expanded layout was already clear (its route has more room here)
  and did not need to move.
- Rasterised both with the coordinator's own Chromium command and inspected by
  eye (cropped to the `RawFrame` and `venue` regions at 3x for a close look):
  no label in either image is struck by any line. `001`/`002` and the golden
  snapshot were re-diffed too and are unaffected — this fix only touches the
  external-edge label-placement code path, which only fires when a document
  declares an `externals:` map.

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
`no_drawn_element_intersects_a_box_it_is_not_inside` with its new per-top-level-
component `--collapse` sweep (watched go red against the coordinator's finding
before the routing fix, green after), and every pre-existing test (render, check,
model, kernel, schedule). Re-run after `cargo fmt` to confirm formatting changed
nothing behaviorally. `cargo fmt --check`: clean. `cargo clippy --release
--all-targets -- -D warnings`: clean (one `#[allow(clippy::too_many_arguments)]`
added to `walk_component`, matching the existing precedent on
`render_deny`/`render_external_edge` for functions whose argument count is
inherent to what they thread through, not accidental).

**Visual confirmation, both committed 003 SVGs, this round**: rasterised with the
headless Chromium at `/opt/pw-browsers/chromium` (1600×2000 window;
`dbus`/`UPower` connection errors in its stderr are sandbox noise, not render
failures — both screenshots wrote successfully) and inspected by eye. No line
in either image crosses a box it is not attached to; the `RawFrame`/`entry`
edges that used to strike through `strategy`/`signals` now run cleanly down the
left margin, alongside (not through) `risk`, to `venue`.

**Third round (label-vs-line fix)**: `cd tools && cargo test` (whole
workspace, release): **green** — 22 test binaries, 0 failures, including
`no_drawn_element_intersects_a_box_it_is_not_inside` now also passing the
`edge-label`-vs-any-line check (13 pre-existing, out-of-scope violations
reported via `eprintln!`, not failing the test — see "The second coverage
gap" above). Watched red first against the currently-committed SVG (named the
exact `RawFrame` label and the `entry` edge's segment striking it) before the
label-placement fix landed. Mutation-tested: disabling only the new
line-avoidance clause in the escalation predicate (leaving box-avoidance
intact) reproduced the failure; reverted, confirmed green again. `cargo fmt`
(one block needed reformatting after the new test function; re-ran the full
suite after to confirm no behavioural change) then `cargo fmt --check`: clean.
`cargo clippy --release --all-targets -- -D warnings`: clean, no new
`#[allow(...)]` needed this round. Rasterised both committed 003 SVGs again
with the coordinator's own Chromium command, cropped to the `RawFrame` and
`venue`/`Fill` regions at 3x, and confirmed by eye: no label is struck by any
line in either image.

## NOT RUN / left for the maintainer

- **The holistic squint test — does this *read well*, not just "does nothing
  overlap" — is still the maintainer's own**, per the task brief. This session
  rasterised both committed 003 SVGs with headless Chromium and confirmed by eye
  that no line crosses a box it shouldn't (the specific defect the coordinator's
  own review found), and separately judged the frame reads as a boundary and
  `venue` reads as clearly outside it — but "is this a *good* diagram" beyond
  that specific correctness property is the maintainer's call, not mine to
  certify. The lesson this round leaves on the record: a first look that stops
  at "looks fine to me" is not the same check as the coordinator's — theirs
  computed the exact box a specific line's y-coordinate falls inside before
  ever opening the image. That is the standard the *invariant* now meets
  automatically; a human glance at a PNG is not a substitute for it, only a
  sanity check on top of it.
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
- **13 pre-existing `edge-label`-vs-line violations**, none touching this
  feature's own constructs (`BookUpdate`, `OrderIntent`, `Order`, `Fill`
  between `gateway`/`oms`/`pnl` — all declared before this session; see "The
  second coverage gap" above for the exact classification rule and where the
  full list is printed). The invariant now surfaces every one of them on every
  test run rather than hiding them, but fixing them means extending the
  two-pass geometry-then-labels restructure and multi-anchor escalation this
  session built for external edges to the regular-edge and deny-edge label
  placement code too — a larger, riskier change to well-tested code this task
  was not asked to touch. Left as a recorded, visible gap, not a silent one.

## TODO.md

Ticked in the same session, per CLAUDE.md; see that file for the entry and its
commit hash.
