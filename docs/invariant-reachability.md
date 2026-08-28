# What Ply can check of what this library's author actually cared about

*Measurement, 2026-08-27. Read-only: no product code was written or changed, and no
workspace test was run. Sources read: `tests/fixtures/ratelimiter/` (all nine modules and
its integration test), `tests/fixtures/ratelimiter/INVARIANTS.md`,
`docs/greenfield-ratelimiter-design.md`, `crates/ply-core/src/harness.rs`,
`crates/ply-core/src/callgraph.rs`, `crates/ply-core/src/fuzz_gen.rs`, `The-Ply-Spec.md`
§5.4–§5.4d and its out-of-scope list, `TODO.md`.*

---

## TLDR

**Of the eleven properties the rate limiter's author wrote down, Ply can check none today,
one after the method-resolution work now in flight, and still only that one after enums and
structs. Four are permanently out of reach of any per-function contract, because they are
about sequences of calls or about threads, not about single functions.** The remaining six
*are* per-function properties in substance, and every one of them is blocked by something
that is not on the roadmap at all: floating-point numbers, mutable output parameters,
generic type parameters, and constructing a value to call a method on.

| | count | which |
|---|---|---|
| Checkable today | **0** | — |
| After methods resolve | **1** | #10 (reject a zero refill interval) |
| After enums and structs | **1** | unchanged — enums and structs unlock nothing here |
| Never, as stated: sequences or threads | **4** | #6, #7, #8, #9 |
| Per-function in substance, blocked by unbuilt capabilities | **6** | #1, #2, #3, #4, #5, #11 |

The single most useful sentence in this document: **the one property Ply is about to be
able to check is the one the author would have been least worried about** — three lines of
code, one obvious test, no arithmetic — **while the property the author explicitly said
they trusted least is blocked by one missing type, `f64`.** The library's entire token
count is a `f64`, and Ply's type vocabulary has no floating-point at all: not in the
implementation, not in the spec, not in the TODO list. That is the finding, and it is not
one a type-coverage percentage could ever have surfaced, because floats do not appear on
this library's public surface even once — they are all internal.

Second finding, in the good direction: **contract expressions are far less restricted than
the spec claims.** The spec describes a closed list of allowed constructs, but nothing
enforces it — the checker only tests that the expression parses. The shipped acceptance
fixture already writes `result.subsec_nanos()` and `n.get()`, neither of which is on the
list. So "can you even say the property" is almost never the blocker. Reaching and building
the function is.

---

## What Ply can do, stated plainly

So the reasoning below can be followed without opening anything:

- A promise is attached to **one function**: a condition required of its inputs, and a
  condition guaranteed of its result. Ply then either proves it exhaustively for small
  inputs, or throws hundreds of random inputs at it.
- To do either, Ply must **build every argument from nothing**. The types it can build
  today: the integers (including `usize`/`isize`), `bool`, `char`, `Option`, `Result`,
  fixed-size arrays, `Vec`, `BTreeSet`, the `NonZero` integer family, and `Duration`. That
  is the entire list. No floats, no structs, no enums, no references it writes back
  through, no generic type parameters, no opaque standard-library types like a timestamp.
- To find the function at all, Ply indexes **free functions only**. There is no handling of
  `impl` blocks anywhere in the resolver, so any promise naming `Type::method` currently
  finds nothing. This is the work in flight.
- Ply has **one** way to talk about "before and after": the entry value of something the
  function *reads*. It cannot talk about the before-and-after of something the function
  *writes*, and the spec says so in as many words.
- Ply has **no** way to talk about a sequence of calls, and **no** way to talk about
  threads. The spec puts both out of scope deliberately, and offers a different mechanism
  for them: record the property as a human-attested claim with its external evidence, so
  the picture stays honest rather than silently green.

One structural fact dominates everything below: **thirty-six of this library's thirty-seven
functions are methods or associated functions inside `impl` blocks.** The single free
function is `refill_and_debit`, which is where the arithmetic lives — and it takes a
generic clock, a mutable float, and a mutable opaque timestamp.

---

## The eleven

### 1. A bucket's token count never exceeds capacity, however long the gap between checks

