# Working on Ply

[Ply-Spec.md](Ply-Spec.md) is the source of truth. Start from it by § reference; amend it rather
than contradicting it. Session rules are §11.

## Test-driven, always

Write the failing test first. Watch it fail, and read the failure message — if it doesn't
name the actual defect, the test is wrong. Only then make it pass.

**Assert the observable outcome, not the shape of the output.** The renderer once emitted
30 green tests' worth of correctly-classed, well-formed SVG that rasterised as a solid
black rectangle: every test checked structure, none checked that anything was visible.
For a rendered artifact that means opening it (`qlmanage -t -s 900 -o <dir> <file>.svg`,
then look at the PNG). For a verdict it means the verdict a user would read.

Prefer one invariant test over a pile of spot-checks. `every_painted_element_resolves_a_style_rule`
and `every_drawn_item_resolves_a_tooltip` in `tools/render/tests/render.rs` are the model:
they walk the real output and fail on the first unexplained item, so a construct added
later cannot quietly skip the rule.

Goldens are reviewed, never blind-accepted. When a snapshot changes, look at the diff and
say why it changed.

## Ply proves its own kernel

The verdict kernel — the evidence order, worst-of aggregation, and status propagation
(D6, D5) — is where a rule interaction could make evidence lie. That kernel is written
as one pure module carrying `#[ply::...]` contracts, and its invariants are checked with
the same engines Ply routes to (Kani bounded proofs over small verdict trees), not just
unit tests. The standing obligations:

- aggregation never reports evidence stronger than the weakest child
- `conditional` never disappears without its assumptions being discharged
- a `violation` anywhere always reaches the root
- no rule sequence assigns one node two different verdicts

New aggregation or status rules don't merge until they hold under these. If a rule can't
be expressed in the kernel's pure module, that is a design smell to raise, not route
around.

## Talk like the `/vibe-coding` skill

Report outcomes, not code churn. Skip file names, function names, and diff-speak unless
asked. Say what changed in behaviour, where to see it, and whether it works. Make routine
technical calls yourself; only ask questions that can be answered without reading code.

## Every user-facing sentence passes the newbie bar

Tooltips, diagnostics, CLI output: written for someone who has never seen Ply. Name the
visual if the glyph is unusual, say what it means, say why it matters — in that order.
A code (E0203) or § reference may follow a plain sentence, never replace one. The test
for new wording is exact-string, so the words are reviewed like code. If a term needs
the spec to decode (`bounded`, `unclaimed`, `instantiation`), the sentence carries its
own gloss.

## Delegation

Use the cheapest model that can do the job. Implementation goes to sonnet-tier agents
once the design is settled; mechanical sweeps (renames, fixture generation, source
hunting) can go cheaper still. The top model is for spec changes, design decisions,
review judgment, and verifying agent output — never for typing out code an agent could
write from a precise brief.

## Scope

Build what was asked and nothing adjacent. A legend nobody requested is not a bonus.
If you think something extra is needed, say so in one line and let the user decide.

## Vetting

`vetting/` holds scenarios written in the grammar before the tool exists, each recording
where the grammar held and where it broke. Findings become spec changes. New grammar
features must be drawable (§7.1) — if there is no visual form, the feature doesn't enter.
