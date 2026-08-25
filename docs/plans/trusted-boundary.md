# Proposal — trusted boundary (`given:` regions), in reduced form

> **SUPERSEDED BY ITS OWN GATE, 2026-08-25 — `tests/spike/havoc/FINDINGS.md`.**
> The gate this document sets for itself in §7 has run, and it returned the negative
> outcome §10's closing paragraph names in advance. Measured: **2 of 8 crossings pass
> under havoc (25%)**, and both passes are 004's own functions; **none of the six
> callers written without this experiment in mind passed**. The §7 prediction about
> `tier_fee_cents` **held** (133.74s, inside §6's 300s floor) and cost is not an
> objection (havoc costs the same as the declared-contract stub), but the empirical
> claim the recommendation rests on — "real callers are defensive at boundaries" —
> did not survive a sample that was not chosen for it. The spike also found three
> outcomes this document has no row for: a havoc'd loop bound that times out with no
> witness; breaking values that name the direction of the missing contract but never
> its threshold; and `extract_witness_bytes` reporting the least useful witness where
> Kani emits several. **The standing recommendation is now open question 6's fallback**
> — a clause-free per-callee boundary entry means havoc, no new grammar — and the
> region, the fill-channel restatement, the lock inventory and the §7.2 slot are
> refused. Everything below is left as written, as the record of the argument the gate
> was run against.

Status: **recommend adoption in reduced form, gated on a vetting re-run (an extension of
004's own `run.sh`) before any spec amendment, with the failure-side payoff explicitly
sequenced behind the Kani pin spike already in flight (which reported 2026-08-25:
witness recovery through a stub already works at the pinned Kani, so that sequencing
constraint is discharged — `tests/spike/kani-pin/FINDINGS.md`).** Not specced, not built. Origin:
the maintainer's idea, 2026-08-25; TODO.md carries the three conditions agreed up front
(never reads as evidence; counted on the audit surface; must draw). Date: 2026-08-25.

The reduced form, in one paragraph: a component may declare **`given: "<reason>"`** —
the component and everything under its anchor is taken as given. At a `bounded`
crossing, a call into a given region is stubbed with an **unconstrained symbolic return**
of its resolved type — the empty contract — never descended into, and no longer refused
outright. A proof that passes anyway is real evidence *about the caller* (it holds for
every value the region could return) and keeps its verdict with status `conditional`,
the assumption naming the region and what little was assumed (the call returns, does not
panic, mutates nothing the caller reads). A proof that fails is **not a violation** — it
is the named fact that the caller's claim depends on what the region returns, reported
as an absence whose diagnostic names the callee and the breaking return value — which
is precisely the contract the proof needs, with a witness for why. (Witnesses were the
pin spike's question; it reported 2026-08-25 that they already survive stubbing at the
pinned Kani, fabricated callee return included — `tests/spike/kani-pin/FINDINGS.md`.) A per-callee declared contract always wins over
the region. The region is counted on `audit`'s trust surface, and its fn inventory is
fingerprinted in `ply.lock` so that *growth* — not content change — trips `stale`.
It draws as an amber-washed component box (§6 below). Everything cut from the idea as
framed, and why, is marked *Disagreement with the framing* inline.

Two readings of the idea are **refused** inside this proposal, by name, because each is
the rubber stamp in a different coat:

- **Trust-and-descend** ("take the body at face value, so inline it"): 004 measured
  this. Inlining one `BTreeMap`-behind-`OnceLock` lookup took the flagship function from
  `bounded(2)` in 1m20s to `timeout` at 600s (11m23s wall clock). Trust does not make
  symbolic execution cheaper; the refusal in §5.5 exists because descending was never
  the honest *or* the useful option. A boundary declaration cannot re-license it.
- **Trust-and-hush** ("the region means refusals inside it stop failing CI"): the
  verdict would stay `unclaimed` and nothing would be checked, so the region would be
  `--fail-on error` with a fence around it — a declaration whose entire effect is that
  absence of evidence stops being loud. That is the fourth-time failure mode verbatim,
  and it also delivers nothing: the user wanted the feature *checked*, not the nagging
  scoped. If the gate strips the reduced form down to this, the honest outcome is
  refusal, not the quiet version.

## 1. The gap, in the project's own evidence

Vetting 004 built exactly the situation this idea is for: a fragment-first feature
beside a two-year-old module, one function calling across. §5.5's per-callee rule (just
landed) gives that crossing three honest outcomes: refuse and name the callee
(`W0512`); or declare a contract for the callee and earn `conditional` `bounded(2)`
(measured: 201.77s, `W0511`, the assumption verbatim in the envelope); or drop to
`fuzz(n)`, which crosses by running. That works, and it scales per function.

The maintainer's complaint is the scaling: a feature touching a dozen legacy functions
means a dozen hand-written contracts nobody can verify anyway. Interrogated, this
complaint is **stronger than it looks, and for a worse reason than ergonomics**. The
per-callee contract is not just tedious — it is the *launderable* route. The post-004
adversarial review's G2 (open on TODO.md as one item) says why: a declared boundary
contract is unchecked against the real body, unfingerprinted, and not even
vacuity-checked — `ensures: ["|result| false"]` yields a clean `conditional bounded(2)`
from a contract that describes nothing. Twelve contracts invented under W0512 pressure
are twelve opportunities to write green paint, each producing conditional *bounded*
evidence resting on a human guess. The tool's current shape actively pushes a user
toward bulk-authoring exactly the artifact its own review flagged as the laundering
channel.

There is also a taxonomy hole, recorded in TODO.md when the idea was: §7.2 now
distinguishes four kinds of unspecified (the floor; the below-watermark body;
`unresolved!`; the external). *Our code, checkable in principle, deliberately not
checked* is a fifth, and today it renders as `unclaimed` — 004's `ledger` draws as a
dashed hollow box, the form whose declared meaning is "nothing to zoom into *yet*,
expected to solidify." A two-year-old module nobody intends to claim is not "yet", and
a diagram of any legacy-extension codebase will be dominated by dashed boxes that all
read as unfinished when most of them mean *settled by decision*. This is the same class
of misreading that admitted externals (vetting 003: a boundary representable only as
absence), one ring closer in.

What the gap is **not**: a checking gap. §5.5 already never checks legacy code — the
refusal is not about the region, it is about what the *caller's* proof may do at the
call site. That reframing decides the whole design, next.

## 2. What "trust" can mean mechanically — the crux

*Disagreement with the framing*: "a region Ply takes at face value and does not check"
suggests trust is a checking policy. For the `bounded` tier it cannot be: Kani must
replace the call with *something*. The menu is closed and short —

1. **The real body.** Measured: `timeout` (004, twice, at 120s and 600s). Refused above.
2. **A contract-constrained symbolic return.** Needs the contract. This is §5.5's
   second branch, and it is per-callee by nature — the contract *is* the per-callee
   artifact. A region declaration carries zero information a stub can consume.
3. **An unconstrained symbolic return** — the empty contract, havoc. This is the only
   stub that exists without anyone writing anything, and it is what "trusted boundary"
   can honestly *mean* at a `bounded` crossing.

So the executable content of a trusted boundary is option 3, and its two outcomes have
exactly the right honesty properties, which is the reason to adopt rather than refuse:

- **Pass under havoc ⇒ strong evidence.** The caller's contract was proved for *every*
  value the callee could return. That is more than the declared-contract route
  establishes, not less — and it is evidence the caller *earned* by being defensive
  (004's `tier_fee_cents` caps the lookup with `.min(10_000)`; that line is why its
  proof should survive an unconstrained `u32`). The verdict stands, status
  `conditional`, because real assumptions remain: the call returns at all, does not
  panic, and does not mutate state the caller reads. These are the same residual
  assumptions §5.5's second branch already accepts silently; here they are the *whole*
  assumption, printed.
- **Fail under havoc ⇒ not a violation.** The breaking counterexample contains a
  fabricated callee return that the real body may never produce; reporting it as
  `violation` would be false evidence, and §8 forbids the witness-free form anyway.
  (**Corrected 2026-08-25 by the pin spike, which has now reported**: at the pinned
  Kani a stubbed failure *does* carry a witness, and the witness *does* include the
  fabricated callee return — `tests/spike/kani-pin/FINDINGS.md`, where
  `tiered_fee_halfclaim` yields `amount = 39663841, tier = 255, stubbed rate = 9217`.
  So the diagnostic can name the breaking value today, with no pin move. What the
  witness cannot do is become a red D7 test: written out against the real code it is
  green, because the real callee never returns 9217. That strengthens the case for an
  absence over a `violation` rather than weakening it.) The honest
  report is an **absence**: the caller's claim depends on what the region returns, so
  nothing was established — verdict `inconclusive`, run fails by default (§1, §6),
  diagnostic naming the callee and the breaking value when recoverable. Which is the
  quiet payoff: instead of twelve contracts written blind, the user writes the two or
  three the proofs actually demand, each dictated by a concrete witness of why.

Note what this makes the region **structurally unable to do**: trust can never
manufacture a claim about the region itself, and it can never make a non-defensive
caller green. Declare the entire workspace given and every claimed fn either passes for
reasons entirely its own (flagged `conditional`) or goes `inconclusive` and fails the
run. The whole-tree-goes-green failure this project has caught four times has no path
through this construct — condition 1 from TODO.md holds by construction, not by review.

And note the contrast with the route it relieves: the empty contract cannot be vacuous
(there is no `ensures` to make unsatisfiable), cannot be wrong (it claims nothing), and
cannot go stale in content (there is no content). G2's three-part laundering loop does
not apply to it. The blanket, precisely because it claims nothing, cannot lie; the
per-callee contract can. That inversion — the coarse construct being the *harder* one
to abuse — is this proposal's single strongest argument.

One near-equivalent must be named, because it decides distinctness honestly: the
per-callee havoc already almost exists. `StubSpec::render` generates exactly the
unconstrained stub when its clause lists are empty (`crates/ply-core/src/harness.rs` —
even `assumption_text` has a "(contract declared with no clauses)" branch), but the
config path skips clause-free entries (`crates/ply-cli/src/verify.rs`,
`if claim.requires.is_empty() && claim.ensures.is_empty() { continue; }`), so today the
route is dead code. A minimal alternative to this whole proposal is therefore: let an
explicit clause-free boundary entry mean havoc, per callee, no new grammar. What the
region adds over that fallback: one declaration instead of one per callee; coverage of
call sites nobody anticipated (a new call into the region next month is covered, not a
fresh `W0512`); a named boundary audit can group by and the picture can draw (the same
argument that carried `externals:` over `entry: true`); and the §7.2 slot. The fallback
is real and is listed as open question 6 — if the gate finds the named region
unjustified, the fallback keeps the semantics and drops the grammar.

## 3. Grammar shape — the reduced form, with cuts named

```yaml
components:
  ledger:
    anchor: ledger
    given: "pre-Ply ledger core; in production two years; rewrite not scheduled"
    fns:                              # optional, and this is the shrink path:
      fees::bps_for_tier:             # a declared contract inside a given region
        ensures:                      # wins over the region (§4) — the region
          - "|result| *result <= 10_000"   # shrinks callee by callee, visibly
```

- **The region is a component, not a path list.** *Disagreement with the framing*
  ("a named region of the workspace"): everything in Ply that names code does it
  through a component anchor, resolved, typo-checked (`E0301`), namespaced (§5.1a
  rule 6), and drawable as a box. A free-floating path list would be a second way to
  name code, with none of that. A given region is a component with `given:` on it;
  nesting gives containment for free (the subtree under the anchor is inside).
- **The name is `given`, not `trusted`.** *Disagreement with the framing.* §5.4d's
  `trusted` is the strongest word in the vocabulary and it is load-bearing: per-claim,
  human-attested, **evidence-named**, fingerprinted for staleness. A region declared
  with no evidence must not wear the same word — the maintainer's own first condition
  ("it must never read as evidence") argues against naming it with the evidence word.
  `given` is TODO.md's own phrase ("a region taken as given") and reads correctly cold.
- **`reason:` is the value, and it is required.** A bare `given: true` tells a newbie
  nothing; the tooltip and the audit line must carry their own gloss, same rule as
  externals' `note:`.
- **No `owner:` field.** Cut as decoration: the YAML line has `git blame`, and a name
  field rots into exactly the stale attestation this repo caught two days ago —
  machinery would then be owed for a field nothing needs.
- **No `expiry:`.** Cut deliberately, against the maintainer's own list of candidate
  strengtheners. An expiry converts honesty pressure into a snooze ritual — the date
  arrives, CI fails, someone bumps the date, and the field's existence now *implies*
  freshness that no one re-established. The pressure surface is audit counting plus
  inventory staleness (§5), which cannot be reset by editing a date.
- **No region-level contracts, checks, capabilities grants, or verdicts.** The moment
  someone wants to *claim* something about code in the region, the existing honest
  forms apply: a boundary contract on the fn (§5.5), or a human-attested `trusted`
  entry with evidence (§5.4d). An enriched region would be `externals` with an anchor —
  the "looks checked but isn't" surface both proposals refuse.
- **Schema surface**: one optional string field on `Component`
  (`tools/model/src/lib.rs`), mirrored in `validate_keys`'s vocabulary (§5.1a rule 1
  binds every reader — the post-004 fix that made two tools agree about one document
  must not be un-made by the first new key).

