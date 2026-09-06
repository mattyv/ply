---
name: ply-author
description: Write or extend a ply.yaml — components, dependency rules, contracts, and structure promises — checking each addition against the real code before adding the next, and never declaring something the code does not support.
---

# Ply Author

The document is the input to everything else Ply does. A valid-looking but inaccurate
declaration can produce a confident picture of a system that does not exist. For existing code, resolve each addition before treating it as
implemented. For a design without code, label declarations as proposed intent; rendering
them does not establish implementation or verification.

Use `cargo ply check` after every addition. It runs no engines and takes about a second.

## Workflow

1. **Find the root.** A `ply.yaml` resolves function claims against one crate's
   `src/lib.rs` in this implementation. A virtual workspace root has no library, so its
   document can describe crates and dependencies but needs crate-local documents for
   function claims. A workspace root that is also a package may have its own library;
   inspect the manifest and source rather than assuming. If the crate has only
   `src/main.rs` or a custom library path, report the resolver limitation before proposing
   a layout change.

2. **Start with components and stop.** Name the parts, anchor each one, and check:

   ```bash
   cargo ply check path/to/crate
   ```

   An anchor is a crate name or a `crate::module::path`. Anchor a component at the module
   its code actually lives in — a function key is read relative to its own component, so
   the anchor is what makes the claims inside it resolve.

3. **Add the dependency rules.** `edges:` says who may call whom; `deny:` says what is
   forbidden. Check again. An isolated component can be intentional; add connections
   only when the design calls for them. Distinguish crate dependency checks from
   item-level call rules, and read any report that a declared rule was not checked.

4. **Add contracts one at a time**, checking after each. A claim that does not resolve is
   reported by name, with the nearest name Ply can see — ready to paste over a typo. Read
   that suggestion rather than guessing at the fix. (`cargo ply verify` reports the same
   unresolved claim without the suggestion, which is one more reason to run `check` first.)

5. **Verify the first useful slice before expanding the document.** Once a meaningful
   contract resolves and its implementation compiles, hand off to `$ply-verify`. Run the
   engines now; do not collect a feature's worth of unchecked claims first. Repeat for
   each coherent addition using its [incremental verification workflow](../ply-verify/SKILL.md#verify-while-building).
   `check` validates the declaration; it does not establish the contract. A design-only
   document without code can stop at declaration checking and rendering, with no claim
   that behavioral evidence was earned.

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
| `fuzz(n)` | A useful first check when the inputs are constructible and running the real code is appropriate |
| `bounded(k)` | You need exhaustive checking within an explicit bound and the engine supports the code and inputs |
| `mutate` | Beside `test` or `fuzz`, to find out whether they have teeth |
| `prove` | Not built. Do not declare it and describe it as evidence |

Start with `fuzz` for suitable new claims. Add `bounded` when the task needs that
evidence, not merely because a signature resolves. A refusal can reflect an engine
limitation rather than a design defect; read its reason before changing an API. See
[checkable code](../ply-checkable-code/SKILL.md) for input construction and side effects.

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

The text form explains the YAML declarations. Read it before declaring the document
done to find missing claims or checks that say less than intended. It does not read Rust
contract attributes or establish whether a declared edge is used; inspect source and
completed verification evidence for those questions.

`cargo ply explain <CODE>` decodes any diagnostic code the checks report.

## Change authority

The table states defaults when the task has not already authorized the change. Honor
existing user authorization; do not ask again for work already approved. A request to
review alone does not authorize changing requirements.

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
