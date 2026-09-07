# Component-proof feasibility spike — findings

Asked by `docs/component-proof-design.md`: **is a deductive verifier capable
on a shape a user would actually write?** The kernel spike
(`tests/spike/verus/`) proved a pure recursive tree. This asks about a `Vec`
of pairs mutated in place -- the `tests/fixtures/boundedcache` shape, whose
`state:` clause is the plainest invariant Ply supports.

Run 2026-09-07 on Verus 0.2026.08.23.fbbbbcf, the same release the kernel
spike pinned, installed by the four steps its FINDINGS.md records -- all four
still work verbatim, and the archive is byte-for-byte the size recorded
there (301,751,770). Toolchain 1.97.1-x86_64-unknown-linux-gnu, already
present.

**Control first:** the kernel spike's own proof re-run before anything new,
so a later failure would be about the cache and not the setup. 22 verified,
0 errors, 12s.

## Headline: init and preservation discharge, in under a second

`proof/cache.rs` is a faithful shadow of the fixture. Verus proves, for
**every** value and **every** operation sequence:

1. **Init** -- every cache the constructor builds satisfies `len <= capacity`.
2. **Preservation** -- `put` and `get` both preserve it.

**8 verified, 0 errors, 0.93s.**

For contrast, the same property by sampling: after the sequence-bound work of
2026-09-07, 106 of 3,072 generated cases could even reach the off-by-one that
breaks it, and before that work, 2 of 3,072.

**The proof is non-vacuous, checked rather than assumed.** Planting the
fixture's own off-by-one (`>=` becomes `>` in the fullness test) makes Verus
report `postcondition not satisfied` and name `put`. 7 verified, 1 error.

## RETRACTED: "coverage is not free"

**The section below is wrong, and an adversarial review found it the same
day. Kept rather than deleted, because how it was wrong is the useful part.**

It concluded that Verus discharges init and preservation while boundary and
coverage fall to Ply "or nobody". That was an artefact of how this shadow was
written -- the invariant as per-method pre/postconditions, which only ever
answers about the methods you list. Verus has a purpose-built feature for
this, `#[verifier::type_invariant]`, and with it:

- the uncontracted `force_push` **is** rejected, and so is a free function in
  the same module that reaches into the struct -- both with "value may fail
  to meet its declared type invariant after mutation";
- a struct with public fields is refused outright, which *is* obligation 3
  enforced by the verifier;
- the honest code still proves, same 8 verified, 0 errors.

Re-run independently before retracting: `ti_ok2.rs` 8 verified 0 errors,
`ti2.rs` 8 verified **2 errors**. So the table below is wrong: **all four
obligations are the verifier's**, and the load-bearing question is not "can
the receiver scan enumerate the mutators" but "is every function that can
touch this type's fields inside the translated unit" -- a
translation-completeness question, and a different problem.

Two costs come with the feature and belong in any plan that uses it. Every
std call that mutates a field must be marked `no_unwind`, and vstd's
`Vec::push`/`remove`/`pop` are not, so trusted wrappers have to be written --
a real trust surface, since `push` genuinely can unwind. And the invariant is
checked at the end of *every* call that mutates a field, so a method that
breaks and restores it internally is refused, which is stricter than `holds:`
and will reject code the sampler accepts.

**A second error in this spike, same review:** this shadow's fields are
`pub`, where the fixture's are private. Private fields *are* the fixture's
boundary, so the shadow discarded the very obligation the section below then
declared unreachable -- and the measurement was taken on that weaker shadow.
Closed spec accessors are the fix and they work.

## The original section, as written and now retracted: coverage is not free

`docs/component-proof-design.md` called obligations 3 (boundary) and 4
(coverage) load-bearing and easy to skip. That is now measured, not argued.

Adding a public mutator with **no contract on it at all**, which flatly
breaks the invariant:

```rust
pub fn force_push(&mut self, key: u32, value: u32) {
    self.entries.push((key, value));
}
```

**9 verified, 0 errors.** The verifier answers what it is asked and nothing
else. A proof of init and preservation over a set that omits one public
mutator is worth nothing about the type, and it reads as *stronger* than the
sampling it would replace.

So the split is:

| obligation | who discharges it |
|---|---|
| 1 init | Verus |
| 2 preservation | Verus |
| 3 boundary | **Ply, or nobody** |
| 4 coverage | **Ply, or nobody** |

Ply already has the machinery to do 3 and 4 honestly: the receiver scan
enumerates a type's public operations today for the sampling tier, and
`W0520` already discloses which ones it could not call. That enumeration
becomes load-bearing rather than informational -- and a type whose operation
set cannot be closed must be refused by name, the way `V0508` already
refuses exhaustive checking on a shape it cannot reach.

