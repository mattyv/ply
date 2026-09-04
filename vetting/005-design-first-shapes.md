# Vetting 005 — design-first state shapes

Scenario: a settlement service being designed before any code exists. Written 2026-09-03
against The-Ply-Spec.md as of that date, to test one question: **can the grammar say what
a component holds, at the moment a designer actually wants to say it?**

At the time, it could not, for a rule the spec argued for explicitly. This scenario argued
back.

**Status (2026-09-04): accepted.** The shape-versus-type distinction below was checked
against the real classifier and held. The first proposed mark — a dotted outline —
rasterised badly and was refused; the mark that shipped instead is the ordinary,
unmodified glyph, with the type column carrying the word `declared`. That measurement,
the retirement story open question 4 asked for, and the "should a kept promise be
reported" question are all closed below, in place, dated. `show:`'s mapping form is now
part of the grammar (The-Ply-Spec.md's `state:` section, "A document may declare a field's
shape", 2026-09-04) and this document validates and renders exactly as written.

## Where the grammar broke

At the time this was written, `vetting/005-design-first-shapes.ply.yaml` was refused by
the tool:

```
components.ledger.state.show: invalid type: map, expected a sequence at line 37 column 9
```

`show:` was a list of field names, and nothing else. With no crate under the document, Ply
drew `state Ledger` and no rows — correctly, because it had nothing to read. Every state
block in `vetting/001`, `002`, `003` and the two demo documents that have one was in
exactly this position (the third demo, `verified-green`, has code but declares no state):
a type named, fields listed, and not one shape drawn. The one feature that makes structure
legible was unavailable in the documents the grammar was invented for.

## The rule, and why it was right

From §5's `state:` section, as it read at the time:

> `show:` lists field *names* only — never their types, never their shapes. Ply reads
> `OrderBook` from source and draws each named field as whatever it actually is. Writing
> the shapes in the document would be a second, hand-maintained copy of a fact the
> compiler already owns, and it would drift the first time somebody changed a field: the
> exact rot this project has spent its documentation budget removing.

That was a good rule and this proposal did not overturn it. A document must not restate a
type. `Vec<Order>` in a ply.yaml is rot waiting to happen, and still is — the accepted
grammar refuses it by name (see the parse-error test in `crates/ply-core/src/model.rs`).

## Why it did not settle this case

**A shape is not a type, and the drawing never needed the type.**

§7.1 draws seven forms: scalar, text, list, map, set, might-be-missing, a-shape-of-your-own.
That is the whole visual vocabulary. `Vec<Order>`, `VecDeque<Order>` and a hand-rolled ring
all draw the same glyph — three stacked bars — because to a reader the fact that matters is
*ordered, many*. The type column beside the glyph is a separate thing, and it can stay
empty.

So a declared **shape** is strictly coarser than a type. It says "this is a lookup table"
and stops. That is not a copy of a compiler fact; it is design intent, and it is the kind
of intent that is worth being wrong about in public. The `dedupe` component in this
scenario is the case in miniature: its entire risk is that `seen` must be a set and not a
list, because a duplicate settling twice is the bug the design exists to prevent. A
designer wants to write that down before there is code, and wants to be held to it after.

**And Ply already knew how to hold someone to it.** `A0415` fires when `show:` names a
field the type does not have. A declared shape that disagrees with the source is the same
class of fact, one step further in — accepted as `A0416` (see below).

That was the argument: with the shape/type distinction, a declared shape stops being a
duplicated fact and becomes **a promise the compiler later grades** — which is what this
entire tool is for. `state:` had been the one corner of the grammar where you could only
describe what already exists, and could not promise anything.

## The accepted grammar

`show:` accepts either form. A list is exactly the original behaviour and stays the default:

```yaml
state:
  of: Ledger
  show: [by_account, queued]        # names only; shapes read from source
```

A mapping declares the shape of each field:

```yaml
state:
  of: Ledger
  show:
    by_account: map                 # a lookup table, keyed
    queued: list                    # ordered, many
    last_settled: optional          # may not be there
    total_cents: scalar             # one value
    cursor:                         # deliberately undeclared — draws as today
```

One key, two forms, so there is no second key to drift against the first. Values are
§7.1's seven shapes and nothing else, one token each: `scalar`, `text`, `list`, `map`,
`set`, `optional`, `composite`. The last two are this proposal's spellings — §7.1 names
those forms "might be missing" and "a shape of your own", which are not YAML tokens.
Deliberately not a type grammar, so the temptation to write `Vec<Order>` has nowhere to
land — refused by name, at parse time, with the seven legal tokens listed back. A `null`
value declares nothing.

## The drawn form

§7.1 requires a visual form or the feature does not enter. What was proposed, and what
shipped instead:

| where the shape came from | glyph |
|---|---|
| read from source | solid ink — today's drawing, unchanged |
| declared, no code to check it against | **the same silhouette, unmodified** — the type column reads `declared` |
| read from source, cannot be built by the sampling engine | diagonal hatching — unchanged |

**The first candidate — a dotted outline — was measured and failed (2026-09-03).** It was
rasterised at the 12px reading size beside the real glyphs (the Option cell is
`stroke-dasharray: 3 2.4` at stroke-width 1.4). Three collisions. On the filled forms —
scalar, list, set — a dotted outline drawn over the fill was invisible at 12px; dropping
the fill to make it visible produced a hollow dotted cell, the same glyph as a declared
`optional` and a near-neighbour of the read Option. Dotted against dashed at that stroke
width read as *lighter ink*, not a different line style — which is also how a low-contrast
screen or a bad rasteriser reads, so the channel degraded into nothing. And the Option
glyph's silhouette *is* its dash pattern, so "the same silhouette, dotted" could not mark
that form at all.

**What shipped instead needed no new mark at all.** There is no colour left (§7.1), the
hatch already means "the sampling engine cannot build this", and the dotted outline had
just failed at the same stroke weight. The channel that was actually free was the type
column: a declared row has no real type to spell there, so the word `declared` fills that
column and the ordinary glyph is left exactly as it is for a read row. Read against
declared is now a text distinction, not a silhouette one — cheaper than the mark this
document proposed, and it does not compete with the Option glyph's own dash pattern for
attention at 12px.

## What happens when the code arrives

The code wins, always. Three outcomes, all implemented:

1. **Source agrees with the declaration.** The row is drawn from the code (solid ink, the
   real type in the type column) — the declaration contributed nothing to the drawing, and
   `cargo ply check` reports it confirmed and counted (see "should a kept promise be
   reported" below), not silent.
2. **Source disagrees.** `A0416`, naming the field, what was declared, and what the code
   really is, and pointing at both legitimate exits: fix the declaration, or treat the
   mismatch as the regression it is. This is the payoff: the drawing never quietly prefers
   the document, and the design document becomes checkable rather than decorative.
3. **No crate to resolve against.** `W0413` covers this unchanged; a declared shape is
   drawn, and nothing about that claim was checked.

## What is not proposed

- No type declarations, ever. The seven shape names or nothing.
- No change to `N of M shown`. Both numbers are counted from code and stay omitted when
  there is no code — a declaration must not produce a count. (Confirmed by
  `a_declared_only_box_never_draws_a_count` in `tools/render/tests/render.rs`: this
  document's `ledger`, `dedupe` and `reporting` boxes draw their declared rows with no
  count anywhere near them.)
- Nothing about whether the component really holds that type. Still unchecked, still in
  SCHEMA.md's tier table as such.

## Open questions, closed

1. **Is `optional` a shape or a modifier? Closed: a shape, drawn and checked exactly like
   the other six.** The presence wrapper still wins in the classifier —
   `Option<Vec<T>>` draws the dashed "might be missing" cell alone — so a declared
   `optional` over a wrapped collection *agrees* with the code rather than firing a false
   `A0416`; that equivalence is pinned end to end by
   `a_presence_wrapper_wins_the_comparison_exactly_as_it_wins_the_drawing`
   (`crates/ply-cli/src/check.rs`) — declared `optional` over `Option<Vec<u64>>` is
   confirmed, declared `list` there is a real `A0416` — with the token→shape mapping
   itself pinned by `every_declared_token_maps_to_the_shape_it_names`
   (`crates/ply-core/src/visual/state_shapes.rs`). The asymmetry named here stands as
   documented behaviour, not a defect: a read `optional` row still spells the real type
   (`Option<Vec<Order>>`) beside its glyph, while a declared one reads `declared`, because
   there is no real type to spell until the code exists.

2. **Should a kept promise be reported? Closed: yes.** `cargo ply check`'s anchor-tier
   summary now counts agreements out loud — "N declared field shapes were checked against
   the source and all N are what the code says" — rather than leaving a correct
   declaration to pass in silence the way a wrong one used to be the only thing that spoke.
   See `AnchorTally::declared_shapes_checked` and `anchor_detail` in
   `crates/ply-cli/src/check.rs`.

3. **Does this earn its height? Closed: yes, measured rather than assumed.** This
   document's `ledger` (4 rows), `dedupe` (1 row) and `reporting` (2 rows, `cursor`
   contributing none) each pay exactly the same per-row height a read state box does — the
   renderer draws a declared row through the identical `state_rows`/layout path, just with
   `declared` in place of a real type. `tools/render/tests/render.rs`'s canvas invariants
   (`everything_renders_inside_the_canvas`, `every_drawn_label_lies_inside_the_canvas`) run
   over this document unchanged and pass, so no row overflows its box or the canvas at this
   height. `005-design-first-shapes.svg`, rendered and committed alongside this file
   exactly as `001`–`004` are, is the raster to eyeball: three boxes gain their state rows,
   nothing else in the layout moves, and no arrow crosses a row it used not to.

4. **When does a confirmed declaration retire? Closed: whenever the author wants, and both
   the confirmation and the `A0416` message say so.** There is no lint that forces or even
   suggests collapsing a confirmed declaration back to the list form — that was considered
   and rejected as inventing a second, opinionated rule on top of a feature that already
   has an honest escape hatch: dropping a mapping entry's value (or the whole entry back to
   a bare name in the list form) is always legal, at any time, and never fails a check.
   `check`'s confirmation sentence says exactly that ("a confirmed declaration may be kept
   as documentation or dropped back to the plain field name — both are fine"), and so does
   `A0416`'s own message when the declaration is the one that is wrong. The maintenance
   burden §5 removed does not come back with an alarm attached: the alarm only ever fires
   on a genuine disagreement, never on a correct refactor that nobody bothered to also
   update the declaration for, because leaving it there was already always fine.
