# A promise you can attach, and a promise that has to say something

Run 2026-08-25, branch `claude/project-concept-eval-6soxfl`. Two connected fixes, both
about the same route: **a language model writes one promise for each piece of old code the
new code calls.** Per-function promises, nothing region-wide — the fallback the havoc gate
run (`ff15b23`) recommended.

That route had two holes, and each one was fatal to it on its own:

1. **You could not attach a promise to a function inside a module.** Ply would tell you
   `rates::legacy_rate` was a callee nobody had vouched for, and in the same envelope
   refuse the claim written to vouch for it.
2. **A promise that says nothing bought a confident result.** An unsatisfiable one made
   the caller's proof hold vacuously — measured below: a caller whose own postcondition is
   provably false, reported `bounded(2)`, exit 0.

---

## 1. Red first, verbatim

Both fixtures were written before either fix, and both were run against the binary built
from `3900bff` (the branch head before this work) so the failures below are what the tool
actually said, not a reconstruction.

### Fix 1 — `tests/fixtures/modanchor`

A crate whose legacy callee lives where legacy callees live: `src/rates.rs`, reached by
`mod rates;` and called through `use rates::legacy_rate;`. `ply.yaml` declares a promise
for it, keyed `rates::legacy_rate`.

`cargo ply check`:

```
  anchors       1 of 2 fn claims in this crate point at a function Ply can find.

  E0301 `rates::legacy_rate` exists in this crate, but not where Ply can verify it from. This
    slice reads functions declared at the top level of ./src/lib.rs; `rates::legacy_rate` is
    inside a module or behind a `use`. Move the claim's component to an `anchor:` on that
    module, or move the function up, until Ply learns to descend.
```

`cargo ply verify`, the same crate, the same run — both halves of the asymmetry in one
envelope:

```
workspace — unclaimed
  modanchor — unclaimed
    legacy_rate — unclaimed
    tiered_fee — unclaimed
[E0301] modanchor::rates::legacy_rate — Ply could not find the function `rates::legacy_rate`
  this claim anchors to. E0301: could not find fn `rates::legacy_rate` in ./src/lib.rs
  (unresolvable anchor)
[W0512] modanchor::tiered_fee — Ply did not check `tiered_fee`: proving it would mean
  descending into `legacy_rate` (called at line 26, column 15), and no contract anywhere
  describes what that code promises -- not on the function itself, and not in ply.yaml. ...
```

Read those two lines together. `W0512` says *nobody has vouched for `legacy_rate`*.
`E0301` says *the vouching you wrote points at nothing*. Both sentences were well written
and both were true; between them they closed the route.

The e2e test failed on the verdict:

```
assertion `left == right` failed: the promise describes the callee the caller crosses
into, so the caller is provable against it -- exactly as it is when the same callee sits
at the top level of lib.rs
  left: "unclaimed"
 right: "bounded(2)"
```

### Fix 2 — `tests/fixtures/emptypromise`

Two callees with two empty promises, and two callers that cross into them.
`vacuous_fee`'s own postcondition is `|result| *result == 0`, which is **plainly false**:
at `(100_000, 0)` it returns 1500. The promise declared for the callee it crosses is
`|result| *result > 10_000 && *result < 5`, which no `u32` satisfies.

```
exit=0
workspace — bounded(2)
  emptypromise — bounded(2)
    havoc_fee — bounded(2)
    vacuous_fee — bounded(2)
[W0511] emptypromise::havoc_fee — ... Assumed: `legacy_cap`: ensures |result| *result >= 0.
  ... Nothing has checked them against the real code yet, so each one is owed evidence
  rather than settled: an assumed contract nobody exercises is green paint. (W0511, §5.5)
[W0511] emptypromise::vacuous_fee — ... Assumed: `legacy_rate`: ensures |result| *result >
  10_000 && *result < 5. ... (W0511, §5.5)
```

That is the whole failure in six lines. **A false claim, reported green, exit 0**, with the
impossible promise printed beside it in a sentence saying it is owed evidence. And
`legacy_cap`'s `|result| *result >= 0` — true of every `u32` — printed in exactly the same
words, sending a reader off to discharge a debt that does not exist.

---

## 2. Fix 1 — one resolver answers both questions

The two halves of Ply had separate ideas about where a function is:

