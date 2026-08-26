# The hash now covers the code the checks run

*2026-08-25. Written against the build in this repository; every block of output below is
copied from a real run rather than reconstructed.*

Ply remembers a checked result beside a hash of what it depended on, and reuses the
result while the hash still matches. An adversarial review (`docs/review-result-reuse.md`)
proved that hash did not cover **the code the check actually runs**. It covered the
checked function's own tokens and the promises declared for callees a proof replaces —
and not the body of an ordinary helper the check calls, not the body of a contracted
callee a proof descends into, not the worked examples a `test` check asserts, and not the
versions the crate's dependencies resolve to.

The consequence was the one failure this project cannot ship. A function with a contract,
calling a plain local helper; break the helper so the function genuinely violates its own
guarantee; the stored record turns that into a confident pass:

```
$ cargo ply verify . --engine-timeout 120        # with the stored record
workspace — fuzzed(64)
  repro — fuzzed(64)
    widen — fuzzed(64)  [reused]

  [reused]         this result was not re-run: an earlier run recorded it, and everything it depended on — the code, the promises it assumes, the checks, the engines, Ply's own version — hashes the same today

real	0m0.032s

$ rm ply.lock && cargo ply verify . --engine-timeout 120     # identical source
workspace — violation
  repro — violation
    widen — violation
[P0502] repro::widen — `widen` breaks its own postcondition `|result|*result >= x` for at
least one input -- proptest shrank a failing case to this minimal example. (P0502)

real	0m0.592s
```

The printed line was not merely incomplete. It said "everything it depended on — the
code … hashes the same today" while the code it ran had changed underneath it.

---

## What was done, and why this shape

The two defensible options were to hash what a check reaches, or to refuse reuse whenever
a check reaches code the hash cannot cover. **The first was taken**, using the call-graph
walker the boundary rule already uses (`ply_core::callgraph::Resolver`) rather than a
second one. Where that walk cannot see far enough, the answer is neither of the two: it is
a third, coarser hash — the whole crate's source — which keeps reuse working and keeps it
honest. Refusing outright was rejected because it throws the feature away on any crate
that defines a type, which is every real one.

`ply_core::reach` answers one question per claim: **which first-party bodies can this
check reach?** It walks out of the claimed function, following calls, plain mentions of a
function by name (`map(helper)` never writes `helper(..)` and still runs the body), and
the functions named inside the claim's own contract expression (`#[ply::ensures(|result|
*result == expected(x))]` runs `expected` on every generated case, and it lives in an
attribute no walk of the body would ever see), transitively, and hashes the token stream
of every body it reaches. The claimed function's own tokens stay in their own input, so
one edit is reported as one input moving rather than two. It stops at two
places, deliberately:

- **A callee replaced by a declared promise.** The proof never looked inside it, so the
  body is not what the result stood on — the promise is, and the promise has always been
  hashed. This is what keeps the record from being a cache of function bodies, and it is
  narrowed correctly: only a claim whose *every* check is `bounded` gets it, because
  `fuzz`, `test` and `mutate` run the real body however many promises are declared for
  it.
- **Anything outside the workspace** — `std`, a registry crate. There is no source to
  hash, and two inputs cover it instead: the compiler identity (already hashed) and, new
  here, the versions the crate's dependencies resolve to.

### The limit, and the fallback that makes it safe

A syntactic walk can follow a call written as a call. It cannot follow `x.helper()` —
which body that names depends on the type of `x` — nor an operator that some `impl`
defines, nor a macro expansion, nor a trait method reached under a different name
(`x.into()` runs somebody's `from`). Resolving those needs a type checker. Ply is not one,
and reshaping the walk until it *looked* like one is how this defect happened the first
time.

So the walk is trusted **only** under conditions that make every one of those
impossible: every item in first-party source is a function, a module, an import, a type
alias, or a plain data type with only `std` derives; no reached body invokes a macro; and
no reached function carries an attribute Ply does not recognise, since an attribute macro
rewrites the body into something the walk never saw. (That last one is checked per reached
function rather than over the whole crate — a macro on a function nothing reaches says
nothing about this claim, and a crate-wide check would fire on every `#[test]` in a
dependency.) When any of that does not hold — which is most real crates — Ply abandons the walk and
hashes **every line of first-party source in the crate and its path dependencies**
instead, recording *which* of the two it did as a hashed input of its own. That is
coarser: an edit anywhere in the crate then re-earns every claim in it. It is never
wrong.

The condition is an **allowlist** on purpose. An item kind nobody anticipated falls into
"hash everything and re-run", which costs engine time. A denylist would have put it into
"reuse anyway", which costs a user a green verdict over code nobody checked. This branch
has now produced that failure seven times; the direction of the safety margin is the whole
point.

