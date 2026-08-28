# Is Ply ready to hand to someone else? Measured, 2026-08-26

**No — two blockers, one of them serious, neither on the tidy-up list.** Found by pointing
Ply at a crate shaped like ordinary code (a module, a struct parameter, a `String`, an
enum, an `Option`, a `Vec`) rather than at a fixture. Six functions, **zero** checked.

The good news first, because it is the thesis and it held: **nothing falsely passed.**
Every failure was reported as a failure, in plain English, with the reason. The refusal to
resolve a module-relative key even told the user exactly how to rewrite it, and that
worked first time. Ply's core promise — that a green result means something — survived
contact with real code.

## Blocker 1 (serious): one broken function takes down the whole crate

A function whose generated harness fails to compile makes **every other function in the
crate** report a tool error, including functions that are fine.

Reproduced and confirmed by removal. With a `Vec` function present whose `ensures`
references a by-value parameter after the call has consumed it:

```
    billing::scalar — tool_error      <- takes a plain u32; nothing to do with the Vec fn
    billing::opt    — tool_error
    billing::vector — tool_error
```

Delete that one function, change nothing else:

```
    billing::opt    — fuzzed(32)
    billing::scalar — fuzzed(32)
```

Worse than the false alarms: the error message quotes the compiler's first error,
`borrow of moved value: v`, on functions that have no `v`. A user would debug the wrong
function. And the suggested cause it offers ("the usual cause is an `examples:` entry")
is wrong here, so it sends them somewhere else again.

Two defects, really. The generated harness moves a by-value parameter into the call and
then evaluates an `ensures` that reads it — Ply should refuse that shape by name (it is
what `old()` exists for) instead of emitting code that cannot compile. And one function's
failure must not be reported against its neighbours: the harness is shared per crate, so
a compile failure is currently indistinguishable from "this crate is broken".

## Blocker 2: the commonest parameter shapes are unsupported

| parameter | result |
|---|---|
| `u32` and other scalars | works |
| `Option<u32>` | works |
| `Vec<u8>` | works when the contract does not read a moved parameter |
| `String` | **unsupported** |
| an enum | **unsupported** |
| a struct | **unsupported** |

Reported honestly as `unsupported` rather than attempted, which is right. But a struct
parameter is the commonest shape in application code, and `String` is not far behind. On
a real codebase most functions would come back "cannot check this", and a tool that
declines most of your program is hard to justify keeping in CI, however honest it is
about declining.

Struct support was cut from the fuzz/test milestone deliberately and recorded at the
time; this measurement is what that cut costs in practice.

## Also found, minor

The component name is duplicated in diagnostics: `billing::billing::opt` where the tree
above it says `billing::opt`.

## What this means

The tidy-up items (config in several files, line numbers in errors, the missing
commands) are real but they are not what stands between here and an external trial.
These two are. Blocker 1 is a correctness-of-reporting bug and should be fixed first: it
manufactures false alarms and points them at the wrong code, which is the same family of
failure as a false pass — the user cannot trust what the tool tells them about where the
problem is.


---

# Second measurement, same day: a design made blind

The first measurement used a crate I wrote while knowing Ply's limits, which risks
flattering it. So a rate limiter was designed by someone told explicitly **not** to think
about checkability, and not told the exercise was about Ply at all — idiomatic Rust, real
traits, real generics, whatever a good reviewer would approve. `docs/greenfield-
ratelimiter-design.md` is that design: 38 functions, 11 stated invariants.

**Ply can check 0 of the 38.**

And the reason is not the one the first measurement found. It is not that the types are
exotic:

## Blocker 3 (the real one): Ply only finds free functions

The configuration schema says, in its own words: *"Fn claims, keyed by path relative to the
anchor. **Impl methods use `Type::method`**."* Written exactly that way, against the
simplest possible method — no arguments, returns a plain `u32`:

```
    capacity — unclaimed
    free     — fuzzed(32)
[E0301] Ply could not find the function `Bucket::capacity` this claim anchors to.
```

The free function beside it passes. The method is not reported as unsupported, or as a
type Ply cannot build — it is **not found at all**. The documented syntax resolves to
nothing.

**No fixture in the repository claims a method.** Not one. The single fixture containing
an `impl` block at all (`reusewiden`) has one so that the reuse hash widens its scope, and
does not claim the method inside it. So a feature the schema documents has never been
exercised, and its absence has never been visible.

That is what makes this the blocker rather than the type gap. Idiomatic Rust puts behaviour
on types: constructors, accessors, the operations themselves. In the blind design every
single public entry point is a method. A tool that checks only free functions is not
checking a minority of a real API — it is checking none of it.

## Revised readiness

Ordered by what actually stands between here and someone else's codebase:

1. **Methods must resolve and be checkable.** Documented, untested, absent. Without this
   the answer for any idiomatic Rust library is zero, whatever else is fixed.
2. **One broken function must not take down its neighbours** (blocker 1 above). Manufactures
   false alarms and aims them at innocent code.
3. **Struct, `String` and enum parameters** (blocker 2 above). Needed for methods to be
   worth resolving, since `&self` is a struct.

Items 1 and 3 are the same wall approached from two sides: `&self` is a struct parameter,
so making methods resolvable without making struct inputs constructible would move every
method from "not found" to "unsupported" and change nothing a user experiences.

**What this does not undermine.** Nothing falsely passed here either. The unfound method was
reported as unclaimed with its own diagnostic, the tree said `unclaimed` rather than green,
and the free function beside it was checked correctly. Ply's honesty held under a design
built to ignore it. The gap is coverage, not truthfulness — which is the better of the two
problems to have, and the one that is merely expensive rather than fatal.