## The other honest cost: translation

The fixture writes its search as `self.entries.iter().position(..)`. The
shadow spells it out as a loop with an explicit `invariant` and `decreases`,
because that is what the verifier reasons about. Nothing else in the fixture
needed reshaping, but that one line did, and an adapter would have to either
generate such a loop or refuse the method.

This is the same honesty condition the kernel spike carries: **Verus proves a
translation, not the Rust.** The kernel ties its shadow back with a
differential test over generated trees. A component proof needs that tie too,
and this spike does not have one -- `proof/cache.rs` is hand-written and
nothing checks it still matches `tests/fixtures/boundedcache`. That is the
first thing a real implementation would owe.

## What this does and does not establish

**Does:** the instrument reaches this shape, cheaply, and fails when it
should. The brief is fundable on that ground.

**Does not:** that an adapter can generate this shadow automatically; that
coverage and boundary can be settled for types more open than this one; that
anything holds for interior mutability, trait objects, or generics. One
fixture, hand-translated, is one data point -- a decisive one for the
go/no-go question that was asked, and nothing more.


---

# Follow-up: the three steps the review asked for — 2026-09-07

## 1. The shadow, rewritten with private fields and a type invariant

`proof/cache.rs` is now that version. **8 verified, 0 errors**, same speed.
It rejects the uncontracted mutator *and* a free function in the same module
reaching into the struct (2 errors), and a struct with crate-public fields is
refused outright. All four obligations are the verifier's.

**The cost, recorded because it is a trust surface and not a formality.**
A type invariant may not be left broken across a call that can unwind, and
vstd's `Vec::push`/`remove`/`pop` are not marked `no_unwind`, so each needs a
trusted `external_body` wrapper. `push` genuinely can unwind on capacity
overflow. An adapter would have to generate these, and every one is a place
where the proof rests on an assertion nobody checked.

Second cost: the invariant is checked at the end of **every** call that
mutates a field, so a private helper that breaks and restores it is refused.
That is stricter than `holds:`, which only looks after each public
operation, and it will reject code the sampler accepts.

## 2. The token bucket — and this is what decides the fork

`proof/bucket.rs` shadows round 3's token bucket twice over: as a type
invariant (`available <= capacity`, all `state:` can say today) and as
two-state contracts on the mutators (what `try_take` and `refill` actually
promise). **6 verified, 0 errors, ~4s.**

Then the round-3 bugs, planted one at a time:

| bug | invariant only | with transition contracts |
|---|---|---|
| refill can exceed capacity | caught | caught |
| take succeeds with too few tokens | caught, as **arithmetic underflow** | caught |
| `refill(0)` resets the bucket to full | **6 verified, 0 errors** | caught, postcondition |

The third row is the measurement. `refill(0)` silently refilling the bucket
preserves `available <= capacity` perfectly, so the invariant proof passes
and the bug is invisible. The transition contract names it immediately.

Note the second row honestly: the invariant-only proof *does* catch that one,
but not because of the invariant -- the off-by-one guard makes
`available - tokens` underflow, and panic-freedom comes free with the type
invariant machinery. Crediting that to the invariant would be wrong.

**So the fork is settled on evidence.** Proving the `holds:` clause is real
but cannot reach transition bugs by construction, and transition bugs are
what the A/B rounds measured as the loss. Two-state contracts reach them, and
they discharge in seconds on the same shadow.

## 3. The rule for a proof that does not discharge

An idiomatic `HashMap` + `VecDeque` cache is *correct* and its proof
**fails**, with no counterexample, because the invariant is not inductive on
its own. Any invariant relating two containers kept in sync has this shape.

The rule, for the spec:

- A proof that does not discharge is **`not proved`**, never `violation`.
  A `violation` says Ply found an input where the promise is false. Verus
  failing to discharge says only that this proof, with these facts, did not
  close -- which is also what happens to correct code missing a lemma.
- It **falls back to the sampled tier** and reports what that earned, so the
  claim is not left with less evidence than it had before the proof was
  attempted.
- It says which obligation did not close and that no counterexample exists,
  because a reader who sees "not proved" with nothing else will assume the
  code is wrong. `V0508`'s wording is the model: unsupported, not unchecked.

Arithmetic is the same shape one level down: a bucket's `tokens + n` fails
"possible arithmetic overflow" where `saturating_add` proves. Real in the
strict sense, and a user whose code is fine in practice will read it as the
tool being wrong unless the wording carries its weight.

## Timing, corrected

This document said 0.93s. That is the verifier's own clock. Wall clock over
several runs: 1.5s to 4.4s. Both numbers are fine; saying which one is meant
is the point.
