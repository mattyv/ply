# Working on Ply

[The-Ply-Spec.md](The-Ply-Spec.md) is the source of truth. Start from it by § reference; amend it rather
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
as one pure module, and its invariants are checked by exhaustive enumeration over every
verdict tree up to a small bound (991,389 of them, ~2s under `cargo test --release`) —
which for a bounded domain *is* a proof, and is the gate that must stay green.

That enumeration is not a consolation prize for a failed proof — it is the same *kind* of
evidence Ply calls `bounded`: exhaustive within a stated bound, checked against an
independent oracle. It covers strictly more than the Kani harness would have (which was
scoped to depth 2, ≤2 children). Two honesty conditions attach: the enumeration uses a
reduced configuration set (one representative status, one fixed assumption string), so the
argument for why that reduction loses nothing — per-bit uniformity of `StatusSet`,
content-independence of the assumption merge — must be written alongside the claim, or
"exhaustive" is overclaiming by quotient.

Kani harnesses for the same four invariants exist and are `#[cfg(kani)]`-gated, but as
of 2026-08-23 **none of them terminate**: CBMC symbolically unwinds `BTreeMap`'s generic
clone algorithm on every recursive call because the kernel's real types use
`BTreeSet`/`Vec`, and no unwind bound, solver, or object-bits setting tried changed that.
The investigation is documented in the module. Do not report the kernel as
"Kani-proved" until a harness actually returns a verdict. The scale spike later showed
why it never will: the kernel is a recursive tree, and recursive shapes are outside
Kani's measured reach (§5.4b). Reshaping the kernel to suit the verifier is refused on
evidence as well as principle — the stall simply moves to the next unbounded field, and
"Ply proposes, never rewrites" applies most strictly where we are our own user. The standing obligations:

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

**Every report opens with a TLDR, and the maintainer should never have to ask for one.**
Two or three sentences, at the very top, carrying the answer — not the setup, not the
approach, not what you were about to do. Assume it is the only part read. If it survives
alone, the detail below it is a bonus; if it needs the detail to make sense, it is not a
TLDR, it is a preamble. The same report, twice:

> I verified the fix independently. I wrote my own fixture at the correct depth and ran
> four cases against it, including two that had to keep reusing, because a fix that
> invalidates everything is the feature deleted with extra steps. Here is what each one
> did. [six paragraphs, then the verdict]

> **It works, and I checked it myself rather than taking the agent's word.** Breaking a
> helper is now caught and reported with a failing input; untouched code still comes back
> in 0.039s, so the speed-up survived. One caveat: build flags are still outside the hash,
> and that is now written down rather than implied.

The second one is the report. Lead with what is true now, then the one thing that is
still wrong or still open — bad news goes in the TLDR, never held back for paragraph
nine. Length below it is fine; burying the answer is not.

**No project jargon in conversation, ever, unless asked for it.** The newbie bar below
governs what the *tool* says; this governs what *you* say. The same sentence, twice:

> D5's third branch now fires on an unclaimed callee: `W0512`, verdict `unclaimed`,
> and §5.5 is amended.

> When your new code calls old code that carries no promises, Ply now stops and names
> the function it refused to walk into, instead of grinding for eleven minutes.

The second one is the report. Banned unless the reader asked: § references, decision
letters (D5, D7), diagnostic codes (`W0512`, `E0204`), verdict and status words as
though they were English (`conditional`, `bounded(2)`, `owed-evidence`, `unclaimed`),
milestone and phase numbers, engine and crate names, and anything whose meaning needs
the spec open. Name the *thing that happened*, in words a competent developer who has
never read this repo would follow.

Numbers and evidence still belong in the report — "3 of 8 passed", "eleven minutes to
five milliseconds", "the test goes red when you break it". Those are the substance;
the jargon was never carrying it.

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

## TODO.md is the running state, and it is not optional

Anything agreed but not yet done goes in TODO.md the moment it is agreed, and gets
ticked with its commit hash the moment it lands — in the same session, not later. A
stale list is worse than none: it makes finished work look pending and pending work
look forgotten, and the user ends up asking which is which. Keep the honest caveats
in it too — a KNOWN GAP left open on purpose is a state worth recording, not a
failure to hide.

The same rule covers the spec, the vetting docs, and `.archi/`: when behaviour changes,
the artifact describing it changes in the same commit. If a claim in The-Ply-Spec.md
stops being true — including a claim you added earlier in the session — retract it
rather than leaving it to be discovered by review.

## Scope

Build what was asked and nothing adjacent. A legend nobody requested is not a bonus.
If you think something extra is needed, say so in one line and let the user decide.

## Vetting

`vetting/` holds scenarios written in the grammar before the tool exists, each recording
where the grammar held and where it broke. Findings become spec changes. New grammar
features must be drawable (§7.1) — if there is no visual form, the feature doesn't enter.