| | followed |
|---|---|
| call classification (`callgraph::Resolver`, §5.5) | `use` incl. renames/groups/globs, inline `mod`, file modules (`foo.rs` and `foo/mod.rs`), re-exports, path dependencies |
| anchor resolution (`harness::discover_fn`, §5.2) | the top-level items of one file |

Anchor resolution now goes through the same resolver, which grew three things rather than
a second walker:

- **`Resolver::lookup_fn`** — resolves a path to the function it names, and returns the
  `syn::ItemFn`, the file it was declared in (so its `type` aliases are the right ones),
  and its **canonical path**: the path from the crate root with every `use` expanded.
- **Canonical matching for declared contracts.** This is the part that actually makes a
  promise attach. The call site writes `legacy_rate`; `ply.yaml` writes
  `rates::legacy_rate`. Both canonicalise to `rates::legacy_rate`, so one lookup finds the
  other. The as-written spelling is still tried first, which is what a cross-crate
  `anchor:` produces, so nothing about the existing cross-crate route moved.
- **`Resolver::fn_index`** — the item index behind `E0301`'s nearest-name suggestions,
  walking modules the same way. Suggestions and resolution must cover the same set: a
  suggestion naming something resolution would then refuse is worse than no suggestion.

`ContractFn` gained a `path` (where the function lives) beside its `name` (its bare
identifier), because generated code sits at the crate root and has to call the function by
where it is. Generated *identifiers* are derived from the path
(`ply_proof_rates_legacy_rate`), so two same-named functions in different modules cannot
collide into one harness. For a top-level function `path == name` and every generated byte
is unchanged, which is why no golden moved.

### What genuinely stays closed, and why

**A private item below the crate root.** Ply's harness is a module *at* the crate root, so
a private `fn` inside `rates`, or a private `mod` between it and the root, is a name that
harness cannot write. This is a fact about Rust's visibility, not a limit of the walk, and
it now gets its own sentence:

```
E0301 Ply found `util::helper` but cannot verify from it: `helper` is private to the module
  it is declared in, so the harness Ply generates at the crate root cannot call it by name.
  Make it (and every module between it and the crate root) `pub` or `pub(crate)`, or move
  the claim to a function that is reachable.
```

Items private *at* the crate root are unaffected — the generated module is a child of the
root and sees them, which is why top-level private functions have always worked.

Three other cases were already refusals and stay refusals, unchanged: a `mod` whose file
Ply cannot open or parse (`Opaque`), a path dependency whose `src/lib.rs` will not read,
and a bare name that could only have come from a glob into either. And the two first-party
gaps §5.5 already records are untouched: `#[path = "..."]` module attributes are not
followed, and macro-generated calls are not seen.

**One narrower reachability gap is deferred rather than closed.** The `bounded` tier's
harness lives *inside* the crate, so crate-internal visibility is enough for it. The
`fuzz`/`test` tier's harness is a *separate crate* that imports the function, so it needs
`pub` all the way out. A `pub(crate)` function inside a `pub mod` therefore passes anchor
resolution and works under `bounded`, and would fail to compile in the generated harness
crate under `fuzz`. That failure is loud (a compiler error carried out as a tool error),
never a silent pass, and no fixture exercises it. Closing it means a second, stricter
reachability question asked per tier rather than per anchor — recorded below rather than
guessed at here.

The `check.rs` unit test that asserted the old limit —
`a_function_ply_can_see_but_not_verify_from_says_which_of_the_two_it_is`, whose expected
string was "exists in this crate, but not where Ply can verify it from" — is now two
tests: the module case resolves, and the private case reports the real obstacle.

---

## 3. Fix 2 — a promise has to say something

Before a proof stubs a callee, Ply asks the engine two questions about each declared
clause, over the clause alone with **no function body anywhere in the harness**:

| generated harness | asserts | verified means |
|---|---|---|
| `ply_promise_sat_<callee>_<half>` | `!(c1 && … && cn)` | nothing satisfies the promise → **unsatisfiable** |
| `ply_promise_taut_<callee>_<half>_NN` | `ci` | nothing violates the clause → **trivially true** |

The satisfiability question is asked about the promise as a **conjunction**, because that
is what the stub assumes: two clauses can each be satisfiable and still contradict each
other. Triviality is asked per clause, because one empty clause beside a real one is still
an empty clause somebody wrote.

The probes ride in the same generated module as the proof, so the crate compiles once for
the whole set. Measured 2026-08-25 on this machine: six probes in one `cargo kani`
invocation, **3.9s total**; **0.43s each** once compiled. This is exhaustive over the value
space, not sampled — CBMC solves it symbolically.

