# Making the render land at a glance: research and a proposed visual language

Reviewed against a real external spec (a 4-group, 15-component, 29-fn trading-service
design) rendered with the current `ply-render`, and against the visual-notation and
perception literature. The goal stated by the user: *a trained human glances at the
diagram and it lands instantly — what needs work, what is evolving, what is missing,
what is broken, what is working well.*

The short version: ply's current grammar encodes the wrong axis loudly. It answers
"how much was promised?" with its strongest visual channel (green saturation), and
answers the five questions above weakly or not at all. The research says salience
must be spent on *deviation from healthy*, never on healthy itself.

## The research that applies

**Moody, "The Physics of Notations" (IEEE TSE 2009)** — the canonical framework for
exactly this problem: nine evidence-based principles for cognitively effective visual
notations (semiotic clarity, perceptual discriminability, semantic transparency,
complexity management, cognitive integration, visual expressiveness, dual coding,
graphic economy, cognitive fit). Ply already satisfies complexity management
(`--depth`/`--focus`) and partially dual coding (tooltips). It currently violates
perceptual discriminability (pure seal, strict notch are sub-threshold), semantic
transparency (green = promise reads as green = passing; red = capability reads as
red = error), and dual coding on the densest glyphs (check badges carry no text
expansion and no tooltip).

