# Two defects found by running the tool, not reading it

2026-08-25. Both were found while writing `docs/SCHEMA.md`, which recorded them as
current limits (§6, §14). Both are now fixed or honestly refused, and SCHEMA.md is
amended.

---

## 1 — "the value this had before the call" worked on one checking path and broke on the other

`ensures` may refer to a parameter's value on entry — `old(x)`. Three cases were run
before anything was changed.

### Case A — a plain by-value parameter, under the model checker: already worked

```rust
#[ply::requires(x < u32::MAX)]
#[ply::ensures(|result| *result == old(x) + 1)]
pub fn bump(x: u32) -> u32 { x + 1 }
```

```
workspace — bounded(2)
  oldrepro — bounded(2)
    bump — bounded(2)
```

Unchanged by this work, and re-run after it to confirm it stays that way.

### Case B — the same function, under the random-input checker: broke, and blamed itself

Before:

```
workspace — tool_error
  oldrepro — tool_error
    bump — tool_error
[X0901] oldrepro::bump — `bump`'s `fuzz(64)` check ran zero cases: the test harness Ply
generates for it failed to compile, so nothing was checked at all. This is reported as a
tool error -- never as a pass, because no evidence was gathered, and never as a
violation, because there is no failing input to show. The compiler's own first error was:
error[E0425]: cannot find function `old` in this scope. The usual cause is an `examples:`
entry in ply.yaml that does not type-check against `bump`'s real signature: Ply compiles
those entries exactly as written (they are ordinary Rust `==` expressions), so a wrong
type or a typo first shows up here. (X0901)
```

The suggested cause was wrong — there was no `examples:` entry. The clause reached the
generated test verbatim, so the test called a function named `old` that exists nowhere.

**Fixed.** The generated test/property harness now reads each `old(...)` expression into
a binding of its own *before* the call and compares against that binding. This is exactly
what the design specification already prescribed and what the model checker does with its
own primitive. After:

```
workspace — fuzzed(64)
  oldrepro — fuzzed(64)
    bump — fuzzed(64)
```

The replay test Ply writes when a check finds a failing input had the same defect — it is
a file the user's own `cargo test` compiles. It is fixed the same way, and was run for
real against a deliberately broken body:

```
test ply_generated_cex::ply_cex_bump_01 ... FAILED

Broken promise in `bump`: the function declares the postcondition
`|result|*result == old(x) + 1` -- a postcondition is the guarantee a function makes
about its return value. For this input, the left side of the contract evaluated to 0,
and the right side evaluated to 1, which does not satisfy the contract's comparison.
One of the two is wrong: fix the body or fix the `#[ply::ensures]` line, and this test
will pass. (P0502)
```

It compiles, it goes red, and it goes red naming the broken promise rather than a
compiler error inside Ply's own generated file.

### Case C — a function that changes something in place and returns nothing: honestly refused

```rust
#[ply::ensures(|result| *counter == old(*counter) + 1)]
pub fn bump_in_place(counter: &mut u32) { *counter = counter.saturating_add(1); }
```

This is the shape the construct exists for, and it was the worst of the three. Before,
under the model checker:

```
[X0901] oldrepro::bump_in_place — Ply's Kani adapter could not interpret Kani's output:
neither VERIFICATION:- SUCCESSFUL nor VERIFICATION:- FAILED appeared in Kani's output
```

and under the random-input checker:

```
[X0901] oldrepro::bump_in_place — `bump_in_place`'s `fuzz(64)` check ran zero cases: the
test harness Ply generates for it failed to compile ... The compiler's own first error
was: error[E0308]: mismatched types.
```

The cause: Ply read a parameter the function writes back through as if it were an
ordinary value of the same type, then generated a harness that handed the function a
read-only value where a writable one was wanted. Neither engine can build and observe
such a parameter — the supported-signature list stops at a read-only reference — so this
is a refusal, not a fix. It now reports, identically on both paths:

```
workspace — unsupported
  oldrepro — unsupported
    bump_in_place — unsupported