### What happens then

**Unsatisfiable → `E0502`, error, verdict `unclaimed`, and the proof is never started.**
Running it would produce a green verdict that means nothing.

```
[E0502] emptypromise::vacuous_fee — Ply did not check `vacuous_fee`: the promise declared in
  ply.yaml for `legacy_rate` cannot be true of anything. Ply searched every value a `u32` can
  hold -- that is what `legacy_rate` returns -- and found none that satisfies `|result|
  *result > 10_000 && *result < 5`. A proof that assumes something impossible proves
  everything -- it would have come back green for `vacuous_fee` whatever `vacuous_fee`
  actually does, and that green would have meant nothing. So Ply did not run it: this check
  earned no evidence and the verdict is `unclaimed`, never `bounded(2)`. Fix the promise and
  re-run. (E0502, §5.5)
```

**Trivially true → `E0503`, error, and the caller's verdict stands.** This asymmetry is
deliberate and it took some working out. A tautological `ensures` on a stub is *havoc*: the
callee is replaced by an unconstrained value, which is a **weaker** assumption than a real
promise, not a stronger one. So `havoc_fee`'s `bounded(2)` is real evidence and holds
whatever `legacy_cap` returns. What was wrong was the *report* — calling it an assumption
owed evidence. `E0503` says so, and `W0511`'s `conditional` sentence stops counting the
empty clause among the promises that are owed anything:

```
[W0511] ... an assumed contract nobody exercises is green paint. Not all of them, though,
  and that is why this run does not pass: `legacy_cap`'s `ensures: |result| *result >= 0`
  constrained nothing -- it is true of every value, so the proof assumed nothing there and
  there is nothing to owe. E0503 below says what to do about it.
```

`E0503` is error severity on purpose. A warning would leave the run at exit 0 — the
`owed-evidence` status is not an absence, so it does not fail the default `--fail-on
evidence` either. An absence of real assumption is not a pass, and the only way to say that
in the exit code is an error.

**Neither answer → `W0514`, warning, unchecked rather than sound.** A `requires` over a
parameter type the bounded codegen cannot build an arbitrary value for, a clause Ply cannot
parse, or an engine that timed out. The diagnostic names the reason and says the verdict
beside it still assumes the promise.

### The gate does not merely refuse

With `legacy_rate`'s promise rewritten to something that says something
(`|result| *result <= 10_000`), the *same* caller is reported as what it is:

```
exit=1
workspace — violation
  emptypromise — violation
    vacuous_fee — violation
[K0502] emptypromise::vacuous_fee — `vacuous_fee` breaks its own postcondition
  `|result|*result == 0` for at least one input ...
```

So the impossible promise was hiding a real bug, and the gate is not trading a false green
for a false red.

### What the check catches, and what it provably cannot

**Catches**, exhaustively over the value space, for every declared `ensures` (any return
type the stub can already build) and every declared `requires` whose parameters are
bounded-supported scalars:

- a promise no value satisfies, including one whose *clauses individually* are fine and
  whose conjunction is not;
- a clause every value satisfies — `|result| true`, `|result| *result >= 0` on an unsigned
  integer, `tier >= 0` on a `u8`;
- an empty clause sitting beside a meaningful one.

**Cannot**, and each is a deliberate boundary rather than an oversight:

- **Weakness that is not emptiness.** `|result| *result <= u32::MAX - 1` excludes one value
  out of four billion. It is neither unsatisfiable nor trivially true, so it passes this
  gate while carrying almost no information. Satisfiability is a binary test, not a measure
  of strength; strength is the `mutate` tier's question (`W0502`), and that tier does not
  reach declared contracts.
- **Whether the real callee honours the promise.** Entirely separate: that is the
  `owed-evidence` debt, discharged by fuzzing the callee against the declared contract.
  A promise can say a great deal and still be false of the code.
- **Vacuity from the harness as a whole.** The gate asks about each declared promise
  alone. Over-constraint arising from the interaction between the *caller's own*
  `requires` and a stub's assumptions is not caught. Kani's `kani::cover` is the instrument
  for that (see `docs/kani-docs-sweep.md` §9); it is not built, and the caller's own
  `requires` is not probed at all.
- **A verified function's own inline `#[ply::ensures]`.** A vacuous inline spec on the
  function being checked is out of scope here.