## 4. Semantics, exactly

**Resolution gains one outcome.** §5.5's classification currently produces Contracted /
Assumed / Unclaimed / Opaque / Unresolved (`crates/ply-core/src/callgraph.rs::CalleeStatus`).
A resolved first-party callee whose path falls under a `given` component's anchor and
that no declared contract covers classifies **Given** instead of Unclaimed. Precedence,
answering the layering question directly:

| callee inside a given region… | outcome |
|---|---|
| has a declared contract (inline or `ply.yaml`) | **contract wins** — Assumed, stubbed with the contract, `W0511`, `owed-evidence`, exactly as today. The region is a default, not a wall. |
| resolved, no contract, return type the codegen can build | **Given** — havoc stub; pass ⇒ verdict stands + `conditional` (assumption kind `given_region`, riding `W0511`'s existing `assumptions` machinery); fail ⇒ `inconclusive` + new `W05xx` naming the callee, region, and (pin permitting) the breaking return |
| resolved, no contract, return the codegen cannot build (`-> ()`, unsupported type), or taking `&mut` parameters | **refusal stands** — the existing `unstubbable` path, wording extended to say the region was seen and why trust could not help. A havoc that silently assumed "no mutation" for a `&mut` param would be the fifth appearance of the fail-open pattern; refuse it. (The same question stands unasked for the existing Assumed stub — noted, one line, not fixed here.) |
| Opaque (`W0513` — source Ply could not read) | **refusal stands.** Havoc needs the resolved signature; a region declaration is not permission to guess a return type. Not being able to look is still not the same as there being nothing there. |

**`W0512` for a call into a given region: it does not fire** — its two fixes (declare a
contract, or drop to fuzz) are exactly what the region supersedes and preserves,
respectively. Its replacement is not silence: the pass side carries the `conditional`
diagnostic with the region assumption verbatim; the fail side carries the new absence
diagnostic. There is no crossing that produces *no* line of output.

**Only the proof tier changes.** `fuzz`/`test` cross a given region the way they cross
everything — by running the real code — and that is unchanged; the region grants them
nothing and costs them nothing. The architecture tier (`ply-check`) is also unchanged:
edges, `deny`, capabilities, `owns`, profiles still bind a given component. Trust here
is about verification evidence at a call, not about structure — a legacy module that
suddenly opens a socket should still trip its missing `net` cap.

**Verdict and statuses.** A havoc-pass keeps the earned verdict (`bounded(k)`) with
status `conditional` — never clean, per condition 1 — and **without** `owed-evidence`:
that status means "nothing has checked the assumed contract against the real body," and
here there is no contract to check. The residual assumptions are dischargeable only by
the run tiers exercising the caller (a `fuzz` beside the `bounded` does cross for
real); whether audit should count "crossings no run tier has ever exercised" is open
question 3. A havoc-fail is an absence (`inconclusive`), so §6's default fails the run
— stated again because it is the load-bearing property: **a given region never makes a
run green that established nothing.** A genuine bug in the caller also surfaces as this
absence rather than as a `violation` (the harness cannot tell the two apart); that is
not a regression — today the same caller is refused outright and no violation is
reported either — and the diagnostic must say plainly that the fuzz tier can tell them
apart.

**No kernel change.** `conditional` already structurally carries assumptions;
worst-of, status propagation, and the 991,389-tree enumeration are untouched — the same
scoping argument that carried external elements, and the same evidence the scope is
right.

**Fn claims with `checks:` inside a given component**: the claim wins and a `W05xx`
notes that the region no longer covers that fn. Proposed as a warning, not an error,
because the natural shrink path is claims appearing *inside* the region one fn at a
time, and an error would force restructuring before the first claim. Open question 4.

## 5. Staleness — fingerprint the inventory, not the content

The maintainer's hazard question, taken seriously: `trusted` shipped without staleness
and a human's word would have outlived the code it vouched for; what is the analog
here, when a path-defined region has no content being attested?

The honest answer is that **content change inside the region is not the hazard —
growth is.** Nothing about `given:` attests what any function in the region does, so a
bug fix to a legacy fn invalidates nothing (there was nothing to invalidate);
fingerprinting bodies would trip `stale` on every routine legacy commit until the flag
meant nothing, or force re-attestation rituals that re-establish nothing. What the
declaration *does* silently license is a checking-free zone that new code can move
into: the team that writes next quarter's feature *inside* the given paths has opted
out of Ply without anyone deciding that. That is this construct's version of the
attestation outliving its subject.

Mitigation with a real handle: `ply.lock` records, per given region, the **item
inventory** under its anchor (fn names, per D14's existing fingerprint machinery — the
names, not the bodies) at declaration and at each `accept`. When the inventory grows,
the region carries `stale` (drawing the existing stale corner marker) and `audit` says
so in counting terms: *"region `ledger`: 14 fns when accepted, 19 now (+5)"*.
`accept` re-blesses, and re-blessing is a human act reviewing a diff of names —
something a human can actually judge, unlike re-attesting content nobody read.
Shrinkage (fns deleted, or claimed out of the region) clears without ceremony; the
direction of pressure is the point. This lands with `accept`/`audit` at M5; the spec
sentence and the lock-file shape are written at adoption so the gap is a stated IOU,
not a discovered one.

## 6. The §7.1 channel argument

**Proposed form: a pastel amber wash as the given component's fill, solid border, plus
the hollow shield in the box header.**

The channel case, made in §7.1's own terms: hue amber already carries exactly one
meaning — *a human's attention (owed, or vouched)* — via the numbered pin and the
hollow shield. Fill currently carries: verdict greens/red saturated (earned), verdict
pastels (promised ceiling), violet (machine authorship), unfilled (nothing declared).
The proposed restatement, which subsumes rather than adds: **fill hue answers "what
stands behind this box's contents"** — green family: machine evidence; violet: machine
authorship; amber: human say-so; none: nothing. Saturation discipline is obeyed
(pastel = not earned — and a given region can never be the saturated form, condition 1
again, this time as a style rule a test can pin). Border stays solid: dashed means
"nothing inside yet, expected to solidify," which is precisely what a given region is
*not* — 004's `ledger` drawing dashed-hollow is the standing misreading this fixes.
Like external-elements' dash-channel restatement, this is an amendment to a channel's
declared meaning and therefore the gate's own call, flagged as open question 1 rather
than assumed.

Squint test, predicted (the vetting render decides): pale amber patches read "resting
on people," distinct from pale green "promised," white "unspecified," dashed "hollow."
One honest weakness: amber vs pale green sits on the axis red-green color-blindness
collapses, and the verdict scale already spends that axis. The redundant encoding is
the hollow shield in the header — already the human-attested mark, so its reuse is the
channel-consistent one — carrying the meaning at reading distance where the wash fails.
If the gate refuses the fill restatement, the fallback form is shield-plus-solid-border
without the wash: weaker under squint, still honest, and probably below the
worth-adopting line — which would push toward open question 6's fallback rather than a
third form.

Tooltip, drafted to the newbie bar (exact-string territory when built):

> *"⟨name⟩ — part of this codebase, deliberately taken as given: ⟨reason⟩. Ply checks
> nothing inside this box. A checked function that calls into it keeps its verdict
> only conditionally — the proof assumed a call in here returns some value of its
> declared type, and nothing else. 14 functions inside when this was accepted; 14 now."*

Composition needs no new rules: boundary-contract chips appearing inside the amber box
*are* the shrink progress, drawn by existing forms; the stale corner marker and the
collapsed-stack form compose as they do today; both renderer invariant walks
(`every_painted_element_resolves_a_style_rule`,
`every_drawn_item_resolves_a_tooltip`) extend automatically and a new construct cannot
skip them.

## 7. What it does to vetting 004 — and the gate this proposal sets for itself

Applied to 004, concretely and falsifiably:

- `ledger` gains `given: "…"`. The SVG's dashed hollow box becomes amber and solid:
  the boundary reads "settled by decision," not "nobody got here yet."
- **s3** (the qualified-path crossing, no declared contract) — today: `unclaimed`,
  `W0512`, 0.015s, exit 1. After: havoc stub of `bps_for_tier -> u32`; predicted
  `bounded(2)` + `conditional` at the 300s stubbed floor. The prediction is grounded,
  not hoped: the caller's own `.min(10_000)` makes s5's declared `<= 10_000` redundant,
  so the havoc proof discharges the same obligations as the run already measured at
  201.77s. **This prediction is the gate's first question**; if the havoc proof fails
  or blows the floor, that is a finding that reshapes or refuses the proposal.
- **s5** (declared contract) — unchanged, and re-run as the layering regression:
  contract wins over region, same verdict, same `W0511`, same `owed-evidence`.
- A **new stage**: a caller *without* the defensive `.min`, to measure the fail side —
  the absence verdict, the diagnostic's usefulness (does it actually tell the user
  which contract to write?), and, at the pinned Kani, the witness gap in the flesh.
  (The pin spike has since measured that gap: the witness *is* recoverable and names
  the fabricated return; what is not recoverable is a red test of the real code.)
