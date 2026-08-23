# Vetting 003 — trading system (one layer up)

Scenario: the whole of 002 becomes one nested component (`ingest`) inside a trading
system — strategy with a nested signals library, risk (strict), order management,
positions/PnL, and a venue gateway. Written to exercise what no scenario has touched:
**nesting** (`components:` inside a component, in the grammar and renderer since day
one, never used), **dotted references** (`strategy -> ingest.book`, §5.1a rule 6), a
tree deep and wide enough to stress aggregation, and the zoom model's first contact
with reality.

Canonical YAML: [003-trading-system.ply.yaml](003-trading-system.ply.yaml).

## The design under test

Seven top-level concerns; market data flows in the left column, orders flow down the
right, fills flow back:

```
ingest [ feed → ring → decoder → book ]      (all of vetting 002, one level down)
   book ~> strategy [ signals ] ~> oms ~> gateway ~> venue
                                   oms -> risk   (pre-trade check)
                                   oms ~> pnl    (fills)
```

- `risk.check_order` is the highest-value verification target in the system — a pure
  decision function (order + limits in, allow/deny out) carrying the heaviest checks
  (`bounded(3), fuzz(4096), mutate`).
- `gateway.send` is honestly unclaimed: venue I/O can't be harnessed; its evidence is
  a `trusted` claim naming the exchange certification run, plus the deny wall
  (`* -> gateway except oms`).
- Two open decisions ride along: `#8` venue failover (workspace-level) and `#9` order
  id policy after a reject (pinned to `Oms::submit`).

## What this probes (and 001/002 could not)

1. **Nesting.** `ingest` holds four child components; `strategy` holds `signals`.
   First recursive render, first nested aggregation.
2. **Dotted references.** Outer edges target `ingest.book`, `ingest.ring`; a deny
   rule's `except` list carries dotted entries (`except ingest.decoder, strategy`).
   Bare `signals` tests unique-leaf resolution from outside its parent.
3. **Aggregation at depth.** The kernel's container rule (fold children only) now has
   a two-level tree to be right about; `ingest`'s rolled-up verdict is the number a
   collapsed box should someday display.
4. **The zoom model.** §7 promises collapse/expand; §7.1 has a visual form for a
   collapsed box (weakest descendant's fill) but verdicts don't exist yet and the
   renderer has no collapse. Expected finding, recorded when the render pass runs.
5. **Component reuse pressure.** 002's pipeline is hand-copied here, dotted paths
   rewritten throughout — 002's finding 2 (no import mechanism) felt at full size.

## Planned probes for the tool runs

- Ambiguity (E0206): temporarily refer to bare `book` while both `ingest.book`
  exists — the validator should name the candidates and demand the dotted form.
- Scoping gap: all of `ingest`'s *internal* edges must be written at top level with
  full dotted paths (`ingest.feed -> ingest.ring`) because `edges:` only exists on
  the document — there is no component-scoped edge list. Candidate finding.

## Runs

Pending — the renderer is mid-rework (findings layer); `ply-check`, the render pass,
and findings will be recorded here when it lands. Nothing below this line is verified
yet.
