# Vetting 005 — design-first state shapes

Scenario: a settlement service being designed before any code exists. Written 2026-09-03
against The-Ply-Spec.md as of that date, to test one question: **can the grammar say what
a component holds, at the moment a designer actually wants to say it?**

Today it cannot, and the reason is a rule the spec argues for explicitly. This scenario
exists to argue back.

**Status after review (2026-09-03): the argument stands, the drawing does not.** The
shape-versus-type distinction below was checked against the real classifier and holds. The
proposed dotted outline was rasterised at reading size and fails — it collides with the
glyph it must be told apart from. So this is not ready to enter the grammar: it needs a
different mark for "declared, not read", and an answer to open question 4. Both are
recorded rather than papered over.

## Where the grammar broke

`vetting/005-design-first-shapes.ply.yaml` is refused by the current tool:

```
components.ledger.state.show: invalid type: map, expected a sequence at line 37 column 9
```

`show:` is a list of field names, and nothing else. With no crate under the document, Ply
draws `state Ledger` and no rows — correctly, because it has nothing to read. Every state
block in `vetting/001`, `002`, `003` and the two demo documents that have one is in
exactly this position (the third demo, `verified-green`, has code but declares no state):
a type named, fields listed, and not one shape drawn. The one feature that makes structure
legible is unavailable in the documents the grammar was invented for.

## The rule, and why it is right

From §5's `state:` section, added the same day:

> `show:` lists field *names* only — never their types, never their shapes. Ply reads
> `OrderBook` from source and draws each named field as whatever it actually is. Writing
> the shapes in the document would be a second, hand-maintained copy of a fact the
> compiler already owns, and it would drift the first time somebody changed a field: the
> exact rot this project has spent its documentation budget removing.

That is a good rule and this proposal does not overturn it. A document must not restate a
type. `Vec<Order>` in a ply.yaml is rot waiting to happen.

## Why it does not settle this case

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

**And Ply already knows how to hold someone to it.** `A0415` fires today when `show:` names
a field the type does not have. A declared shape that disagrees with the source is the same
class of fact, one step further in.

That is the argument: with the shape/type distinction, a declared shape stops being a
duplicated fact and becomes **a promise the compiler later grades** — which is what this
entire tool is for. `state:` is currently the one corner of the grammar where you may only
describe what already exists, and may not promise anything.

## Proposed grammar

`show:` accepts either form. A list is exactly today's behaviour and stays the default:

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
land. A `null` value declares nothing.

## The drawn form

§7.1 requires a visual form or the feature does not enter. The candidate channel:

| where the shape came from | glyph |
|---|---|
| read from source | solid ink — today's drawing, unchanged |
| declared, no code to check it against | **the same silhouette, dotted outline** |
| read from source, cannot be built by the sampling engine | diagonal hatching — unchanged |

Dotted rather than a new colour, because §7.1 forbids a new colour channel and every hue is
spoken for. Dotted rather than hatched, because hatching already means something else and
precise: *Ply read this field and cannot construct a value of it*. The two must not blur —
one is "nobody has written this yet", the other is "this exists and the engine is stuck".

The type column reads `— declared` where a declaration is standing in for a type, so a
reader is never shown a shape and left to assume a compiler confirmed it.

**Measured, and the dotted outline fails as written (2026-09-03).** The candidates were
rasterised at the 12px reading size beside the real glyphs (the Option cell is
`stroke-dasharray: 3 2.4` at stroke-width 1.4). Three collisions. On the filled forms —
scalar, list, set — a dotted outline drawn over the fill is invisible at 12px; dropping
the fill to make it visible produces a hollow dotted cell, which is the same glyph as a
declared `optional` and a near-neighbour of the read Option. Dotted against dashed at
this stroke width reads as *lighter ink*, not a different line style — which is also how
a low-contrast screen or a bad rasteriser reads, so the channel degrades into nothing.
And the Option glyph's silhouette *is* its dash pattern, so "the same silhouette, dotted"
cannot mark that form at all. The `— declared` type column survives the measurement; the
dotted outline does not, and the feature needs a different declared/read mark before it
can pass §7.1's gate.

## What happens when the code arrives

The code wins, always. Three outcomes:

1. **Source agrees with the declaration.** The row becomes solid and the type appears. The
   promise was kept and stops being a promise.
2. **Source disagrees.** A new diagnostic naming both — the declared shape, the real one,
   and the field. Proposed `A0416`, beside `A0415`. This is the payoff: the drawing does
   not quietly prefer the document, and the design document becomes checkable rather than
   decorative.
3. **No crate to resolve against.** `W0413` already covers this and needs no change,
   though its wording should mention that declared shapes were drawn unchecked.

## What is not proposed

- No type declarations, ever. The seven shape names or nothing.
- No change to `N of M shown`. Both numbers are counted from code and stay omitted when
  there is no code — a declaration must not produce a count.
- Nothing about whether the component really holds that type. Still unchecked, still in
  SCHEMA.md's tier table as such.

## Open questions for review

1. **Is `optional` a shape or a modifier?** Today the presence wrapper wins and the
   inner shape is dropped: `Option<Vec<T>>` draws the dashed "might be missing" cell
   alone, by documented design in the classifier. So a declared `optional` that says one
   word is parity with the read drawing at the glyph level, not new loss. What a
   declaration does lose is the type column — a read row still spells `Option<Vec<Order>>`
   beside its glyph, a declared row says only `— declared`. Not fatal to the mapping
   form, but the asymmetry should be stated wherever the feature is taught.
2. **Should a kept promise be reported?** Outcome 1 is currently silent. There is an
   argument that `check` should say "4 of 5 declared shapes confirmed", since a confirmed
   design claim is evidence and this project counts evidence out loud.
4. **When does a confirmed declaration retire?** This is the strongest objection to the
   proposal and it has no answer yet. Once code arrives and agrees, the declaration sits in
   the document as exactly the second copy §5 forbade — checked rather than silent, but
   still hand-maintained. Every legitimate refactor after that (map to list-of-pairs,
   required to optional) now demands a document edit or fires `A0416` on a correct change.
   Without a retirement story — a lint saying "these five declarations are all confirmed,
   collapse them to the list form" — the maintenance burden the spec removed comes back
   with an alarm attached, which is better but is not nothing. Found in review, not by the
   author.

3. **Does this earn its height?** §5 records that state rows cost vertical space and move
   arrows, measured per component. A dotted row costs exactly what a solid one does, and
   the documents that would gain rows are the vetting scenarios — the ones whose layouts
   were already measured to the crossing count. That measurement has to be redone, not
   assumed.
