# Review — should the architecture tier be LLM-enforced? (2026-08-26)

Scope: a design question, reviewed against The-Ply-Spec.md (§1, §5.2a, §5.3, §7.1,
§7.2, D3, D11, D14, ADR-0001's `trait Extractor` seam, the M2 milestone entry) and
CLAUDE.md. Under review: a recommendation to split the tier — crate tier deterministic,
item tier deterministic-approximate, LLM upstream only ("propose, never decide") — plus
a proposed spike (have an LLM apply the rules to this repo by hand before building
anything). The reviewer asked to be attacked, not agreed with. One rough measurement was
taken on this workspace and is cited where used.

**Bottom line first.** The split is right, and the answer to the maintainer's question
is no: the architecture *verdict* must never come from a model. But the recommendation
as written under-argues its own case in one place, over-argues it in another, and misses
the two designs that actually matter. Under-argued: before any model enters, there is an
unexhausted ladder of *deterministic* resolution (rust-analyzer or rustdoc-JSON behind
the `trait Extractor` seam ADR-0001 already carved) — the framing "deterministic syn vs.
LLM" is a false binary the spec itself doesn't commit to. Over-argued: "no replayable
counterexample" is the recommendation's decisive argument, but §1 already concedes
architecture violations carry no input witness — "they carry spans and evidence of their
own kind" — so the witness argument is weaker here than stated. The honest version is
stronger and points somewhere useful: *the span is the witness, and a span is exactly
the kind of claim a machine can confirm without a type checker* — which yields the one
hybrid worth building (finder/confirmer, below). And the spike as proposed produces a
plausible list nobody can score; reframed with seeded violations it becomes a real
measurement. Details follow, keyed to the five questions asked.

---

## 1. The split, and the strongest case against it

The steelman for LLM enforcement of the item tier, made properly, has three legs. Two
survive contact with the spec as *feature requests*; none survives as *enforcement*.

**Leg 1 — coverage.** The syn extractor resolves "best-effort through `use` maps and
local type declarations" (§5.3). In idiomatic Rust most call sites are method calls: a
crude count over this workspace's own 45 source files finds ~6,900 method-call-shaped
sites against ~3,300 plain calls and ~1,000 fully-qualified ones — roughly 60% of call
sites are the shape the extractor can only resolve when the receiver's type is declared
in the same scope. Yesterday's fingerprint work hit exactly this wall and chose to
abandon the walk and hash the whole crate rather than guess. So the deterministic item
tier may end up honestly reporting that it resolved a minority of the call graph, and
its false negatives — the violations routed through trait objects — are invisible. An
LLM reads the code the way the human architect meant it and catches the `Box<dyn
Store>` call the extractor structurally cannot see.

This is the strongest leg, and it is an argument about *which resolver*, not *which
verdict-giver*. Three answers, in order of preference: (a) the D11 coverage metric
exists precisely to make this weakness a printed number, so the decision can be made on
measurement rather than fear — see §5 below; (b) the deterministic ladder is not
exhausted: `trait Extractor` was made a trait so a rust-analyzer- or rustdoc-JSON-backed
implementation could replace syn without touching the rules — heavy, but deterministic,
and strictly ahead of a model in the queue; (c) where neither reaches, the
finder/confirmer hybrid (§2) lets a model *propose* a resolution that a machine
*confirms syntactically* before anything is reported. At no point does the verdict need
to become an opinion to close the coverage gap.

**Leg 2 — intent.** "Storage may not touch the network" is checked by proxy: paths into
`std::net`, plus `capmap.toml`. Code can honor the proxy and violate the intent —
storage building the request that another component fires, callbacks flowing effects
backward across an edge that constrains "**direct** calls only" (§5.3, stated with eyes
open). An LLM reviewer catches intent violations the rule language cannot express at
all.

True — and it describes a different product. Checking the unstated real rule the YAML
approximates is *architecture review*, and Ply already has both a channel and a
doctrine for it: the amber/attested channel (hollow shield: "attested by named evidence,
not machine-checked", §7.1), and §7.2's answer to "the grammar can't say X" — extend the
grammar drawably, or accept that X is below the watermark. An LLM review report,
committed and shield-badged, would be a legitimate future feature. Letting it paint red
and green is not, because red and green are the earned-evidence channels and the channel
discipline forbids a second meaning per channel as firmly as it forbids undrawable
constructs.

**Leg 3 — the audience.** Ply's consumers are coding agents in repair loops; for an
agent, a probably-right finding is cheap to verify by editing and re-running, so
unverified LLM warnings might carry positive expected value. This leg fails on Ply's own
evidence base: §1 finding 2 is that feedback quality is *the* dominant variable and
"feedback without a witness performs little better than none" — measured, with a 12%→97%
spread. A finding that flickers across runs teaches the loop that findings are noise.
And there is a correlated-error problem the recommendation never names: the code being
checked was written by a model with the same priors as the model checking it. A checker
that finds architecture violations plausible or implausible for the same reasons the
generator did will miss precisely the violations the generator committed. The whole
point of routing to Kani and proptest is that solvers and RNGs do not share the
generator's blind spots. An LLM architecture checker reintroduces the correlation Ply
exists to break.

So: the split is right, and not because "determinism good" — because each leg of the
best opposing case resolves into either a better deterministic resolver, a different
(attested) channel, or a measured refutation from §1.

## 2. Third options

Two are real; one is a trap that looks like a third option.

**Real, and worth speccing: the finder/confirmer.** The extractor already emits
`calls_unresolved(from, span)` and W0412 ("possible undeclared edge (call unresolved)")
when an unresolved site's *textual candidates* include a cross-component item. The
hybrid: an optional, explicitly-invoked pass (never inside `check`'s fast path) hands
unresolved sites to a model, which must answer in a closed schema — "site S resolves to
item I in component C" — and the confirmer checks mechanically what needs no type
checker: the span exists and is in the unresolved set; the named item exists in the
named component; the method name matches; where the receiver is a trait object, an
`impl <Trait> for <Type>` block exists in C. A claim that fails confirmation is
discarded silently; a claim that passes is reported as what it verifiably is — an
upgraded W0412 with a named, machine-confirmed candidate, still a warning, still
approximate, and *deterministically re-derivable* because the confirmation, not the
model, is what got reported. The model narrows a search space; every reported fact has
a mechanical proof-of-existence. Note the ceiling honestly: confirmation is
weaker than resolution — "an impl exists with this method name" is not "this call
dispatches there" — which is exactly why the output stays a W-severity candidate and
never becomes an A-severity edge violation.

**Real, and already half-designed: the pinned artifact.** An LLM drafts — the
architecture description itself (the `ply.yaml` this workspace still doesn't have),
proposed `#[ply::allow]` escapes with their written reasons, triage of the unresolved
registry. Every one of these lands in a *committed, human-reviewed file* that
deterministic checks then consume. This is not a new mechanism; it is D3 (YAML validated
against a normative schema), the escape-with-reason audit list, and the numbered-pin
unresolved registry, used as designed. The model's judgment enters the system only by
passing through a human's diff review, after which the artifact is input like any other
input — fingerprinted, drawable, reproducible. The recommendation's "propose, never
decide" names this but doesn't notice the spec already built the receiving apparatus.

**The trap: "LLM verdict, but cached."** Pin the model version, record the verdict in
`ply.lock`, reuse it — determinism restored? No. §5.2a's contract is "it matches, the
result is reused; it does not, the check runs again", and its stated refusal is showing
"a verdict the run did not produce" — the *remembered opinion*. An LLM verdict inverts
the design: re-running is the dangerous operation (same inputs, different answer), so
reuse becomes mandatory forever, and the lock file becomes a store of exactly the
remembered opinions §5.2a exists to refuse. The fingerprint machinery cannot rescue a
check whose *re-run* is the thing that lies. Any hybrid that ends with a model's output
in the `verdict` field is this trap wearing a different coat.

## 3. The spike, attacked

As proposed — "have an LLM apply the architecture rules to this repository by hand and
see whether the findings are worth acting on" — it is close to uninformative, for three
reasons. First, circularity: this repo has no `ply.yaml`, so the model would draft the
architecture description and then check code against its own draft; a model grading its
own rubric finds what it expects. Second, wrong quantity measured: M2's deliverable is a
*standing gate*, whose value is realized when future code violates it; a one-shot audit
of a small, disciplined, spec-driven codebase measures "are there violations today" —
plausibly zero — and says nothing about whether the gate earns its keep. Third,
unscoreable output: with no ground truth, every finding costs a hand-verification and
every non-finding proves nothing; that is precisely the "plausible list nobody can
validate" the question anticipates.

The salvage is cheap and changes what gets measured:

1. **Keep the drafting half.** Have the model draft this workspace's `ply.yaml` — that
   is the LLM's correct job under the split, it forces the vocabulary of §5.3 through a
   real codebase, and M2's acceptance already requires self-hosting. If the *grammar*
   can't express this repo's real shape, that is a vetting finding (per `vetting/`'s
   whole method) and worth four sessions of warning by itself.
2. **Replace the audit half with fault injection.** Seed N deliberate violations — a
   cross-component call, a capability touch in a pure component, a foreign mutation of
   an owned type, at least one routed through a trait object so it is invisible to syn —
   and measure the model's recall and precision against known ground truth. The repo
   already trusts this method (the fault-injection demo is what exposed `bounded(0)`).
   This also produces the number the build decision actually needs: how many of the
   seeded violations are *only* catchable by resolution beyond syn's reach — which
   prices leg 1 of §1 above.
3. **Measure resolution coverage directly.** A half-session syn walk over this workspace
   counting resolvable vs. unresolvable call sites turns the ~60% grep estimate into the
   D11 number before the tier exists. If coverage is dismal, that argues for the
   rust-analyzer extractor or the finder/confirmer rising in priority — not for an LLM
   verdict; if it is decent, the syn tier is vindicated cheaply.

## 4. Does architecture escape the thesis?

No, and the tempting conflation should be named: **approximate is not nondeterministic.**
The item tier expects to be *incomplete* — and every one of its accommodations is a
deterministic mechanism for saying so: warnings by default, the D11 coverage share,
`calls_unresolved` accounting, escapes that take a written reason and land in an audit
list. Run it twice and it is wrong in exactly the same places; its uncertainty is
enumerated, printed, and diffable. An LLM check is wrong in *different* places each run
and cannot enumerate what it missed — it converts the tier's known-unknowns (counted,
pinned, numbered) into unknown-unknowns. The spec's humility about the item tier is an
argument *for* the deterministic design, because that humility is implemented as
machinery only a deterministic check can have.

Three structural reasons the exception cannot be contained if granted. The evidence
order: an LLM verdict needs a rung, and the kernel's invariants are proved by exhaustive
enumeration against an independent oracle — there is no oracle for "a model felt this
way", so the one part of Ply that is actually proved becomes unprovable at the rung
where the opinion enters. The grammar: §7.1's gate is that every construct has one
honest visual form; an LLM verdict has no channel — solid means machine-checked, dashed
means declared-not-checked, the shield means human-attested, and it is none of these —
so drawing it green is lying on the evidence channel, and a construct that cannot be
drawn does not enter. The precedent: "architecture is approximate anyway" applies
verbatim to weak-spec detection, mutation triage, and every future warning-tier feature;
and "it's only warnings" is false on the spec's own terms, since `strict: true` upgrades
this tier to errors — it is *designed* to gate merges. §1 already names the failure
mode this would ship at scale: the green nothing. The project's stated reflex for a
"looks checked but isn't" surface is refusal on principle (§5.3's E0207 gives externals
exactly that treatment); an LLM verdict inside the checked channel is that surface,
built on purpose.

## 5. Build it? Build it next?

Build it, next, as specced — with one sequencing note inside M2 and one addition.

The crate tier is cheap, sound, reads `cargo metadata`, and errors honestly; it should
land first and immediately self-host (the M2 acceptance already asks for this repo's own
`ply.yaml` in CI — the spike's drafting half feeds directly in). Then run the coverage
measurement (§3, item 3) *before* committing to the item tier's depth: if syn resolves
enough of this workspace to catch the seeded violations, build the item tier exactly as
§5.3 writes it; if it doesn't, the session budget shifts toward the extractor seam —
still deterministic — with the finder/confirmer specced as the explicitly-invoked,
confirmation-gated pass described in §2, not as part of `check`. What should *not* be
built now: the LLM review report (leg 2's product) — real, attested-channel, and
adjacent; per CLAUDE.md's scope rule it gets this one line and the maintainer's
decision, not a design.

One risk in the recommendation's own framing, worth saying because nobody else will:
"propose, never decide" fails quietly when proposals are rubber-stamped. An LLM-drafted
`ply.yaml`, skim-approved, is architecture-as-vibes with a deterministic enforcement
layer laundering it — the checks would then faithfully enforce a description nobody
actually decided. The countermeasure is already in the house style: LLM-drafted
artifacts are reviewed like goldens — never blind-accepted, diff read, reasons said out
loud. That norm, applied to `ply.yaml` and to every proposed escape, is what keeps the
split honest.
