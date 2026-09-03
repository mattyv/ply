# Vetting 005 — design-first state shapes

Scenario: a settlement service being designed before any code exists. Written 2026-09-03
against The-Ply-Spec.md as of that date, to test one question: **can the grammar say what
a component holds, at the moment a designer actually wants to say it?**

Today it cannot, and the reason is a rule the spec argues for explicitly. This scenario
exists to argue back.

## Where the grammar broke

`vetting/005-design-first-shapes.ply.yaml` is refused by the current tool:

```
components.ledger.state.show: invalid type: map, expected a sequence at line 37 column 9
```

`show:` is a list of field names, and nothing else. With no crate under the document, Ply
draws `state Ledger` and no rows — correctly, because it has nothing to read. Every state
block in `vetting/001`, `002`, `003` and both demo documents is in exactly this position:
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
§7.1's seven shape names and nothing else — deliberately not a type grammar, so the
temptation to write `Vec<Order>` has nowhere to land. A `null` value declares nothing.

## The drawn form

§7.1 requires a visual form or the feature does not enter. The channel is available:

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

1. **Is `optional` a shape or a modifier?** `Option<Vec<T>>` draws as both today. The
   mapping form above can only say one word per field. Either the vocabulary grows a
   compound form, or a declared `optional` loses the inner shape — and losing it silently
   would be the drift this proposal claims to avoid.
2. **Should a kept promise be reported?** Outcome 1 is currently silent. There is an
   argument that `check` should say "4 of 5 declared shapes confirmed", since a confirmed
   design claim is evidence and this project counts evidence out loud.
3. **Does this earn its height?** §5 records that state rows cost vertical space and move
   arrows, measured per component. A dotted row costs exactly what a solid one does, and
   the documents that would gain rows are the vetting scenarios — the ones whose layouts
   were already measured to the crossing count. That measurement has to be redone, not
   assumed.
