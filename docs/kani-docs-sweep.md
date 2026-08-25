# Kani documentation sweep — what the docs say vs. what we measured

Date: 2026-08-23. Read systematically rather than searched narrowly, after a night of
expensive empirical discovery.

**Sources read end to end.** The Kani book (`docs/src/`, all 34 pages), the RFC book
(`rfc/src/rfcs/`, all 14 RFCs), the Kani blog index plus the performance post, the open
GitHub issue tracker, **the `verify-rust-std` project and Kani's own CI test suite** (§2 —
how the tool is actually driven by the people who wrote it), and — the part that settled most
of the disputes — the Kani source itself at two points:

- `git@github.com:model-checking/kani.git` `main` @ `8185456` (2026-08-23), i.e. the docs
  as published today at <https://model-checking.github.io/kani/>, and
- the `kani-0.67.0` release tag, which is **what we actually run** (`cargo-kani 0.67.0`,
  CBMC 6.8.0, pinned in ADR-0003).

**That split is the single most important thing in this document.** The published book is
built from `main`, not from the latest release. Several capabilities described at
model-checking.github.io **do not exist in the binary on this machine**, and several
limitations documented on `main` were undocumented at 0.67.0. Every claim below is
tagged `[0.67.0]` (true of our toolchain) or `[main only]` (documented, unreleased).

The gap is not small. **0.67.0 was published 2026-01-16 and is still the latest release
today (2026-08-23)** — seven months of unreleased `main`, including both machine-readable
output formats. Anyone reading model-checking.github.io is reading documentation for a Kani
nobody can install.

**Standing rule for this document.** Where the docs and our measurements conflict,
**measurement wins and the doc is stale**. Kani is a v0 tool whose docs lag its driver;
RFC-0009 is the proof of that, and it is not the only case.

---

## 1. Our findings vs. the docs

Tally: **4 DOCUMENTED · 4 CONTRADICTED BY DOCS · 2 UNDOCUMENTED.**

This section classifies each finding against *the documentation*. It does **not** ask whether
the finding is a genuine Kani limit or a mistake of ours — that is §2, which reviews all ten
against how `verify-rust-std` and Kani's own CI actually drive the tool, and reaches a
different verdict on three of them.

### Finding 1 — `--concrete-playback` does not reproduce an `ensures` violation

> The generated test does not reproduce a contract violation, because playback never
> evaluates contract closures; it only reproduces failures that panic at runtime.

**CONTRADICTED BY DOCS.** Twice over, and the second contradiction explains why we saw no
warning.

The book promises the opposite by construction. `reference/experimental/concrete-playback.md`
(0.67.0 and main, identical):

> "When the result of a certain check comes back as a `FAILURE`, Kani offers the
> `concrete-playback` option to help debug. This feature generates a Rust unit test case
> that plays back a failing proof harness using a concrete counterexample."
>
> "When concrete playback is enabled, Kani will generate unit tests for **assertions that
> failed** during verification, as well as cover statements that are reachable."

A contract violation *is* a failed assertion in Kani's own encoding. RFC-0009 §User
Experience spells the lowering out:

> "It inserts postconditions (`ensures`) as `kani::assert` checks *after* the call to
> `my_div`, enforcing the contract."
>
> — <https://model-checking.github.io/kani/rfc/rfcs/0009-function-contracts.html>

So by the documented model, an `ensures` failure is an `assertion`-class FAILURE, playback
generates a test for it, and that test should be red. We measured that it is green.
Measurement wins.

The generated test's shape (verified in the 0.67.0 source, `format_unit_test`) is a plain
`#[test]` whose body is `kani::concrete_playback_run(concrete_vals, <harness_name>)` — it
re-enters the *original harness function by name*. Our inference for why the contract does
not fire is that the `requires`/`ensures` assume/assert pair is injected by the
`proof_for_contract` expansion on the verification path only, so the replay executes the
real body and nothing else. We have not confirmed that mechanism in Kani's macro source;
the *behaviour* is confirmed both ways (ADR-0003 caveat 3: moving the same overflow into
the function body makes the identical generated test fail).

The stated limitation does not cover our case and is itself stale:

> "## Limitations
> * This feature does not generate unit tests for failing non-panic checks (e.g., UB
>   checks). This is because checks would not trigger runtime errors during concrete
>   playback. **Kani generates warning messages for this.**"

Two problems. (a) A contract failure is not a "non-panic check", so the documented carve-out
does not name it. (b) The promised warning does not exist per-property. In
`kani-driver/src/concrete_playback/test_generator.rs` at the 0.67.0 tag, the filter is:

```rust
.filter(|prop| {
    (prop.property_class() != "unwind" && prop.status == CheckStatus::Failure)
        || (prop.property_class() == "cover" && prop.status == CheckStatus::Satisfied)
})
```

— every failing property except unwinding assertions yields a test, no per-property
warning. The only warning in the whole module fires when *nothing at all* could be
generated:

```rust
println!(
    "WARNING: Kani could not produce a concrete playback for `{}` because there \
    were no failing panic checks or satisfiable cover statements.", ...)
```

**This closes the TODO.md item that says "Kani reportedly warns when it declines to
generate a test; no warning appeared in our run, so don't depend on it."** It is not
flakiness. Kani did not decline — it generated a test it had no business believing in.
Silence here is guaranteed, not incidental, and `docs/plans/d7-replayable-tests.md` is
right to make Ply assert the postcondition itself.

### Finding 2 — a CBMC timeout and a contract failure print identically as `VERIFICATION:- FAILED`

**CONTRADICTED BY DOCS — and, more usefully, contradicted by the 0.67.0 source.** The
`VERIFICATION:- FAILED` *line* is identical, but the surrounding output is not, and the
difference is mechanically detectable. From `kani-driver/src/call_cbmc.rs` at the
`kani-0.67.0` tag:

```rust
ExitStatus::Timeout => (
    String::from("CBMC failed"),
    "CBMC timed out. You may want to rerun your proof with a larger timeout \
     or use stubbing to reduce the size of the code the verifier reasons about.\n",
),
ExitStatus::OutOfMemory => (
    String::from("CBMC failed"),
    "CBMC appears to have run out of memory. ...",
),
```

rendered as:

```
CBMC failed
VERIFICATION:- FAILED
CBMC timed out. You may want to rerun your proof with a larger timeout ...
```

A genuine failure instead emits a `RESULTS:` block, a `SUMMARY:` block, and `VERIFICATION:-
FAILED` with no `CBMC failed` header. Internally these are different variants —
`VerificationResult.results` is `Result<Vec<Property>, ExitStatus>`, so an exhausted run
carries `Err(ExitStatus::Timeout)` and has **no properties at all**, where a violation
carries `Ok(properties)` with at least one `FAILURE`.

**The catch, and it is the reason our measurement looked the way it did:** that output only
appears when Kani itself enforced the bound, via `--harness-timeout <N>` — which is gated
behind `-Z unstable-options`. If the harness is killed by an external `timeout(1)`, by CI,
or by our own patience, Kani prints nothing at all and the distinction genuinely does not
exist. Our runs used the latter. **This one is our mistake, not Kani's limit** — see §2.

Also worth knowing, from the arg-validation comment on `main` (this behaviour predates it):

> "`--output-format=old` bypasses CBMC's structured output entirely: `run_cbmc` mocks a
> result with no properties, and **treats a timeout as success**. An export produced from
> that would be indistinguishable from a real clean run."

`--output-format=old` must never be used by a Ply adapter. It converts exhaustion into a
green verdict — precisely the evidence-lying failure §5.4c exists to prevent.

### Finding 3 — `stub_verified` checks only that a `proof_for_contract` harness *exists*

**CONTRADICTED BY DOCS at 0.67.0; DOCUMENTED (correctly, and quietly changed) on `main`.**

This is the finding the brief flagged, and it holds. RFC-0009 §Workflow and Attribute
Constraints Overview:

> "1. By default `kani` or `cargo kani` first verifies all contract harnesses
> (`proof_for_contract`) reachable from the file or in the local workspace respectively.
> 2. Each contract ... that is used in a `stub_verified` is required to have at least one
> associated contract harness. Kani reports any missing contract harnesses as errors.
> **3. Kani verifies all regular harnesses *if* their `stub_verified` contracts passed step
> 1 and 2.**"

and §User Experience, phase 3:

> "In addition, by default, it requires all `proof_for_contract` harnesses to **pass
> verification** before attempting verification of any harnesses that use the contract as a
> stub."

0.67.0 does not do this; we demonstrated a caller reporting SUCCESS while stubbing a
deliberately falsified callee whose own harness failed in the same run. **Measurement wins;
RFC-0009 is aspirational text, not a specification of shipped behaviour.**

Three pieces of corroboration make this more than "we found a bug":

1. **The RFC itself files the gating under Open Questions,** not Detailed Design:
   > "- How to **check the right contracts at the right time**. ... **By default**
   > (`kani`/`cargo kani`) all local contracts are checked, harnesses are only checked if
   > the contracts they depend on succeeded their check."

   Listing the behaviour as an open question is an admission it was never settled. The
   normative-sounding §Workflow text is the same proposal restated.

