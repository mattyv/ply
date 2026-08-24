# Proposal — external systems and actors (C4 Level 1), in reduced form

Status: **recommend adoption in reduced form, gated on a vetting re-run before any spec
amendment.** Not specced, not built. This document supersedes the 64644c6 draft; where it
disagrees with that draft it says so inline, because the disagreements are findings too.
Origin: C4's Level 1 (context) suggested the shape; vetting 003 and §5.4a supplied the
reason. Date: 2026-08-24.

The reduced form, in one paragraph: a top-level `externals:` block declaring named
outside parties (name + required `note:`, **no `kind:` field**); externals may appear as
endpoints of `~>` flow edges only, never `->` call edges or `deny` patterns; a per-fn
`entry:` field names the externals that can reach that fn, which turns the fn's
`requires` clauses into **environmental assumptions** listed on `cargo ply audit`'s
trust surface (audit-only — no verdict change, no new status, not counted as open
items); externals draw as solid-bordered, unfilled, anchor-less boxes **outside the
workspace frame**, with flow and entry edges crossing the frame border. Everything cut
from the draft, and why, is in §3.

## 1. The gap, in the project's own evidence

Vetting 003 declares a `gateway` component whose entire correctness story is a boundary:
a `trusted` claim reading "venue protocol conformance", a deny wall
(`* -> gateway except oms`), and two open decisions (#8 "venue failover", #9 "order id
after venue reject"). The **venue itself does not exist in the model.** Grep the
scenario's YAML: "venue" appears three times, every time as prose inside another
construct (the trusted claim's text, and both unresolved notes). Four constructs orbit a
boundary the picture cannot show.

So the system's most important line — the one past which Ply can never verify anything,
on purpose — is representable only as absence. And absence already means something:
`unclaimed`. Today a component nobody has specified yet and a system somebody else
operates render identically, and §7.2's taxonomy has no slot for the latter. §7.2
distinguishes three kinds of unspecified: the **floor** (permanently unspecifiable by
design), the **below-watermark body** (verified, not owed), and `unresolved!` (owed but
missing, tracked and numbered). An external is a genuine fourth: **out of scope by
ownership.** It will never be claimed, and that is correct rather than pending.

Vettings 001 and 002 do not have this gap at the same severity, which localizes it: 001
is a single library component with no boundary story at all, and 002's boundary
(`Feed::pump`'s socket) is adequately told by the `net` capability badge plus honest
unclaimedness, because nothing else in 002 *refers* to the outside party. The gap
appears exactly when multiple constructs share one boundary — which is what a system
diagram, as opposed to a component diagram, is for. That is also why this was C4's
Level 1 in the first place.

## 2. The semantic argument, interrogated

The draft's claim: an entry point's `requires` cannot be discharged by any caller Ply
can check, so it is an assumption about the environment — a different evidence status
the tree cannot express. **Verified against the spec, this is true:**

- D5 discharges a `requires` at call sites: every caller Ply verifies is checked against
  its callee's contract. A fn whose real callers are outside the workspace has no such
  site. Its own proof (`proof_for_contract`) *assumes* its `requires` — correctly, that
  is what `requires` means — and nothing anywhere records that for this fn the
  assumption is load-bearing against the world rather than against checked callers.
- The failure mode is the classic one: `decode` proved under `requires(frame.len <= 64)`
  is proved conditionally on nobody outside ever sending a longer frame. Ply's audit
  surface lists assumed contracts and trusted claims precisely so conditional strength
  is countable; the entry-point condition is the same kind of fact and is currently
  invisible.

**But the argument is narrower than the draft presents it.** Two honest qualifications:

1. The audit payload alone does not require drawn externals. A bare per-fn marker
   (`entry: true`) would surface requires-as-assumptions with zero visual grammar. What
   requires the drawn element is (a) §7.1's bijection — if entry-point-ness is declared
   it must be drawable, and the natural drawing of "an obligation dischargeable by
   nobody inside" is an arrow from outside — and (b) naming the assumption's owner.
   Vetting 003 is the evidence for (b): venue's boundary is shared by a trusted claim, a
   deny wall, an egress flow, and two open decisions, and only a shared name lets audit
   group assumptions by counterparty and lets the picture show one boundary instead of
   n anonymous ones. If the gate finds (b) unconvincing, the honest fallback is the
   flag-only form, not refusal — the semantic core survives without the named node.
2. Why not *compute* entry points instead of declaring them? Ply's extractor knows the
   call graph, so "pub fn with no in-workspace caller" is derivable. Rejected on D11's
   own grounds: syn-based call data is approximate, so a fn can look caller-less merely
   because its call sites didn't resolve — a computed entry-point list would silently
   inflate the trust surface with extraction noise. Declaration is the honest form here,
   the same way D4 only default-denies over facts that are sound.

Is the decorative reading avoidable — "this just makes diagrams prettier"? For the
`entry:`/audit half, yes: it changes what `cargo ply audit` reports, which is evidence
surface, not decoration. For the drawn external, the test is whether it changes what a
reviewer can *see wrong*: in 003, the deny wall currently guards the route to a place
that isn't on the map, and `gateway`'s unclaimedness reads as omission rather than
boundary. Those are misreadings of the review surface, the same class of defect as
`owns` being parsed-but-drawn-nowhere (vetting 001 finding 2, fixed as a bijection
violation). The part of the draft that *was* decoration — the actor/system `kind:`
distinction — is cut below.

## 3. Grammar shape — the reduced form, with cuts named

```yaml
externals:
  venue:
    note: "the exchange: accepts orders, returns fills; market data source"

components:
  oms:
    fns:
      Oms::submit:
        entry: [venue]            # reachable from outside; requires become
                                  # environmental assumptions on the audit surface
edges:
  - "oms ~> gateway : Order"
  - "gateway ~> venue : FixMessage"    # egress: flow to an external, declared-not-checked
  - "venue ~> ingest.feed : RawFrame"  # ingress: flow from an external
```

Rules:

- **Namespace.** Externals share the component reference namespace: a name collision
  with any component (or another external) is the existing duplicate-name error
  (E0202), and §5.1a rule 6's unique-leaf resolution applies unchanged. Externals are
  top-level only — they have no interior and cannot nest.
- **`note:` is required.** An external is nothing but a name and a sentence; a bare
  name tells a newbie nothing, and the tooltip must carry its own gloss.
- **Externals appear in `~>` flow edges only.** A `->` call edge or a `deny` pattern
  touching an external is an error (new E02xx code) whose message says why and points
  at `~>` or `entry:`. This is deliberate and load-bearing: today **every solid arrow
  in a Ply diagram is a checked claim** (crate tier, §5.3) and every dashed arrow is
  declared-not-checked (`~>`, "parsed and rendered, NOT checked in v1"). An external
  endpoint can never be checked — there is no crate. Routing externals through the
  flow syntax keeps the existing honesty line intact with zero new machinery: solid
  stays checked, dashed stays declared. A `deny` over an external is unenforceable
  theater (Ply cannot observe the venue's calls) and is refused for the same reason.
- **A flow needs one workspace endpoint.** `external ~> external` involves nothing of
  ours and is an error.
- **`entry:` lives on the fn claim, not in `edges:`.** *Disagreement with the draft*,
  which wrote `trader -> strategy.Strategy::submit` — an edge targeting a fn. That
  sketch quietly introduces fn-level edge endpoints, a namespace surface the whole
  edge/deny/resolution/rendering stack lacks (fn claims are deliberately not edge or
  deny endpoints today), and it would immediately invite fn-level *internal* edges —
  large adjacent scope the draft never costed. Putting the declaration on the fn claim
  keeps `edges:` component-only, puts the entry fact next to the `requires` it
  reinterprets, and follows the existing pattern of per-fn declarations (`trusted`,
  `unresolved`). Each name in `entry:` must name a declared external (error otherwise);
  the renderer derives a drawn crossing edge from it (derived drawings are established
  practice — the ceiling fill is computed too).
- **No `kind: system | actor`.** *Disagreement with the draft*, which carried the field
  and admitted the visual distinction was its weakest part. Cut entirely: a person and
  an exchange differ in prose, not in evidence semantics — both are "a party we cannot
  check" — and the distinction is exactly the decoration this grammar refuses. The
  `note:` says what the thing is. If a future vetting scenario shows the distinction
  doing semantic work (e.g. different default assumptions), it can re-enter through
  this same gate with that evidence.
- **Externals carry no verdict, no ceiling, no checks, and are not nodes of the §7
  verdict tree.** Nothing to aggregate: they appear in the §8 envelope (if at all) as a
  sibling list, never inside `root`. *Correction to the draft*, which said externals
  "fold into aggregation the way a container with no claimable descendants does,
  contributing nothing rather than `unclaimed`" — §7's container rule says the opposite
  (such a container reads `unclaimed`). The clean statement is that externals are not
  in the tree, so the kernel never sees them and no kernel invariant is touched.
- **Environmental assumptions are trust surface, not open items.** *Disagreement with
  the draft*, which wanted them "counted as open items". Open items are things owed
  (unresolved markers, weak specs, stale claims); a correctly declared environmental
  assumption is permanent, like a profile escape or a trusted claim — audit-listed,
  never pressure-counted. Counting it as open would push users to delete honest
  declarations to make a number go down.
- **Verdicts are untouched.** No `environmental` status, no `conditional` downgrade for
  entry fns (draft open question 2 — resolved here as audit-only). The fn's own proof
  legitimately assumes its `requires`; that is unchanged and correct. The new fact is
  *where the assumption's discharge would have to come from*, and that is an audit
  fact. If experience shows audit-only is too weak, a status is an additive later step;
  the reverse migration would be a breaking retraction.
- **Staleness: externals have none, and the spec must say so.** There is nothing to
  fingerprint — no body, no contract, no evidence string. An external can silently
  outlive the reality it names (the venue is replaced; the entry list rots). This is
  accepted with eyes open (see §5), and the mitigation is wording, not machinery: the
  tooltip and the audit line always carry "declared, never checked by Ply".

## 4. The §7.1 channel argument, corrected

**Proposed channel: position — outside the workspace frame.** The draft argued this is
"not a new channel at all" because "position already carries exactly one meaning:
containment", and "nothing else currently lives outside the frame, so the region is
free". Checked against the actual renderer, that needs three corrections, after which a
weaker but still sufficient version survives:

1. **Position inside the frame also carries derived layout meaning.** Since the vetting
   002 render findings, top-level components sit in *ranked rows* computed from the
   edges — upstream above downstream — so vertical position visibly encodes flow
   direction. This is layout, not grammar (nothing declared maps to rank; §7.1's table
   and channel list never mention it), so the "one declared meaning" claim survives,
   but the proposal must state it as *containment is the only declared meaning of
   position*, not "position means only containment".
2. **The frame's margins are not empty.** Deny wildcard `*` nodes occupy dedicated
   margin columns just inside the frame edge (left for `from`, right for `to`), and
   they mean "any component *inside* the workspace" — insiders drawn at the periphery.
   Externals one border-width further out invite the misreading that `*` is external
   too. The discriminators are real (circles inside vs boxes outside; the frame line
   between them) but the proposal owes an invariant test, not an assurance: *no
   external box intersects the frame; every `*` node stays inside it; every edge with
   an external endpoint crosses the frame border exactly once.* Same family as
   `no_drawn_element_intersects_a_box_it_is_not_inside`.
3. **The frame border must survive the squint test as a boundary, not a decoration.**
   Blurred, "beyond the titled frame" must still read as *not ours*. The frame is
   currently a thin 1px line sized to be the canvas; with content outside it, it may
   need more visual weight (it stops being the canvas edge and starts being a drawn
   boundary). That is a renderer judgment the implementation makes; the gate should
   rule on the mapping, and the vetting re-run (§6) is where the squint test actually
   gets applied to a real picture.

With those corrections the core argument stands: *inside the box = part of the
component* extends to *inside the frame = part of the system* with no new channel and
no second meaning — it borrows the instinct §7.1 demands, and the region is genuinely
unused.

**The external box's own form**, chosen to steal nothing:

- **Solid border.** Dashed is taken (hollow sketch, "nothing inside *yet*") and would
  be actively wrong: an external is not expected to solidify.
- **No fill, ever.** It is not on the verdict scale — not even `unclaimed`, which
  means "owed nothing declared yet". The tooltip says so: outside the workspace, Ply
  checks nothing about it, and never will.
- **No anchor subtitle, no badges, no chips.** The absence of the anchor line — which
  every component box carries — is itself a quiet secondary cue at reading distance.
- **Tooltip (newbie bar):** name the thing, say what it means, say why it matters:
  *"⟨name⟩ — a system or person outside this codebase: ⟨note⟩. Ply draws it so the
  boundary is visible, but checks nothing about it — every arrow touching it is a
  declaration, not a verified fact."* Exact wording golden-tested like all user-facing
  text.

**Edges crossing the frame.** Egress/ingress flows are ordinary `~>` dashed arrows —
already the declared-not-checked form — that happen to cross the border. The derived
`entry:` arrow is also drawn dashed, labeled `entry`, with a tooltip listing the fn and
each `requires` clause now standing as an environmental assumption. This means the dash
channel's stated single meaning, "data flows", becomes **"declared, not
machine-checked"** — a *restatement that subsumes* the current meaning (flows are
exactly that) rather than a second meaning, and arguably the more honest name for what
dashes always meant here. But it is an amendment to a channel's declared meaning, which
is the gate's own territory: **flagged as open question 1 rather than assumed.** If the
gate refuses the restatement, the fallback is to draw `entry:` arrows as flows with no
label-type (weaker, still honest) or to find a third line style — and a third line
style for a construct this rare is probably decoration, which would argue for the
flag-only reduced-reduced form of §2.1.

**The entry-point mark:** none, agreeing with the draft — the crossing itself says it,
and a chip mark restating what position already encodes is the channel reuse §7.1
refuses.

Explicitly not touched: hue (red forbidden, green evidence, amber human attention, ink
structure), saturation (pastel promised, saturated earned), border weight meanings,
small marks.

## 5. Actors — the anchor-nothing tension, faced directly

Every other construct in Ply anchors to real code, and `check` breaks CI when the
anchor rots (E0301: "a renamed function must break CI, not silently orphan its
claims"). An external anchors to nothing, so nothing can ever break: the venue can be
decommissioned and the box stays forever, fresh-looking. Is that fatal?

The judgment here: **acceptable, because the construct is engineered to have nothing to
lie about** — and the reduced form is what makes that true:

- An external carries **no verdict, no fill, no ceiling, no checks** — it can never
  render as evidence, stale or otherwise. The failure mode of rot is a mislabeled
  boundary in a picture and a misattributed assumption owner in audit: real costs, but
  the *trusted-claim class* of cost (a human-maintained statement going quietly
  outdated), not the evidence-lying class (a green thing that isn't).
- The precedent is squarely `~>` flows: declared, rendered, never checked, accepted in
  the grammar since day one. The reduced form routes all external edges through exactly
  that syntax, so no new "looks checked but isn't" surface exists — which was the
  draft's own stated red line, here made structural instead of promised.
- Trusted claims got staleness machinery because they *attest something about code*,
  and the code's content hash gives the machinery a handle. An external attests
  nothing; there is no handle, and inventing one (hash the note?) would be theater.
  The honest treatment is the standing label "declared, never checked", present in
  every tooltip and audit line, plus the fact that the `entry:` list lives in the same
  reviewed YAML as everything else.
- The one place rot could touch evidence is `entry:`: a stale entry declaration keeps
  listing environmental assumptions for a fn no longer externally reachable. That
  overstates the *trust surface*, never the verdict — the conservative direction. The
  reverse omission (an entry point nobody declared) is today's status quo, so the
  feature cannot make it worse.

What would *not* be acceptable, and is therefore excluded: externals with checks,
contracts, capabilities, ownership, or nested structure — anything that feeds the
kernel or wears evidence channels. The moment someone wants to *claim* something about
the venue's behavior, the existing honest form is a `trusted` entry on the workspace
side of the boundary, with evidence and staleness — not an enriched external.

## 6. What it does to vetting 003 — and the gate this proposal sets for itself

Applied to 003: `venue` becomes a box outside the frame; `oms ~> gateway : Order`
continues out as `gateway ~> venue : FixMessage`; the return path enters as
`venue ~> gateway : Fill`; market data's true source appears as
`venue ~> ingest.feed : RawFrame` — closing the left column's currently-dangling start.
The deny wall `* -> gateway except oms` now visibly guards the only route to a place
that is on the map; `gateway`'s honest unclaimedness reads as a boundary rather than an
omission; the `trusted` claim and both venue-flavored unresolved notes (#8, #9) gain a
referent. `Oms::submit` (via `entry: [venue]` — order rejects come back from the venue)
surfaces its future `requires` as environmental assumptions, and open decision #9 is
revealed as a question about the boundary, which is why nobody has closed it from
inside.

Two honest limits of the demonstration:

- `Feed::pump` as an entry point surfaces *nothing*: it has no contract, so there are
  no assumptions to list. The audit line for a bare entry point should say exactly that
  ("externally reachable, no preconditions declared") — which is itself information,
  arguably the sharpest kind. Whether that deserves a lint is left as a one-line note
  for the user, not proposed (scope discipline).
- The draft's `trader` actor is not in 003's prose — 003's order source is `strategy`.
  Inventing a trader to show off the feature would be exactly the decoration this
  proposal refuses. If a scenario needs a human actor, it must be a scenario that
  actually has one.

**The gate condition.** The project's rule is that vetting findings drive spec changes,
and 003 recorded this gap only as prose, never as a numbered finding. So the sequencing
this proposal binds itself to, per draft open question 4 — answered **yes**:

1. Record the gap as a numbered finding in 003 (a one-paragraph addendum, separate
   change, not made here).
2. Re-run 003 with the proposed grammar in a scratch copy — YAML written, rendered,
   squint-tested, tooltips read cold — and record what held and broke, exactly as 001's
   render pass did for the forms that existed only as table rows. §7.1's own gate-debt
   note is the cautionary tale: a table row is not a drawn form.
3. Only then amend The-Ply-Spec.md, with the vetting record as the finding.

If step 2 breaks the channel argument on a real picture (the frame doesn't read as a
boundary; the `*`-node adjacency confuses), that is a refusal of the visual form, and
the flag-only fallback (§2.1) goes to the gate instead.

## 7. Implementation cost, checked against the actual code

Not fiction: the touch points below were read, not guessed.

- **`tools/model`** (~small; hours): `External { note }` struct + `externals:` map on
  `Document`; `entry: Vec<String>` on `FnClaim`; no micro-syntax changes (flow edges
  already parse; externals are just names that resolve differently). Serde +
  `deny_unknown_fields` handles the schema surface.
- **`tools/check`** (~half a session): fold externals into the leaf index /
  `resolve_component_ref` namespace (E0202 duplicate, E0206 ambiguity — both extend
  naturally, both already tested); new rules: external endpoint in `->` or `deny` (new
  E02xx, message with the `~>`/`entry:` redirect), `external ~> external`, unknown name
  in `entry:`, external named nowhere (a `W` — declared but unreferenced, mirrors
  nothing existing so needs a wording pass). All document-local; no anchors involved,
  so no E0301 interaction.
- **`tools/render`** (the bulk; 1–2 sessions): an external band outside the frame
  (placement: externals stack on the side nearest their first workspace counterpart —
  algorithmic detail left free, the mapping fixes only "outside the frame"); canvas
  grows beyond the frame for the first time (today `width/height` *are* the frame —
  a real but mechanical layout change); frame-crossing edge routing (border_toward
  already computes box-edge intersections; the frame is just another rect); derived
  `entry:` arrows; tooltips with exact-string tests; the three existing invariant walks
  extend automatically (`every_painted_element_resolves_a_style_rule`,
  `every_drawn_item_resolves_a_tooltip` — new constructs cannot skip them, by design),
  plus the new frame-crossing invariant from §4.2.
- **`crates/` (cargo ply), later, at M5 audit**: ~1 session incremental — `entry:`
  flows through config/model, audit gains an "environmental assumptions" section
  (fn, external, each requires clause verbatim), envelope addition is additive
  (allowed post-M3). No kernel change of any kind: externals never enter the verdict
  tree, so the enumeration gate is untouched — which is itself an argument that the
  reduction is correctly scoped.
- **Spec deltas when adopted** (sketch, not applied here): §5.1 document structure +
  example; §5.1a rule 6 note; §5.3 a short "external edges" paragraph (declared-never-
  checked, the `->`/`deny` refusal); §7.1 two table rows (external box; entry edge) and
  the dash-channel restatement if accepted; §7.2 the fourth kind of unspecified; §6
  audit line. Schema + goldens accordingly.

Total: roughly **2–3 sessions** for the tools + vetting re-run, ~1 later session in
`cargo ply` at audit time, spec/schema pass alongside adoption.

## 8. Scope limits

Level 1 only. Explicitly not entering: C4's container/deployment levels (would break
the anchors-to-real-code invariant for things that *should* have anchors),
dynamic/sequence views (§7.2 parks temporal rules; one at a time through this gate),
legends-by-default (§7.1 argues the opposite; `--legend` exists), external contracts or
capabilities (§5), `kind:` (§3), any verdict/status change (§3), fn-level edge
endpoints (§3), computed entry-point inference (§2.2), and any lint on unguarded entry
points (noted in §6 for the user to want or not).

## 9. Open questions a human must settle

1. **The dash-channel restatement** (§4): is amending "dashed = data flows" to "dashed
   = declared, not machine-checked" a legitimate subsuming restatement or the channel
   reuse §7.1 refuses? This is the proposal's one genuine gate-rule question; the
   fallback paths are stated in §4.
2. **Named externals vs the flag-only form** (§2.1): does the assumption-owner /
   shared-boundary argument justify the `externals:` block, or should v1 ship
   `entry: true` + audit only, with drawn externals waiting for more vetting pressure?
   This proposal says the 003 evidence (four constructs orbiting one unnamed boundary)
   justifies the named form, but it is a judgment call, not a proof.
3. **Envelope timing**: do externals enter `--json` output now (as a top-level sibling
   of `root`) or only when `cargo ply audit` lands? Additive either way; deciding late
   costs nothing.
4. **`entry:` granularity**: fn-only (proposed) is the minimal form that carries the
   requires payload. Is a component-level `entry:` (whole surface reachable) wanted for
   coarse early modeling, or is that an invitation to over-claim reachability?
