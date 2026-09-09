# Component-level proof: a short design

**Status: superseded in part. Read the addendum at the bottom first — it
retracts three claims in the sections below, including "`&mut self` methods
cannot be claimed", which stopped being true when transition promises
shipped. The original text is kept because how it was wrong is the useful
part.**

## The gap, as measured rather than argued

A `state:` clause is checked today by sampling: build the value through its
own constructor, run a generated sequence of its public operations, assert
every clause after each one (§5.3). That is real evidence and it is what the
sequence-bound work of 2026-09-07 made reachable at all.

It is also the *only* promise expressible about a type that changes.
`ply-checkable-code` rule 9 says so plainly: `&mut self` methods cannot be
claimed. Two A/B rounds measured what that leaves uncovered -- a bounded
cache and a token bucket, three pre-registered bugs each:

**Ply caught 1 of 6. Four of the five misses were correctly outside what
could be declared at all.** Two agents, working independently and without
seeing each other, read that limit out of the guide and fell back on
ordinary tests for the mutating methods.

So the gap is not that the sampling is weak. It is that for a stateful type,
the bugs live in the transitions and the transitions are unclaimable.

## What a proof would have to establish

For an invariant `I` over type `T`, built by constructor `C`, with public
operations `O`:

1. **Init** -- every value `C` can produce satisfies `I`.
2. **Preservation** -- for every `o` in `O` and every `v` with `I(v)`,
   `I(o(v))` holds.
3. **Boundary** -- no public path can change a `T` except through `O`.
4. **Coverage** -- `O` is exactly the public mutators, not a subset someone
   listed.

3 and 4 are the load-bearing pair and the easy ones to skip. Preservation
over a set that quietly omits one method proves nothing about the type: it
is the planting-scope defect of 2026-09-07 in a new place, and it would be
harder to see, because a proof reads as stronger than a sample.

## Where the evidence would come from

`Check::Prove` already exists, already names `verus` as its engine
(`verify.rs`), and today reports the adapter as missing (`W0110`, M7).

The instrument is known to work on a hard shape. `tests/spike/verus/`
discharged the verdict kernel's four standing obligations **unbounded, by
structural induction, in ~1.4 seconds** -- on a recursive tree where bounded
model checking could not terminate at all. Induction is the right kind of
tool for "every value, every sequence", which is exactly what obligations
1 and 2 are.

## The honesty conditions that have to travel with it

- **Verus proves a translation, not the Rust.** The kernel spike proved a
  faithful *shadow* and tied it back to production with a differential test
  over generated trees. A component proof needs the same tie, or it is a
  proof about a different program. This condition is not optional and not
  a footnote.
- **Refuse rather than downgrade.** A type Verus cannot translate must
  report unsupported, the way `V0508` already refuses exhaustive checking
  on a shape it cannot reach. Sampling wearing the word "proved" is the
  worst outcome available here.
- **The verdict attaches to the state node, not the component.** The kernel
  has no way to represent "a component claims its own verdict":
  `NodeKind::Container` carries no `Evidence`, deliberately. `state_node`
  already exists and is the right home. The current model enforces this, and
  the design should not fight it.

## Corrections from review, 2026-09-07

Two, both changing the plan.

**The obligations split differently than this document says.** Verus's
`#[verifier::type_invariant]` closes boundary and coverage as well as init
and preservation -- see the retraction at the top of
`tests/spike/verus-component/FINDINGS.md`. The "Ply, or nobody" framing came
from an encoding choice in the spike, not from the instrument.

**Worse: proving the `holds:` clause does not close the gap this document
opens with.** Of the six pre-registered bugs behind the 1-in-6, four were
transition bugs no invariant expresses, and the one that *was* an invariant
violation is the one the sequence-bound work already catches on every seed.
So a proof of the declared clause scores what the improved sampler scores.
The measured loss is in properties that cannot be *stated* today, because
`&mut self` methods cannot be claimed at all (rule 9).

That makes this a fork, not a detail:

- **"Prove the invariant"** -- cheap, real, and adds coverage, boundary and
  panic-freedom, which sampling structurally cannot reach. But it is a
  certainty upgrade for the one property already covered, and citing the
  1-in-6 as its motivation is not honest.
- **"Claim transitions"** -- contracts on mutating methods, which is where
  the misses actually are. It changes rule 9, and Verus's two-state specs
  (`old`/`final`) are the natural engine for the proved tier.

A third thing this document has no rule for: a proof that does not
*discharge* on correct code. An idiomatic `HashMap` + `VecDeque` cache fails
its proof with no counterexample, because the invariant is not inductive on
its own. "Refuse rather than downgrade" covers types Verus cannot translate;
it says nothing here. The outcome must be "not proved, fell back to
sampling", never "violated" -- and any invariant relating two containers kept
in sync has this shape.

## The fork, settled — 2026-09-07

Measured, not argued (`tests/spike/verus-component/FINDINGS.md`, follow-up 2).
Round 3's token bucket, shadowed both ways, with its three pre-registered
bugs planted one at a time:

