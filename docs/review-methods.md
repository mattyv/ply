# Adversarial review: finding methods (commit `c1ea364`)

*Read-only review, 2026-08-27. No product code was written or changed and no workspace
test was run; every claim below was settled by running the already-built
`target/debug/cargo-ply` against six throwaway crates in `/tmp/rev/`, listed with their
commands at the end.*

---

## TLDR

**Ply can now attach a clean, exit-zero passing verdict to a function it never called.**
I built a nine-line crate, ran the shipped binary against it, and got a green
`fuzzed(16)` for a promise that the function it names *violates* — because Ply read the
promise off one function and ran a different one that happens to satisfy it. That is
the false pass this project has spent eight rounds hunting, and it is now reachable
through an ordinary Rust shape. Everything else here is secondary.

Second: **the headline number is not measuring what the summary says it measures.** The
claim is that all thirty-nine promises in the rate-limiter now point at real code and
that none of them get checked only because none carry a promise. I added the promise the
plan says this work unlocks — the one about rejecting a zero refill interval — to a
throwaway copy of that crate and it still does not check, for two separate reasons, one
of them introduced by this very commit. The plan document that says it would check was
not updated and is now wrong.

Third, and in the good direction: **the two defects found before this landed really are
fixed, and I confirmed both by reading the generated code, not by trusting the report.**
The refusals for methods that need a value to be called on, for methods that belong to a
trait, and for methods Ply cannot tell apart, are all real, all reached, and all worded
so a stranger could act on them. The ambiguity guard in particular does the right thing.

---

## 1. The blocker: a passing verdict on a function that was never called

Ply decides *which function a promise is about* by reading one file's `impl` blocks, and
separately decides *which function the test will call* by re-spelling the name the user
wrote. Nothing ties those two decisions together. When they disagree, the only thing that
catches it is the Rust compiler — and the compiler only complains when the wrong spelling
names nothing at all. When the wrong spelling names something real, the run goes green.

Here is the whole crate (`/tmp/rev/wrongfn`):

```rust
// src/lib.rs
pub mod inner;
pub struct Root;                       // one type called Root, at the top level

// src/inner.rs
pub mod sub;
pub struct Root;                       // a different type, also called Root

impl super::Root {                     // this block belongs to the TOP-LEVEL Root
    #[ply::ensures(|result| *result == 999)]
    pub fn five() -> u32 { 5 }         // ... and it breaks its own promise
}

// src/inner/sub.rs
impl super::Root {                     // this block belongs to inner::Root
    pub fn five() -> u32 { 999 }       // no promise on this one at all
}
```

The promise is written `inner::Root::five`. Ply finds the block in `src/inner.rs`, reads
the promise "the answer is 999" off a function that returns 5, and then generates a test
that calls `inner::Root::five` — the *other* one, in `src/inner/sub.rs`, which returns 999
and carries no promise of its own. The generated test reads, verbatim:

```rust
use wrongfn::inner::Root;
...
let __ply_call_result = Root::five();
let result = &__ply_call_result;
... ((* result as i128)) == ((999 as i128)) ...
```

The run:

```
workspace — fuzzed(16)
  wrongfn — fuzzed(16)
    inner::Root::five — fuzzed(16)
EXIT=0
```

A promise that is false of the function it names is reported as evidence that it holds.
To rule out any doubt about which function is which, I compiled and ran the crate
directly: the top-level one returns 5, the inner one returns 999.

Two details make this worse than a curiosity.

**The correct spelling does not resolve at all.** In the same crate, a promise written
`Root::five` — which is what the function is actually called — is reported as "Ply could
not find a function called `Root::five` ... so this claim describes nothing." The only
spelling Ply accepts for that function is the one that makes it run a different function.

**The compiler is the only guard, and it guards by accident.** I hit the same
path-mismatch two more times with shapes that are not exotic at all:

- a type declared at the top level whose `impl` block is written inside a module — Ply
  emits an import for a type that is not in that module, and the run comes back as a tool
  error quoting `unresolved import`;
- a *private* type in a module with a public associated function — Ply emits an import
  the harness is not allowed to write, and the run comes back quoting `struct Secret is
  private`.

Both of those are loud, so no user is misled by them today. They are loud only because
the wrong path happened to name nothing. The first case above shows what happens when it
names something.