2. **The changelog shows what was actually implemented,** and it is exactly the weaker
   check we observed. Kani 0.66.0, under *Breaking Changes*:
   > "Fail if stub verified doesn't have a contract harness — #4295"

   Existence, not success. That PR is the whole of the enforcement — it was the fix for
   issue **#4294, "Kani doesn't check that `stub_verified` contracts are verified"**,
   labelled `[C] Bug` and **`[F] Soundness`**, filed against 0.65.0 with a reproducer
   (`#[kani::ensures(|res| *res == 7)] fn foo() -> u8 { 8 }` verifying SUCCESSFUL through
   `stub_verified`) and closed 2025-08-13.
   <https://github.com/model-checking/kani/issues/4294>

   **And the author of the fix says in the PR body that it does not close the hole:**

   > "Note that this compilation error is **not really sufficient to ensure that the stub's
   > contract holds**, since a user can just pass `--harness` to skip the contract harness.
   > A better design would be to insert the check automatically rather than requiring a
   > separate harness. But this is better than nothing and less invasive to implement, so
   > start with this."
   > — PR #4295, <https://github.com/model-checking/kani/pull/4295>

   That is a Kani maintainer, in 2025, naming the exact hole we spent a night measuring and
   naming `--harness` as the way through it. **There is no open follow-up issue tracking the
   residual gap.** It is not a bug someone is fixing; it is a known, accepted limitation
   with no owner. Our scheduler is permanent, not a stopgap.

3. **`main`'s stubbing page now says so in plain words** — and this sentence **does not
   exist in the 0.67.0 book at all** (`grep` for `stub_verified` in
   `k067/docs/src/reference/experimental/stubbing.md` returns nothing):
   > "**Function contracts:** Stubbing works with function contracts. Use
   > `#[kani::stub_verified(fn)]` to replace a function with its verified contract
   > abstraction. **This requires a `#[kani::proof_for_contract(fn)]` harness to exist.**"
   > — <https://model-checking.github.io/kani/reference/experimental/stubbing.html>

   The docs have been quietly corrected to match the implementation, downwards, without a
   changelog entry.

**Two further RFC-0009 clauses we had not read, both of which bear directly on D5:**

> "**When specific harnesses are selected (with `--harness`) contracts are not verified.**"

This is enormous and we should treat it as the load-bearing sentence of the whole sweep.
Ply's whole execution model is per-function, per-harness invocation — which is the *one*
mode in which even the aspirational gating is documented not to apply. Whatever 0.67.0
does or does not do in whole-crate mode is irrelevant to us: in Ply's mode, contract
checking is documented off.

> "For mutually recursive functions using `stub_verified`, Kani will check their contracts
> in **non-deterministic order and assume each time the respective other check succeeded**."

Documented circular reasoning for call-graph cycles. `ply-schedule` already lands SCCs in
one batch and refuses to credit them; that decision is now backed by Kani's own text rather
than only by our inference.

### Finding 4 — cross-crate `stub_verified` fails to compile; caller-local `proof_for_contract` works

**DOCUMENTED (as a policy, not as a compile error).** RFC-0009, footnote
`external-contract-checking-expectations`:

> "Contracts for functions from external crates (crates from outside the workspace, which
> is not quite the definition of `extern crate` in Rust) are **not checked by default**.
> **The expectation is that the library author providing the contract has performed this
> check.**"

That is the design intent behind what we hit: Kani has no cross-crate proof channel, so it
declines rather than reasoning across the boundary. Our workaround — declare a caller-local
`proof_for_contract` for the remote `pub` fn and let CBMC check the real linked body — is
undocumented but consistent with RFC-0002's linking model ("we are only able to apply
abstractions to some dependencies if the user enables the MIR linker").

0.67.0 also ships a flag we had not noticed that speaks to exactly this:

```
--no-assert-contracts   Do not assert the function contracts of dependencies.
                        Requires -Z function-contracts
```

i.e. by default 0.67.0 *does* assert dependency contracts. Worth an experiment before M3.

RFC-0009's footnote `assigns-encapsulation-breaking` also warns that cross-crate `modifies`
clauses reference private fields of third-party types and break on any minor version bump
of the transitive dependency — a real hazard for D5's cross-crate story that we have not
yet accounted for.

### Finding 5 — `Vec<u8>` bounded to length 1 times out without an explicit `#[kani::unwind(N+1)]`

**DOCUMENTED — but only by example, never by rule.** The formula is in the book twice and
stated as a rule once, in a context where nobody reading about collections would look for it.

`reference/bounded_arbitrary.md` (identical at 0.67.0 and main) writes the annotation with
no comment on why it is mandatory:

```rust
#[kani::proof]
#[kani::unwind(17)]
fn check_reverse_is_its_own_inverse() {
    // We use BoundedAny to construct a vector that has at most length 16
    let input: Vec<bool> = kani::bounded_any::<_, 16>();
```

Bound 16, unwind 17. The *rule* lives in `reference/attributes.md`, under `#[kani::unwind]`:

> "In general, the required unwinding value is equal to **the maximum number of iterations
> for all loops, plus one**."

and the tutorial repeats it with a warning we should heed:

> "Kani needs the unwinding bound to be 'one more than' the number of loop iterations."
>
> "**NOTE**: Presently, there are some situations where 'number of iterations of a loop'
> can be less obvious than it seems. This can be easily triggered with use of `break` or
> `continue` within loops. Often this manifests itself as needing 'two more' or 'three
> more' iterations in the unwind bound than seems like it would actually run. In those
> situations, we might still need a bound like `kani::unwind(13)`, despite looking like a
> loop bounded to 10 iterations."
> — <https://model-checking.github.io/kani/tutorial-loop-unwinding.html>

**§5.4b's `N+1` codegen rule is therefore correct but not safe as an invariant.** A body
with `break`/`continue` over a bounded `Vec` will fail the unwinding assertion at `N+1`, and
Ply's adapter must treat an unwinding-assertion failure as an *engine-limit* status to
re-try or report, never as a `violation`. Kani's own output tips this:

```
Failed Checks: unwinding assertion loop 0
VERIFICATION:- FAILED
[Kani] info: Verification output shows one or more unwinding failures.
[Kani] tip: Consider increasing the unwinding value or disabling `--unwinding-assertions`.
```

The property class is `unwind` — the one class Kani's own playback filter excludes — which
gives us a clean mechanical discriminator (see §3).

### Finding 6 — fixed-size arrays `[T; N]` need no unwind annotation and are cheap

**DOCUMENTED.** `tutorial-nondeterministic-variables.md`:

> "You can use `kani::any()` for `[T; N]` (if implemented for `T`) **because this array
> type has an exact and constant size**. But if you wanted a slice (`[T]`) up to size `N`,
> you can no longer use `kani::any()` for that. Likewise, there is no implementation of
> `kani::any()` for more complex data structures like `Vec`."

and `reference/arbitrary.md` puts the reason in the trait contract:

> "For a type to implement `Arbitrary`, Kani must be able to represent every possible value
> of it, so **unbounded types cannot implement it**."

§5.4b's preference for fixed arrays is the documented-correct call.

### Finding 7 — `BTreeSet` is intractable at 2 elements even with the unwind fix

**UNDOCUMENTED as a specific limit; the general shape is documented.**
`debugging-slow-proofs.md`:

> "### Complex/Large Non-deterministic Types
> Some types are inherently more expensive to represent symbolically, e.g. strings, which
> have complex validation rules for UTF-8 encoding, or **large bounded collections, like a
> vector with a large size**."

and `tutorial-nondeterministic-variables.md` gives the only quantitative hint anywhere:

> "One thing you'll quickly find is that **the bounds must be very small**. Kani does not
> (yet!) scale well to nondeterministic-size data structures involving heap allocations. A
> proof harness like `safe_update` above, but starting with `any_inventory(2)` will
> probably **take a couple of minutes** to prove."

Two elements, a couple of minutes, on Kani's own tutorial example. Nobody documents that
`BTreeSet` in particular falls over at 2 — that number is ours. Note the blog's
counter-signal: the "Turbocharging Rust Code Verification" post credits union-field constant
propagation with specifically improving `Vec<T>` and `BTreeSet<T>` and turning 5 previously
timing-out harnesses solvable
(<https://model-checking.github.io/kani-verifier-blog/2023/08/03/turbocharging-rust-code-verification.html>).
That work is from 2023 and is already in our 0.67.0. It did not save us.

The issue tracker corroborates our number even though the book does not:

- **#2517 "Memory blows up on a deterministic BTreeSet harness"** — **open** since 2023-06-12,
  `[C] Bug`, `[E] Performance`, `T-CBMC`. A *fully deterministic* `BTreeSet` harness with
  `#[kani::unwind(4)]` and `#[kani::solver(cadical)]` exhausts memory. Deterministic — no
  symbolic input at all. <https://github.com/model-checking/kani/issues/2517>

So `BTreeSet` is a three-year-old open performance bug, not a tuning error on our side.
§5.4b's exclusion is correct and should cite this rather than only our measurement.

### Finding 8 — `HashMap` with the default hasher is a compile error, not a timeout

**UNDOCUMENTED.** No Kani page mentions `HashMap`, `HashSet`, `RandomState`, or hashers at
all. The behaviour is a straightforward consequence of the documented `Arbitrary` rule
(finding 6's quote) plus the orphan rule (finding 10), but the conjunction is never drawn,
and the resulting `rustc` error names `RandomState` rather than anything Kani-shaped. A user
cannot get from that error to "swap the hasher" without help.

**No open issue exists for it either** — searches for `Arbitrary HashMap` and
`hasher RandomState` return nothing relevant. But note that the hasher swap only gets us to
the starting line, not to a verdict: **#3965 "Using function contracts to verify insertion
into HashMap runs into OOM (>8GB)"** (closed, `[E] Performance`) and **#4132 "Slowdown in
hashset performance regression"** (open, 2025-06-05 — enabling `sse`/`sse2`/`neon` target
features regressed `hashset/check_contains_str` by **+2221%**, 33s → 767s) both say the
shape is ruinous even once it compiles. **This is a §5.4b correction: `HashMap`/`HashSet`
should be excluded outright, not promised behind a codegen hasher swap** — the swap fixes
the compile error and delivers a timeout instead, which by §5.4c's own standard is a worse
outcome than an honest `unsupported`.

§5.4b is currently right to say Ply's
codegen must substitute a deterministic hasher itself.

### Finding 9 — recursive / self-referential types don't verify at 3 nodes

**DOCUMENTED as advice; our numbers are new.** `tutorial-real-code.md`, in the list of
things to avoid when starting out:

> "2. **Inductive data structures.** These are data structures with unbounded size (e.g.,
> linked lists or trees.) These can be hard to model since you need to set bounds on their
> size, similar to what happens with loops."
> — <https://model-checking.github.io/kani/tutorial-real-code.html>