- **`withdraw` — unchanged, `unsupported`, and this limit is stated up front**: the
  region fixes nothing about unclaimable *signatures*. 004's usefulness verdict was
  half about the shell, and a given region does not touch that half. Anyone reading
  this proposal as "004's problem, solved" should reread 004.
- Runs still fail wherever absences stand. The region is not a CI-silencer, and the
  re-run must show a failing exit surviving it.

The gate condition, same sequence external-elements bound itself to: (1) record the
§7.2 taxonomy gap as a numbered finding (TODO.md already carries the prose); (2) extend
004's `run.sh` in a scratch copy with the given-region stages, run real engines, render,
squint-test, read the tooltips cold, record what held and broke; (3) only then amend
The-Ply-Spec.md, citing the vetting record. Additionally and unlike external-elements:
the fail-side reporting depends on witness recovery through a stub, so the **Kani pin
spike (`tests/spike/kani-pin/`) must report before the fail-side wording is specced** —
**it has, on 2026-08-25: witness recovery through a stub works at the pin, so this gate
is discharged and no pin move is needed for it** — the pass side and the refusal-stands rows have no such
dependency and can gate first.

## 8. Scope limits

Explicitly not entering: path-list regions (components only); any region-level
contract, check, capability, ownership, or verdict; `owner:`/`expiry:` fields; any
change to the fuzz/test tiers or the architecture tier; any automatic check rewriting
(`bounded` → `fuzz` remains the user's move, offered in `fixes` — Ply proposes, never
rewrites); havoc through `&mut` parameters; any fix for §5.5's two recorded first-party
gaps (the transitive inlining gap and invisible call sites — a given region neither
closes nor widens them); any kernel change; any new visual form beyond the one argued
in §6.