The stated rule in the resolver's own comments — a promise names the module the `impl`
block lives in, never the module the type lives in — is what produces all three. That
rule is not written down anywhere a user would see it, is the opposite of how anyone
would spell a method, and is the direct cause of the false pass.

**What I would want before this is trusted:** after resolving, Ply should confirm that
the path it is about to write names the same item it just read, and refuse by name when
it does not. Anything less leaves a class of silent wrong answers whose only backstop is
whether a typo happens to be a typo.

I also note, without having tested it, that a caller's exhaustive proof substitutes
callees by the same computed path. If the path can be wrong for a direct promise, it can
be wrong for a substituted callee, where a wrong answer is quieter still.

## 2. The measurement flatters the work

The summary says resolution went from one of thirty-nine to thirty-nine of thirty-nine,
and that nothing is checked only because the measured crate carries no promises — "a fact
about the measured crate, not a gap in the fix."

That last clause is not true, and it is the load-bearing one. I copied the rate limiter
to `/tmp/rev/rl`, added exactly one promise to exactly the function the planning document
names as the single property this work unlocks, and ran it:

```
quota::RefillRate::new — unsupported
```

Two independent reasons, both of which I isolated:

- **Its answer is `Result<Self, ...>`.** This commit added a new gate that refuses a
  function whose return shape Ply does not model, and a `Result` wrapping `Self` is
  explicitly not modelled. The planning document written the day before says of this very
  function: "The return type is not gated (Ply only needs to build a function's *inputs*;
  the return type only has to exist)." That sentence stopped being true in this commit and
  was not retracted. Constructors returning `Result<Self, E>` are not a corner — they are
  the normal shape for exactly the "reject bad configuration" property that was supposed
  to land.
- **Every module in that crate is private and re-exported at the top.** I swapped the
  return type for a plain integer to isolate the first reason, and got a tool error:
  `module quota is private`. There is no spelling that avoids it — writing the public
  re-exported name gets rewritten back to the private path and fails identically. This one
  is not new and is not method-specific (a free function in a private module fails the
  same way; I checked), but it means the thirty-nine promises could not have produced a
  check even with promises attached and even with every type supported.

So the honest version of the measurement is: **thirty-three promises moved from "I cannot
find this function, go look for a typo" to "I found it, and here is why I will not check
it."** That is a real and worthwhile improvement — a false "not found" is a lie and it is
gone. But "thirty-nine of thirty-nine resolve" is counting *the resolver returned
something other than nothing*, and it invites the reading that the crate is now
thirty-nine promises away from being checked. It is not.

The tool itself does not agree with the number either. `cargo ply check` on the same
document prints:

> anchors — 6 of 39 fn claims in this crate point at a function Ply can find.

Six, not thirty-nine, from the same resolver on the same file. Whichever count is right,
a user reading the tool and a reader of the commit get different answers.

## 3. "A `Self` answer is always fine" is true on one tier and false on the other

The rule added here is that when a function's answer is `Self`, nothing is blocked,
because Ply never has to build an answer — the real call produces it. The reasoning is
correct as far as it goes, and I found no case where an answer genuinely has to be
constructible.

But it is not the only question. Ply never *builds* the answer; it does have to *look at*
it, because that is what the promise says. And where the harness lives differs between the
two tiers: the exhaustive tier writes its harness inside the crate, the sampling tier
writes it as a separate crate. So a promise about a returned value's private innards works
on one and cannot compile on the other.

Demonstrated (`/tmp/rev/selfret`), with the exact shape the shipped fixture uses:

```rust
pub struct Bucket { capacity: u32 }        // private field
impl Bucket {
    #[ply::ensures(|result| result.capacity == cap)]
    pub fn new(cap: u32) -> Self { ... }
}
```

Checked exhaustively, this earns a real verdict. Checked by sampling — one word changed in
the configuration — it comes back as a tool error quoting `field capacity of struct Bucket
is private`. The shipped fixture pairs the private-field promise with the exhaustive tier
only, and pairs the sampling tier only with a promise that says `true`. That pairing is
why the suite is green.

This is the same class as the two defects the pre-landing review caught: a gate that says
"supported" for a shape whose harness cannot be built. It is smaller — it lands as a loud
tool error, not a wrong answer — but it is not fixed, and the new gate is what claims the
support.

## 4. The refusals are true, and each names one blocker out of several

