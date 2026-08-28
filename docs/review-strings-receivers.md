# Adversarial review: strings, floats, receivers (everything since `c1ea364`)

*Read-only review, 2026-08-27. No product code was written or changed; `cargo test
--workspace` was never run. Every claim below was settled by running the already-built
`target/debug/cargo-ply` against throwaway crates in `/tmp/rev3/`, listed with their
commands at the end. Two claims are marked **read, not run** where I could not run them
without rebuilding the tool.*

---

## TLDR

**There is an eleventh, and it is the same shape as the tenth: a check that runs nothing
and reports it held.** Declare the worked-examples check on a method that needs a value
to be called on, give it a promise that is false, and Ply prints a green pass and exits
zero — because for that shape it generates no test at all, the filter matches nothing,
and "no failing test" is read as success. The same false promise on the same function is
caught immediately by the sampling check. The nothing is then written into the committed
record and reused on later runs, with no diagnostic of any kind.

**Second, in the other direction: a correct function is reported as breaking its own
promise.** A crate with a top-level `parse` and a `util::parse` — nothing exotic — makes
Ply blame the correct one for the broken one's failing tests, and the sentence it prints
contradicts itself in the same line ("failed 2 of its own tests: `util_parse_...`"). This
is a side effect of the fix for the tenth.

**Third, and this is the good news, stated plainly because most of the work is good:**
the ninth false clean is genuinely gone — I rebuilt the two-same-named-types crate that
produced it and Ply now refuses it, and the spelling that used to be unfindable now
resolves and catches the real violation. The receiver bound is exactly and honestly
three, measured: a bug that needs three earlier calls is found, a bug that needs four is
not, and the sentence printed beside the verdict says precisely that. The float and
string exclusions really do hold. A proof asked for on a sample-only type is refused by
name and never launders into a weaker result under a stronger word.

---

## Ranked by whether a user could be misled

### 1. BLOCKER — the eleventh false clean: the worked-examples check on a method runs nothing and reports it held

The whole crate:

```rust
pub struct Bucket { cap: u32 }

impl Bucket {
    pub fn new(cap: u32) -> Self { Bucket { cap } }

    #[ply::ensures(|result| *result == 42)]     // FALSE: it returns cap
    pub fn capacity(&self) -> u32 { self.cap }
}
```

with `checks: [test]`. The run:

```
workspace — tested
  testrecv — tested
    Bucket::capacity — tested
EXIT=0
```

No diagnostics. Nothing else printed. Asked directly, the generated harness says what
really happened:

```
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Change one word in the document — `test` to `fuzz(64)` — and the identical crate reports
a violation. So the promise is false, Ply can catch it, and the worked-examples check
says it holds.

**Why.** For a method that needs a value to be called on, Ply deliberately generates no
concrete-input tests (it has no fixed receiver to write, only the randomly built one the
sampling tier makes). With no worked examples declared either, the function contributes
no tests to the shared harness at all — so no test module is written for it, and the
filter that selects "this function's tests" matches zero of them. `cargo test` exits
zero, and the code reads "did not fail" as "held".

This is structurally the tenth bug, not a new one. The tenth was fixed by correcting the
filter *string*; nothing was added to confirm that the filter actually selected anything.
The success path still has no positive evidence that a single case ran. The
"harness never ran" guard only fires when the run *failed*, which is precisely the case
this shape avoids.

**The same mechanism, one step milder:** a function with no promise and no worked
examples, declared with the worked-examples check, also reports `tested`. Nothing ran
there either. The sampling check refuses that shape by name ("no postcondition to check
against"); the worked-examples check gives it a green verdict.

**And it persists.** The committed record holds the entry as `tested`, with no evidence
block at all, and later runs mark it reused and print nothing. A reviewer reading that
diff sees a green entry for a promise that is false.

### 2. BLOCKER-adjacent — a correct function reported as breaking its own promise

```rust
pub mod util {
    #[ply::ensures(|result| *result == 0)]   // FALSE
    pub fn parse(x: u32) -> u32 { x }
}

