# Review: how should Ply build a receiver? (2026-08-26)

## TLDR

**The provisional call is right, and it will not move the number.** Build receivers by
calling the type's own constructors; allow field-by-field only for all-public-field types;
refuse everything else by name. Keep all three of those. But three corrections attach, and
the third is the one that matters:

1. **The reason given for the public-fields rule is false**, and the rate limiter document
   falsifies it in two places. Keep the rule, drop the justification, and record it as a
   named assumption instead — which is what the spec already says type invariants are.
2. **Constructor-only is not the coverage-losing option. It is the coverage-*winning*
   one**, for a reason nobody has stated: it is the only rule that can build a type whose
   fields Ply could never represent — a mutex, a hash map, an atomic. Field-by-field is
   stuck at types made entirely of things the checker already understands, which is the
   class of type that least needed help. Constructor-only reaches roughly 26 of the rate
   limiter's 40 functions at ceiling; field-by-field reaches about one.
3. **A freshly built value is the wrong value, and this is measurable rather than
   philosophical.** A fresh token bucket is full. So a single call to `check_n` on a
   fresh bucket is *always* admitted — the deny branch is unreachable by construction.
   That branch is where the arithmetic lives that the designer wrote down as the thing he
   trusts least. Constructor-only checks the branch nobody is worried about and
   structurally cannot reach the one they are. Six of the eleven stated invariants are
   out of reach for the same reason.

**The fourth option nobody named, and my actual recommendation: build the value with the
constructor and then apply a bounded sequence of the type's own operations, with the
sequence length named in the verdict the same way a loop bound already is.** Constructor-
only is that with the sequence length fixed at zero. It needs no new trust, no declared
invariant, and it is the single largest coverage lever in the document — it brings six
invariants into range that none of the three candidate rules can reach.