RFC-0009 confirms this is a known open frontier rather than a tuning problem, twice — in
Future Possibilities:

> "**Inductive Reasoning:** Describing recursive functions can require that the contract
> also recurse, describing a fixpoint logic. **This is needed for instance for linked data
> structures like linked lists or trees.**"

and in footnote `write-set-recursion`:

> "For inductively defined types the write set inference will only add the first 'layer' to
> the write set. If you wish to modify deeper layers of a recursive type an explicit
> `modifies` clause is required."

So Kani's contract machinery does not model recursive structures at all beyond one layer.
Our "64,147 verification conditions, no result in 180s" is the price tag; the docs' position
is that the feature does not exist. §5.4b's v1 exclusion is not pessimism, it is alignment.

### Finding 10 — private-field invariant types need a hand-written `Arbitrary` in the defining crate

**DOCUMENTED.** The FAQ names the error verbatim:

> "I implemented the `kani::Arbitrary` trait for a type that's not from my crate, and got
> the error `only traits defined in the current crate can be implemented for types defined
> outside of the crate`. What does this mean? What can I do?
>
> This error is due to a violation of Rust's orphan rules for trait implementations ... In
> that case, you'll need to write a function that builds an object from non-deterministic
> variables."
> — <https://model-checking.github.io/kani/faq.html>

`tutorial-nondeterministic-variables.md` gives the same escape hatch and names the exact
constraint we hit:

> "This approach is also necessary when you need to generate a nondeterministic variable of
> a type that you're importing from another crate, since Rust doesn't allow you to implement
> a trait defined in an external crate for a type that you don't own."

ADR-0003 caveat 2 (witnesses via the `pub` smart constructor + `kani::assume`) is the
documented recommended workaround, arrived at independently. Kani also points at a third
option we have not considered: implementing `bolero_generator::TypeGenerator` instead, which
serves Kani *and* property testing from one generator — potentially relevant to Ply, which
has to produce both a `bounded` harness and a `fuzz(n)` harness for the same type.

---

## 2. Are our findings actually Kani's limits, or our mistakes?

The challenge that prompted this section is a good one: Kani is AWS's tool, `verify-rust-std`
verifies the Rust standard library with it, and if Kani genuinely fell over on a `BTreeSet` at
two elements that project could not exist. The right prior is that we were driving badly.

I went and read how the professionals do it. **The prior is partly right and importantly
wrong, and the split is not where I expected.** Two of the ten are substantially our mistake,
one is our mistake in its recommended *fix* rather than its diagnosis, and the rest are
limits that Kani's own developers encode in their own regression suite.

### Evidence base

- **`github.com/model-checking/verify-rust-std`** @ `main`, full `library/` tree — the actual
  std verification effort. 75 `#[kani::proof]`, **177 `#[kani::proof_for_contract]`**, 17
  `#[kani::stub_verified]`, 20 `#[kani::unwind]`, 8 `#[kani::loop_invariant]`, 325
  `requires`/`ensures`, and **zero** uses of `bounded_any`.
- **Kani's own test suite** (`tests/`), which is CI-enforced: `tests/expected/`,
  `tests/perf/`, `tests/kani/`.
- The `verify-rust-std` challenge book (`doc/src/challenges/`), which records which data
  structures have been verified, by whom, and **with which tool**.

### Verdict, finding by finding

| # | Finding | Verdict |
|---|---|---|
| 1 | playback doesn't reproduce `ensures` violations | **Kani's limit** — and undocumented |
| 2 | timeout and failure print identically | **Ours** — we never passed `--harness-timeout` |
| 3 | `stub_verified` doesn't check the proof passed | **Kani's limit** — maintainer-confirmed |
| 4 | cross-crate `stub_verified` doesn't compile | **Kani's limit** — by design |
| 5 | `Vec` needs an explicit unwind | **Mixed** — rule is right, but we used the wrong API |
| 6 | fixed arrays are cheap and need no annotation | **Confirmed, and we undersold it badly** |
| 7 | `BTreeSet` intractable at 2 | **Kani's limit** — encoded in Kani's own test suite |
| 8 | `HashMap` default hasher is a compile error | **Diagnosis right, our prescribed fix wrong** |
| 9 | recursive types don't verify at 3 nodes | **Kani's limit** — nothing in either repo does this |
| 10 | private-field types need an in-crate `Arbitrary` | **Kani's limit** — it's the Rust orphan rule |

### The three where we were wrong

**Finding 2 — ours.** Covered in §1: `--harness-timeout` (behind `-Z unstable-options`) makes
Kani print a distinct `CBMC failed` / `CBMC timed out` block with **zero properties**, where a
real violation carries properties. We killed harnesses externally and then concluded Kani
could not tell us apart. It can. §5.4c's MUST stands, but its stated justification ("Kani
renders a CBMC timeout and a genuine contract failure identically") is **factually wrong and
should be corrected** — the correct justification is that the distinction exists only if you
ask for it, and that `--output-format=old` destroys it.

**Finding 5 — the rule is right, the API was wrong.** `#[kani::unwind(N+1)]` for a
`bounded_any::<Vec<_>, N>` is exactly what Kani's own test encodes
(`tests/expected/bounded-arbitrary/reverse_vec/`: bound 4 → `unwind(5)`, bound 5 →
`unwind(6)`, bound 16 → `unwind(17)`). So §5.4b's codegen rule is correct.

But **`bounded_any` on a `Vec` is not how anyone builds a symbolic vector.** It is used **zero
times in all of `verify-rust-std`**. There are three better APIs, and we used none of them:

```rust
// 1. EXACT length, no unwind annotation at all — tests/expected/any_vec/
let data = kani::vec::exact_vec::<Dummy, 17>();     // 17 elements, no #[kani::unwind]

// 2. Up-to-N length — tests/kani/NondetVectors/, needs a *body-sized* unwind
#[kani::unwind(22)] let data = kani::vec::any_vec::<u8, 8>();   // note: 22, not 9

// 3. What verify-rust-std actually does — library/alloc/src/vec/mod.rs
const ARRAY_LEN: usize = 3;
let arr: [i32; ARRAY_LEN] = kani::Arbitrary::any_array();
let mut vect = Vec::from(&arr);                      // no unwind annotation
```

Kani's own issue **#1322 "`kani::vec::any_vec` performance is subpar compared to
`exact_vec`"** (open, `[E] Performance`) confirms the ranking. **§5.4b should name
`exact_vec` / `any_array` + `Vec::from` as the preferred `Vec` construction and demote
`bounded_any`.** Also note the `any_vec::<u8, 8>` case needs unwind **22**, not 9 — the
`N+1` rule is about *the body's loop count*, not the collection's length, exactly as
`reference/attributes.md` says ("the maximum number of iterations for all loops, plus one").
§5.4b's "sized to the declared bound" phrasing is a simplification that is true only when the
body loops once. That is a latent codegen bug worth fixing before it ships.

**Finding 8 — right that it fails, wrong about the fix.** §5.4b says *"Ply's codegen must
substitute a deterministic hasher itself."* That is not the technique. Kani's own perf test
(`tests/perf/hashset/src/lib.rs`) **stubs the hasher constructor**:

```rust
fn concrete_state() -> RandomState {
    let keys: [u64; 2] = [0, 0];
    assert_eq!(size_of_val(&keys), size_of::<RandomState>());
    unsafe { transmute(keys) }
}

#[kani::proof]
#[kani::stub(RandomState::new, concrete_state)]
#[kani::unwind(5)]
#[kani::solver(kissat)]
fn check_insert() {
    let mut set: HashSet<i32> = HashSet::default();
    ...
}
```

and `tests/expected/bounded-arbitrary/hash/hash.rs` sidesteps it entirely by leaving the
hasher parameter inferred:

```rust
let hash_map: std::collections::HashMap<u8, bool, _> = kani::bounded_any::<_, BOUND>();
```

Both are cheaper and less invasive than substituting a type in the user's signature. **But
neither makes `HashMap` *tractable*.** Kani's own test caps `BOUND` at 1 with the comment
*"A larger bound causes this to take a long time"*, and the perf test needs `-Z stubbing` plus
a non-default solver to handle **one insertion**. So §5.4b's exclusion of `HashMap` survives;
only its stated remedy needs rewriting. And note the remedy uses plain `#[kani::stub]`, which
per §3 item 2 costs us concrete playback at 0.67.0.

### The one where we were wrong in our own favour

**Finding 6 — fixed arrays are far better than §5.4b claims.** We wrote *"cheap at every N
measured up to 16"*. The professionals routinely go an order of magnitude higher. Across
`verify-rust-std`'s harnesses the fixed-array bounds in use are `MAX_SIZE = 32` (13 harnesses),
`ARRAY_LEN = 40`, and `MAX_LEN = 512`. Their slice-iterator harnesses build a fixed array and
then take a *symbolically bounded subslice* of it:

```rust
// library/core/src/slice/iter.rs
fn any_slice<T>(orig_slice: &[T]) -> &[T] {
    let last  = kani::any_where(|idx: &usize| *idx <= orig_slice.len());
    let first = kani::any_where(|idx: &usize| *idx <= last);
    &orig_slice[first..last]
}
let array: [$elem_ty; MAX_LEN] = kani::any();
let mut iter = any_iter::<$elem_ty>(&array);
```

That is a symbolic-length sequence with **no heap allocation, no loop, and no unwind
annotation** — the cost of a symbolic length is paid in two `usize` constraints rather than in
CBMC's allocator. It is the single best technique in either repository and Ply's codegen should
adopt it.

A second technique from the same source, worth stealing outright: **quantify with a symbolic
index instead of a loop.** From `library/alloc/src/vec/mod.rs`, checking "every other element is
unchanged" without iterating:

```rust
let k = kani::any_where(|&x: &usize| x < original_len - 1);
if k != index {
    assert!(vect[k] == arr[k]);
}
```

One symbolic index replaces an N-iteration loop and its unwinding assertion. **Ply's generated
contract assertions over sequences should be lowered this way, not as `for` loops.**

### The six where the limit is real

**Findings 7 and 9 are the ones the challenge was aimed at, and both survive it.**

**Finding 7 — `BTreeSet`.** Kani's *own regression test*, written by Kani's developers:

```rust
// tests/expected/bounded-arbitrary/btree/btree.rs
#[kani::proof]
#[kani::unwind(5)]
fn check_btreeset() {
    // a larger bound causes this to take a long time, see bounded-arbitrary/hash
    const BOUND: usize = 1;
    let btree_set: std::collections::BTreeSet<u8> = kani::bounded_any::<_, BOUND>();
```

Bound **1**, with an in-source comment giving our exact reason. The perf suite's
`tests/perf/btreeset/insert_any` documents that inserting **one** nondeterministic element
*"takes ~10 seconds and consumes ~255 MB"*. Open issue **#2517** reports a *fully
deterministic* `BTreeSet` harness exhausting memory, unfixed since 2023. And in
`verify-rust-std`, `library/alloc/src/collections/btree/` contains **zero `kani::proof`
harnesses** — challenge **#4, "Memory safety of BTreeMap's `btree::node` module", carries a
$10,000 reward, an end date of 2025-04-10, and status *Open***. Nobody has claimed it.

One genuine refinement: `tests/perf/btreeset/insert_multi` does reach `N = 2` — but by
*incremental construction* (`BTreeSet::new()` then two `.insert(kani::any())` calls) with
`#[kani::unwind(3)]` and `#[kani::solver(cadical)]`, not by making a whole set symbolic. If we
ever need two elements, that is the shape. It does not rescue a `BTreeSet`-shaped *field* of an
arbitrary struct, which is our case.

**Finding 9 — recursive types. The strongest result in this section.**

- **Kani's entire test suite contains no nondeterministic recursive data structure.** `Box<Self>`
  appears only as a method receiver (`tests/expected/trait-receiver/`,
  `tests/kani/FunctionContracts/receiver_contracts.rs`). `tests/smack/recursion/` is recursive
  *functions* (factorial, fibonacci), not recursive *types*. There is no linked-list, tree, or
  arena test anywhere in CI.
- **`verify-rust-std` challenge #5 is "Verify functions iterating over inductive data type:
  `linked_list`", reward $20,000, status *Resolved*** — and
  `library/alloc/src/collections/linked_list.rs` contains **zero Kani annotations**. It was
  solved by Bart Jacobs with **VeriFast**, a separation-logic tool the project had to onboard
  as a separate approved tool precisely because it does modular, unbounded reasoning over
  inductive structures. The challenge text states the requirement Kani cannot meet: *"The
  verification must be unbounded — it must hold for linked lists of arbitrary shape."*
- RFC-0009 lists inductive reasoning under **Future possibilities**, not features, and its
  footnote states that write-set inference *"will only add the first 'layer'"* of a recursive
  type.

So: the flagship project needed a different class of tool for exactly our shape, and paid
$20,000 for it. **Our 64,147 verification conditions at 3 nodes is not a harness mistake. It is
the reason VeriFast is in that repository.** §5.4b's v1 exclusion is correct and should cite
challenge #5 rather than only our own timing.

**Findings 1, 3, 4, 10** are unchanged by this review — 1 and 3 are confirmed by Kani's own
source and by a maintainer's own words (§1), 4 is documented design intent, and 10 is the Rust
orphan rule, which no amount of skill routes around.

### One correction to the prior that cuts the other way

The hypothesis was that we took "the least-trodden path" by combining `-Z function-contracts`
with `-Z stubbing`. Half right, and the half that is right is worse than the hypothesis.

- **Contracts are the mainstream path, not the fringe.** `proof_for_contract` outnumbers plain
  `proof` in `verify-rust-std` by 177 to 75. Our use of contracts is orthodox.
- **`stub_verified` is essentially untrodden.** All 17 uses in the entire std verification
  effort are in **one file** (`library/core/src/intrinsics/mod.rs`), all stubbing **one
  function** (`transmute_unchecked_wrapper`). Nobody uses it for compositional call-graph
  reasoning — the thing D5 is built on.

That is not reassuring, it is the opposite: **the mechanism at the centre of Ply's design is
one the flagship user has essentially declined to use.** It also explains why finding 3's hole
went unnoticed for so long and why no open issue tracks it. We are not misusing a well-worn
feature; we are the load test.

### A harness sketch for the case that decides everything

Ply's kernel is a recursive verdict tree, and finding 9 says Kani cannot take it as such. The
evidence above also says exactly what shape Kani *can* take at size 32–512: **a fixed-size
array with symbolic indices**. So the fix is not to shrink the tree, it is to stop making the
tree a *type* and make it *data in an arena*:

```rust
// Sketch to test — NOT a proposal to reshape the kernel's public API.
const N: usize = 8;                     // nodes; professionals run 32+ on this shape

#[derive(kani::Arbitrary, Copy, Clone)]
struct FlatNode {
    verdict: u8,                        // encoded Evidence
    statuses: u16,                      // the existing StatusSet bitmask
    first_child: u8,                    // index into the arena, or N for "none"
    n_children: u8,
}

#[kani::proof_for_contract(aggregate_flat)]
fn check_aggregate() {
    let arena: [FlatNode; N] = kani::any();          // no heap, no unwind annotation

    // Well-formedness as assumptions, not as a constructed shape:
    // children strictly after parents => acyclic, no recursion needed.
    let i: usize = kani::any_where(|&i| i < N);      // symbolic index, not a loop
    kani::assume(arena[i].first_child as usize > i || arena[i].n_children == 0);
    kani::assume((arena[i].first_child as usize).saturating_add(
                  arena[i].n_children as usize) <= N);

    aggregate_flat(&arena, 0);
}
```

Three properties make this worth a measurement rather than a guess. The arena is `[T; N]`, the
one shape both repositories scale to the tens or hundreds. The acyclicity assumption
(`first_child > i`) replaces recursion with a **single reverse pass**, so `aggregate_flat` is a
bounded `for` loop over a constant-size array rather than a recursive call — the shape
`tests/perf/` is full of. And the per-node property is checked at one symbolic index, the
`verify-rust-std` `swap_remove` technique, so there is no inner loop to unwind.

**Caveats, stated plainly.** This is untested — it is a sketch to run, not a result. It is a
*representation* change to Ply's own kernel, which TODO.md already records as rejected once
("the stall just moves to the next unbounded field") — though that attempt swapped a `BTreeSet`
for a bitmask and kept the recursive `Box`/`Vec<Self>` shape, which is a different change from
flattening the recursion itself. And §8's "Ply proposes, never rewrites" applies to *users'*
data structures; the kernel is ours, so we may take our own advice. If it works, `bounded` is
central and §5.4b gains an "arena-encodable" category. If it does not, finding 9 is final and
`bounded` is a niche verdict that the evidence order must stop flattering.

Either way this is one afternoon and it is the highest-value experiment on the list — it decides
whether Ply's headline check applies to Ply's own headline data structure.

---

## 3. What we're about to rediscover

Ranked by what it would cost us to learn the hard way. Everything here is documented and we
have not yet hit it.

### 1. `--harness` turns contract checking off. Documented. This is D5's whole ground.

RFC-0009: *"When specific harnesses are selected (with `--harness`) contracts are not
verified."*

Ply invokes Kani per function. That is `--harness` mode, permanently. Every reassurance in
RFC-0009 about contracts being checked before stubs are honoured is scoped to whole-crate
mode and does not reach us. **Our scheduler is not merely compensating for a 0.67.0 bug — it
is supplying a guarantee Kani documents itself as not providing in the mode we use.** ADR-0003
caveat 1 and D5 both currently frame this as "RFC-0009 promises X, 0.67.0 doesn't do X",
which understates it: even a fixed Kani would not do X for us. If a future Kani implements
the gating and we are tempted to delete `ply-schedule`, this sentence is why we must not.
Cost of learning late: we relax the scheduler on a Kani upgrade and silently produce unsound
`bounded` verdicts. There is nothing downstream to catch it.

### 2. At 0.67.0, `-Z stubbing` requires `--harness` **and** is incompatible with `--concrete-playback`

This is the second load-bearing sentence of the sweep, and it is in the 0.67.0 book only —
the paragraph was **deleted on `main`**, so reading the published site would have hidden it.
`k067/docs/src/reference/experimental/stubbing.md` §Limitations, under the heading *"In the
following, we describe **all** the limitations of the stubbing feature"*:

> "### Usage restrictions
>
> The usage of stubbing is limited to the verification of a single harness. Therefore, users
> are **required to pass the `--harness` option** when using the stubbing feature.
>
> In addition, this feature **isn't compatible with concrete playback**."