**A refill of zero that silently tops the bucket back up passes an invariant
proof — 6 verified, 0 errors — and fails a transition contract immediately.**

It preserves `available <= capacity` perfectly. No invariant over a single
state can see it, and four of the six bugs behind the 1-in-6 are that shape.

So: **claim transitions.** Contracts on mutating methods, which is where the
misses are, sampled first and proved where the shape allows. That changes
`ply-checkable-code` rule 9, which is the real cost and should be taken
deliberately. Proving the `holds:` clause stays worth having -- it brings
coverage, boundary and panic-freedom, which sampling structurally cannot --
but it is a certainty upgrade for a property already covered, and this
document should stop citing the 1-in-6 as its reason.

## What to do first

**One measurement, before any adapter work.** Take
`tests/fixtures/boundedcache`, already in the repo and already the gate for
the sequence-bound change. Hand-translate it to a Verus shadow and ask
whether init and preservation for `len() <= capacity()` discharge over its
real operation set, and how long it takes.

That answers the only question that decides the rest: **is the instrument
capable on a shape a user would actually write?** The kernel is a pure
recursive tree with no interior mutability; a cache is a `Vec` of pairs
mutated in place, which is a different proposition for a deductive verifier.

- If it discharges, the adapter is engineering and the brief is fundable.
- If it does not, we say so and the brief stays parked -- with a measured
  reason instead of an assumption.

Not to be built before that answer: the adapter, result semantics, reuse and
fingerprinting, the drawing, and the boundary/coverage analysis. All of them
are downstream of whether obligations 1 and 2 can be discharged at all.

---

# Addendum: composition, not implementation — 2026-09-09

**Status: this section supersedes the stale claims above and sets the shape
of the implementation. It also carries its own retractions — the timing
comparison below was wrong, and the arithmetic rule it first stated was
unsound. Both are corrected in place, with the measurements that corrected
them.**

**The probes are not yet in the repository.** The entailment probe and its
breakage variants were run from a session scratchpad; only `bucket.rs` and
`cache.rs` are committed. Until those land under
`tests/spike/verus-component/`, the numbers here are reported rather than
reproducible, and saying which is the point of this note.

## Retractions

Three statements above are no longer true and are withdrawn.

1. **"`&mut self` methods cannot be claimed" is false.** It was true when
   this document was written; transition promises shipped and `old(...)`
   reads the before-state (`ply-checkable-code` rule 9, rewritten). Every
   sentence above resting on rule 9's old form — including "the transitions
   are unclaimable" in the opening section — is withdrawn.
2. **"What to do first: one measurement, before any adapter work" is
   discharged.** The measurement was taken (`FINDINGS.md`) and the answer was
   yes. That instruction no longer gates anything.
3. **"Nothing here is built"** is withdrawn as of this addendum.

## The theorem, stated exactly

For an invariant `I` over state type `T`:

> **Every state reachable from a covered constructor by finitely many
> permitted, normally-returning operations satisfies `I`.**

Every word in that sentence is load-bearing, and three of them bound the
claim rather than extend it:

- **permitted** — operations are invoked only where their preconditions
  hold. This proves `I` holds *under that calling discipline*. It does not
  prove any external caller obeys it. A caller who violates a precondition
  is outside the theorem, and the report must say so rather than let a
  reader infer the stronger claim.
- **normally-returning** — a transition that panics or unwinds partway is
  excluded. The state it leaves behind is not covered.
- **covered** — constructors Ply enumerated and accounted for. An
  unaccounted construction path voids the theorem, it does not weaken it.

Also excluded from this first theorem, each because it needs its own model
and its own obligations: concurrency, re-entrancy, and any observation of
the state from another thread.

## The central decision: prove the composition, not the bodies

The existing spike (`proof/bucket.rs`) proves the token bucket's
*implementation* — the real method bodies, against their contracts. That is
not what this work needs. Function proofs are taken as sound premises; the
open question is whether their contracts **entail** the component property.

That is a pure logical entailment over abstract state and contract
predicates. It needs no function bodies at all. Measured today, both halves
on the same machine and the same Verus (0.2026.08.23.fbbbbcf):

**RETRACTED the same day it was written.** This first claimed "12.1s vs
1.8s, seven times faster". That was a cold first invocation of Verus
compared against a warm one. Re-measured warm, three runs each, same binary
and same files:

| what is proved | obligations | wall clock, warm |
|---|---|---|
| the bucket's bodies (`proof/bucket.rs`) | 3 (+ empty `main`) | 783 / 740 / 742 ms |
| the bucket's contracts entail the invariant | 3 (+ empty `main`) | 816 / 731 / 706 ms |

**The ratio is 1:1.** Neither does measurable solver work; nearly all the
wall clock is `vstd` import. Two further corrections: the original
"6 verified"/"4 verified" counted the empty `fn main()`, so the honest
counts are 3 and 3; and the "vstd wrappers, `no_unwind`" cost belongs to
`proof/cache.rs`, not to the bucket, which is two `u32` fields and plain
arithmetic and never paid it.