#[ply::ensures(|result| *result >= x)]       // TRUE — this function is correct
pub fn parse(x: u32) -> u32 { x }
```

Both declared with the worked-examples check:

```
parse — violation
util::parse — violation
```

and the diagnostic on the correct one reads:

> `parse` failed 2 of its own example/generated direct-contract test(s):
> `util_parse_harness::ply_direct_util_parse_01`, `util_parse_harness::ply_direct_util_parse_02`.
> Each of these is a concrete input asserted directly against the contract, so this is a
> real, reproduced violation, not a probabilistic one.

It names another function's tests as "its own", in the same sentence, and calls the
result "a real, reproduced violation".

**Why.** The per-function filter is a plain substring, and a module-nested function's
generated name (`util_parse_harness::`) contains a top-level function's filter
(`parse_harness::`) as a substring. So the run for `parse` executes both functions' tests.
The sampling half is immune — it compares the failing test name for exact equality — but
the worked-examples half only asks "does any failed test name look like an example or a
concrete-input test", with no check that it belongs to this function's own module.

Reachable names are ordinary: `parse` / `util::parse`, `add` / `readd`, `run` / `dry_run`,
any method whose flattened name ends in another claim's flattened name. I confirmed two
of these by running.

The mutation check shares the same filter and additionally matches function names
unanchored, so its scoping has the same weakness in both directions. I did not run it
(the mutation engine is a separate install).

### 3. The "reproduce it yourself" test written for a method does not compile, and it breaks the user's own test run

When a method breaks its promise, Ply writes a replayable test into the user's `src/`
directory and adds a module line to their `lib.rs`. For any method that needs a value to
be called on, that test calls the method with no such value:

```rust
let result = &K::bump(x);      // K::bump takes &self and x
```

The user's own `cargo test` then stops compiling:

```
error[E0061]: this function takes 2 arguments but 1 argument was supplied
error: could not compile `cexrecv` (lib test) due to 1 previous error
```

Two separate problems. The tool's own rule is that a rendered replay test must fail under
`cargo test` and pass once the bug is fixed; this one cannot even build, and nothing
detects that. And Ply has edited the user's source tree in a way that breaks their test
suite — a developer who runs the checker and then runs their own tests sees a compile
error pointing at a file they never wrote.

It is also unreproducible in principle for the interesting cases: the failure often
depends on the earlier calls Ply made to build the receiver, and the written test contains
none of them. The recorded failing input names only the final call's argument.

Ply already knows how to refuse this — a method with an argument it cannot write back out
as source correctly declines to render a test and says so in plain words. That refusal is
attached only to the fallback path taken when the body *panics*; a plain broken promise
takes the other path, where no receiver check exists.

### 4. Every run accuses the user of hand-editing the record, for an ordinary no-argument function

A function with no parameters, checked by sampling, correctly earns the concrete-case
verdict (this was a fix in this range, and it is the right one). But the record layer's
own sanity rule does not know about that special case, so on the *second* and every
subsequent run:

> The recorded result for `zeroparam::seven` says `tested`, and the checks recorded beside
> it (fuzz(64)) cannot produce that answer. **A result file Ply wrote never contains this,
> so something else edited it — a merge that went wrong, or a hand edit.** Ply ignored the
> stored result and ran the checks again.

Ply wrote it. Every time. On a correct, ordinary function. This is the tool's tamper alarm
crying wolf at itself, forever, and it also permanently defeats reuse for that claim.

This is *known* — it is recorded as the first of three product bugs in the notes attached
to the abandoned caching work — but it is not in the running-state list and it is not
fixed. It is the same class of mistake the record's own design notes describe having
learned "the hard way" once already.

### 5. A proof refused on a method blames a type that is not the problem

`checks: [bounded(2)]` on `B::cap(&self) -> u32`:

> Ply did not run `bounded(2)` on `B::cap`: **its return type `u32`** can be checked with
> random sample values just fine … but `bounded(2)` needs to reason about *every* possible
> value at once, and **this type is real, substantial work for that**.

`u32` is the cheapest type the prover handles. The actual reason is the receiver, which
the proving tier does not build. Two features written the same day cross here: the
per-engine split assumes anything the prover refuses was refused for a *type* reason, and
the receiver work added a non-type reason to refuse. When no parameter and no return type
is at fault, the message falls back to naming the return type anyway.

A reader is told their `u32` is too hard to prove. That is false, it hides the real
blocker, and the previous review specifically praised this tool for naming blockers
correctly.

### 6. The two commands disagree again, and the wrong one is the discouraging one

On the crate where `verify` just built a receiver and found a real bug, `check` says:

> Ply found `K::bump` but cannot yet build a value of `K` to call it on — **constructing a
> receiver is not supported yet.**

That statement was true four commits ago and is false now. The checking command was
changed earlier in this range specifically so the two commands would stop disagreeing
about methods; the receiver work then made them disagree again, and in the direction that
costs the most: a user who runs the cheap command first is told their method cannot be
checked, and never runs the expensive one that would have found their bug.

### 7. Strings: the exclusion is real, and it is never disclosed

Measured, and both halves behave as designed: a promise that generated strings contain no
control characters holds over 256 cases (so control characters really are excluded), and a
promise that generated strings are ASCII is violated (so multi-byte content really is
generated). Length is bounded at 32 characters.

But the float exclusion prints a paragraph on every float run explaining what was left out
and that the run says nothing about it; the string exclusion prints nothing at all. A
function that sanitises log lines or parses CSV gets a clean sampling verdict with no hint
that the entire control-character class was never generated. The mechanism to say so
exists in the code and is marked "not yet wired"; the honesty half of the feature did not
ship with the feature.

The specification is worse off than the code: it gained per-type entries for floats,
pointer-width integers, non-zero integers and durations in this range, and gained nothing
at all for `String` — not the support, not the 32-character bound, not the exclusion.

### 8. A constructor returning a result is refused for a reason that is not the reason

```rust
pub fn new(cap: u32) -> Result<Self, BadCfg> { ... }
```

> Ply cannot build a receiver for `Limiter`: it has no associated function in the file it
> is declared in that builds a `Limiter` value **and takes only types Ply's checkers
> already know how to build** — constructing a receiver needs a constructor to call, and
> none was found.

`Limiter::new(cap: u32)` builds a `Limiter` and takes only a `u32`. The sentence sends the
reader hunting through parameter types; the actual obstacle is the result wrapper, which
is never mentioned, and neither is the workaround (add an infallible constructor). Compare
the sibling refusal for a constructor taking a user-defined type, which names the exact
blocker and is a model of how this should read.

### 9 & 10. Two pre-existing hazards, not introduced here, not previously reported

- **After `cargo clean`, the user's crate no longer builds.** Ply permanently adds a
  workspace member pointing inside `target/`. `cargo clean` deletes it, and then
  `cargo build`, `cargo test` and `cargo metadata` all fail with a manifest error naming a
  path the user never wrote. Ply's own checking command reports this as an unrelated
  architecture failure. Dates from the sampling tier's original commit.
- **The checking command still exits 101 on a broken pipe** (`… | head`). Reported in the
  previous review, unfixed. That exit code is in no table anywhere.

---

## Where I went looking and found nothing wrong

Said plainly, because this is most of the work.

**The ninth false clean is really fixed.** I rebuilt the exact crate that produced it —
two types both called `Root`, one's methods written from inside the other's module — and
the promise that used to read a contract off one function and run a different one is now
refused, because the function it truly names carries no promise. The spelling that was
previously unfindable now resolves and catches the real violation. I ran both.

**The receiver bound is exactly three, and the sentence beside it is exactly true.** I
wrote a counter whose promise breaks on the fourth call — three earlier calls, the stated
bound — and it was found. I changed one digit so it breaks on the fifth call, and it was
not found, and the verdict came with the sentence "a bug that only shows up on the 5th
call is outside what this run checked". That is a bound that is genuinely reachable,
genuinely capped, and honestly reported. It is the best-calibrated disclosure in the tool.

**The generated receivers are genuinely reachable.** Every value comes from calling the
type's own constructor and its own read-only operations, arguments generated fresh each
step; nothing is assembled by writing fields directly. The disclosure names which
operations were used, so a reader can see which ones were left out.

**The exclusions hold.** Over 256 cases each: no NaN, no infinity, no control characters;
multi-byte string content and negative floats are generated. Measured against promises
written to be false if the exclusions leaked.

**No evidence laundering across the engine split.** A proof asked for on a float, a string
or a vector is refused by name, never downgraded into a sampled result wearing a proof
word. Declaring both a proof and a sampling check on the same function runs the sampling
check for real and leaves the refusal as a refusal — I confirmed by making the promise
false and watching the sampling half catch it while the proof half stayed refused.

**Feature combinations behave.** A method with a string argument, and a method taking a
float on a receiver built by a sequence, both caught their false promises; the float
disclosure fires correctly on a method as well as a free function; the constructor gate
uses the same predicate as the value generator, so there is no gap between "Ply accepted
this constructor" and "Ply can generate its arguments".

**A counterexample that cannot be written out says so.** For every receiver method with
arguments, and for every string, float and vector argument, Ply declines to render a
replay test, says in plain words that it cannot write that value back out as Rust source,
and adds that it never invents one. That is the right instinct — it is only the
zero-and-scalar-argument receiver case (finding 3) that slips past it.

---

## The three side questions

**Was abandoning the caching work the right call?** Yes — but the measurement is the weak
part of the argument, and the argument does not need it. Two single runs, 2533s against
2569s, a 1.4% difference declared "within noise" without ever establishing what the noise
is; on that evidence alone the honest verdict is "no measurable difference", not "cannot
work". What actually settles it is the mechanism: the cache can only hit on a second
invocation against a populated store, and the duplicate work runs concurrently, so there
is no first-then-rest ordering to exploit. The number that would have proved it in one
line — how many times the cache hit during the instrumented run — is not reported. Right
conclusion, reached partly by the wrong evidence. Everything else about that write-up is
exemplary: the safety test was watched red first, the code was removed rather than left
dead, and the cost is stated in hours.

**How much does refusing constructors that return a result cost in practice?** Less than
the previous review implied, and the previous review named the wrong dominant blocker. In
the rate-limiter yardstick, one of nine constructors returns a result. The blocker that
actually bites is the *other* narrowing: a constructor must take only values Ply can
generate, and four of those nine take a type of the crate's own (a quota, a config, a
clock). So the receiver work reaches types whose constructor takes plain numbers and
essentially nothing else — the outermost layer of a real crate. Both narrowings are
documented honestly in the code; only one of them is in the running-state list as a known
gap, and it is the smaller one.

**Do the two commands disagree about receivers in a way that misleads?** Yes — see finding
6. It is the second time in twelve commits that these two commands have been made to agree
and then pulled apart again, both times by the newer feature forgetting the cheaper
command exists.

---

## Pinned by nothing

Per this project's rule that a defect found in review enters the suite as a fixture of its
own shape, each of these needs a fixture, and none exists today:

- **A worked-examples check on a method.** Every use of that check in the whole fixture
  set is on a plain function with a promise, and four of the six also declare worked
  examples. The shape in finding 1 — that check, a method, no examples — appears nowhere.
- **A worked-examples check on a function with no promise.**
- **Two claims in one crate whose flattened names overlap** (finding 2). Trivial to write
  and would have gone red immediately.
- **A replay test rendered for a method** (finding 3). The replay-validity oracle is wired
  to one free function only.
- **A second run of anything with a no-argument function** (finding 4). The tamper warning
  only appears on the second run; nothing runs anything twice.
- **A proof requested on a method** (finding 5).
- **The two commands compared on one document containing a method** (finding 6).
- **The string exclusion asserted from the outside** (finding 7) — the code test pins the
  generated strategy text; nothing pins that a run over a string-shaped function tells the
  user what was excluded, which is where the float precedent puts its test.

## Two things I read but could not run

- **The receiver bound is in nothing the record remembers.** The stored fingerprint has
  twelve named inputs and none of them is the sequence length. Today nothing can drift
  through that gap, but only by luck: every method lives in an `impl` block, and an
  `impl` block forces the coarse fingerprint mode in which any edit anywhere in the crate
  re-earns everything — I confirmed that by editing a constructor and watching the result
  correctly re-run. The moment the fingerprint walk learns to handle `impl` blocks, the
  constructor and the operation pool fall outside it, and a promise that depends on them
  will be carried forward across a change to them.
- **Ply's own version is a constant that has not moved.** The record's design says the
  tool's version is "the input that makes this scheme sound rather than merely fast",
  because a defect fixed in Ply changes what a recorded result means. It is the crate
  version string, `0.1.0`, and it is identical across all twelve commits reviewed here.
  So a record file written by yesterday's build — the one with the tenth false clean, in
  which every method check passed without executing anything — still hash-matches today's
  fixed build and is reused unchanged. I confirmed the reuse half directly: the false
  clean in finding 1 is written to the record and comes back marked reused on the next run
  with no diagnostic at all.

---

## Reproductions

All under `/tmp/rev3/`, all against `/home/user/ply/target/debug/cargo-ply` as built at
`9c056ee`. Each scratch crate depends on the attribute crate by absolute path.

| # | what | command | result |
|---|---|---|---|
| 1 | eleventh false clean | `/tmp/rev3/testrecv`, `checks: [test]`, `cargo-ply ply verify .` | `tested`, exit 0, no diagnostics |
| 1b | zero tests really ran | `cargo test -p testrecv-ply-harness --lib Bucket_capacity_harness::` | `running 0 tests` |
| 1c | same crate, sampling check | same crate, `checks: [fuzz(64)]` | `violation`, exit 1 |
| 1d | the nothing is recorded and reused | second `verify` run, `--json` | `"verdict": "tested", "reused": true`, no evidence block |
| 1e | no promise, no examples | `checks: [test]` on an uncontracted fn | `tested` |
| 2 | correct function blamed | `/tmp/rev3/suffix` (`parse` + `util::parse`) | both `violation`, correct one cites the other's tests |
| 3 | replay test does not compile | `/tmp/rev3/cexrecv`, then `cargo test --lib` | `error[E0061]`, user's crate no longer builds |
| 4 | tamper alarm on an ordinary fn | zero-parameter fn, `checks: [fuzz(64)]`, run twice | second run: "something else edited it" |
| 5 | proof refusal blames `u32` | `checks: [bounded(2)]` on `B::cap(&self) -> u32` | "its return type `u32` … real, substantial work" |
| 6 | the two commands disagree | `cargo-ply ply check .` on `/tmp/rev3/cexrecv` | "constructing a receiver is not supported yet" |
| 7 | exclusions hold | six promises over strings/floats/vectors, `fuzz(256)` each | no control chars, no NaN, no infinity; multi-byte and negatives generated |
| 8 | result-returning constructor | `/tmp/rev3/ctorres` | refused, reason names the wrong obstacle |
| 8b | constructor taking a user type | `/tmp/rev3/ctorarg` | refused, reason names the exact obstacle |
| 9 | ninth false clean is gone | rebuilt two-`Root` crate, both spellings | refused / real violation, never a green pass |
| 10 | the bound is exactly three | counter breaking on the 4th call, then the 5th | found / not found, with the sentence that says so |
| 11 | engine split does not launder | proof + sampling on one vector-shaped fn, false promise | proof refused by name, sampling catches it |
| 12 | clean breaks the crate | `cargo clean` then `cargo build` after any verify | manifest error naming a generated path |