- **Clauses it cannot range over.** A `requires` mentioning a parameter of a type with no
  `kani::any()` in Ply's codegen. Reported `W0514`, never silently passed.
- **Anything when Kani is absent.** The gate runs only on the `bounded` tier, which needs
  Kani anyway; a `fuzz`-only claim with a declared promise is not probed.

One thing worth stating plainly: the two questions are *duals over the same domain*, so an
uninhabited return type would make both come back "verified". No such type reaches this
code — every type the stub can build a `kani::any()` for is inhabited — but the pair is not
independent evidence, and the `Unsatisfiable` answer suppresses the triviality report for
the same promise so one defect gets one sentence.

---

## 4. Evidence

| | |
|---|---|
| new fixtures | `tests/fixtures/modanchor`, `tests/fixtures/emptypromise` |
| new e2e | `tests/e2e/tests/modanchor_fixture.rs` (2), `tests/e2e/tests/emptypromise_fixture.rs` (2), `check_command.rs` (+2) |
| new unit tests | `harness.rs` (+7, the resolution walk and the case that stays closed), `promise.rs` (+9, generation and the rule that decides what a green probe *means*) |
| changed unit tests | `check.rs`'s anchor test split in two — the module case now resolves |

The rule that turns a probe result into a finding is a pure function
(`promise::findings`) taking the answers as data, tested with no subprocess anywhere near
it. That is the one place this check could be wrong in the reassuring direction.

---

## 5. TODO deltas

`TODO.md` was not edited (out of scope for this run). These are the deltas it needs.

**Tick, with this run's commits:**

- `KNOWN GAP, deliberate: discover_fn sees only top-level fns in src/lib.rs.` — closed.
  Anchor resolution now walks `use` imports, inline `mod`s, file modules and nested
  modules through the same resolver call classification uses. The remaining refusal is a
  *private* item below the crate root, which is a Rust visibility fact and has its own
  diagnostic.
- `KNOWN GAP (review G2)` part **(1) no vacuity check** — closed. Parts (2) *no staleness
  for a declared boundary contract* and (3) *no accumulating surface* are unaffected by
  this run; (3) was already partly closed by `audit`/`worklist` in Phase 1b.

**Add:**

- Probe the **caller's own `requires`** for over-constraint, via `kani::cover!(true)` after
  the call in the generated proof. Catches the vacuity this run's gate cannot: an
  interaction between the caller's precondition and its stubs' assumptions. Cheap (one
  extra check in a run that already happens), and named as the standard defence in
  `docs/kani-docs-sweep.md` §9.
- **Strength, not just emptiness, for declared contracts.** `W0502`'s mutation tier
  measures spec strength for inline contracts and does not reach a `ply.yaml`-declared one.
  A promise that excludes one value passes today's gate.
- **`W0514`'s reach.** A `requires` over a non-scalar parameter is reported unchecked. If
  declared preconditions over collections turn out to be common, the probe needs the same
  `kani::any()` coverage the harness codegen has.
- **Per-tier reachability for an anchored function.** Anchor resolution asks one
  reachability question: can the crate-root harness name this? That is the right question
  for `bounded` and too weak for `fuzz`/`test`, whose harness is a separate crate and needs
  `pub` all the way out. A `pub(crate)` fn inside a `pub mod` claimed with `fuzz` would
  fail in the generated harness crate rather than at the anchor. Loud, not silent, and no
  fixture hits it today.

**Amend:**

- `KNOWN GAP (review G3) — declared-contract keying assumes the anchor equals the
  Cargo.toml dependency key.` Still open and still fails closed, but narrower now: for a
  **local** anchor the two spellings are reconciled through the canonical path, so the
  mismatch only survives across a renamed path dependency.

Spec amendments landed with the code: §5.2 (anchor resolution follows the same walk as
call classification; the private-item refusal), §5.5 (the promise-content gate, its two
outcomes, its three-way "could not decide", and what it does not reach), §6's `check`
paragraph (the anchor tier's second sentence).

`docs/SCHEMA.md` landed from another session (`94d3d4d`) while this work was in flight,
and carried two passages this run falsified: its "what this build can actually reach"
callout said a function inside a module is not reachable, and its boundary section had no
account of an empty promise. Both are rewritten, and the three new codes are in its
diagnostic table. That correction is in the third commit rather than the two it belongs
to, because rewriting the branch's history under a concurrently active session would be
worse than a late commit.