I checked all four receiver forms — borrowed, mutably borrowed, taken by value, and boxed
— and every one produces the same sentence: "Ply found `X::m` but cannot yet build a value
of `T` to call it on." In every case that statement is true: Ply cannot build one, so the
check cannot run, so the refusal is not covering for a different reason. On the rate
limiter the ordering also holds up — the seven that get the receiver sentence really are
blocked by the receiver, and the fourteen blocked by a generic block and twelve blocked by
being part of a trait get their own correct sentences instead.

The gap is that the sentence reads like a single missing capability, and for two shapes
it is not:

- **A method that mutates through `&mut self`.** Building the value is one blocker.
  The other is that Ply has no way to say what such a method is *for* — the change it
  makes to the thing it was called on. That limit is real, is documented elsewhere, and is
  not going away when receivers get built. A reader is told to wait for one thing when two
  are missing.
- **A method with a receiver *and* an argument Ply cannot build.** I wrote
  `fn scale(&self, f: f64) -> f64` on a unit struct; the refusal names only the receiver.
  Ply has no floating-point support at all, so fixing receivers would move this function
  from one refusal to another. Same problem: the sentence implies a shorter queue than
  there is.

Neither is a wrong answer. Both would mislead someone deciding what to build next, which
is precisely what the reachability document exists to inform.

Two smaller wording problems in the same family. A function refused for its answer shape
says only "none of its declared checks apply to this function's shape" — it never names
the type that caused it, so a user cannot tell whether to change the promise, the
signature, or nothing. And a *generic method* inside a non-generic block is not refused at
all: it resolves, generates a harness, and fails to compile with `type annotations needed`.
The same is true of a generic free function, so this is pre-existing — but method
resolution opens a new door into it, and the intended behaviour (refuse by name, and point
at the setting that pins the type) is not what happens.

## 5. `fuzzed(64)` on a function with no inputs overstates what happened

Confirmed against the machine-readable output for a zero-argument function:

```
verdict: fuzzed(16)
evidence: {engine: proptest, seed: fd0221...91ec, cases: 16}
```

Sixteen cases were not run. One case was run sixteen times, and the seed determined
nothing. A reader who knows what a sampling verdict means will read "sixteen samples of
the input space", and will read a bigger number as stronger evidence. Raising it to
`fuzz(1024)` would change nothing at all about what was learned.

The more interesting half: for a function with no inputs and no hidden state, one call is
not weak evidence — it is *complete* evidence. The same function checked on the exhaustive
tier gets a verdict that says so correctly. So the tool currently has the honest verdict
available on one tier and the misleading one on the other, for an identical function.

The catch is that Ply cannot tell whether a zero-argument function is pure. If it reads a
clock or a global, repeated runs really are repeated samples and the count means
something. So I would not promote these to exhaustive automatically. What the verdict must
stop doing is implying breadth it does not have. Something like "this function takes no
input; the one possible call was made" — with the repetition count kept only if it is
carrying real information — is honest under both readings. Whatever the wording, the fix
is to stop letting a number the user chose appear as if it measured coverage.

## 6. Two commands, one fact, two different answers

The same refusal is reported one way by the checking command and another way by the
verifying command:

- verify: a warning, worded "Ply found `Bucket::capacity` but cannot yet build a value of
  `Bucket` to call it on".
- check: an *error*, carrying the same code that means "this claim describes nothing",
  worded "Ply found `Bucket::capacity` and cannot read its shape: Ply found
  `Bucket::capacity` but cannot yet build a value of `Bucket` to call it on."

Three problems in one line. The commit's stated goal — a function Ply found is never again
reported under the code that means "not found" — is met in one command and not the other.
The opening clause is printed twice. And the summary line above it counts these as
functions Ply *cannot find*, which is what produces the six-versus-thirty-nine
disagreement in section 2.

Unrelated but noticed while running: piping either command into `head` makes it panic with
a broken pipe and exit 101.

## 7. What is pinned by nothing

The nine new end-to-end tests cover the shapes the implementation handles. Every one of
them runs against a single-file crate in which every type is public, every `impl` block
sits beside its type, and there are no modules at all — which is the one arrangement in
which the path Ply writes is guaranteed to be the path it read. Not one test would have
gone red on the false pass in section 1.

Still pinned by nothing:

- **A promise whose spelling and whose resolved item disagree.** No fixture has more than
  one module. All three broken-path shapes I found — impl away from its type, private type,
  private module — are invisible to the suite, and so is the wrong-function pass.