RFC-0002 §Limitations says the same thing from the other side: *"Our proposed approach will
not work with `--concrete-playback` (for now)."*

Two consequences, both immediate:

1. **Composed with §3 item 1, the gating is structurally impossible at our version.** Stubbing
   requires `--harness`; `--harness` is the mode in which RFC-0009 documents that *"contracts
   are not verified"*. There is no configuration of Kani 0.67.0 in which the callee's contract
   proof gates the caller's stubbed verdict. **`ply-schedule` is not a workaround for a bug —
   it is the only mechanism that exists.** ADR-0003 caveat 1 and D5 should say this; both
   currently describe it as Kani failing to honour its own RFC, which is true but weaker than
   the real situation.
2. **D7 and `#[kani::stub]` are mutually exclusive at 0.67.0.** If a Ply harness carries a
   plain `#[kani::stub(orig, mock)]` — which §5.4c's engine-limit `fixes` and any future
   "stub the expensive callee" advice would produce — then a failure yields *no witness at
   all*, and §8's rule "MUST NOT emit a `violation` without a witness" forces the verdict to
   something other than `violation`. Worth pinning as a test before we generate our first
   stub.

   **RETRACTED 2026-08-25, by measurement** (`tests/spike/kani-pin/FINDINGS.md`). The book
   sentence quoted above is stale about its own release: at 0.67.0 a stubbed harness that
   fails **does** print a full concrete-playback witness, including the value the stub
   invented, and Ply's `extract_witness_bytes` accepts it. This paragraph's inference from
   the doc was wrong, and this is exactly the case the standing rule at the top of this
   file covers — measurement wins. What *is* broken, identically at 0.67.0 and on `main`,
   is one step later: the generated playback test **does not apply the stub** (Kani says so
   itself in the test's doc comment, from
   `kani-driver/src/concrete_playback/test_generator.rs`), so replaying a stub-caused
   failure panics on leftover concrete values rather than reproducing the failure. Silent,
   and worse than the limitation this paragraph claimed.