### What else went into the hash

- **The worked `examples:`** a `test` check compiles into assertions. Each one *is* part
  of what that check asserts.
- **The resolved dependency versions**, read from `Cargo.lock` and filtered to the
  packages reachable from this crate's own package — so the harness crate Ply generates,
  which depends on the target and never the reverse, cannot move the target's
  fingerprint. Where the crate's whole dependency closure is inside the workspace, the
  input reads "(nothing outside this workspace)" whether or not a lockfile exists, so a
  first build does not invalidate what the run before it earned. Where there are outside
  dependencies and no lockfile, Ply records *that* rather than guessing, and a result
  recorded with a lockfile is not reused without one. The fingerprint a result is stored
  under is taken after the run, so a crate that had never been built records the versions
  its own run actually compiled against.
- **A short digest per named input, stored beside the result.** It decides nothing — the
  whole fingerprint does that — it is what lets a run explain a re-run. It costs about
  700 bytes per claim in the committed file.
- **The signature a stubbed callee's replacement is built from.** A promise is only half
  of what replaces a body; the other half is the shape of the value the proof invents in
  its place, and a widened return type the caller absorbs through inference used to
  change the proof while changing no hashed input.

The record's format number moves from 1 to 2. Every fingerprint changes anyway, so
entries written by the old build could never match; ignoring them by format rather than
by hash is the reading this module already has for a file it does not understand.

---

## The failures watched first

The reproduction at the top of this page is the first one, on the real release binary.
Then seven end-to-end tests over two new fixtures, run against the pre-fix build. Five
failed, each naming the actual defect:

**1. A broken helper is carried forward as a pass.**

```
---- breaking_a_helper_the_check_runs_re_earns_the_claim_and_finds_the_bug stdout ----
assertion `left == right` failed: the recorded result was about a helper body that is no
longer there, so it says nothing about the code as it stands now:
{… "id":"doubled","kind":"fn","reused":true,"verdict":"fuzzed(64)" …}
  left: Bool(true)
 right: Null
```

**2. A proof's walked-into body is carried forward as a proof.**

```
---- editing_a_body_a_proof_descends_into_re_earns_the_proof stdout ----
assertion `left == right` failed: the proof read `inner`'s body, so a rewritten `inner`
is a different proof: {… "id":"outer","kind":"fn","reused":true,"verdict":"bounded(2)" …}
  left: Bool(true)
 right: Null
```

**3. An edited worked example never runs.**

```
---- editing_a_worked_example_re_runs_the_check_that_asserts_it stdout ----
assertion `left == right` failed: the example is what the check asserts, so changing it
is changing the check: {… "id":"doubled","reused":true,"verdict":"fuzzed(64)" …}
  left: Bool(true)
 right: Null
```

**4. A hand-edited verdict is believed.**

```
---- a_recorded_verdict_those_checks_could_never_earn_is_refused stdout ----
assertion `left != right` failed: sampling cannot earn a proof, so a stored `proved`
beside a `fuzz` check must never reach a reader:
{… "id":"bumped","reused":true,"verdict":"proved" …, "id":"doubled","reused":true,"verdict":"proved" …}
  left: String("proved")
 right: String("proved")
```

**5. A surprise re-run explains nothing.**

```
---- a_result_that_could_not_be_carried_forward_says_which_input_moved stdout ----
a surprise re-run must explain itself: workspace — fuzzed(64)
  reusehelper — fuzzed(64)
```

And one unit-level failure watched separately, by removing the reachable code from the
hashed inputs while leaving the field in place — the shape the fingerprint's single loop
test exists to catch:

```
---- record::tests::every_input_the_spec_lists_changes_the_fingerprint stdout ----
assertion `left != right` failed: changing the body of a helper the check runs or
descends into must change the fingerprint, or a result earned before the change is
reused after it
  left: "01043b851cac74eabbae34a69e0f836ca7187ff6ae1608038c11baeac192e801"
 right: "01043b851cac74eabbae34a69e0f836ca7187ff6ae1608038c11baeac192e801"
```

**The fixtures are the point.** `tests/fixtures/reusehelper` is a contract calling an
ordinary local helper — the shape almost all real code has, and the one shape no fixture
in the branch that built this feature contained. `tests/fixtures/reuseproof` is a proof
descending into a callee with its own contract. Their absence is why 284 green tests said
nothing.

---

## Six demonstrations

Two new crates, one existing one, all through the release binary.

### 1. Nothing moved — everything is still carried forward