**Is it a per-function property?** In substance, yes, with a caveat. As written it
quantifies over an entire history ("no matter how much time passes between checks"), but
its content is a one-call preservation step: *if the count was within capacity on entry, it
is within capacity on exit*. That step is a genuine contract on `refill_and_debit`, and the
full sentence follows from it by an induction over calls that Ply does not perform and does
not claim to. Worth being precise: the step is over an explicit parameter (`tokens: &mut
f64`), not over hidden state, so unlike #6/#8/#9 it is at least nameable.

**What blocks it.** Four separate things, on the one function Ply can already find:
`tokens` is an `f64` (no float support anywhere in Ply), it is a `&mut` (explicitly
unsupported; the spec calls this out as the honest limit that makes the before-and-after
primitive unusable for mutation), `quota: &Quota` is a struct with private fields holding
`NonZero` values that Ply refuses to nest, and `clock: &C` is a generic parameter whose
associated timestamp type would also have to be pinned. The configuration key for pinning a
generic (`check_with`) is parsed by the config reader and used by nothing.

**Worth checking?** Moderate. The clamp itself is one `.min(capacity)` call and is obvious
on sight. The non-obvious part is what happens to `elapsed_nanos as f64 * rate` after a
month-long gap — whether it can reach infinity or NaN and slip past the clamp. That is a
real question, and a random-input run would answer it in seconds if floats were supported.

---

### 2. The count never goes negative, and is never debited unless the whole request is met

**Is it a per-function property?** Yes, cleanly. Single call, single function, both halves
expressible: the count on exit is non-negative, and it either dropped by exactly the
requested amount (admitted) or did not change at all (refused).

**What blocks it.** "Did not change at all" needs the before-and-after of a parameter the
function *writes*, which is the one thing the two-state primitive explicitly cannot do.
Plus the same `f64`, `&mut`, struct, and generic blockers as #1.

**Worth checking?** Low on its own. The code is a plain if/else with the debit inside one
branch — three lines, and the existing tests assert the no-debit-on-refusal case directly.
It would come free alongside #1 and #5, though.

---

### 3. The last-updated timestamp only moves forward; a backwards clock costs zero elapsed time

**Is it a per-function property?** It is two properties, and they split.

*The clock half* — "elapsed time is never negative, and saturates to zero when the
arguments are reversed" — is a textbook single-function postcondition, with no state
involved at all. This is the most contract-shaped sentence in the entire list.

*The bookkeeping half* — "the stored timestamp only ever moves forward, and does not let
the bucket catch up on a gap that never happened" — is before-and-after over a written
parameter again, and its second clause ("once the clock resumes normal progress") is
explicitly about what a *later* call sees. That clause is out of reach.

**What blocks the clock half.** All three clock implementations fail, for three different
reasons, which is itself informative. The production clock and the wall clock take
`std::time::Instant` / `SystemTime` — opaque types with no public way to build an arbitrary
value, so Ply cannot generate an input. The test clock takes `Duration` on both sides,
which Ply *can* build as of today — but it is a method on a struct holding a shared atomic
counter, so Ply cannot build the receiver to call it on, and the resolver cannot find it in
the first place. Note the shape of that: the one function in this library whose argument
types are fully supported is unreachable for reasons that have nothing to do with types.

**Worth checking?** Moderate, and higher than it looks. Three implementations each
saturate by a *different* mechanism — `saturating_duration_since`, an `unwrap_or` on a
failed subtraction, and a `checked_sub().unwrap_or()`. Three hand-rolled variants of one
rule is exactly where one of them is quietly wrong.

---

### 4. An over-capacity request is refused as impossible, never as "retry later"

**Is it a per-function property?** Yes, entirely. Given the requested amount and the
configured capacity, the returned decision must be the "no wait will ever help" variant.
One call, one function, no state read or written.