**Preattentive processing (Treisman; Healey's survey)** — only a handful of channels
pop in <250ms: color hue, luminance, size, orientation, simple shape. Two hard limits
matter here: (1) *conjunctions don't pop* — "green AND dashed" cannot be read at a
glance; each state needs one dominant channel; (2) *absence doesn't pop* — a missing
feature (a fn with no checks, a component with no claims) is invisible unless encoded
as a positive mark. Ply currently encodes its riskiest states (unclaimed, hollow) as
absence: white fill, thin dash. They are the quietest thing on the canvas.

**Cleveland & McGill's perceptual hierarchy** — position and length are read far more
accurately than area or color saturation. "How much evidence has this earned" is a
quantity; it should be a length (a small meter), not a saturation step.

**Alarm-display practice (EEMUA 191 / ISA-18.2, control-room "dark screen"
philosophy; Few's dashboard work)** — a healthy system should render *quiet*: muted,
low-saturation. Saturated color is a budget spent only on things demanding action.
The current render inverts this: the best-specified groups are the most saturated
objects on screen, so the eye is dragged to precisely the components that need no
attention.

**Traffic-light semantics and colorblindness** — ~8% of males cannot rely on
red/green separation. Every status must be redundantly encoded (color + icon or
pattern or label), and red must be reserved for one meaning. Ply currently spends
red on capability chips (`net`, `time`, `fs`) — neutral declarations — and on deny
bars — *protections*. When `verify` gains failures, there is no alarm color left
that hasn't already been used for something harmless.

**CodeCity controlled experiments (Wettel & Lanza)** — the empirical result worth
keeping: overview-metaphor visualizations improved *big-picture* task correctness
and speed, and did nothing for detail tasks. So each zoom level should be designed
for its task: the folded view for "where does attention go", the focus view for
"what exactly is owed here". Don't make one view serve both.

## What the current render gets right (keep these)

- **Worst-case rollup on fold** — "the weakest function sets the whole box's shade"
  is exactly how alarm aggregation should work. Extend it to every state below.
- **Honest-uncertainty posture** — the canvas tooltip already says "a promise scale,
  not results". The posture is right; the pixels contradict it.
- **Small symbol vocabulary** — ~8 glyph types is within Moody's graphic-economy
  budget of ~6 ± 2. The problem is discriminability, not count.
- **`--depth`/`--focus`** — correct complexity management; needs cognitive
  integration between levels (breadcrumbs / click-through), which the external HTML
  viewer already prototyped with `<a>`-wrapped boxes.

## The core redesign: five states, one visual signature each

The user's five questions, made into a fixed vocabulary. Each state gets ONE
dominant preattentive channel, redundantly encoded, and rolls up worst-case when
folded. Nothing else on the canvas may use these signatures.

| State | Question it answers | Signature (dominant + redundant) |
|---|---|---|
| **Broken** | what is broken | Saturated red fill + ✕ icon. Reserved: failed check, violated deny, undeclared capability use. Nothing else is ever red. |
| **Missing** | what is missing | Diagonal hatch (a positive mark, never blank) + count chip ("3 fns unclaimed"). Absence must be drawn, not implied. |
| **Needs work** | what needs work | Amber corner flag + count: unresolved decisions, `trusted` claims gone stale. Attached to the owning box, not parked in the canvas corner. |
| **Evolving** | what is evolving | Half-filled evidence meter (see below): promised but not yet earned. Grey/blue outline, never green. |
| **Working well** | what is working well | *Quiet.* Low-saturation fill, thin border. Verified-and-clean earns visual silence, not celebration. A small ✓ suffices. |

Supporting changes that make the table work:

1. **Split promise from evidence.** Today one channel (green depth) encodes declared
   check strength, and nothing encodes results. Give each fn/box a two-part meter:
   outline ring or track = what is promised, fill length = what has been earned by a
   run. Unverified promise is an empty track — visibly *owing*, not white/absent.
   Length is the Cleveland-McGill-correct channel for this quantity, and the meter
   aggregates on fold (sum of tracks, sum of fills).
2. **Retire green-as-promise.** Green may only ever mean "evidence earned, nothing
   failed". Before any run, there is no green anywhere — which is the truth.
3. **Free the color red.** Capabilities become neutral chips (grey; blue if they
   need any accent). A capability used-but-undeclared is what turns red. Deny rules
   become a small guard glyph (e.g., a lock) on the *protected* box, neutral until
   violated; the current floating red bars read as errors and sit far from what
   they protect.
4. **Badges become meters + words.** `B3 F4096 M T ex×2` fails dual coding twice
   (arbitrary letters, no tooltip). Replace with the evidence meter plus an
   expandable text line; keep the letters only inside the tooltip.
5. **Make strict/pure discriminable or demote them.** The corner notch and seal are
   below glance threshold. Either give them a visible signature (border weight,
   background tint) or accept they are hover-tier information.
6. **Layout: let position mean something.** Enforce rank bands (app / edge / domain
   layers) so vertical position encodes architecture depth — position is the
   strongest channel there is, and it is currently spent on nothing. Route edges
   orthogonally with collision avoidance; an edge striking through a label (current
   `--depth 1` bug) costs more trust than any single feature adds.
7. **One-line verdict strip.** Top of every render: `2 broken · 5 unverified ·
   1 trusted (stale) · 3 unresolved · 21 quiet`. The glance before the glance.
   Colorblind-safe because each count carries its icon.
8. **Colorblind check in CI.** Render the golden SVGs through a deuteranopia
   simulator; the five signatures must survive. They do, if each has its icon or
   pattern — which is the point of redundant encoding.

## The one-sentence version

Spend saturation on deviation, encode quantity as length, draw absence as a mark,
reserve red for broken, and let healthy be quiet — then a trained glance reads the
canvas in the order that matters: red, hatch, amber, half-empty meters, and only
then the calm green mass that needs nothing.

## Sources

- Moody, *The "Physics" of Notations*, IEEE TSE 2009 —
  https://www.semanticscholar.org/paper/bcd2c5379a34068040750a751e4fd2710d90c15c
- Healey & Enns, *Perception in Visualization* — https://www.csc2.ncsu.edu/faculty/healey/PP/
- *Comparing Pre-attentive Visual Variables … for Glanceable Visualizations* —
  https://www.researchgate.net/publication/399215440
- Cleveland & McGill, *Graphical Perception*, JASA 1984
- Wettel & Lanza, *Software Systems as Cities: A Controlled Experiment* —
  https://www.researchgate.net/publication/221555737
- EEMUA 191 / ISA-18.2 alarm-management guidance; Few, *Information Dashboard Design*
- *Dashboard Design Patterns* — https://arxiv.org/pdf/2205.00757
- CatPAW: unifying color & shape encodings — https://arxiv.org/pdf/2602.06792
- Accessible color sequences — https://arxiv.org/html/2107.02270v3