The scope caveat, stated plainly because it matters: this paragraph is about `-Z stubbing`
and `#[kani::stub]`. We have *not* established that it binds `#[kani::stub_verified]`
identically — indeed our own whole-crate run exercised `stub_verified` without `--harness`.
Both restrictions are **lifted on `main`** ("Stub annotations are specified per-harness. When
a crate contains multiple harnesses with different stub configurations, each harness is
verified independently"; "**Concrete playback:** Stubbing is compatible with
`--concrete-playback`"), which is a real argument for bumping the pin — and a D13-shaped
spike question, not something to settle from documents.

**Spiked 2026-08-25; the argument did not survive** (`tests/spike/kani-pin/FINDINGS.md`).
Kani `main` @ `2457093` was built from source and run against Ply's own shapes beside the
untouched 0.67.0. The `--concrete-playback` restriction was already absent at 0.67.0, and
`main`'s stronger claim — "Kani can generate a concrete test case that reproduces the
failure using the stub's behavior" — is contradicted by `main`'s own source and by the
run. Nothing was gained by moving, so the recommendation is to stay at 0.67.0.

`main` also documents stubbing capabilities that do not exist at 0.67.0 and that we should
not design around yet: `kani::stub_set!` / `#[kani::use_stub_set(..)]` for reusable composable
stub sets, stubbing of `extern "C"` foreign functions, and trait-method stubbing via
fully-qualified syntax including through `dyn` dispatch (*"Stubs apply even when the method is
called through a trait object"*). Both versions agree on the standing advice that bears on
Ply's D5: *"In general, we don't recommend stubbing for private functions/methods. Doing so
can lead to brittle proofs."*

### 3. Four open bugs land squarely on the configuration Ply has already chosen

These are not general Kani caveats. Each one is a currently-open bug that fires on a choice
already written into the spec, and each was found in the tracker rather than in the docs.

**(i) `--quiet` turns a failing verification into exit code 0.** Open, filed against 0.67.0
on 2026-08-18, unlabelled:

> **#4745** — *"not sure if issue, more a question, is this expected or not: `--quiet` makes
> a failing verification run exit 0"*. `kani f.rs` exits 1; `kani f.rs --quiet` exits 0 on
> the same failing harness. Root-caused in the report to `print_final_summary` early-returning
> under `--quiet` before `std::process::exit(1)`.
> <https://github.com/model-checking/kani/issues/4745>

Ply is precisely the kind of consumer that reaches for `--quiet` — we parse structured-ish
output and do not want Kani's chatter in our own. **Do not pass `--quiet`,** and do not gate
anything on Kani's exit code alone. Related: **#4746** (deterministic SIGFPE, exit 136, in
CBMC 6.8.0 via Kani 0.67.0 unwinding `core`'s `u128::pow`) is another exit-status-versus-verdict
confusion on our exact pinned pair. Cost of learning late: a green CI run over a broken
codebase — the single worst outcome this project can produce.

**(ii) Contract stubs fail to compile on slice references — which §5.4b explicitly admits.**
Open, filed **three days ago**:

> **#4748 "Verified function contract stubs fail compilation on slice references"** —
> `[C] Bug`, `Z-Contracts`, 2026-08-20. Fails on `&[u8]` and `&mut [u8]` arguments. The
> reporter's motivating case is a real `std::io::Write` impl that takes **1h40 and 72 GB**
> without stubbing. <https://github.com/model-checking/kani/issues/4748>

§5.4b's supported list ends with *"`&T`/`&[T]` of the above (built from an owned value in the
harness)"*. That entry is measured for plain proofs but **not** for the `stub_verified` path
D5 depends on. See also **#3682** ("Failed to `stub_verified` contracts with slices in
`kani::modifies`" — open since 2024-11-05, compiler panic in `reachability.rs:446`). A
cheap M0-style spike settles whether §5.4b needs to split its `&[T]` entry into
"provable" and "stubbable".

**(iii) `#[kani::proof]` and `#[kani::proof_for_contract]` disagree about a violated
`requires`.** Ply uses `proof_for_contract` exclusively (§5.4c):

> **#4021 "Verification results of `#[proof]` and `#[proof_for_contract]` are different"** —
> open, `Z-Contracts`, `T-User`, 2025-04-15. A `requires(a > 0)` violated by calling
> `contract(0)` **fails** under `#[kani::proof]` but **succeeds** under
> `#[kani::proof_for_contract]`. <https://github.com/model-checking/kani/issues/4021>

That is the vacuous-success hazard of §3 item 9, in the specific form Ply will meet it:
`proof_for_contract` *assumes* the precondition, so a harness that can only reach states the
`requires` excludes proves nothing and reports SUCCESS. It is the strongest argument for the
`kani::cover` guard proposed there.

**(iv) Plain `#[kani::stub]` on a contracted function does not compile.**

> **#4591 "`#[kani::stub]` on contracted functions"** — open, `[C] Bug`, `Z-Contracts`,
> 2026-04-23. `#[kani::stub(target, repl)]` where `target` carries `#[kani::ensures]` fails
> with `Failed to find contract closure __kani_recursion_check_<fn>`.
> <https://github.com/model-checking/kani/issues/4591>

D5's fallback path — a callee that is merely fuzzed or tested, verified under an *assumed*
contract with a `conditional` verdict — must not be implemented as a plain `#[kani::stub]`
over a contracted callee. That combination is broken today.

**Confirmed 2026-08-25, and still broken on `main`** (`tests/spike/kani-pin/FINDINGS.md`):
the same error, word for word, from a source build of `main` @ `2457093`. #4591 is not a
release-lag problem.


### 4. `autoharness` is converging on Ply's M4, fast, and it already handles cases we don't

`-Z autoharness` exists in our 0.67.0. It scans a crate, generates `#[kani::proof]` harnesses
for every function whose arguments implement or can derive `Arbitrary`, generates
`#[kani::proof_for_contract]` instead when the function has a contract, and runs them —
without touching source:

> "Kani scans the crate for functions whose arguments all implement the `kani::Arbitrary`
> trait, generates harnesses for them, then runs them. **These harnesses are internal to
> Kani — i.e., Kani does not make any changes to your source code.**"
>
> "if Kani detects that `foo` has a function contract, it will instead generate a
> `#[kani::proof_for_contract]` harness and verify the contract"
>
> "**Kani will detect if a struct or enum could implement `Arbitrary` and derive it
> automatically.** Note that this automatic derivation feature is only available for
> autoharness."
> — <https://model-checking.github.io/kani/reference/experimental/autoharness.html>

That last capability is one Ply does not have and would have to build. It also takes
`--include-pattern`/`--exclude-pattern` regexes (function paths prefixed with the crate
name), and `--list --format json`.

The delta between 0.67.0 and `main` shows the trajectory, and it is steep. Everything in
this list is `[main only]`, i.e. landed in the **seven months** since our pinned release
(0.67.0 was published 2026-01-16 and is *still* the newest release as of today; `main` has
run ahead unreleased ever since):
`--bounded-arguments` (auto-bounded `&[T]` up to 16 elements, `&str` up to 4 bytes,
`BoundedArbitrary` types at bound 4 via `kani::bounded_any`); raw-pointer arguments in a
nondeterministic allocation state; `Invariant`-trait-aware value generation; and generic
functions verified at one monomorphic instantiation:

> "For a generic function, Kani generates a harness for a single monomorphic instantiation
> of the function: it substitutes every type parameter with the first candidate from a fixed
> list of primitive types (starting with `i32`) such that all of the function's trait bounds
> are satisfied ... `usize` const generic parameters (e.g. array lengths) are instantiated
> with the value 2."

That is §5.4b's `check_with: { T: u64 }` mechanism, built by Kani, with the same
underapproximation caveat we wrote and the same mitigation (display the instantiated name):

> "verifying a single instantiation is an underapproximation of all of the function's
> possible behaviors ... Kani makes this explicit by displaying the instantiated name of the
> verified function."

**Two limitations of theirs we should copy into §5.4b before we hit them ourselves,** both
`[main only]` and both of which apply equally to any harness Ply generates:

> "Each reference, pointer, slice, or string argument is generated from its own independent
> nondeterministic storage. Autoharness therefore does *not* explore aliasing *between*
> distinct arguments: for example, given `fn f(a: &T, b: &T)`, the generated harness always
> passes two references to separate allocations, so `a` and `b` never share an address
> (`core::ptr::eq(a, b)` is always `false`), even though a caller could pass the same
> reference twice. **A successful automatic harness is thus an underapproximation with
> respect to caller-controlled aliasing.**"

§5.4b admits `&T`/`&[T]` "built from an owned value in the harness" and says nothing about
aliasing. A Ply `bounded` verdict on `fn f(a: &mut T, b: &T)` therefore quietly excludes the
aliased case. That is an evidence-honesty defect of exactly the kind §1 exists to prevent,
and it is cheap to fix now (a sentence in §5.4b plus a `conditional`-style note) and
expensive to fix after someone trusts such a verdict.

> "Kani assumes that the nondeterministic struct and enum values it generates for automatic
> harnesses respect the type's safety invariant, i.e., each generated value `v` satisfies
> `v.is_safe()` ... Note that automatic harnesses do not *assert* type invariants, e.g.,
> they do not check that a function's return value satisfies `is_safe()`."

Cost of learning late: we spend M4 building a harness generator whose distinguishing
features Kani ships for free, and we ship two underapproximations (aliasing, invariants) as
unqualified `bounded`.

### 5. Loop contracts require crate-root nightly features — which Ply may not add

`reference/experimental/loop-contracts.md` puts these at the top of every single example:

```rust
#![feature(stmt_expr_attributes)]
#![feature(proc_macro_hygiene)]
```

`#[kani::loop_invariant(..)]` is a *statement attribute on a loop expression*, and Rust
requires both crate-level features to permit that. They are crate-root attributes: they must
be in the user's `lib.rs`, not in generated harness code. Even `#![cfg_attr(kani,
feature(...))]` is an edit to the user's crate root.

**§5.4c already lists `induct` (loop contracts) as planned-not-v1 for the right reasons.
This is a third, harder reason, and it collides head-on with "Ply proposes, never rewrites"
(§8).** Ply cannot emit a loop contract without editing a file it has no licence to edit.
Any future `induct` design must be a `fixes` proposal a human applies, never an adapter
action. Cost of learning late: an M-series milestone gets planned around loop contracts and
dies on contact.

### 6. Machine-readable output exists — but not in our version. Do not build a text parser.

See §3(a). Ranked here because the cost is a whole subsystem written twice.

### 7. `-j` parallel verification requires `--output-format=terse`

From `kani-driver/src/args/mod.rs` (0.67.0): *"Conflicting options: `--jobs` requires
`--output-format=terse`"*, and `--concrete-playback` is rejected outright with multi-threaded
`--jobs` because *"concrete playback currently embeds a lot of assumptions about the order in
which harnesses get called."*

Ply wants both parallelism and witnesses. **They are mutually exclusive in 0.67.0.** Since
D7 now generates witnesses for every falsified claim by default, Ply's Kani adapter is
single-threaded-per-invocation whenever a counterexample is possible — which is always, since
we don't know it failed until it fails. The parallelism has to come from Ply running multiple
`cargo kani --harness X` processes, not from `-j`. That also happens to be the shape D5's
scheduler already produces. Worth writing down before someone "optimises" it.

### 8. Unwinding-assertion failure is a distinct property class, and it is how you detect exhaustion mechanically

Property names are `<function>.<class>.<number>` and `attributes.md` documents the class
field as the discriminator (`should_panic` uses it: *"we check if its class is `assertion`"*).
The unwinding case has class `unwind`, is excluded from playback generation by name in the
0.67.0 source, and produces `Failed Checks: unwinding assertion loop 0`.

That gives Ply three mechanically distinguishable engine-limit signals, none of which is a
violation, and all of which §5.4c requires us to separate:

| signal | how to detect | Ply status |
|---|---|---|
| solver exhaustion | `Err(ExitStatus::Timeout)` → `CBMC failed` header, zero properties, needs `--harness-timeout` | `timeout` |
| out of memory | `CBMC failed` header + "run out of memory" | `timeout` (distinct cause in `fixes`) |
| bound too low | any property with class `unwind` and status `FAILURE` | *not* a violation — raise bound or report |
| unsupported construct | property status `UNDETERMINED` | `unsupported` |

That last row is one we had not planned for. `verification-results.md`:

> "4. `UNDETERMINED`: This indicates that Kani was not able to conclude whether the property
> holds or not. This can occur when the Rust program **contains a construct that is not
> currently supported by Kani**."

and `rust-feature-support.md` explains the mechanism, which is nastier than a hard error:

> "the general rule is that Kani generates an `assert(false)` statement followed by an
> `assume(false)` statement when compiling any unsupported feature. `assert(false)` will
> cause verification to fail **if the statement is reachable** ... However, the analysis will
> not be affected **if the statement is not reachable** from the code under verification."

So an unsupported construct on a *reachable* path is a FAILURE that looks like a violation
and has no meaningful counterexample; on an unreachable path it is invisible. §8's rule that
a `violation` MUST carry a witness is the right guard, but the adapter must also positively
recognise `UNDETERMINED` and the unsupported-construct failure shape, or we will report
"your contract is broken" for `asm!` in a branch.

Cost of learning late: exactly the evidence-lying failure §5.4c was rewritten to prevent,
reintroduced through a channel we hadn't enumerated.

### 9. `SUCCESS` can be vacuous, and Kani says so

`verification-results.md`:

> "1. `SUCCESS`: This indicates that the check passed (i.e., the property holds). Note that
> in some cases, **the property may hold _vacuously_**. This can occur because the property
> is unreachable, or because the harness is **_over-constrained_**."

and:

> "In contrast to an `UNREACHABLE` result for assertions, an unreachable (or an
> unsatisfiable) cover property **may indicate an incomplete proof**."

A `requires` clause that is too strong yields a green `bounded` verdict that proves nothing.
Ply currently has no defence against this: §5.4a constrains the *syntax* of contract
expressions, not their satisfiability. Kani's own recommended defence is `kani::cover`, and
the FAQ makes it the standard diagnostic:

> "If you didn't expect certain checks in a harness to be `UNREACHABLE`, we recommend using
> the `kani::cover` macro to determine what conditions are possible in case you've
> over-constrained the harness."

Cheap concrete option: Ply's generated `proof_for_contract` harness emits a
`kani::cover!(true)` after the call. If it comes back `UNSATISFIABLE`, the `requires` is
contradictory and the verdict must be downgraded, not shown green. Also
`--coverage -Z source-coverage` gives per-line coverage, which `tutorial-real-code.md`
recommends for exactly this: *"Whether you're over-constrained. Check the coverage report
using `--coverage -Z source-coverage`. Ideally you'd see 100% coverage, and if not, it's
usually because you've assumed too much."*

Cost of learning late: high, and quiet. A vacuous proof is the most expensive kind of lie
this tool can tell, and today nothing in Ply detects it.

### 10. `modifies` inference is coarse and gets `RefCell` wrong

RFC-0009: Kani infers the write set from argument types — *"any data pointed to by a mutable
reference or pointer is considered part of the write set"* — and havocs all of it, so
`&mut self` on a `Vec` havocs every element. Narrowing it needs an explicit `#[modifies(...)]`
whose place expressions *"break Rust's `pub`/no-`pub` encapsulation"* (the `Vec::pop` example
in the RFC reads `(*self).buf.ptr.pointer.pointer[self.len]`).

And footnote `inferred-footprint`:

> "While inferred memory footprints are sound for both safe and unsafe Rust **certain
> features in unsafe rust (e.g. `RefCell`) get inferred incorrectly and will lead to a
> failing contract check**."

Ply's spec expression subset (§5.4a) has no `modifies`. Any contracted `&mut self` method
over a collection will be slow for a documented reason, and any type with interior mutability
will fail its contract check for a reason that is not the user's bug. Both need naming in
§5.4b/§8 as engine-limit diagnostics with real `fixes`.

### 11. Quantifiers exist, are `usize`-only, and are slower through array indexing than raw pointers

`-Z quantifiers` is in 0.67.0. `kani::forall!(|i in (0,10)| ...)` / `kani::exists!`.

> "We now assume that all quantified variables are of type `usize`."
>
> "the performance of quantifiers can be affected by the depth of call stacks in the
> quantified expressions ... **array indexing in Rust leads to a deep call stack**, which can
> cause issues with quantifiers. To mitigate this, consider using *unsafe* pointer
> dereferencing instead of array indexing"
> — <https://model-checking.github.io/kani/reference/experimental/quantifiers.html>

Relevant to §5.4a: if we ever want "for all elements of this slice, P" in a Ply contract, the
naive lowering is the documented-slow one. Low urgency; §5.4a doesn't have quantifiers today.

### 12. `--synthesize-loop-contracts` exists in 0.67.0 and nobody has tried it

RFC-0004's `goto-synthesizer` integration ships as a plain flag on our binary. It attempts to
synthesise loop invariants so CBMC need not unwind:

> "With the loop-contract synthesizer, Kani can synthesize the loop invariant `y >= 0`, with
> which it can prove the post-condition `y == 0` without unwinding the loop."

