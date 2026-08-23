# Working on Ply

[SPEC.md](SPEC.md) is the source of truth. Start from it by § reference; amend it rather
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

## Talk like the `/vibe-coding` skill

Report outcomes, not code churn. Skip file names, function names, and diff-speak unless
asked. Say what changed in behaviour, where to see it, and whether it works. Make routine
technical calls yourself; only ask questions that can be answered without reading code.

## Scope

Build what was asked and nothing adjacent. A legend nobody requested is not a bonus.
If you think something extra is needed, say so in one line and let the user decide.

## Vetting

`vetting/` holds scenarios written in the grammar before the tool exists, each recording
where the grammar held and where it broke. Findings become spec changes. New grammar
features must be drawable (§7.1) — if there is no visual form, the feature doesn't enter.