**On sequencing: this is not the top blocker, and the readiness measurement does not say
it is.** With methods resolving and field-by-field construction built and nothing else
changed, all three candidate rules reach **zero** of the rate limiter's functions —
exactly the number reached today. The binding constraint on that document is the checker's
type vocabulary: it has no `usize`, no `f64`, no `Duration`, no `NonZeroU32`, no `String`,
no struct or enum of any kind, and no way to instantiate a generic. Fix resolution first
because it is cheap and turns a lie ("not found") into a true statement ("cannot build
this"); fix the type vocabulary second because it is what unblocks both methods and free
functions; settle the receiver rule third, when there is something to measure it against.

---

## What I checked, and where

Everything below is read from the repository, not assumed.

- **Resolution.** `top_level_fn` in `crates/ply-core/src/callgraph.rs` matches
  `syn::Item::Fn` and nothing else. There is no `Item::Impl` arm anywhere in that file.
  `Type::method` therefore cannot resolve, which is exactly what the readiness measurement
  saw.
- **The receiver refusal.** `crates/ply-core/src/harness.rs:646` — the parameter loop
  destructures each argument as a typed one and bails on anything else, which is precisely
  a `self` receiver.
- **The type vocabulary.** `rust_type_from_syn` in the same file accepts `u8`–`u64`,
  `i8`–`i64`, `bool`, `char`, `Option`, `Result`, fixed arrays, `Vec` of scalars,
  `BTreeSet` of scalars, and shared references to those. There is no `usize` arm, no
  `f64`, no `String`, no `NonZero*`, no `Duration`, and no struct or enum handling at all
  — the enum of supported types has no variant for one. A mutable reference is refused by
  name deliberately.
- **Generics.** `check_with` is parsed into the model and read by no other code
  (`crates/ply-core/src/model.rs:105` is its only non-test appearance). The schema doc says
  so in as many words.
- **Two tiers, two different reaches.** The exhaustive checker's harness is written *into
  the crate* as `ply_generated.rs` with a `mod` declaration added to the crate root
  (`harness.rs:1073`). The random-input checker's harness is a *separate crate* under
  `target/ply/fuzz/` depending on the target by path (`harness_crate.rs`, module doc). The
  first can name private items at the crate root; the second can name only public ones.
- **The earlier measurement of constructor-only.** `tests/spike/FINDINGS.md`, smaller
  findings: a harness built only from a public smart constructor verifies, but "witness
  coverage capped at what `pub` constructors can reach — provably narrower than what
  in-crate code can produce", with a worked case where in-crate code produces a value the
  constructor refuses. Constructor-only was already measured once and already found to
  under-reach.

---

## Is the public-fields rule sound?

**No, and the rate limiter falsifies it — but the rule is still worth keeping, under a
different description.**

The claim is: if anyone can already build any combination of fields, there is no invariant
left to violate. That is false the moment an invariant is maintained by a function rather
than by the type's constructor. Two cases from the document under review:

- `SweepReport` has three public `usize` fields and an arithmetic relation between them:
  `keys_before - keys_removed == keys_after`. Nothing enforces it. `sweep` maintains it.
  Anyone can write `SweepReport { keys_before: 0, keys_removed: 5, keys_after: 99 }`.
- `Decision` is an enum whose payloads are all public. `Unsatisfiable { capacity, requested }`
  is only ever produced when `requested > capacity` — that is invariant 4, stated in prose
  in the document and enforced by a guard in two methods, not by the type. `Decision::Unsatisfiable
  { capacity: 5, requested: 2 }` is buildable by anyone and produced by nothing.

So "public fields" tracks "the author didn't need a constructor", not "there is no
invariant". Those come apart exactly where invariants are documented rather than enforced,
which is the common case in Rust application code.

**Three further things worth knowing before the rule is written down:**

*Visibility is not a capability boundary for Ply, and it means different things on the two
tiers.* The exhaustive checker's harness lives inside the target crate, so it can already
construct a private-field struct declared at the crate root, field by field, today. The
random-input checker's harness is a separate crate and cannot. So a single rule stated as
"only when every field is public" is a *policy* on one tier and a *capability limit* on the
other. If the verdict uses the same words for both, it will be telling a reader something
false on one of them. This asymmetry is not in the design as posed and should be.

*`#[non_exhaustive]` changes nothing useful.* It restricts construction from *other*
crates only. The exhaustive checker's in-crate harness is unaffected by it; the
random-input checker's separate harness is blocked by it, and blocked as a compile error,
so it is self-enforcing there. It is a statement about future versions, not about
invariants — a type carrying it may have none, and a type without it may have several. Using
it as a gate refuses for the wrong reason on one tier and is redundant on the other. Nothing
in the code reads it today; I would not add a reader for it.

*"Constructible by anyone" and "reachable in this program" are different sets, and they
differ in both directions.* Reachable-but-not-constructible is the case the spike already
measured: in-crate code produced a value the public constructor refuses. Constructible-but-
not-reachable is `SweepReport { 0, 5, 99 }`. And reachable-only-after-a-sequence is the
third, which is the subject of the next two sections. Ply should care about the difference,
and its own doctrine already says how: **name which set was checked, in the verdict, rather
than picking one silently.** The spec already carries the sentence this needs — that type
invariants are assumed and never asserted, so a proof may rest on an invariant the code
itself breaks, and that this belongs in the verdict's own explanation. The public-fields
rule is that assumption, made concrete. It does not need a soundness argument. It needs the
sentence it already has.

**So: keep the rule, delete the reasoning.** Written as "Ply assumes a public-field type has
no invariant, says so on every verdict that rested on the assumption, and lists the
assumption where the other assumptions are listed", it is honest and it is consistent with
everything else the tool does. Written as "public fields means no invariant", it is a claim
that this repository's own measurement document disproves, and it will be quoted back.

---

## Is constructor-only sound?

**Yes in the narrow sense, and the narrow sense is narrower than it sounds.** Every value it
builds is one the program could produce *through that constructor*. That is not the same as
every value the program can reach, in three ways:

1. **Other constructors reach other subsets.** `RefillRate` has two: `new`, which can fail,
   and `per_second`, which cannot and always sets the interval to exactly one second. A
   harness built from `per_second` alone explores a one-dimensional slice of a
   two-dimensional type. Building from all of them is the obvious answer, and it multiplies
   the state space by the number of constructors — which is fine, but it needs saying,
   because "the constructor" in the decision as posed is singular and real types have
   several.
2. **Constructor arguments have their own invariants, and the recursion has to bottom out.**
   `Quota::new` takes a `RefillRate`, which comes from `RefillRate::new`, which takes a
   `NonZeroU32` and a `Duration`. That chain terminates only if the checker can build the
   leaves. Today it cannot build any of them. So constructor-only is not a self-contained
   rule; it is a rule *plus* a requirement that the type vocabulary reach far enough down
   the chain. On this document the chain is three deep before it hits a type Ply has never
   heard of.
3. **The spike already measured the gap.** A public-constructor harness never explored a
   state in-crate code produces. That is recorded in `tests/spike/FINDINGS.md` as "reduced
   coverage, not impossibility", and it is the correct verdict.

**What is genuinely good about it, and is not in the reasoning offered.** Constructor-only
never names a field, so it can build a type whose fields Ply could not represent in a
hundred years: `TokenBucket` holds a `Mutex<BucketState<C::Instant>>`; `KeyedRateLimiter`
holds a `Vec<RwLock<HashMap<K, Entry<C::Instant>, S>>>` and an `AtomicU64`. Field-by-field
construction cannot touch either. `TokenBucket::new(quota, clock)` builds one without Ply
ever knowing a mutex was involved.

That inverts the framing of the decision. Constructor-only is described as the safe rule
that costs coverage. On real code it is the *high*-coverage rule, and field-by-field is the
one that is stuck — restricted to types assembled entirely from things the checker already
handles, which is the class of type that needed the least help. The right ordering is the
one the maintainer picked; the reason is better than the one he gave.

---

## The question that is actually the real one

> If methods are usually called on values that have been mutated by earlier calls, then
> constructing a fresh value and calling one method may check a case that barely matters.

**Yes. On the measured document this is not a risk, it is a certainty, and it is provable
by reading one function.**

A fresh `TokenBucket` starts full: `new` sets `tokens` to `capacity`. `check_n` first
rejects `requested > capacity` as unsatisfiable, so by the time it reaches the token math,
`requested <= capacity == tokens`. Elapsed time since construction is zero, so no refill
happens. The comparison `tokens >= requested` is therefore **always true**.

The deny branch of `check_n` is unreachable from a freshly constructed bucket. So is the
deny branch of `refill_and_debit`, which is where the deficit division, the `ceil`, and the
`retry_after` computation live — invariant 5, which the designer wrote down as the one he'd
"trust least from a read-through alone". And the refill ceiling — invariant 1, "no matter
how much time passes between checks" — is a statement about a bucket that has been sitting
around, which one call after construction cannot express at all.

Counted against the eleven stated invariants, constructor-only at its ceiling reaches:

| invariant | reachable from a fresh value + one call? |
|---|---|
| 1 — token count never exceeds capacity, however long the gap | **no** — needs elapsed time between two calls |
| 2 — never negative, never a partial debit | half — the "never debited unless satisfied" half needs the deny path |
| 3 — timestamp only ever moves forward | **no** — needs a second call with a backwards clock |
| 4 — over-capacity request is unsatisfiable, not denied | yes |
| 5 — `retry_after` is long enough and not too long | **no** — deny path unreachable |
| 6 — an unseen key is judged only against capacity | yes |
| 7 — concurrent calls never over-admit | **no** — not an input-generation question at all |
| 8 — eviction is equivalent to never having been seen | **no** — needs several keys and a sweep |
| 9 — key count never exceeds `max_keys` | **no** — needs an adversarial stream of calls |
| 10 — unreal configuration is rejected at construction | yes |
| 11 — `remaining` reflects real state | half — allow arm only |

Three and two halves out of eleven. Every one of the six it misses is missed for the same
reason: the invariant is a statement about a *history*, and a freshly built value has none.
And every one of those six except invariant 7 is reachable by a value built with the
constructor and then driven through two or three of the type's own public operations.

So no, the approach is not aimed at the wrong target — a receiver is genuinely the thing
that must be built. But the rule as posed stops one step short of where the invariants live.

---

## The fourth option

**Build the receiver with a constructor, then apply a bounded sequence of the type's own
public operations, and name the sequence length in the verdict.**

- Every state so produced is reachable by construction. No invariant argument is needed, no
  declaration is needed, and nothing is taken on trust.
- Constructor-only is the special case with length zero. This is not a competing rule; it
  is the same rule with a knob, and the knob's default can be zero on day one.
- The honesty story is one Ply already tells. The tool already reports a loop bound in the
  verdict and refuses to report a deeper one than it earned. "Checked on buckets reachable
  in at most three operations from a fresh one" is the same sentence about the object
  instead of the loop, and it fails in the same honest direction: a reader knows exactly
  what was and was not covered.
- It is the standard answer elsewhere — stateful property testing has done this for years —
  so it is glue, not a solver, which is the line the spec draws around what this project
  builds.
- It costs: the state space multiplies by (operations × arguments) per step, which the
  random-input tier absorbs happily and the exhaustive tier will not, past one or two steps.
  That asymmetry is fine and already familiar: the random-input tier is the workhorse and
  reaches shapes the exhaustive one cannot.

There is a **fifth** option worth naming, as the right shape for the deferred user-declared
route rather than as a competitor:

**Let the author declare the type's invariant, not its builder.** Then field-by-field
construction becomes legitimate — build freely, assume the invariant — and the invariant
itself becomes a claim that is owed evidence until something shows the constructors and
mutators preserve it. That is precisely the boundary-promise pattern this tool already
ships: a declared promise, listed in the trust surface, checkable by the cheap tier,
carried on the verdict until discharged. The machinery for "can any value satisfy this" and
"can any value break this" already exists and already runs in well under a second.

The difference from the deferred option 3 matters: a declared *builder* is a recipe taken
on trust, and trust that is never checked is green paint. A declared *invariant* is a claim,
and claims are what this tool is for. If option 3 is ever built, it should be built as this.

---

## Coverage against the rate limiter

The document contains 40 `fn` items (its own headline count of 38 excludes the bare trait
signatures). They break down as: 19 inherent methods taking `&self`, 8 trait-implementation
methods, 8 associated functions with **no receiver at all**, 3 bare trait signatures with no
body, 1 trait default body, and 1 free function.

**Today: 0 checked.** Confirmed by the readiness measurement.

**With methods resolving and field-by-field construction built, and nothing else changed:
still 0 — under all three rules.** This is the number that should decide the sequencing.
Every path is blocked before the receiver rule ever gets a say:

- `Decision::is_allowed` — the single cleanest field-by-field candidate in the document,
  no other parameters — blocked because one enum payload is a `Duration`.
- `RefillRate::new` and `per_second` — no receiver, so the decision under review does not
  apply to them at all — blocked on `NonZeroU32` and `Duration`.
- `TokenBucket::check_n`, `time_until_ready` — blocked on `NonZeroU32`, and on the generic
  clock parameter, which needs the instantiation feature that is parsed and unused.
- `KeyedRateLimiter`'s seven methods — blocked on two more type parameters and a hasher.
- `refill_and_debit` — a **free function**, which Ply already resolves today, holding five
  of the eleven invariants, deliberately factored out "so the two public types can't drift".
  Blocked on: a generic clock, two `&mut` out-parameters (refused by name, correctly), an
  `f64`, and a struct reference. **The most invariant-dense function in the design is one
  Ply can already find and still cannot check, and `self` is not why.**

**Ceilings, if everything else is also built** — method and associated-function resolution,
generic instantiation, and a type vocabulary reaching `usize`, `f64`, `Duration`,
`NonZeroU32`, `NonZeroUsize`, plus nested constructor chains:

| rule | functions reached (of 40) | invariants reached (of 11) |
|---|---|---|
| constructor-only | ~26 | 3½ |
| public-fields-only | ~1 | 0 |
| the two together | ~27 | 3½ |
| constructor + bounded operation sequence | ~27 | ~9½ |

Field-by-field reaches one function — `Decision::is_allowed`, and only because `Decision` is
an enum with no constructor, which is the one case where field-by-field earns its place and
constructor-only cannot substitute. That is a real reason to keep it. It is not a coverage
argument.

The last row is the point of the whole review. Adding the sequence knob roughly triples the
invariant coverage for the same type-vocabulary investment, and it is the only one of the
four that reaches the two things the designer said out loud he was unsure about — it reaches
invariant 5 directly, and it does not reach invariant 7, which no input-generation strategy
will (that one wants a concurrency-aware checker or a stress test, and saying so plainly is
better than letting it look reachable).

These are estimates read off a design document, not measured runs. **What would settle
them:** write the rate limiter as an actual crate — it is 700 lines of already-written Rust
— put it in the fixtures, and re-measure after each step. That crate is also the honest
regression test for every one of these features, and it exists as prose already.

---

## Sequencing

The readiness document orders the work: methods first, then the neighbour-contamination
bug, then struct/`String`/enum parameters. It argues items 1 and 3 are "the same wall
approached from two sides".

**They are not two sides of one wall. They are two of at least five layers, and the receiver
rule is not the bottom one.** Against the measured document the order that actually moves
the number is:

1. **Make methods and associated functions resolve.** Cheap — it is an `Item::Impl` arm in
   one function and a two-segment path lookup. It changes "not found" (a false statement
   about the user's program, and the one thing this project cannot ship) into "cannot build
   this shape, here is which part" (a true one). It also immediately covers the **8
   receiverless associated functions**, where the whole receiver question does not arise —
   including the two that carry invariant 10. Do this whether or not the receiver rule is
   settled, because it carries no design risk and it is what makes every later measurement
   legible.
2. **Fix the neighbour contamination.** Unchanged from the readiness document's ordering,
   and for its reason: aiming a false alarm at innocent code is the same family of failure
   as a false pass.
3. **Widen the type vocabulary**, aimed at the random-input tier first: `usize`, `f64`,
   `String`, `Duration`, the `NonZero` family, and structs and enums of supported fields.
   This is shared between methods and free functions, and it is what unblocks
   `refill_and_debit` — the highest-value single function in the document — as well as
   nearly every method. Note that the random-input engine has no trouble with any of these
   types; the limit is Ply's own type mapping, not proptest's.
4. **Then the receiver rule**, with the sequence knob built in from the start and defaulted
   to zero, so the honest verdict sentence exists before anyone needs it.
5. **Generic instantiation** is what stands between step 4 and the rate limiter's core.
   Whether it comes before or after step 4 depends on whether the next target is generic;
   this one is, thoroughly.

The one thing I would not do is settle the receiver rule and ship it as the answer to the
"zero of 38" measurement. It will still be zero, the measurement will be re-run, and the
conclusion will be that the fix did not work — when in fact the fix was fine and was aimed
at the third layer of five.

---

## Where I am uncertain, and which way I lean

- **Whether the rate limiter is a fair target.** It is floating-point, generic, concurrent,
  and collection-heavy — close to the worst case. It is fair for the *coverage* question,
  because it was written blind and it is what idiomatic Rust looks like. It is arguably
  unfair to the exhaustive tier, which the spec already says cannot do collections. I lean
  toward: it is the right target for the random-input tier and the wrong one for the
  exhaustive tier, and the receiver work should be scoped and measured against the
  random-input tier first for that reason.
- **How expensive the sequence knob is on the exhaustive tier.** I did not measure it and
  the existing spike does not cover it. I lean toward: it is affordable at length 1–2 and
  falls off a cliff after, the same shape as every other bound in this project. A one-
  afternoon spike — one small stateful type, sequence lengths 0 through 4 — would settle it,
  and should run before the knob is designed rather than after.
- **Whether refusing a private-field type is even necessary on the exhaustive tier**, given
  that its harness is already inside the crate and can name those fields. I lean toward:
  refuse anyway, because the reason to refuse is the invariant risk and not the visibility,
  and a rule that fires differently on the two tiers for a reason unrelated to the risk will
  be impossible to explain in a verdict. But this is the one place I would accept being
  overruled on grounds of coverage.