**What blocks it.** The receiver. All three functions this constrains are methods on
state-bearing types: a bucket holding a mutex, and a keyed limiter holding a vector of
read-write-locked hash maps over a generic key type with a generic hasher. Ply cannot build
either, and this is *not* fixed by adding enums and structs as currently scoped ("public,
invariant-free fields") — every field on both types is private, and a mutex is not a shape
any generator here derives. It also needs generic instantiation, which is parsed but not
implemented.

Worth noting what is *not* a blocker: naming the returned decision variant. The contract
language admits `matches!`, and in practice admits anything that parses, so writing "the
result is the impossible-request variant" is fine.

**Worth checking?** Yes — genuine, and among the better candidates. The guard is duplicated
in three places (both limiters' admit paths and the "how long until ready" query), each
comparing against a capacity it reaches by a slightly different route. Duplicated guards
drift; that is what a check is for. Unfortunately this is also the invariant furthest from
being reachable.

---

### 5. A "retry after N" is long enough to actually succeed, and not needlessly longer

*The author's own least-trusted invariant, first of two.*

**Is it a per-function property?** As written it is a two-call statement — refuse, wait
exactly that long, then succeed. But its real content is one call's arithmetic: the
returned wait, multiplied by the refill rate, must cover the shortfall exactly, with the
ceiling operation landing on the right side of the discrete comparison the admit path uses.
Both halves ("long enough" and "not too long") restate as pure arithmetic over the token
count, the requested amount, and the rate — inputs and output of a single call. This is a
contract.

**What blocks it.** `f64`, and essentially only `f64`. The shortfall, the rate, the
division, and the ceiling are all floating-point, the bucket count comes in as a mutable
float, and Ply has no float type. Nothing else here is exotic.

**Worth checking? This is the one.** The author named it as what they would trust least
from reading the code, and their reasoning is exactly right: `ceil(shortfall / rate) * rate
>= shortfall` holds in exact arithmetic and can fail by a hair in floating-point, which is
the kind of bug that survives every example a human picks and shows up on some customer's
odd refill rate. The existing test suite checks it at one clean value (three tokens short
at one per second equals three seconds). Random inputs over the three numbers involved
would either find the boundary or build real confidence, and would do it in seconds. This
is the highest-value single item this exercise found.

**Which engine.** Worth separating: the random-input engine could take this today if floats
were in the type vocabulary, because generating random floats is trivial. The exhaustive
engine is a genuine open question — whether the underlying solver handles `ceil` and `min`
on floats well enough to finish is not something I can answer from reading, and it should
not be assumed either way (see *Uncertainties*).

---

### 6. A brand-new key is judged only against capacity, never against anyone else's history

**Is it a per-function property? No.** The sentence says a result depends only on *this*
key's absent history and not on *other* keys' presence — a statement relating the outcome
to the contents of a shared table across everything that came before. The one-call
approximation ("if this key is absent on entry, the result is a full-capacity admit") is
about state hidden behind `&self` that no contract here can name, and even that
approximation drops the "never against any other key's history" clause, which is the actual
claim. Per-function contracts cannot say this.

**Worth checking?** Moderate risk, and the existing tests cover the practical case. The
right home for it is a sequence-driven random test, not a contract.

---

### 7. Concurrent calls never collectively admit more than the math allows

*The author's own least-trusted invariant, second of two.*

**Is it a per-function property? No, and never will be.** It is a property of what happens
when several threads execute the same function at once. Ply's spec puts concurrency
verification out of scope by name and explicitly, not by omission — the engines reason about
single-threaded behaviour.

**Worth checking?** It is the highest-risk item on the list by the author's own judgment,
and Ply is the wrong instrument for it entirely. The honest thing here is not a verdict —
it is the mechanism the spec already designed for exactly this case: record it as a
human-attested claim pointing at the sixteen-thread stress test that already exists in this
library, so the picture does not render green through silence. That test, incidentally, is
better evidence than anything Ply could offer even in principle.

---

### 8. Evicting an idle key is equivalent to that key never having been seen

**Is it a per-function property? No.** It compares two different histories — evict-then-
check against never-seen-then-check — and asserts they are indistinguishable. That is an
equivalence between execution traces. No per-function contract expresses it, and no
induction over single-call steps recovers it either; it is a genuinely different shape of
claim.

**Worth checking?** Moderate. It is the kind of thing a randomly generated sequence of
operations would test well and a contract would not test at all.

---

### 9. With a key cap set, the number of tracked keys never exceeds it

**Is it a per-function property?** As stated, no — it quantifies over an adversarial
stream of requests. It does have a one-call preservation step, like #1: *if the table was
within its cap on entry, it is within its cap on exit*. But unlike #1, that step is over
state hidden behind `&self` — a sharded map of a generic key type — which contracts here
cannot name at all. So it is out of reach both as stated and in its reduced form.

**Worth checking?** Real operational risk: this is the memory-exhaustion defence, and the
cap is enforced per shard rather than globally, which means the honest global bound is
`shards × cap`, not `cap`. Whether that gap between the stated invariant and the
implementation matters is a design question worth raising with the author, and it is a
question a reader spots by reading — not one Ply would have caught.

---

### 10. Configuration that cannot describe a real rate is rejected at construction

**Is it a per-function property?** Yes, perfectly. One function, two inputs, one result:
a zero interval yields an error, anything else yields success.

**What blocks it today.** Only that the resolver cannot see inside `impl` blocks. Both
argument types — a non-zero integer and a `Duration` — are supported as of today's change.
There is no receiver: this is an associated function, not a method, so nothing needs to be
built. The return type is not gated (Ply only needs to build a function's *inputs*; the
return type only has to exist). And the contract is easy to write: "the result is an error
exactly when the interval is zero", using either the `.is_err()` form or a plain comparison.

**This is the one that lands** when the method work ships — with one caveat flagged under
*Uncertainties*: it lands only if that work indexes associated functions (no `self`) and not
just methods with receivers.

**Worth checking?** Honestly, barely. It is a two-line guard with an obvious test that
already exists and passes. The author did not flag it as a worry, and no reader would.
That it is the *only* thing about to become reachable is the most useful piece of
information in this document.

---

### 11. The reported "remaining" reflects the bucket's real state, not an optimistic estimate

**Is it a per-function property?** Yes in substance. As phrased it invokes a hypothetical
follow-up call, but its content is a one-call relation: the reported number equals the
floor of the count *after* the debit, not before.

**What blocks it.** The same four as #1: the count is a mutable float, and stating it needs
the after-value of a written parameter. Plus the struct and generic arguments beside it.

**Worth checking?** Low-to-moderate on its own — but it is the same rounding family as #5
(`floor` after a floating-point subtraction, versus `ceil` after a floating-point division),
and if floats were supported it would come almost free alongside it. A number reported to
callers that is off by one is the sort of thing that shows up in someone's rate-limit
header.

---

## What would actually unblock this library

Ordered by how many author-stated properties each unlocks, rather than by how much it moves
a coverage number:

1. **Floating-point numbers.** Unlocks the substance of #1, #2, #5, #11 — including the
   author's own least-trusted arithmetic invariant. Currently absent from the type
   vocabulary, from the spec's supported-signature list, and from the TODO list. For the
   random-input engine this is close to free; for the exhaustive engine it needs a spike
   before anyone promises it.
2. **Building a receiver by calling the library's own constructors.** The spec already
   designs this — a user-named constructor function that lifts a type into the supported
   set — and it is not built. It is worth far more here than enums and structs are: chaining
   three of this library's own constructors (test clock, quota, bucket) produces a callable
   bucket, which is what #4 needs and what any state-bearing method needs. Enums and structs
   as currently scoped (public, invariant-free fields) unlock *nothing* in this library,
   because every relevant type has private fields, a lock, or a generic parameter.
3. **Mutable output parameters with before-and-after contracts.** Needed by #1, #2, #3's
   second half, and #11. The spec names this limit honestly and names what it costs
   (mutation clauses for the exhaustive engine).
4. **Pinning a generic type parameter to a concrete one.** Parsed in the configuration
   model, used by no code path. Every function in this library that touches time is generic
   over a clock.
5. **Enums and structs.** Genuinely useful in general; zero effect on these eleven.

And one thing that is not on any list: **four of the eleven need a check that drives a
sequence of operations and asserts a property after each one.** That is not a per-function
contract and should not be forced into one. Ply already has a random-input engine that
could do it. Whether that is worth building is a design call, not this measurement's to
make — flagging it in one line, as scope requires.

---

## Was type-coverage percentage a misleading instrument?

**Yes, in three distinct ways, and it steered real work.**

**It measured a gate that was not binding.** The percentage counts parameter and return
types on the public surface and asks whether each is buildable. But at 21% coverage, zero of
these eleven properties were checkable; at roughly 80% coverage — the figure implied by the
recorded counts after today's change — zero of eleven are still checkable. The number moved
sixty points and the answer did not move at all, because the binding constraints were
finding the function, building a receiver, mutation, and floats. None of those is a "type"
in the counted sense. A metric that can quadruple while the outcome stays at zero is not
measuring the outcome.

**It weighted by frequency, and frequency is not importance.** The counted types were
dominated by the ones appearing in getters and configuration: `Duration` and the non-zero
integers together were the large majority of uses. Those got built, correctly and
carefully — and they unlock exactly one invariant, the least interesting one on the list.
Meanwhile the type the library's entire correctness argument rests on, `f64`, has a
public-surface count of **zero**, because it is internal state. The metric was structurally
blind to it. A getter returning a `u32` counted the same as the arithmetic core.

**Its own denominator was soft.** Two paragraphs of the same commit quote different totals
for the same measurement — 82 type uses in one place, 70 in another — depending on what was
being counted where. Nobody noticed, because a percentage invites comparison to itself over
time rather than scrutiny of what is underneath it.

To be fair to the number: it was not *wrong*, and the work it drove was good work, correctly
done, with real acceptance evidence. `Duration` and the non-zero integers genuinely needed
building. The problem is that it answered "which types appear most often" when the question
was "which properties can we check", and those turned out to be almost unrelated questions
for this library.

### What to measure instead

**Invariant reachability, on a real library whose author wrote their properties down.** The
procedure is exactly this document: take the properties someone independently cared enough
to enumerate, and for each one record (a) whether it is a single-function property at all,
(b) what specifically stops it, and (c) whether the author flagged it as risky. Three numbers
fall out that a coverage percentage cannot produce:

- **How many properties reach an engine.** The headline. Today: zero of eleven.
- **How many are out of the tool's shape entirely.** Four of eleven here — and this number
  should be reported proudly, not hidden. It says where the honest-recording mechanism
  belongs rather than a verdict, which is the whole premise of the project.
- **Blocker attribution: which single missing capability unblocks the most properties.**
  This is the roadmap input the percentage never gave. It ranks floats first here, and it
  ranks enums and structs last — the opposite of what the coverage number implied.

A useful secondary check, since this library already has fifty-two passing tests covering
most of these properties: **would Ply's answer beat the test that already exists?** For #5,
clearly yes — random inputs over three floating-point numbers beat one hand-picked example.
For #7, clearly no — a sixteen-thread stress test is better evidence than anything a
single-threaded engine offers. Asking that question per property keeps the tool pointed at
work that is worth doing rather than work that is merely countable.

---

## Uncertainties, and what would settle them

**Does the in-flight method work index associated functions, or only methods with
receivers?** The entire "1 after methods" number rests on this: #10's constructor takes no
`self`, so if the new code indexes every function inside an `impl` block it lands, and if it
indexes only receiver-taking methods it does not. *Lean: it lands* — the natural
implementation walks every function item in an `impl` block, and receiver handling is a
separate concern layered on top. *Settled by:* after it ships, put a promise on
`RefillRate::new` and see whether it resolves and reaches an engine.

**Can the exhaustive engine handle this library's float arithmetic?** The underlying solver
has floating-point support in principle, but `ceil`, `floor`, and `min` are compiler
intrinsics whose modelling I cannot confirm by reading, and floating-point reasoning is
expensive even when supported. *Lean: partial — comparisons and basic arithmetic yes,
rounding intrinsics unknown.* *Settled by:* a ten-line spike — one function taking two
`f64`s with a contract involving a division and a ceiling — before anyone puts floats on a
roadmap with a promise attached. The random-input engine has no such doubt; floats are
trivial for it, which is an argument for doing that half first.

**Is the contract language really unrestricted?** The spec describes a closed list of
allowed constructs, and the shipped acceptance fixture violates it in two places without
complaint, because the only enforcement is that the expression parses. *Lean: unrestricted
in practice today.* This is good news for every "can you even say it" question above — but
it is a spec claim that has stopped being true, and by this project's own rules it should be
retracted or implemented rather than left to be discovered. Flagged, not fixed: this
document changes no source.

**Is the picture better than feared?** In one specific way, yes: the properties that are
blocked are mostly blocked by *one* thing each, not by a tangle, and the biggest single one
(floats) is a well-understood addition rather than a research problem. The pessimistic part
is not the count of blockers — it is that four of eleven are the wrong shape for the tool
entirely, and that includes one of the two the author lay awake about.