## 9. Implementation cost, checked against the actual code

Touch points read, not guessed:

- **`tools/model`** (`src/lib.rs`, 718 lines): `given: Option<String>` on `Component`
  + parse tests. Hours.
- **`tools/check`** (`src/lib.rs`, 614 lines): interaction rules — checks-on-fns
  inside a given component (the `W`), `given` + `pure`/`strict` composition (no
  conflict: architecture tier unchanged), reason-required enforcement via the schema.
  Half a session.
- **`tools/render`** (`src/svg.rs`, 3607 lines + `layout.rs`): the amber class in the
  style constants, the header shield, the tooltip with exact-string tests; both
  invariant walks extend automatically. ~1 session, dominated by wording and goldens.
- **`crates/`**: the region set built where `declared` is built today
  (`ply-cli/src/verify.rs` ~line 151, including the non-local-anchor keying — G3's
  rename caveat applies to regions identically and should be fixed once for both);
  `CalleeStatus::Given` and anchor-prefix matching in `ply-core/src/callgraph.rs`; a
  `boundary_plan` arm building a clause-free `StubSpec` (the codegen for it already
  exists and emits the unconstrained `kani::any()`); the fail-side handling in the
  `KaniOutcome::Violation` arm (absence, not violation, when the harness carried a
  given-stub); two new diagnostics with exact-string tests; the `assumptions` kind.
  1.5–2 sessions, plus multi-minute Kani fixtures (the `boundarycontract` pattern,
  ~2 min each, priced into the suite deliberately as G1's fix was).
- **`ply.lock` inventory + `accept`/`audit`**: spec and lock shape at adoption; the
  machinery lands with M5, stated as an IOU in the spec the way `audit` already is.
- **Spec deltas at adoption**: §5.1 structure + example; §5.5 the Given row and the
  precedence table; §6 audit line + the absence note; §7.1 one table row + the fill
  restatement if accepted; §7.2 the fifth kind; D6 untouched; schema + goldens.

Total: roughly **3.5–5 sessions** for tools + crates + the vetting re-run, ~1 later
session at M5 for audit/accept/lock, spec pass alongside adoption. The fail-side
witness payoff was gated on the Kani pin spike; that spike has now run
(`tests/spike/kani-pin/FINDINGS.md`, 2026-08-25) and reports the gate open at the
current pin, so no engine work is budgeted against it.

## 10. Open questions a human must settle

1. **The fill-channel restatement** (§6): is "fill hue = what stands behind the box —
   green machine evidence, violet machine authorship, amber human say-so" a legitimate
   subsuming restatement, or the second meaning §7.1 refuses? Fallback stated in §6,
   with the judgment that the fallback alone probably drops below the adoption line.
2. **The fail-side verdict name**: reuse `inconclusive` (proposed — it is already in
   §6's absence vocabulary and already fails the run) or coin a distinct absence
   (`boundary-blocked`?) so audit can count havoc-fails apart from engine-fails?
   Additive later either way.
3. **Should audit count "given crossings never exercised by any run tier"?** The
   residual assumptions are fuzz-dischargeable; counting unexercised crossings would
   put the same pressure on them that `owed-evidence` puts on declared contracts —
   at the cost of a second debt-like counter users must learn.
4. **Claims inside a given region: warning or error?** Proposed warning (claims win),
   because the shrink path should be cheap. The error reading ("one file says check,
   the same file says don't") is defensible and stricter.
5. **The `&mut` refusal row**: keep (proposed), or havoc the referent too? And the
   adjacent fact, recorded not fixed: the existing Assumed stub already accepts `&mut`
   params without havocking referents — the same question, one branch over.
6. **Is the named region worth its grammar over the per-callee fallback** (let an
   explicit clause-free boundary entry mean havoc, no new construct)? This proposal
   says yes — one declaration, unanticipated call sites covered, a boundary audit can
   group by and the picture can draw, the §7.2 slot filled — but 004 is one scenario
   and the count of real callees was one. If the gate re-run shows the region's extra
   surface doing no work the fallback wouldn't, adopt the fallback and refuse the rest.

## Recommendation, restated

**Adopt in reduced form**, gated as §7 binds. For: the havoc semantics makes the
blanket structurally incapable of lying — trust buys at most `conditional` evidence the
caller earned by its own defensiveness, and it fails louder, not quieter, than the
per-callee contracts it relieves, which are today's actually-launderable route (G2).
Against, unanswered: the construct's usefulness rests on the empirical claim that real
callers are defensive at boundaries — measured on exactly one function, predicted on
one more. If most crossings fail under havoc, this is a hint generator wearing a
grammar construct, and only the vetting re-run can say. That is why the gate is the
recommendation's load-bearing half.