It requires no source annotation, so it dodges finding #3's crate-feature wall entirely. It is
the cheapest untried lever we have against the unwinding wall — an afternoon, not a milestone.
Worth a spike alongside the outstanding iterator-chain measurement in TODO.md.

### 13. Solver choice is worth up to 200×, and is a per-harness attribute

The blog's headline numbers: 2–8× typical, and `random::tests::gen_range_biased_test` went
from **1460s under MiniSat to 5.5s under Kissat**; per-harness optimal solver selection cut
total runtime on `s2n-quic-core` by 85%. Kissat was fastest on 47% of harnesses running >10s,
CaDiCaL on 24%; MiniSat wins on sub-second harnesses.

0.67.0 exposes `--solver` and `#[kani::solver(..)]` with `bitwuzla, cadical, cvc5, kissat,
minisat, z3, bin=<binary>`. `attributes.md` warns the default is not stable across versions:

> "Note that the default solver may vary depending on Kani's version. We highly recommend
> users to annotate their harnesses if the choice of solver has a major impact on
> performance, even if the solver used is the current default one."

Ply's fingerprint (D14) already records "engine name + version + flags". **The solver must be
in that flag set**, or a cached passing verdict can be replayed against a different solver
and mean something different. Cheap to get right now.

Note z3/bitwuzla/cvc5 were added in 0.65.0 and are *not packaged with Kani* — they must be on
PATH, which is a `W01xx` environment diagnostic if Ply ever selects one.

### 14. `--prove-safety-only` — a cheap second verdict shape we don't have

0.67.0: *"Compute verification results under the assumption that no panic occurs."* Added in
0.65.0 as *"a new `--prove-safety-only` option for focused safety verification, allowing you
to concentrate on memory safety and undefined behavior detection."* Strictly cheaper than a
full contract proof. If §5.4b's gate rejects a signature today, this is a real fallback
between `unsupported` and nothing — memory-safety evidence where functional evidence is out
of reach. Not v1, but it belongs in the evidence-order conversation before someone decides
`unsupported` is the only floor.

### 15. Things Kani will simply never do for us, worth stating once in §5.4b

From `soundness.md` `[main only]` and `rust-feature-support.md` `[0.67.0]`:

- **Concurrency: no.** *"Kani verifies sequential code only."* And silently: *"Kani emits a
  warning whenever it encounters concurrent code and **compiles as if it was sequential
  code**."* A `bounded` verdict on a function touching an `Arc<Mutex<_>>` is a verdict about
  a program that does not exist. This is what §5.4d `trusted` claims are for, and the
  connection should be explicit.
- **Stack unwinding: not supported** (`#3134`, `#692`). Panic-path resource cleanup is not
  modelled.
- **`async`/`await`: `No`** in the feature table. There is a `-Z async-lib` ("Kani's unstable
  async library") but the Reference table row 8.2.18 "Await expressions" is a flat `No`.
- **Trait objects, closures, fn pointers, `impl Trait`, type parameters: `Partial`.** §5.4b
  excludes these already; the table is the citation.
- **Function pointers are modelled as a call to any signature-compatible function** — sound
  but imprecise, i.e. spurious counterexamples. `--restrict-vtable` narrows it and is *"a
  known soundness issue"* (`#3134`) producing false negatives. Ply must never pass it.
- **`sin`/`cos`/`sqrt` are over-approximated** to nondeterministic values in range: *"they
  largely ignore their input and give very conservative answers ... Kani could raise
  spurious errors that cannot actually happen."* Any numeric contract over transcendental
  functions will produce a counterexample that isn't real. `#1342`.
- **Object-bits limit:** CBMC models pointers with 16 object bits by default; programs
  allocating more than 2^16 objects *"may exhibit incorrect wrapping behavior"* (`#1150`).

### 16. `--output-into-files` and `--fail-fast` (0.67.0)

`--output-into-files` writes per-harness results to separate files rather than stdout —
useful even before `--export-json` lands, because it removes interleaving as a parsing
hazard. `--fail-fast` stops on first failure; Ply must **not** use it, since it would leave
later functions with no verdict at all rather than a reported one.

### 17. The `-Z` stability policy: no compatibility promise, and the churn is real

Every mechanism Ply depends on is `-Z`-gated: `function-contracts`, `stubbing`,
`concrete-playback`, `loop-contracts`, `autoharness`, `unstable-options`. RFC-0006
(`unstable-api`) is the governing policy, and it promises nothing:

> "Note that although Kani is still on v0, which means that **everything is somewhat
> unstable**, this allow us to set different bars when it comes to what kind of changes is
> expected, as well as what kind of support we will provide for a feature."
>
> "### API Removal — If we decide to remove an API that is marked as unstable, we should
> follow a regular deprecation path (using `#[deprecated]` attribute), and keep the
> `unstable` flag + attributes, until we are ready to remove the feature completely."

Deprecation before removal, and nothing about semantics changing under a stable flag name.
The changelog shows this is not theoretical over the last four releases alone:

- **0.66.0, Breaking:** *"Fail if stub verified doesn't have a contract harness"* — a
  previously-accepted program now errors.
- **0.65.0, Breaking:** *"Removed unstable list feature and default memory checks"*.
- **0.64.0:** *"Remove `assess` subcommand"* — a whole subcommand deleted.
- **0.63.0, Breaking:** *"Finish deprecating `--enable-unstable`, `--restrict-vtable`, and
  `--write-json-symtab`"*.

**This vindicates D14's decision to put engine name + version + flags in the fingerprint,
and it argues for one thing we do not yet have: a startup version assertion.** Ply should
refuse to run — `W01xx`, environment — against a Kani whose version it has not been tested
against, rather than silently producing verdicts under changed semantics. That is cheap now
and is the difference between "Ply broke" and "Ply lied" on the next upgrade.

---

## 4. Design decisions this should change

### (a) Machine-readable output — yes, it exists; no, not in our version. Change the plan anyway.

**Answer: partially, and the part that matters is coming.**

What exists in **0.67.0**, today, on this machine:

- **`cargo kani list --format json`** — *stable*, not behind `-Z`. Verified against the
  0.67.0 source (`kani-driver/src/list/output.rs`): writes `kani-list.json` **into the
  current working directory** (the path is not configurable) with exactly these keys:

  ```json
  { "kani-version": "...", "file-version": "0.1",
    "standard-harnesses": {...}, "contract-harnesses": {...}, "contracts": [...],
    "totals": { "standard-harnesses": N, "contract-harnesses": N,
                "functions-under-contract": N } }
  ```

  `file-version` is explicitly versioned — *"Increment this version (according to semantic
  versioning rules) whenever the JSON output format changes"* — so it is safe to parse
  against. Critically, `contract-harnesses` is the **which `#[kani::proof_for_contract]`
  harnesses exist for which functions** mapping. The pretty form of the same data is the
  table in `reference/list.md`.
- `--output-format regular|terse|old`. All three are human text.
- `--output-into-files`.

What exists on **`main` only** — i.e. in a Kani we could pin but do not have:

- **`--export-json <PATH>`**: *"Output the verification results to a JSON file at the
  specified path."* Unstable, requires `-Z unstable-options`.
- **`--sarif <PATH>`**: SARIF for GitHub Code Scanning, with a documented section in
  `verification-results.md` and `install-github-ci.md`. `kani-driver/src/sarif.rs`.

Neither appears in `cargo kani --help` at 0.67.0. Verified directly.

**Changes:**

1. **§8 / the Kani adapter — do not build the text parser we planned.** Structure the
   adapter as `run → obtain results → map to Diagnostic`, with the *obtain* step behind a
   seam with two implementations: `TextOutput` (0.67.0, today) and `ExportJson` (a pin bump
   away). The text parser is now explicitly a bridge with a known replacement date, not the
   design. Estimate it accordingly — do not gold-plate it.
2. **§8's "Adapters never pass engine stderr/stdout through raw"** is right and gets easier;
   note in §8 that the target representation is Kani's own JSON export and that our
   `Diagnostic` fields should map onto CBMC property records (`property_class`, `status`,
   `description`, `location`, `trace`) rather than onto rendered lines.
3. **Use `kani list --format json` in `ply-schedule` now.** D5 requires knowing, per callee,
   whether a `proof_for_contract` harness exists before deciding `may_stub`. That is exactly
   what the list JSON reports, mechanically, without compiling or parsing source. It should
   be the scheduler's input rather than anything we infer.
4. **D14 fingerprints must include the solver** (see §3 item 13) and the Kani `file-version`
   / `kani-version` strings the list JSON hands us for free.
5. **§5.4c: forbid `--output-format=old` explicitly.** It treats a timeout as success. That
   is a one-line MUST NOT in the spec and it prevents the exact failure §5.4c names.

### (b) Loop contracts do not rescue iterator-chain bodies. Close the TODO item as answered-no.

**Answer: no — with one narrow exception that does not cover our case.**

The optimistic reading is right there in the docs, and it is a trap.
`reference/experimental/loop-contracts.md` (present at **0.67.0**, unchanged on main):

> "1. `while` loops, `loop` loops are supported. **`for` loops are supported for array,
> slice, Iter, Vec, Range, StepBy, Chain, Zip, Map, and Enumerate.** The other kinds of
> loops are not supported: `while let` loops."

`Map`, `Chain`, `Zip`, `Enumerate` — the iterator adapters. But this covers a `for` loop the
**user wrote in their own source**, over an adapter chain. The scale spike's failing fixture
was `.iter().map(..).sum()`. There is no `for` loop in that source at all: the loop lives
inside `core`'s implementation of `Iterator::sum`/`fold`. `#[kani::loop_invariant(..)]` is a
*syntactic* attribute on a loop expression — you cannot attach it to a loop in `core` that
your code never spells.

