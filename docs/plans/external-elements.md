# Proposal — external systems and actors, and why they belong in an evidence tool

Status: **proposed, for the §7.1 gate to accept or refuse.** Not specced, not built.
Origin: C4's Level 1 (context) suggested the shape; vetting 003 and §5.4a supplied the
reason. Date: 2026-08-24.

## 1. The gap, in the project's own evidence

Vetting 003 declares a `gateway` component whose evidence is a `trusted` claim reading
"venue protocol conformance", and an unresolved entry about "venue failover". The **venue
itself does not exist in the model.** Grep the scenario's YAML: it appears twice, both
times as prose inside other constructs.

So the system's most important boundary — the line past which Ply can never verify
anything, ever, on purpose — is representable only as absence. And absence already means
something else: `unclaimed`. Today a component nobody has got around to specifying and a
system somebody else operates render identically.

§7.2 carefully distinguishes three kinds of unspecified: the **floor** (permanently
unspecifiable by design), the **below-watermark body** (not owed — verified, not
declared), and `unresolved!` (**owed but missing**, tracked and numbered). An external
system is a genuine fourth: **out of scope by ownership**, not by expressibility. It will
never be claimed, and that is correct rather than pending.

## 2. Why this earns its place semantically, not just visually

The decorative version of this feature is not worth building. The load-bearing version is.

**An `ensures` is a promise; a `requires` is an obligation on the caller.** Ply's whole
modular story (D5) rests on discharging that obligation: every caller is checked against
its callee's contract, so a `requires` is someone's job to satisfy, and Ply knows whose.

**Except at an entry point.** When the caller is outside the workspace — a person, an
exchange, a network peer — the `requires` cannot be discharged by anyone Ply can check.
It stops being an obligation and becomes an **assumption about the environment**: a claim
that the outside world will only ever send conforming input. That is a materially
different evidence status from every other `requires` in the tree, and Ply currently
cannot express it or count it.

This matters because it is the classic place correctness arguments leak. A function proved
`bounded(3)` under `requires(tick.px > 0)` is proved *conditionally on nobody outside ever
sending a non-positive price*. If that entry point is reachable from a venue feed, the
proof's real strength depends on a validation step somewhere upstream — and Ply's audit
surface should say so, the same way `conditional` says which contracts a verdict assumed.

So the proposal is not "draw the venue." It is: **name the boundary, so entry-point
preconditions can be counted as environmental assumptions in `audit` rather than passing
as ordinary discharged obligations.**

## 3. Grammar shape (sketch, deliberately minimal)

```yaml
externals:
  venue:
    kind: system              # system | actor
    note: "the exchange: accepts orders, returns fills"
  trader:
    kind: actor
    note: "submits and cancels orders through the strategy API"

edges:
  - trader -> strategy.Strategy::submit     # entry point: an actor reaches this fn
  - "gateway ~> venue : FixMessage"         # egress to an external system
```

Rules the sketch implies, each needing a decision:

- **Externals share the component reference namespace** so edges can name them, and
  collide with components under the existing duplicate-name error (E0202). §5.1a rule 6's
  unique-leaf resolution applies unchanged.
- **An edge touching an external is declared, never checked.** There is no code for
  `trader`, so no call graph can confirm or refute it. This must be structurally obvious,
  not a footnote — the precedent is `~>` flows, which §5.1 already marks "parsed and
  rendered, NOT checked in v1". An external edge that *looked* checked would be exactly
  the evidence-lying failure this project exists to prevent.
- **An edge from an external to a fn marks that fn an entry point.** Its `requires`
  clauses are then environmental assumptions: listed in `cargo ply audit` beside assumed
  contracts and trusted claims, and counted as open items.
- **Externals carry no verdict and no ceiling.** They are not claimable; they fold into
  aggregation the way a container with no claimable descendants does (§7's container
  rule), contributing nothing rather than `unclaimed`.

## 4. The §7.1 channel argument

The gate's question is not "is this useful" but "does it draw, in an unused channel or an
existing one used consistently."

**Proposed channel: position — outside the workspace frame.** The argument for it is that
this is not a new channel at all. Position already carries exactly one meaning in Ply's
grammar: **containment**. Inside a box is inside that component; nesting is the tree. Then
"outside the workspace frame" is the same meaning extended one level up — *not inside our
system* — and it borrows an instinct every reader already has, which is §7.1's stated bar.
Nothing else currently lives outside the frame, so the region is free.

Two secondary questions the gate must settle:

- **system vs actor.** C4 uses a person silhouette. Ply uses shape lightly (boxes,
  chips), so a distinct outline in the external region is probably admissible — but this
  is the weakest part of the proposal and the gate may reasonably say the distinction is
  prose for a tooltip, not a form.
- **the entry-point mark.** My recommendation is **no new mark**: an edge crossing the
  frame boundary already says it, and inventing a chip mark for something position
  already encodes is the channel-reuse §7.1 refuses.

Explicitly *not* proposed: hue (spoken for — red forbidden, green evidence, amber human
attention, ink structure), border (dashed hollow, solid specified, double sealed), or
saturation (pastel promised, saturated earned).

## 5. What it would do to vetting 003

`venue` becomes an element outside the frame with `gateway ~> venue : FixMessage` crossing
the boundary, and the deny wall `* -> gateway except oms` visibly guards the only route to
it. `gateway.send`'s honest unclaimedness stops reading as an omission and starts reading
as a boundary — which is what the scenario's prose already says in words the picture
cannot currently show. The existing `trusted` claim gains something to point at.

If a `trader` actor is added, `Oms::submit`'s preconditions surface in `audit` as
environmental assumptions — and open decision #9 ("order id after venue reject") is
revealed as a question *about the boundary*, which is why nobody has been able to close it
from inside.

## 6. Scope limits

This proposal is Level 1 only. It does **not** bring in C4's container or deployment
levels (they would break the invariant that every construct anchors to real code), its
dynamic/sequence views (§7.2 parks temporal rules deliberately, admitted one at a time
through this same gate), or its legends-by-default convention (§7.1 argues the opposite
via the squint test, and `--legend` is already the opt-in answer).

## 7. Open questions

1. Do actors and systems differ enough to need distinct visual forms, or is `kind:` a
   tooltip distinction?
2. Should an entry point's `requires` change the *verdict* (a status like
   `environmental`), or only the `audit` surface? Changing the verdict is stronger and
   riskier; the audit-only version is cheap and reversible. Recommend audit-only first.
3. Do externals belong in `ply.yaml` at all, or are they a property of edges alone
   (`-> external:venue`)? The block form is more explicit and gives `note:` a home.
4. Does a vetting scenario need to force this before it enters? The project's own rule is
   that findings become spec changes — 003 arguably already produced the finding, but it
   was recorded as prose rather than as a numbered finding at the time.