The feature has to keep working, or the fix is a deletion. `tests/fixtures/reusehelper`,
two claims, verified twice with nothing touched:

```
$ cargo ply verify . --engine-timeout 120
workspace — fuzzed(64)
  reusehelper — fuzzed(64)
    bumped — fuzzed(64)
    doubled — fuzzed(64)

real	0m11.794s

$ cargo ply verify . --engine-timeout 120
workspace — fuzzed(64)
  reusehelper — fuzzed(64)
    bumped — fuzzed(64)  [reused]
    doubled — fuzzed(64)  [reused]

  [reused]         this result was not re-run: an earlier run recorded it, and every input Ply hashes still hashes the same — the function's own source, the code it calls, the promises it assumes, the examples it checks, the checks themselves, the engines, the compiler and target, the crate's features, the resolved versions of its dependencies, and Ply's own version

real	0m0.028s
```

And on the existing `tests/fixtures/resultreuse`, which has a proof standing on a declared
promise: **97.3s the first time, 0.067s the second**, all three carried forward. Editing
one function there still re-earns one claim and no others — the granularity the record
exists for, unchanged by the fix:

```
$ # only safe_increment's body edited
workspace — fuzzed(64)  [assumed, evidence owed]
  resultreuse — fuzzed(64)  [assumed, evidence owed]
    safe_increment — bounded(2)
    total — bounded(2)  [assumed, evidence owed, reused]
    widen — fuzzed(64)  [reused]

  Checked again rather than carried forward from an earlier run, because what each one depended on has changed:
    resultreuse::safe_increment — the function's own source changed since that result was recorded

real	0m35.581s
```

### 2. A helper the check runs is edited

Same crate. The only edit is `scale`'s body, `x * 2` → `x / 2`, which makes `doubled`
genuinely break its own postcondition:

```
$ cargo ply verify . --engine-timeout 120
workspace — violation
  reusehelper — violation
    bumped — fuzzed(64)  [reused]
    doubled — violation

  [reused]         this result was not re-run: …

  Checked again rather than carried forward from an earlier run, because what each one depended on has changed:
    reusehelper::doubled — the code it runs changed since that result was recorded

[P0502] reusehelper::doubled — `doubled` breaks its own postcondition `|result|*result >= x` for at least one input -- proptest shrank a failing case to this minimal example. (P0502)
[R0502] reusehelper::doubled — `doubled` failed 2 of its own example/generated direct-contract test(s): doubled_harness::ply_direct_doubled_01, doubled_harness::ply_example_doubled_01. …

real	0m0.661s
```

Three things at once: the claim that could reach the edited helper is re-run and reports
the real bug; the claim that could not (`bumped` reaches only `shift`) keeps its result;
and the run says which input moved. Under the old build this printed `doubled —
fuzzed(64)  [reused]`.

### 3. A body a proof descends into is edited

`tests/fixtures/reuseproof`: `outer` is proved with `bounded(2)` and calls `inner`, which
carries its own contract — so §5.5's first branch lets the proof read the real body. The
claim on `inner` does not exist; only `outer` is claimed.

```
$ cargo ply verify . --engine-timeout 300          # first run
    outer — bounded(2)
real	1m3.664s

$ cargo ply verify . --engine-timeout 300          # nothing moved
    outer — bounded(2)  [reused]
real	0m0.066s
```

Now the only edit is `inner`'s body, `x * 2` → `x * 2 + 500`:

```
$ cargo ply verify . --engine-timeout 300
workspace — violation
  reuseproof — violation
    outer — violation
  Checked again rather than carried forward from an earlier run, because what each one depended on has changed:
    reuseproof::outer — the code it runs changed since that result was recorded

[K0502] reuseproof::outer — `outer` breaks its own postcondition `|result|*result <= 200` for at least one input -- a postcondition is the guarantee a function makes about its return value, and Kani found a case where that guarantee does not hold. (K0502)

real	1m11.164s
```

**71 seconds of real proving, and the proof fails.** Under the old build the same edit
left `outer — bounded(2)  [reused]` — failure 2 in the red list above, and the reviewer
measured it at 70ms against a cold run of 67 seconds that reported the violation. The
strongest verdict Ply has, carried forward over a callee the proof reads and somebody had
broken.

### 4. A worked example is edited

Same `reusehelper` crate, no Rust touched at all (`md5sum src/lib.rs` identical either
side). The example `doubled(2) == 4` becomes `doubled(2) == 5`:

