# Adversarial review: are these the right caveats? (branch head `9141007`)

*Read-only review, 2026-08-27. No product code was written or changed; `cargo test
--workspace` was never run. Every claim below was settled by running the already-built
`target/debug/cargo-ply` (confirmed to contain the branch's last commit) against
seventeen throwaway crates in `/tmp/rev4/`, listed with their commands at the end.*

---

## TLDR

**He should not start yet, and the thing to wait for is not on the list: today Ply will
not run at all on an ordinary Rust crate.** A crate made by `cargo new --lib` — no
workspace table in its manifest — makes `cargo ply verify` exit with a raw error and a
twenty-line stack trace before checking anything. A crate that is a member of a
multi-crate project fails the same way, and the only workaround (adding a workspace table
to that crate) breaks `cargo build` for the whole project, so there is no way to run Ply
on a multi-crate repository at all. All fifty-seven crates in the test suite carry the
one manifest line that avoids this, which is why nothing caught it.

**Two more things outrank every caveat on the list, and both are wrong answers rather than
refusals.** Ply now builds a value to call a method on — and it ignores the rules the type
itself declares about how that value may be built. I wrote a getter whose promise is
"the answer is at least zero", on an unsigned integer, so it cannot be false; Ply reported
it as breaking its own promise, because it built the object by calling a constructor with
an argument the constructor's own contract forbids. Separately, a promise that is false
after two perfectly ordinary calls gets a clean green pass, because the "up to three
earlier operations" Ply says it ran can never include an operation that changes anything
for a normal Rust type — and the sentence printed beside the verdict tells the reader the
opposite.

**The refusal rate is not the story people think it is, and it is worse than the caveat
says.** On the fairest sample available — the rate limiter written by someone told not to
think about checkability — I put a promise on every public function and asked for a real
check on all thirty-nine. **Zero got a verdict.** Not "a lot of cannot-check": all of it.

**Ranked by how much they would actually hurt** (the three new ones first, because each
outranks everything on the list):

| # | | Why it ranks here |
|---|---|---|
| **N1** | Will not run on a normal crate layout | It is the first thing that happens, it looks like the tool is broken, and for a multi-crate repo there is no workaround that keeps the build working |
| **N2** | Reports correct code as broken | A violation on a promise that cannot be false; the fastest possible way to lose a user's trust |
| **N3** | Green pass on a promise that is false after two calls | A false clean, reachable through ordinary `&mut self` code, dressed in a disclosure that says the opposite |
| 2 | `check` and `verify` disagree | Worse than described: `check` states two capabilities as absent that now work, and runs clean on the very crate where `verify` crashes. A user who runs the cheap command first stops there |
| 3 | Receiver built from constructor plus 3 operations | The number is not the limit that bites — see N3. Stated as a depth bound, it is really "no history at all" for most types |
| 1 | Structs, enums, generics refused | This is the zero-of-thirty-nine result. Honest, loud, and the reason the tool may not be worth running yet on real code |
| 5 | Constructors returning a result are unrecognised | Small in count (one of nine constructors in the sample) and the refusal still names the wrong obstacle |
| 4 | Text and float exclusions | Genuinely fine. Both disclosures fire, both were measured; this is the best-behaved item on the list |
| 6 | Unsigned commits | Not a user-facing caveat at all; a repo hygiene item that does not belong in the same list |

**Also missing from the list, below the three above but above most of it:** the standard
Rust library layout (private modules with public re-exports) blocks everything, with a
message that reads like a Ply bug; declaring two checks on one function silently drops the
one that ran nothing, undoing this morning's fix; worked examples cannot be written for a
method at all, and the failure blames the user's file; and `cargo clean` still leaves the
user's crate unable to build.

**And the good news, because there is a lot of it:** I tried six shapes nobody had — a
method taking text, a float on a built receiver, a claim two modules deep, two checks
declared together, a list argument, and a before-and-after contract — and every single
false promise was caught. Contradictory and very narrow preconditions are reported as "no
evidence", never as a pass. Changing a private helper correctly invalidates the stored
result of the function that calls it. The replayable test written into the user's source
now compiles and fails for the right reason. And Ply's own receiver fixture earns its
keep: the bug it finds is real, and the failing sequence it reports stays inside the
function's own contract.

---

## The three that are not on the list

### N1. It does not run on an ordinary crate

The whole crate is what `cargo new --lib` gives you, plus one promise:

```toml
[package]
name = "plain"
version = "0.1.0"
edition = "2021"
```

```
$ cargo-ply ply verify .
Error: ./Cargo.toml has no `[workspace]` table to add the harness crate to

Stack backtrace:
   0: anyhow::error::<impl anyhow::Error>::msg
   1: ply_core::harness_crate::ensure_workspace_member
   ...  (twenty lines)
```

No plain sentence, no code, no suggested fix, no mention anywhere in the specification or
the docs, and a stack trace pointing into Ply's own source. The fix is to add the two
words `[workspace]` to the manifest — after which the same crate checks fine and finds the
planted bug — but nothing on screen tells the user that, and nothing tells them it is
safe.

For a multi-crate project it is not a nuisance, it is a wall. I built the most ordinary
two-crate workspace there is:

- run it from the repository root: "Ply could not find the function this claim anchors
  to ... could not find it in `./src/lib.rs`" — the root has no source of its own, and
  there is no way to tell Ply where a component's crate lives.
- run it from inside the member crate: the same stack trace as above.
- add the workspace line to the member so Ply will run: Ply then works, and
  `cargo build` at the root stops working — "multiple workspace roots found in the same
  workspace".

So today a user with more than one crate must choose between running Ply and having their
project build. That is not a caveat to live with; it is the reason the answer should be
"wait".

Why nothing caught it: **all fifty-seven fixture crates in the suite carry that manifest
line**, a convention adopted so fixtures never get swept into the product workspace. The
suite has never once seen the layout every real user has.

### N2. It reports correct code as broken

Two independent ways, both introduced by receiver construction, both producing a red run
on code that is right.

**It ignores the constructor's own precondition.** The type says out loud how it may be
built, in the notation Ply itself defines, and enforces it:

```rust
impl Gauge {
    #[ply::requires(n > 0)]
    pub fn new(n: u32) -> Self { assert!(n > 0, "a Gauge reading is never zero"); Gauge { n } }

    #[ply::ensures(|result| *result >= 0)]     // an unsigned integer: cannot be false
    pub fn value(&self) -> u32 { self.n }
}
```

```
Gauge::value — violation
`Gauge::value` fails its own contract `|result|*result >= 0` for at least one input
```

The recorded failing input is `0` — Ply called `Gauge::new(0)`, which the constructor's
own contract forbids and whose assertion fires. The promise Ply says was broken is one no
unsigned integer can break. Any type whose constructor validates its argument — a
percentage, a non-empty name, a capacity that must be positive — will do this, and it is
the single most common shape in configuration code.

**It ignores the checked method's own precondition on the calls it makes before the real
one.** Reading the generated harness confirms the mechanism: the precondition is applied
as a filter to the final call's arguments only, and the earlier calls in the sequence draw
their arguments unfiltered.

```rust
#[ply::requires(k <= 10)]
#[ply::ensures(|result| *result <= 10)]
pub fn set(&self, k: u32) -> u32 { assert!(k <= 10, "…which its contract forbids"); … }
```

Reported as a violation, with the failing sequence `set(11)` — a call the function's
contract forbids and Ply made anyway.

**And in this path every crash is described as a broken promise.** For a plain function
the wording is exemplary: "does not return at all for this input — it panicked before its
postcondition could even be evaluated ... so this is a violation, with a witness." The
identical function as a method gets "fails its own contract for at least one input"
instead. So a user is told the wrong thing about what happened *and* about which function
did it, and the witness they are handed (`(), [(0,11)], 0`) is not something they can run.

Note what this costs beyond the wrong answer: Ply's own honesty sentence beside these runs
says "every value this run saw was reachable by calling the type's own code, nothing else,
so nothing here was assumed". In both cases above that sentence is false — the values came
from calls the type's own contracts forbid.

### N3. The twelfth false clean: a green pass on a promise that fails after two calls

```rust
pub struct Acc { n: u32 }
impl Acc {
    pub fn new() -> Self { Acc { n: 0 } }
    pub fn add(&mut self, k: u32) -> u32 { … }          // the ordinary way to change state
    #[ply::ensures(|result| *result < 5)]                // FALSE
    pub fn get(&self) -> u32 { self.n }
}
```

```
Acc::get — fuzzed(256)
EXIT=0
```

and the promise is false in three lines of ordinary use — I ran it: `a.add(3); a.add(3);
a.get()` is 6.

The reason is in the code and confirmed in the generated harness: the pool of "the type's
own operations" Ply may call before the checked one is restricted to methods that borrow
the value without changing it, *and* to methods whose parameters have exactly the same
shape as the checked method's. A normal Rust type changes state through `&mut self`, so
nothing in the pool can change anything; the "sequence of up to three operations" is three
repetitions of a read. Every check runs against a freshly constructed value.

The second flavour needs no `&mut` at all — only a mutating operation with a different
argument list, which is the norm:

```rust
pub fn spend(&self, amount: u32) -> u32 { … }   // changes state, takes an argument
#[ply::ensures(|result| *result == 10)]          // FALSE after any spend
pub fn remaining(&self) -> u32 { … }             // takes none, so `spend` is not pooled
```

`fuzzed(256)`, exit 0. I ran the real thing: after one `spend(4)`, `remaining()` is 6.

What makes this a caveat problem and not just a bound: the sentence printed beside the
green verdict says *"this run only covers receivers reached in at most 3 such calls from a
freshly built one — a bug that only shows up on the 5th call is outside what this run
checked"*. A reader concludes that three-deep histories were explored. Zero were. The
disclosure names the pool ("repeating `Acc::get` itself") one clause earlier, so the
information is technically on screen — but the sentence written to be the honest one is
the misleading one.

Caveat 3 as stated ("a bug needing 4 prior calls is missed") should read: *for any type
that changes state the normal way, no earlier call that changes anything is ever made, and
the result is a green verdict rather than a refusal.* That is a different caveat, and it
belongs above the refusals, not below them.

---

## The refusal rate: zero of thirty-nine

The rate limiter is the fairest sample in the repository — a library written by someone
told not to think about checkability. I copied it, added a promise to every public
function and asked for a real check on all thirty-nine claims. Every one of them came back
without evidence:

| what happened | claims |
|---|---|
| declared inside a generic block | 14 |
| a trait method, or a method of a trait implementation | 10 |
| its module is private, so the generated harness cannot name it (7 refused outright, 4 as a harness that failed to compile) | 11 |
| no constructor Ply can call | 3 |
| a parameter whose type Ply cannot build | 1 |
| **checked** | **0** |

Two things follow that the six caveats do not say.

**The cheap command reports this as 82% coverage.** `cargo ply check` on the same document
prints "32 of 39 fn claims in this crate point at a function Ply can find". A user reads
that as good news, runs the real command, and gets nothing. "Ply can find it" and "Ply can
check it" are different facts and only the first has a number.

**The private-module wall is not on the caveat list and it is bigger than the type gaps.**
The standard Rust library layout — `mod thing;` with `pub use thing::Thing;` — makes every
claim in the crate fail, whatever spelling the user writes:

```
`math::double`'s `fuzz(64)` check ran zero cases: the test harness Ply generates for it
failed to compile … The compiler's own first error was: error[E0603]: module `math` is private.
```

Changing that one word to `pub mod math;` makes the identical crate check, and both
planted bugs are found. So the fix is one keyword, but the message never says so, and what
the user sees is Ply reporting that Ply's own generated code does not compile — which
reads as a broken tool, not an unsupported layout. Eleven of the thirty-nine claims above
are this.

Is the refusal rate the real story? Partly. The refusals are honest, they name the
function and the reason, and none of them pretends to be evidence. But a first day that
produces zero verdicts on a real crate is not a day anybody repeats, and the two loudest
causes — generic blocks and trait methods — are not on the caveat list either, though
caveat 1 gestures at them.

---

## Two more that a first day would hit

### Declaring two checks silently drops the one that ran nothing

This morning's fix makes a check that executed no cases a tool error rather than a pass. It
counts cases per *function*, not per *check*, so putting two checks on one function
restores the old behaviour:

```yaml
Gauge::value:
  checks: [test, fuzz(64)]
```

The worked-examples check runs zero cases here (that is exactly the shape the fix was
written for — on its own it correctly errors); the sampling check runs and passes; the
result is `fuzzed(64)`, exit 0, no diagnostic, and the machine-readable output carries an
empty status list. A declared check gathered nothing and the run says everything is fine.
Ply's own shipped fixture for before-and-after contracts declares its checks exactly this
way, so the shape is not exotic — it is the recommended one.

### Worked examples cannot be written for a method, and the failure blames the user

```yaml
examples:
  - "Gauge::new(5).value() == 999"
```

```
`Gauge::value`'s `fuzz(64)` check ran zero cases: the test harness Ply generates for it
failed to compile … The compiler's own first error was: error: invalid path separator in
function definition. `Gauge::value` declares `examples:` entries in ply.yaml, which compile
exactly as written … so a wrong type or a typo there is one thing worth checking first.
```

The example is valid Rust. What is invalid is the name Ply derives from it for its own
generated test function. The message sends the user to look for a typo in their own file,
and the broken harness also takes down the sampling check on the same function, which
would otherwise have worked.

### `cargo clean` still breaks the user's crate

Reported in the previous review, unchanged. Ply permanently adds a workspace member
pointing inside `target/`; `cargo clean` deletes it, and then `cargo build`, `cargo test`
and `cargo metadata` all fail with an error naming a path the user never wrote. Running
Ply again repairs it — which nobody would guess. Worth a line in the caveats since it
belongs to the same family as the blocker he just fixed (Ply editing the user's crate).

While there: on a violation Ply modifies the user's `Cargo.toml`, appends a module line to
their `lib.rs` and writes a new file into their `src/`, with no prompt. The replay test it
writes now compiles and fails for the right reason — I confirmed by running the user's own
`cargo test` — so this is by design and it works. It is still four uncommitted changes to
a source tree from a command the user may have thought was read-only.

---

## Re-ranking the six, with what I would change about each

**Caveat 2 (`check` and `verify` disagree) is the worst of the six, not the middle.** It is
not one disagreement, it is three, and they all push the user away from the command that
works:

- On the crate where `verify` had just found two real bugs, `check` says of both methods
  that Ply "cannot yet build a value ... to call it on — constructing a receiver is not
  supported yet", and adds that a `String` parameter "is a shape Ply's checkers do not
  build inputs for either". Both statements are now false. A user reading them concludes
  that methods and text-handling functions are out of reach and stops writing promises for
  them.
- On the rate limiter, `check` reports 32 of 39 found; `verify` checks 0.
- On a workspace member, `check` runs clean and says "1 of 1 claims point at a function Ply
  can find. No problems found in the document" — and `verify` on the same directory dies
  with a stack trace.

Same stale sentence also appears inside `verify` itself, for any method more than one
module deep: "constructing a receiver is not supported yet", when the real limit is module
depth and receivers work fine one level up.

**Caveat 3 (the bound of three) is mis-stated rather than merely optimistic** — see N3.
Restate it as "no earlier call that changes the object is ever made unless the type
mutates through a shared borrow and the mutating method takes exactly the same arguments
as the checked one".

**Caveat 1 (structs, enums, generics) is the refusal-rate caveat**, and the number to
quote is zero of thirty-nine, not "a lot of cannot-check". Add generic blocks, trait
methods and private modules to it — in the fair sample those three account for 35 of the
39, and the type gaps for 1.

**Caveat 5 (constructors returning a result)** is small in count. Its remaining problem is
the wording, unchanged from the previous review: the refusal says the type "has no
associated function ... that builds a value and takes only types Ply's checkers already
know how to build", when the constructor exists and takes a plain integer, and the actual
obstacle — the result wrapper — is never mentioned.

**Caveat 4 (text and floats)** is fine and should stay last of the real ones. Both
disclosures fire, in full, on methods as well as plain functions; I saw them both on a
run that also found real bugs.

**Caveat 6 (unsigned commits)** is not a caveat a user lives with. It belongs in the
running-state list, not in the same sentence as things that change what a verdict means.

---

## Where I went looking and found nothing wrong

Said plainly, because it is a lot, and because a passing check proves nothing unless the
promise was false — every item here was a promise written to be false, or a measurement.

- **Six shapes nobody had tried, all caught.** A method taking text; a float parameter on a
  method whose object Ply builds; a claim on a function two modules deep; two checks
  declared together; a list argument; a before-and-after contract on both a plain function
  and a method. Every false promise was found, every disclosure that should have fired
  did, and the run's exit code was right each time.
- **Impossible and very narrow preconditions are honest.** A precondition nothing can
  satisfy gets "no fuzz evidence at all — its verdict is not a pass", with the counts. A
  precondition satisfied by one value in four billion still found the planted bug. A
  precondition that throws away most draws says so and says the evidence is weaker than
  the number suggests.
- **Stored results are invalidated when they should be.** I broke a private helper that a
  passing function calls, in a crate with no methods at all, and the caller was re-run and
  the new bug found. The same held with methods present, by the coarser rule that re-runs
  everything.
- **The replayable test works.** It compiles inside the user's crate, fails under their own
  `cargo test`, and its message states the promise and both sides of the comparison.
- **Ply's own receiver fixture earns its keep.** Its bug is real and the failing sequence it
  reports — one earlier call with 7, then the checked call with 4 — stays inside the
  function's declared precondition. That finding is not an artifact of the precondition gap
  in N2.
- **A crash in a plain function is reported perfectly**, with a witness and wording a
  stranger could act on. It is only the method path that describes a crash as a broken
  promise.
- **Nothing green came out of a run that gathered no evidence**, except the two cases named
  in N3 and the two-checks case — and in both of those Ply believed it had evidence.

---

## Reproductions

All under `/tmp/rev4/`, all against `/home/user/ply/target/debug/cargo-ply` as built at
the branch head. Each scratch crate depends on the attribute crate by absolute path.

| # | what | where | result |
|---|---|---|---|
| N1 | ordinary crate, no workspace line | `/tmp/rev4/plain` | stack trace, nothing checked |
| N1b | same crate, after adding `[workspace]` | `/tmp/rev4/plain` | checks fine, finds the planted bug |
| N1c | two-crate workspace, run at the root | `/tmp/rev4/ws` | claim not found (`./src/lib.rs`) |
| N1d | same, run inside the member | `/tmp/rev4/ws/crates/alpha` | same stack trace |
| N1e | member given its own workspace line | `/tmp/rev4/ws` | Ply works; `cargo build` at root now fails |
| N1f | how many fixtures carry that line | `tests/fixtures` | 57 of 57 |
| N2 | constructor precondition ignored | `/tmp/rev4/ctorpre` | violation on a promise that cannot be false |
| N2b | same, on a real promise | `/tmp/rev4/mixed` | violation, witness `0, [], ()` |
| N2c | earlier calls ignore the method's own precondition | `/tmp/rev4/seqarg` | violation, witness `set(11)` under `requires(k <= 10)` |
| N2d | crash wording, plain function vs method | `/tmp/rev4/panics` | correct / "fails its own contract" |
| N3 | ordinary `&mut self` type | `/tmp/rev4/mutrecv` | `fuzzed(256)`, exit 0; real value is 6 against a promise of "< 5" |
| N3b | mutating operation with a different argument list | `/tmp/rev4/poolgap` | `fuzzed(256)`, exit 0; real value is 6 against a promise of "== 10" |
| N4 | private module with public re-export | `/tmp/rev4/facade` | every claim fails; `pub mod` fixes it |
| N5 | two checks declared together | `/tmp/rev4/twocheck` | `fuzzed(64)`, exit 0, no status for the check that ran nothing |
| N6 | worked example on a method | `/tmp/rev4/exmethod` | harness will not compile; message blames the user's file |
| N7 | `cargo clean` after a run | `/tmp/rev4/footprint` | build, test and metadata all fail |
| R | refusal rate with promises and checks on everything | `/tmp/rev4/rl` | 0 of 39 checked |
| R2 | the cheap command on the same crate | `/tmp/rev4/rl` | "32 of 39 … point at a function Ply can find" |
| C2 | the cheap command where the real one found bugs | `/tmp/rev4/shapes1` | "receiver … not supported yet", "String … not built" |
| C2b | the cheap command on a workspace member | `/tmp/rev4/ws/crates/alpha` | "No problems found in the document" |
| S | six new shapes, every false promise | `/tmp/rev4/shapes1` | all six caught |
| S2 | impossible and narrow preconditions | `/tmp/rev4/vacuous` | no evidence / bug found |
| S3 | stored results after breaking a helper | `/tmp/rev4/fine`, `/tmp/rev4/stale` | re-run, bug found |
| S4 | replay test under the user's own test run | `/tmp/rev4/footprint` | compiles, fails for the right reason |
| S5 | Ply's own receiver fixture, witness inspected | `/tmp/rev4/seqfix` | sequence stays inside the contract |
| S6 | before-and-after contract on a method | `/tmp/rev4/oldm` | true one passes, false one caught |
| S7 | the stated caveats, confirmed | `/tmp/rev4/stated` | structs, enums, generics, result-returning constructor, deep module all refused by name |