- **Any receiver except a borrowed one.** The commit says receivers are refused; the
  fixture contains `&self` and nothing else. Taken by value, mutably borrowed and boxed all
  work today; nothing stops one of them regressing.
- **A promise that inspects a returned `Self` on the sampling tier.** The one fixture
  that returns `Self` on that tier promises `true`.
- **An answer of `Result<Self, E>` or `Option<Self>`.** The narrowing is written down in a
  comment; no test holds it, so nobody finds out when it silently changes.
- **A generic method in a non-generic block.** Resolves, breaks, untested.
- **The meaning of the case count on a zero-input function.** The test asserts a real
  verdict is earned; nothing asserts the number attached to it means anything.
- **Ambiguity across files.** The guard that refuses two same-named methods is scoped to
  one file, deliberately. Nothing records what happens when the two candidates are in
  different files — which is exactly the shape section 1 exploits.

Per this project's own rule that a defect found in review enters the suite as a fixture of
its own shape: the missing shape here is *a crate with more than one module*. Every one of
the findings above, including the blocker, needs the same fixture.

## 8. What is sound

Said plainly, because most of this work is:

- The two defects caught before landing are genuinely fixed, and I verified the generated
  code rather than the report: the sampling harness now imports the type and calls
  `Type::method(...)`, and a function with no arguments now gets a real strategy instead of
  a bare value. Both produce real verdicts.
- The ambiguity guard is right, and it is the part of this work I would defend hardest. Two
  blocks defining the same method name are refused rather than guessed, and the refusal
  explains itself in terms a stranger could act on. That is the correct instinct applied at
  exactly the point where guessing would be worst.
- Refusing methods that belong to a trait, that live in a generic block, and that need a
  value to be called on is right for this slice, and each refusal names the function and
  the reason.
- An inherent method correctly wins over a trait method of the same name, matching Rust's
  own rule. A free function and a method sharing a name do not collide. A block written
  inside a function body is correctly not found. A method inside an inline module resolves
  and checks. A block written for one concrete instantiation of a generic type resolves and
  checks. All confirmed by running.
- Nothing green came out of a run that gathered no evidence. Every refusal, every tool
  error and every unsupported shape I produced exited non-zero, and the rate-limiter run
  with thirty-three refusals ends on a non-zero exit with a top-level verdict that says
  nothing was established. The absence-of-evidence rule held everywhere except the one
  place in section 1, where Ply believed it *had* evidence.

---

## Reproductions

All under `/tmp/rev/`, all using `/home/user/ply/target/debug/cargo-ply` as built at the
time of review (confirmed to contain this commit's code). Each scratch crate depends on
`ply-attrs` by absolute path.

| # | what | command | result |
|---|---|---|---|
| 1 | wrong function checked, reported green | `cd /tmp/rev/wrongfn && cargo-ply ply verify .` | `fuzzed(16)`, exit 0 |
| 1b | which function is which | `cd /tmp/rev/wrongfn && cargo run --example which` | top-level 5, inner 999 |
| 1c | the correct spelling is not found | `cd /tmp/rev/wrongfn && cargo-ply ply check .` (both spellings claimed) | `Root::five` "describes nothing" |
| 2 | the property the plan says lands | `/tmp/rev/rl`, one promise added to `RefillRate::new` | `unsupported` |
| 2b | isolating the private-module half | same, return type swapped for an integer | tool error, `module quota is private` |
| 2c | the same wall for a free function | `/tmp/rev/privmod` | tool error, both claims |
| 3 | `Self` answer, private field, sampling tier | `/tmp/rev/selfret` | tool error, `field capacity ... is private` |
| 3b | same shape, exhaustive tier | shipped `implmethod` fixture | real verdict |
| 4 | all four receiver forms; receiver plus unbuildable argument | `/tmp/rev/shapes`, `/tmp/rev/attack` | one refusal sentence each |
| 5 | zero-input sampling verdict | `/tmp/rev/selfret`, machine-readable output | `cases: 16`, one real case |
| 6 | the thirty-nine | `/tmp/rev/rl` with the shipped measuring document | 33 refused, 6 with no promise, 0 checked, exit non-zero |
| 7 | inherent beats trait; inline module; one concrete instantiation | `/tmp/rev/attack` | all correct |