```
$ cargo ply verify . --engine-timeout 120
workspace — violation
  reusehelper — violation
    bumped — fuzzed(64)  [reused]
    doubled — violation

  Checked again rather than carried forward from an earlier run, because what each one depended on has changed:
    reusehelper::doubled — the worked examples it asserts changed since that result was recorded

[R0502] reusehelper::doubled — `doubled` failed 1 of its own example/generated direct-contract test(s): doubled_harness::ply_example_doubled_01. …

real	0m0.735s
```

The explanation names the right input: the code did not move, the example did.

### 5. The fallback, doing its job and charging for it

The honest cost of the whole-crate mode, shown rather than described. A crate with a
`struct` and an `impl`, where `helper` reaches a body only through a method call:

```rust
pub struct Money(pub u32);
impl Money { pub fn doubled(&self) -> u32 { self.0 * 2 } }
pub fn helper(x: u32) -> u32 { Money(x).doubled() }

#[ply::requires(x <= 1_000)] #[ply::ensures(|result| *result >= x)]
pub fn grow(x: u32) -> u32 { helper(x) }

#[ply::requires(x <= 1_000)] #[ply::ensures(|result| *result > x)]
pub fn nudge(x: u32) -> u32 { x + 1 }
```

Break the method — `self.0 * 2` → `self.0 / 2` — and no call walk could have found it:

```
workspace — violation
  widen — violation
    grow — violation
    nudge — fuzzed(64)
  Checked again rather than carried forward from an earlier run, because what each one depended on has changed:
    widen::grow — the code it runs changed since that result was recorded
    widen::nudge — the code it runs changed since that result was recorded

[P0502] widen::grow — `grow` breaks its own postcondition `|result|*result >= x` for at least one input …
```

`grow` is caught. And `nudge`, which cannot reach `Money` at all, is re-run too — that is
the price of the coarse mode, paid visibly. A user who wants per-claim reuse back on such
a crate gets it by having no `impl` blocks, which is not advice anyone will take; the
realistic reading is that in a crate with types, reuse is per-crate for source edits and
per-claim for everything else (contracts, promises, examples, checks, engines, toolchain,
dependency versions).

### 6. A hand-edited record

The record is not defended against a text editor, and cannot be. The honest version of
that mistake now is:

```
$ sed -i 's|"verdict": "fuzzed(64)"|"verdict": "proved"|' ply.lock
$ cargo ply verify . --engine-timeout 120
workspace — fuzzed(64)
  reusehelper — fuzzed(64)
    bumped — fuzzed(64)
    doubled — fuzzed(64)
[W0516] reusehelper::bumped — The recorded result for `reusehelper::bumped` says `proved`, and the checks recorded beside it (fuzz(64)) cannot produce that answer. A result file Ply wrote never contains this, so something else edited it -- a merge that went wrong, or a hand edit. Ply ignored the stored result and ran the checks again; what you see below was earned just now.
[W0516] reusehelper::doubled — The recorded result for `reusehelper::doubled` says `proved`, and the checks recorded beside it (fuzz(64), test) cannot produce that answer. …
```

Under the old build both nodes printed `proved  [reused]` — the strongest verdict Ply
has, minted by a text editor and repeated on every future run.

---

## Everything that claimed completeness, and what it says now

