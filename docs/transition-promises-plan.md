# Promises about methods that change things — the plan

Follows `docs/component-proof-design.md`, whose fork was settled on evidence:
an invariant over a single state cannot see a transition bug, and four of the
six bugs behind the measured "1 in 6" are that shape.

The maintainer's framing, which this plan is built on and which was tested
rather than assumed:

> Every member function is a pure function once the object's state is an
> explicit input or output. A mutating method is
> `(state before, arguments) -> (state after, result)`. A getter is
> `(state) -> (result)`.

**It holds.** Verified by patching a copy of the tool -- 23 changed lines,
lifting one gate in the path that already exists -- and running it against
purpose-built fixtures. Promises about mutating methods then found **5 of 5**
planted bugs, including the two no invariant can see: a refill of zero that
silently tops a bucket back up, and a cache that fails to recognise a key it
already holds (round 2's real miss). The bug-planting tier scored them
without modification.

**No new checking machinery is needed.** The harness already builds a
receiver from the constructor plus a random sequence of the type's own
operations, and already evaluates a promise's `old(...)` against a snapshot
taken before the call. That is the pure-function shape. What blocks it is a
single refusal of any receiver that is not `&self`.

## What the plan corrects in this document's own premises

- **The read-only half is not new.** A getter's promise can already name the
  receiver and call the type's other getters -- verified on the *unpatched*
  tool. The premise that the object cannot be named in a promise was false;
  the guidance simply never said otherwise, and every existing fixture names
  public fields rather than readings.
- **"State" can only mean what the type's public read-only surface reports.**
  The harness is compiled outside the type, so private fields are unreachable
  by construction. That is the boundary, and it is what makes the promise
  honest: a promise can only talk about what a caller could observe.
- **The before-state is reached, not generated.** Constructor plus a random
  sequence of the type's own operations. The existing disclosure already says
  which operations were pooled and which were excluded.

## The honesty gap that must close in the first slice

**When a promise about a transition fails, the user is shown half the input.**
The report names the arguments of the failing call and says nothing about the
sequence of operations that put the object into the state where it fails. On
a panic the raw generated tuple leaks through instead.

"Ply never reports a broken promise it cannot show you the input for" is
currently untrue for a transition. Closing that is step 3 below and is not
optional.

## Three other breakages, each found by running

1. **A precondition that names the state fails to compile**, because the
   filter is emitted before the receiver exists. Once fixed, a
   state-dependent precondition behaves correctly as a rejection filter, and
   the existing high-rejection warning fires honestly.
2. **A snapshot can secretly mutate.** `old(self.get(key))` on a cache whose
   `get` takes `&mut self` alters the very state being snapshotted. It still
   caught the planted bug, but a promise about ordering would be evaluated on
   an altered history. Refuse by name: a reading may only call `&self`
   operations.
3. **Snapshotting the whole object** needs `Clone`, and today reports a
   compiler error rather than a refusal. Refuse by name and steer to
   readings, which also removes any cost concern for large states.

## Keep the whole-object rule; it is a different question

`holds:` and method promises overlap in what they can catch, and should still
both exist:

- `holds:` is checked after **every** step of the sequence, with attribution;
  a method promise is checked once, at the end. Same states, thinner sampling.
- On the proved tier the invariant is the induction hypothesis -- the token
  bucket's two-state proof only discharges because the invariant rules out
  underflow. Transition proofs need it; they do not replace it.
- It attaches to the type and its box in the drawing; a method promise
  attaches to the function.

One sentence for the guidance: **the whole-object rule is what is always true
of the value; method promises are what each operation does and what each
reading tells you.** Existing fixtures and documents need no change.

## The slice

1. Failing fixtures first: the token bucket with a refill-of-zero bug, and a
   cache with the update-path bug, each asserting a violation. Watch them fail
   against today's refusal.
2. Lift the gate: accept `&mut self` (owned `self` stays refused), borrow the
   final call mutably, and emit a receiver method's precondition after the
   receiver exists.
3. Show the whole input: constructor call, the sequence with its arguments,
   then the checked call -- on both the promise-false and panic paths.
4. Refuse by name: a mutating operation inside a reading; a whole-object
   snapshot. Exact-string tests on both sentences.
5. Wording and documentation in the same commit, including retracting the
   "cannot be checked" statements in the spec and the two skills.

Deliberately out: owned `self`; the proved tier; a replayable Rust test for a
receiver counterexample; any change to the whole-object rule, the drawing, or
the sequence bound.

## Open decision for the maintainer

**Is a getter added purely so a promise can read a private field acceptable?**
The cache needed a `contains`/`peek` to state its promise. Those are ordinary
cache API. But the guidance currently leaves "adding public API for the tool's
benefit" to the developer, and every real type will meet this on day one.
