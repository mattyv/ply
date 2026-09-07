# Component-level proof: a short design

**Status: design only. Nothing here is built. The recommendation is one
measurement first, not an implementation.**

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