| Where | Was | Is |
|---|---|---|
| `The-Ply-Spec.md` §5.2a | "**The fingerprint covers everything the answer depended on.**" over a seven-item list | "**What the fingerprint covers**" over a ten-item list, plus a paragraph stating exactly what the reach walk can and cannot follow and what it does instead, plus a paragraph naming what is *not* covered (`RUSTFLAGS`, `[profile]`, outside proc-macro expansion, hand edits) |
| `The-Ply-Spec.md` glossary + D14 | "hash of everything a result depended on: item body, contract text, …" | "hash of what a result depended on: item body, contract text, **the first-party bodies the check runs or descends into** (or all of the crate's source, when Ply cannot bound that set), …" |
| `docs/SCHEMA.md` "Results Ply already has" | "a hash of **everything the answer depended on**" | "a hash of these, and this is the whole list", with the reach entry, the caveat paragraph in plain words, and a **What is not in the hash** paragraph |
| `docs/SCHEMA.md` "What `verify` writes into your crate" | "a hash of everything it depended on" | "a hash of what it depended on — including the bodies of the helpers each check actually runs" |
| The `[reused]` line printed on every warm run | "everything it depended on — the code, the promises it assumes, the checks, the engines, Ply's own version — hashes the same today" | "every input Ply hashes still hashes the same — the function's own source, the code it calls, the promises it assumes, the examples it checks, the checks themselves, the engines, the compiler and target, the crate's features, the resolved versions of its dependencies, and Ply's own version" |
| `docs/result-reuse.md` (the implementer's write-up) | a retraction saying the fix was in flight | the retraction now records that the fix landed, points here, and marks its own input table as *what that build hashed*, not what Ply hashes |
| `crates/ply-core/src/record.rs` | "a hash of everything the answer depended on"; "The hash of everything the answer depended on" | the input list by name, and "Not of everything the answer depended on: `reach` states what a syntactic walk cannot see" |
| `tests/fixtures/resultreuse/src/lib.rs` | "a hash of everything it depended on" | "a hash of what it depended on", plus a note pointing at the shape it does not contain |

Note the wording that survives everywhere: **"every input Ply hashes"**, never
"everything". Even with the fix, that is the true sentence.

---

## What the hash still cannot cover

Stated here, in §5.2a, and in the user reference, rather than left to be found:

1. **Environment that shapes a build without appearing in a file Ply reads.** `RUSTFLAGS`,
   `[profile]` settings — `overflow-checks` flips what an arithmetic contract finds — and
   a `#[path]` module attribute the resolver does not follow. None is an input.
2. **What a proc macro from outside the workspace expands to**, beyond the identity of
   the crate it came from (now pinned by the dependency versions). A registry macro whose
   expansion changes within one resolved version is not visible.
3. **The sampling engine's resolved patch version.** Unchanged from before: Ply records
   the version *requirement* it writes into the generated harness (`1`), not what cargo
   resolved. Closing it means reading the generated crate's own lockfile, which does not
   exist on a first run.
4. **A hand-edited `ply.lock`.** The consistency check above catches a verdict the checks
   could not earn. It does not catch a plausible lie — `fuzzed(64)` where the run said
   `violation` — and nothing short of signing would.
5. **A promise still does not go stale.** If the code under a declared promise changes,
   the caller's result is hashed against the promise, because the promise is what its
   proof used. Unchanged, and already documented as one of the things to know before
   trusting a boundary promise.
6. **Reuse across machines needs a committed `Cargo.lock`** when the crate has
   dependencies outside the workspace. Without one Ply cannot know which versions an
   earlier run compiled against, and says so rather than guessing — so a library crate
   that does not commit its lockfile will not reuse results from another checkout.

---

## Everything else stays green

| Suite | Result |
|---|---|
| product workspace (`cargo test --workspace`) | **306 passed, 0 failed**, exit 0 |
| specification tooling (`cd tools && cargo test --release`) | **118 passed, 0 failed**, exit 0 |

Formatting and lint are clean in both workspaces (`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets`). This change adds twenty-one of those tests:
twelve on the reach walk, two on the record (the impossible-verdict table and the
"which input moved" comparison), and seven end to end over the two new fixtures. The
fingerprint's existing single loop over every hashed input gained five entries rather
than tests of its own, which is the point of writing it as a loop.

---

## TODO deltas

Not applied — `TODO.md` was out of bounds for this change. These are the lines it wants:

- **Done, this change**: the recorded fingerprint covers the first-party bodies a check
  runs or descends into (with a whole-crate fallback wherever a syntactic walk cannot
  bound them), the worked examples a `test` check asserts, the resolved versions of
  packages outside the workspace, and the signature a stubbed callee's replacement is
  built from. The record format is version 2.
- **Done, this change**: a stored verdict its own recorded checks could never earn is
  refused (`W0516`) instead of believed.
- **Done, this change**: a claim whose recorded result could not be carried forward names
  the input that moved, in the terminal and in `not_carried_forward` in the JSON envelope.
- **Done, this change**: the completeness claim is retracted in the specification, the
  user reference, the write-up, the record module and the line printed to users; each now
  states what is covered and what is not.
- **KNOWN GAP (new)**: `RUSTFLAGS`, `[profile]` settings and `#[path]` are not fingerprint
  inputs.
- **KNOWN GAP (new)**: reuse across checkouts requires a committed `Cargo.lock` for any
  crate with dependencies outside its workspace.
- **KNOWN GAP (amended)**: hand-editing is caught only where the stored verdict is
  impossible for the recorded checks; a plausible lie is not caught, and cannot be without
  signing.
- **KNOWN GAP (existing, unchanged)**: the fuzz engine's version in a fingerprint is the
  requirement Ply writes, not the resolved version.
- **Open question (unchanged)**: whether every Ply version bump should invalidate every
  stored result.
- **Deferred**: `verify` does not say *why* it widened a claim's scope to the whole crate
  ("`Money` has an `impl` block, so Ply hashed the crate"). The reason is computed and
  carried in `reach::CodeScope::widened_because`; nothing prints it. On a crate with types
  that would turn "the code it runs changed" from true-but-puzzling into actionable.
