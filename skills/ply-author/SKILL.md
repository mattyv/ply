---
name: ply-author
description: Write or extend a ply.yaml — components, dependency rules, contracts, and structure promises — checking each addition against the real code before adding the next, and never declaring something the code does not support.
---

# Ply Author

The document is the input to everything else Ply does. A wrong line here does not fail
loudly; it produces a confident picture of a system that does not exist. So the rule that
governs this whole skill is: **declare nothing you have not just seen Ply resolve.**

Use `cargo ply check` after every addition. It runs no engines and takes about a second.

## Workflow

1. **Find the root.** A `ply.yaml` resolves function claims against one crate's
   `src/lib.rs`. A workspace root has none, so a document there can describe crates and
   their dependencies but can never carry a function claim — put those in the crate's own
   document instead. If the crate has only `src/main.rs`, say so: contracts need a library.

2. **Start with components and stop.** Name the parts, anchor each one, and check:

   ```bash
   cargo ply check path/to/crate
   ```

   An anchor is a crate name or a `crate::module::path`. Anchor a component at the module
   its code actually lives in — a function key is read relative to its own component, so
   the anchor is what makes the claims inside it resolve.

3. **Add the dependency rules.** `edges:` says who may call whom; `deny:` says what is
   forbidden. Check again. A component nothing may reach and that reaches nothing is a
   rule you have not written yet, not a component that is finished.

4. **Add contracts one at a time**, checking after each. A claim that does not resolve is
   reported by name with the spelling that would work; do not guess at the fix, read it.

5. **Only then run the engines**, and hand off to `$ply-verify` for that.

## Write a promise a run can be wrong about

This is the part that decides whether the document is worth anything.

| Write this | Not this | Why |
| --- | --- | --- |
| `result.len() == 64` | `result.len() >= 0` | The second is true of every possible body |
| `\|r\| r.bid <= r.ask` | `\|r\| r.bid >= 0` | Say the relationship, not the type |
| `state.len() <= state.cap()` | `state.len() < 1000` | Tie it to the thing it depends on |

**A promise that cannot fail is worse than no promise**, because it earns a green verdict
and tells the reader nothing. When you cannot tell whether a promise has teeth, declare
`mutate` beside it: that check breaks the function on purpose and reports whether anything
noticed. A promise whose mutants all survive is the case this exists to catch.

Prefer a promise about the *relationship between inputs and output* over one about the
output alone. "Returns a number" is a type; "returns at least what it was given" is a
promise.

## Which checks to declare

| Ask for | When |
| --- | --- |
| `test` | You have specific cases that matter. Write them under `examples:` |
| `fuzz(n)` | Almost always the right first choice: it runs the real code, so it works on nearly anything |
| `bounded(k)` | You want exhaustive search over the whole input space and the shape is simple enough for Ply to build |
| `mutate` | Beside `test` or `fuzz`, to find out whether they have teeth |
| `prove` | Not built. Do not declare it and describe it as evidence |

Declare `fuzz` first. Add `bounded` where it resolves, and read the refusal rather than
fighting it when it does not: a shape Ply will not attempt is reported by name with the
parameter and the reason, and that reason is usually a real fact about the type.

## Structure promises (`holds:`)

Under a component's `state:`, `holds:` says what must always be true of the value it
keeps. Ply builds one through the type's own constructor and calls the type's own public
operations on it, checking every clause after each one.

```yaml
state:
  of: OrderBook
  show: [bids, cap]
  holds:
    - "state.bids.len() <= state.cap"
```

Write each clause as an expression about the value: a bare one names it `state`, a closure
names it whatever you like. Two things to know before writing one:

- **It must compile against the real type.** A field you renamed or a method that takes
  arguments will read as fine YAML and fail as Rust. Ply reports that as a tool error and
  never as a broken promise, but you have still learnt nothing until you fix it.
- **The report says what it could not reach.** An operation whose argument Ply cannot build
  is named rather than skipped quietly. Read that line — a promise checked without the one
  mutator that would break it is worth much less than the number beside it suggests.

## Read what you wrote

```bash
cargo ply render path/to/crate --text
```

Every construct in the document, written out with its meaning inline. Read this before
declaring the document done: it is the fastest way to see a component that promises
nothing, an edge nobody uses, or a check that says less than you thought.

`cargo ply explain <CODE>` decodes any diagnostic code the checks report.

## Change authority

| target | authority |
| --- | --- |
| new_component | may-add |
| new_edge | may-add |
| new_contract | may-add |
| new_check | may-add |
| existing_contract | ask-first |
| existing_check | ask-first |
| existing_architecture_rule | ask-first |
| deleting_any_declaration | ask-first |

Adding a promise is authoring. **Weakening or deleting one that already exists is a
decision about what the codebase is allowed to do**, and it belongs to the developer —
especially when the reason is that a check is failing. Present the failing evidence and
the smallest change that would resolve it, and wait.

Never widen a rule to excuse one file. If one test file needs a dependency the rule
forbids, the rule is not the thing that is wrong.