So:

- `for x in a.iter().map(f) { .. }` written by the user — loop contracts apply, in principle.
- `a.iter().map(f).sum()` — loop contracts **cannot** apply. This is our case.

And even the first case is blocked for Ply by the crate-root nightly features (§3 item 5),
plus RFC-0012's open question *"How do we translate back modify targets that inferred by CBMC
to Rust level?"* and the documented limitation that inferred loop-modifies *"will fail if the
inferred loop modifies misses some targets written in the loops. We observed this happens when
some fields of structs are modified by some other functions called in the loops."*

**Changes:**

1. **TODO.md — the item "Measure whether the unwind annotation rescues ITERATOR-CHAIN bodies"
   is now answered for loop *contracts* but still open for the *unwind annotation*.** Split
   it: loop contracts are ruled out on documentary grounds (record this, don't re-measure);
   the unwind-annotation measurement still needs running. Keep the TODO for the latter only.
2. **§5.4c's `induct` paragraph should carry the real reason.** It currently says loop
   contracts are experimental and we lack a stable-Rust invariant attribute. Add the two
   findings that actually block it: they need `#![feature(stmt_expr_attributes)]` +
   `#![feature(proc_macro_hygiene)]` in the *user's crate root* (which "Ply proposes, never
   rewrites" forbids us to add), and they cannot reach loops inside `core` at all.
3. **§5.4c's "Checkability is a property of the body, not just the signature" paragraph gets
   stronger, not weaker.** The escape hatch we were hoping for is closed. The right move is
   the one §5.4c already describes — make `timeout` cheap, fast, and well-explained — plus a
   spike of `--synthesize-loop-contracts` (§3 item 12), which needs no source annotation and
   is the only remaining lever.

**But do not conclude that iterators are hopeless — §2 changes the shape of this answer.**
`verify-rust-std` verifies slice iterators successfully and at scale, using
`#[kani::proof_for_contract(Iter::next_unchecked)]`-style harnesses over a **fixed array plus
a symbolically-bounded subslice** (`library/core/src/slice/iter.rs`, `MAX_LEN` up to 512) —
no loop contracts, no unwind annotations. What is expensive is not "iterators" but
**an iterator chain consumed inside one expression over a heap collection**, which is what our
`.iter().map(..).sum()` fixture was. So the honest statement for §5.4c is narrower and more
useful than "iterator chains time out": *the cost is in the heap-allocated backing store and
the adapter chain's internal loop, and the documented remedy is to change the input's shape,
which Ply may propose but not perform.* That is a better diagnostic than the one §8 currently
sketches, and it comes with a concrete `fixes` entry we can actually emit.

### (c) `autoharness` overlaps M4 substantially and should change its plan, not cancel it

**Answer: yes, materially — on harness *construction*, which is most of M4's mechanism. It
does not touch Ply's actual product.**

What autoharness already does that M4 planned to build: enumerate eligible functions;
generate `#[kani::proof]` or `#[kani::proof_for_contract]` automatically depending on whether
a contract exists; **derive `Arbitrary` for eligible structs and enums automatically**;
select functions by regex; do all of it without modifying source. On `main` it adds bounded
collection arguments, generic instantiation, raw pointers, and `Invariant`-aware generation.

What it does not and will not do — Ply's whole reason to exist: route a *declared* check to
the right engine; aggregate verdicts worst-of up a call graph (D6); order callees before
callers and gate stubbing on real results (D5); distinguish `timeout` / `unsupported` /
`conditional` from `violation`; render evidence (§7.1); or produce a repair-shaped
counterexample (D7). Autoharness produces harnesses. Ply produces *verdicts*.

**Changes:**

1. **§10 / M4 (harness generation): re-scope from "build a harness generator" to "drive
   Kani's".** For the common case — a contracted fn with `Arbitrary`-able arguments — the
   candidate implementation is `cargo kani autoharness -Z autoharness --include-pattern
   '^crate::path::to::fn$'`, and Ply's work is the verdict mapping, not the codegen. This is
   worth a measured spike under D13 before M4 is planned in detail, and it plausibly moves
   M4 further ahead of M3 than TODO.md's existing "reweigh M4 above the bulk of M3" already
   argues.
2. **Ply keeps its own codegen for the cases autoharness refuses** — and those are exactly
   the ones §5.4b already enumerates: `Vec` needing `#[kani::unwind(N+1)]` (autoharness caps
   `BoundedArbitrary` at bound 4 and slices at 16, with no way to say otherwise), `HashMap`
   needing a hasher swap, private-field types needing a `pub`-constructor witness, and any
   fn where Ply's declared bound differs from autoharness's fixed one. That is a much smaller
   surface than "generate every harness".
3. **§5.4b gains two honesty caveats copied straight from autoharness's documented
   limitations** — arguments never alias each other, and type invariants are assumed but not
   asserted. Both silently narrow what any generated `bounded` verdict means, ours included.
4. **D13 spike list gains one item:** does `autoharness` respect `-Z stubbing` /
   `stub_verified`, and can its generated `proof_for_contract` harness be filtered to one
   function reliably? If yes, M4 gets much cheaper. If no, we build. Either way the answer
   comes from a spike, not from this document.

### (d) §5.4b needs rewriting around how the professionals build inputs

This one is not in the original brief; it comes out of §2 and it is the most immediately
actionable change in this document. §5.4b was written from our own measurements, which used
input-construction APIs that nobody in `verify-rust-std` or Kani's own CI uses.

1. **Demote `bounded_any`, promote `exact_vec` and the fixed-array route.** §5.4b currently
   says `Vec` is supported *"only because Ply's harness codegen emits an explicit
   `#[kani::unwind(N+1)]`"*. True of `bounded_any`; unnecessary for
   `kani::vec::exact_vec::<T, N>()` (Kani's own test runs N=17 with **no** unwind attribute)
   and unnecessary for `[T; N]` + `Vec::from`, which is what `verify-rust-std` does.
2. **Fix the unwind formula before it ships.** `N+1` is the collection bound only when the
   body loops once over it. Kani's rule is *"the maximum number of iterations for **all**
   loops, plus one"*, `any_vec::<u8, 8>` needs 22, and `break`/`continue` push it higher
   still. §5.4b's "sized to the declared bound" is a latent codegen bug. The mitigation is
   already available: an `unwind`-class property failure is mechanically detectable
   (§3 item 8), so codegen can start at `N+1` and escalate rather than guess once.
3. **Raise the fixed-array ceiling.** §5.4b claims "cheap at every N measured up to 16".
   `verify-rust-std` runs `MAX_SIZE = 32` in thirteen harnesses, `ARRAY_LEN = 40`, and
   `MAX_LEN = 512`. Our stated ceiling undersells the one shape that works.
4. **Add the symbolic-subslice idiom as the supported way to get a variable-length
   sequence** — fixed array plus two `any_where` bounds, no heap, no loop, no unwind.
   This is `verify-rust-std`'s standard technique and it is strictly better than anything
   §5.4b currently describes.
5. **Lower sequence assertions to a symbolic index, not a `for` loop.** The `swap_remove`
   idiom. This changes Ply's contract-expression codegen (§5.4a/§5.4c), not just its input
   construction, and it removes an unwinding assertion per assertion.
6. **Rewrite the `HashMap` remedy** from "substitute a deterministic hasher" to "stub
   `RandomState::new`, or leave the hasher parameter inferred" — and keep the exclusion,
   because neither makes it tractable past one element.
7. **Cite the external evidence for the exclusions.** `BTreeSet` → Kani's own `BOUND = 1`
   test comment plus open issue #2517 plus unclaimed challenge #4. Recursive types →
   `verify-rust-std` challenge #5, resolved with VeriFast, zero Kani annotations. Those are
   stronger citations than our own timings and they will not go stale the way a measurement
   does.

---

## Appendix — reproducing this sweep

```bash
git clone --depth 1 --filter=blob:none --no-checkout https://github.com/model-checking/kani.git
cd kani && git sparse-checkout init --cone
git sparse-checkout set docs/src rfc/src kani-driver/src && git checkout   # = published book

curl -sSL https://github.com/model-checking/kani/archive/refs/tags/kani-0.67.0.tar.gz \
  | tar xz --strip-components=1 -C k067 kani-kani-0.67.0/{docs/src,kani-driver/src}   # = what we run

diff -rq k067/docs/src kani/docs/src    # the [0.67.0] vs [main only] split

# §2's evidence: how the professionals actually drive Kani
git clone --depth 1 --filter=blob:none https://github.com/model-checking/verify-rust-std.git
```

Useful one-liners for §2:

```bash
# harness-style census across the whole std verification effort
for p in '#\[kani::proof\]' 'proof_for_contract' 'stub_verified' 'kani::unwind' \
         'loop_invariant' 'bounded_any' 'any_array'; do
  printf '%-22s %s\n' "$p" "$(grep -rho "$p" verify-rust-std/library/ | wc -l)"; done

# the three tests that settle findings 7, 8 and 9
cat kani/tests/expected/bounded-arbitrary/btree/btree.rs   # BOUND = 1, by Kani's own devs
cat kani/tests/perf/hashset/src/lib.rs                     # the RandomState stub idiom
grep -rn 'Box<Self>\|next: Option<Box' kani/tests/         # receivers only; no recursive types
grep -c kani verify-rust-std/library/alloc/src/collections/linked_list.rs   # 0 — VeriFast solved it
```

Anything sourced from `kani/` rather than `k067/` is tagged `[main only]` above and is **not
available to Ply today**.
