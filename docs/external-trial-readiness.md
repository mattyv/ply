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