[V0505] oldrepro::bump_in_place — Ply cannot check `bump_in_place`: parameter(s)
counter: &mut u32 use a type neither the bounded (Kani) nor the fuzz (proptest) codegen
builds inputs for. This is reported as unsupported, not attempted -- it never silently
hangs.
```

Two smaller things came with it. The message spells the type the way the user wrote it
(`counter: &mut u32`); it used to spell it the way Ply stores it internally
(`counter: Unsupported("&mut u32")`), which was true of every unsupported-shape message,
not just this one. And a contract quoted back to a reader now reads `old(x)` rather than
`old (x)`.

### The tests, and what they said before the fix

```
the generated harness must not call a function named `old` -- there is no such function,
and the harness crate fails to compile with "cannot find function `old` in this scope":
    #[test]
    fn ply_fuzz_bump() { ...

assertion `left == right` failed: a `&mut` parameter must be recorded as a shape Ply does
not build, spelled the way the user wrote it -- recorded as a plain `u32` it produces a
harness that does not compile
  left: U32
 right: Unsupported("&mut u32")

the replay test must not call a function named `old`:
// Reproduces the counterexample for `bump` found by check ...
```

Four unit tests (harness generation, concrete-case generation, replay-test rendering,
type reading) plus one end-to-end fixture, `oldvalue`, which claims both shapes in one
crate and asserts that neither ends in an internal error, that the working one earns real
evidence, that the refused one is named, and that the entry value is read before the call.

### What is still genuinely unsupported

A parameter a function writes back through, on both checking paths. Lifting it means
adding that shape to the supported set, and for the model checker also teaching it which
memory a function is allowed to change — neither is in this build. The consequence is
worth stating plainly and is now in the specification: the before-value construct is
usable today over values a function *reads*, not over a mutation's before-and-after.

Deferred, deliberately, and not touched here: writing a before-value reference in a
*precondition* is meaningless and the specification says it should be refused by name.
Nothing validates the contract expression subset yet — that is a pre-existing gap the
reference already records — so such a clause still ends in the same unhelpful internal
error the fix above removed for postconditions. It needs the subset validator, not a
patch at the harness.

---

## 2 — one command's advice named something another command refuses

`cargo ply audit` and `cargo ply worklist` both tell a reader how to settle an assumed
promise. When the promised function lives in another package, the advice was:

```
      ... To close it, add `checks: [fuzz(256)]` to its
      `ply.yaml` entry — fuzzing crosses a legacy boundary by simply calling the code, so
      it tests the promise against the real `ledger::fees::bps_for_tier`.
```

Following it, from the directory the reader was standing in:

```
[W0303] ledger::fees::bps_for_tier — `fees::bps_for_tier` is claimed under a component
anchored at `ledger`, which is not the crate this run is verifying, and `cargo ply
verify` checks one crate at a time. Its `checks:` were not run and no verdict is reported
for it. ... (W0303)
```

Each command is right on its own. Together they are a circle: do this, watch it be
declined, learn nothing about what to do instead.

**Fixed.** The advice now knows whether the promised function's entry belongs to this
crate or another one, and says so. Same run, after:

```
  owed evidence (1)
    `withdrawal::tier_fee_cents` (at line 4, column 15)
      `tier_fee_cents`'s proof stands on a promise `ply.yaml` makes for
      `ledger::fees::bps_for_tier` — ensures |result| *result <= 10_000 — and nothing
      has run the real `ledger::fees::bps_for_tier` against it. That is what `owed-evidence`
      means: trust that is never checked is green paint. Unlike the rest of the trust
      surface this one closes, and cheaply. To close it, add `checks: [fuzz(256)]` to its
      `ply.yaml` entry and run `cargo ply verify` inside the `ledger` crate, which is where
      that function lives — fuzzing crosses a legacy boundary by simply calling the code,
      so it tests the promise against the real `ledger::fees::bps_for_tier`. Adding the
      check changes nothing in this crate: `cargo ply verify` checks one crate at a time and
      will decline to run it from here. If you would rather not leave this crate, pass what
      `ledger::fees::bps_for_tier` returns into `tier_fee_cents` as a parameter instead: the
      value becomes the caller's own data and there is no promise left to owe. (§5.5)
```

Both halves of what the brief asked for are on the line: the package the suggestion has
to be run in, and the route that needs no second crate at all. `cargo ply audit` says the
same thing in its own words. A promised function whose entry already declares a check
gets the matching sentence — "run it inside the `ledger` crate", not "`cargo ply verify`
is what settles it", which from here would be the same circle one step further along.

When the promised function is in the *same* crate, both commands say exactly what they
said before: nothing about a crate name, because there is no second crate to go to.

### The test, and what it said before the fix

```
the advice has to name the package the suggested check would have to be run in, or it
sends the reader to a command that will decline it: `tiered_fee`'s proof never reads
`ledger::fees::bps_for_tier`'s code. ... To settle it, add `checks: [fuzz(256)]` to its
`ply.yaml` entry — fuzzing crosses a legacy boundary by simply calling the code, so it
tests the promise against the real `ledger::fees::bps_for_tier`. (§5.5)
```

One test per command, each building the two-package layout the case actually has — the
crate being audited, and a path dependency holding the old code. This is the first
in-repo coverage of that layout; every previous boundary test had both functions in one
crate, which is exactly why the circle was never noticed.

---

## Documents amended

- `docs/SCHEMA.md` §6 — the before-value caveat now records what works and what is
  refused, instead of "half works"; the settle-the-debt walkthrough quotes the new
  advice, and the paragraph after it explains why the crate name is on the line. (The
  second of those landed in `979d182` rather than in the commit below: a second agent was
  editing this repository's working tree at the same time and staged the whole file.
  The text is the intended text; only its commit is not the one it belongs to.)
- `docs/SCHEMA.md` §14 — the two "what this build does not do" bullets rewritten.
- `The-Ply-Spec.md` §5.4a — the harness substitution is recorded as implemented, with a
  dated note that the mutating shape the construct exists for is refused as an
  unsupported signature, and what it would take to lift that.

## TODO.md deltas (not applied — listed as asked)

Add, under the section that records the boundary-contract gaps:

- `[x]` **A before-value reference works on both checking paths** (`220b4ad`) — the
  generated test/property harness reads the expression before the call. Proved on a real
  function of each shape; the shape it exists for (a parameter written back through) is
  refused by name instead of failing to compile.
- `[x]` **Advice for settling a promise made for another package names the package**
  (commit "Advice for settling a promise made for another package says where to run it")
  — `audit` and `worklist` used to name a check `verify` then declines
  from the caller's directory.
- `[ ]` **KNOWN GAP** — a parameter a function writes back through is unsupported on
  both paths. Lifting it needs that shape in the supported set and, for the model
  checker, a way to say which memory a function may change. Until then the
  before-value construct covers values a function reads, not a mutation's
  before-and-after.

The existing line "Not attempted this session … `impl`-method contracts (`&self`,
`old()`)" is now half-true and should say so: the before-value construct itself is done;
`&self`/`&mut self` methods remain out.

---

## One process note

Another agent was working in the same checkout throughout this session. Two consequences
worth recording rather than discovering later: the `docs/SCHEMA.md` half of defect 2
landed in that agent's commit rather than this one (they staged the whole file), and the
first defect's commit had to be split around a data-model change of theirs that my files
briefly depended on. Nothing is missing from the repository; some of it is under a
neighbour's name.