**Composing from contracts is still the right shape, but not for speed.**
What survives measurement: it needs no function bodies, so it does not
inherit the "invariant re-checked after every field-mutating call"
strictness that rejects a private helper breaking and restoring the
invariant; and it is the only thing that answers the actual question, which
is whether the contracts *entail* the property, not whether the bodies
satisfy the contracts.

**Non-vacuity, checked rather than assumed.** Two deliberate breakages, each
run:

- Weakening `refill`'s contract to `post.available >= pre.available` (it may
  now exceed capacity): **3 verified, 1 error**, "postcondition not
  satisfied".
- Omitting `try_take`'s capacity frame fact: **3 verified, 1 error**. The
  solver is free to vary capacity, exactly as it should be — an omitted
  post-state fact is *unconstrained*, never implicitly unchanged.

## What a premise is

A function proof is not usable as "this function is proved". Composition
needs the contract it was proved against, the domain it was proved over, and
enough identity to tell whether it still describes today's source. A premise
therefore carries:

- **Identity** — canonical item path and contract identity.
- **Fingerprints** — source and contract, plus the active compilation
  configuration. A premise whose source moved is stale, not weaker.
- **Domain** — what the proof covered, and any bound. A premise proved over
  a restricted domain cannot license an unrestricted conclusion.
- **The contract itself** — preconditions, relational postconditions, the
  observers those mention and what they mean, and the frame facts.
- **Assumptions** — anything the proof rested on and did not discharge.
- **Provenance** — which engine produced it, or that it is declared trusted.

Frame facts get their own line because they are the quiet failure. An
omitted post-state fact is unconstrained. A frame fact must come from a
proved contract or from a justified effect analysis — never from the absence
of a mention, and never from `&self`, which is not proof of purity: interior
mutability and shared aliases can mutate through a shared reference.

## The proof-provider boundary

Tests may supply known-good premises directly. Production may not. It
consumes validated existing evidence, or a premise the author has explicitly
declared trusted — and where trust is involved, the result is visibly
conditional on it.

**A missing provider, or a proof-result fixture, can never render an
unconditional "proved".** The current `prove` check reports the adapter as
absent and earns `engine-missing`; that is the right floor and this work
must not lower it.

## Where the result attaches

The state node. `NodeKind::Container` has no `Evidence` payload *by
construction* — a component cannot claim its own verdict, and this design
does not fight that. `state_node` already exists, already carries
`evidence: None`, and already folds worst-of into its component.

## Arithmetic: the first encoding was unsound, and the rule that replaces it

This section first claimed "machine arithmetic is never silently replaced by
unbounded arithmetic; the width is a premise". **The width premise does not
do that**, and review produced the counterexample the same day.

Bounding the *observers* by their declared range says nothing about the
*operators inside the contract*. A Ply contract is a Rust expression, so
`old(available) - tokens` is machine subtraction; transcribed into the
prover's `int` it becomes unbounded subtraction, and those differ exactly
where it matters. Reproduced end to end:

- Contract, as an author would write it: on success `available ==
  old(available) - tokens`, with capacity framed.
- Implementation: `self.available = self.available.wrapping_sub(tokens)`.
- The model **verifies**. The program, compiled and run, has the promise
  evaluate **true** while `available <= capacity` is **false** —
  `available = 4294967295, capacity = 5`, from `available = 3, tokens = 4`.

A premise the program genuinely satisfies composed into a "proved" invariant
the program violates. That is the outcome this whole tool exists to prevent,
and it was in the encoding this document recommended.

**The rule.** Observers still enter with the range facts of their declared
types, but that is necessary and never sufficient. Every arithmetic
operation in a translated contract carries its own obligation: *within the
declared ranges, this operation does not overflow*. Where that cannot be
discharged the property is **not established** and the offending clause is
named — never quietly reinterpreted under wider arithmetic. A contract
written to be overflow-safe discharges it; `tests/fixtures/tokenbucket`
widens to `u64` for exactly this reason. One that is not gets refused, which
is the right answer rather than a limitation.

**Satisfiability is the sibling hole**, disclosed here rather than
discovered later: a contradictory premise set verifies everything. Give
`refill` both `post.available == pre.available + 1` and `post.available ==
pre.available` and the obligations discharge, meaning nothing. Vacuity must
be checked. The non-vacuity checks recorded above tested sensitivity to two
weakenings, which is a different property and does not cover this.

## The outcomes, kept distinct

- obligations closed with adequate premises → **property proved, within the
  stated scope**
- closed only under trusted premises → **conditional**
- insufficient contracts, or the solver could not close → **not
  established**, naming the open obligation. Never `violation`.
- boundary not closed → **incomplete coverage**; no complete property proof
- timeout or tool failure → that reason, retained
- an executable counterexample → violation

A model refuting a contract implication is a **composition countermodel**:
behaviour a weak contract permits, which the implementation may well never
produce. It is not a program bug until replay or stronger reasoning makes it
one, and it must not be labelled as one.

Sampled evidence is preserved alongside an unsuccessful proof. A claim must
never end a proof attempt with less evidence than it had before.
