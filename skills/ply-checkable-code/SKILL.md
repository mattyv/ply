---
name: ply-checkable-code
description: Write Rust that Ply can actually check — separating decisions from side effects, keeping signatures buildable, and stating promises that can fail — so a claim earns evidence instead of coming back unsupported.
---

# Writing code Ply can check

Ply runs your real function against generated inputs. That means the shape of a function
decides whether it can be checked at all, before anyone writes a promise about it. This
skill is the set of shapes that work, and it exists because the alternative is discovering
each one from a refusal after the code is written.

**Every rule below has a real incident behind it, in Ply's own source.** They are not
style preferences.

## 1. Separate deciding from writing

The single highest-value rule. Ply runs the body, so a function that writes files cannot
be checked with generated inputs — it would create files at paths Ply invented. That is
not a limitation to work around; it is a signal that the deciding and the writing are in
one place.

```rust
// Before: the logic cannot be checked, because the write is in the way.
pub fn write_report(dir: &Path, rows: &[Row]) -> Result<PathBuf> {
    let body = /* real logic: formatting, arithmetic, ordering */;
    fs::write(&path, body)?;
    Ok(path)
}

// After: the logic takes data and returns data. Claim this one.
pub fn report_body(rows: &[Row]) -> String { /* the logic */ }

pub fn write_report(dir: &Path, rows: &[Row]) -> Result<PathBuf> {
    fs::write(&path, report_body(rows))?;   // three lines nobody needs to check
    Ok(path)
}
```

**The shell is meant to stay unclaimed.** Not everything should be checked, and a function
that only opens a file and hands off is one to leave alone deliberately. What matters is
that the logic is not trapped inside it.

## 2. Do not take an index into a separate argument

```rust
pub fn order(domain: &BTreeSet<usize>, node_ids: &[String]) -> Placement  // don't
```

`domain` holds indices into `node_ids`, and nothing makes them agree. Real callers built
both from the same list, so they always did — until Ply generated `domain = {15}` with an
empty `node_ids` and the function panicked on an index nobody had thought about.

Two fixes, in preference order: let the two travel as one value, or make the lookup total
(`node_ids.get(i)`) so an unexpected index is handled rather than fatal. **Declaring the
agreement as a precondition instead is a trap — see rule 5.**

## 3. Return values; do not write through `&mut` parameters

A `&mut` parameter is a shape neither engine builds an input for, so the function is
refused outright. A function that computes something should return it.

## 4. Keep a public struct under about a dozen fields

A struct Ply builds field by field generates one strategy per field, and the trait that
composes them stops being implemented past twelve. Ply's own `FingerprintInputs` has
twenty public fields and is refused for exactly this reason — the one claim in its library
that still earns nothing.

If a type is genuinely that wide, give it a public constructor that takes fewer arguments,
or declare a `routes:` entry naming a public function that returns one.

## 5. Watch what a precondition throws away

`requires` is a filter on generated inputs. A precondition that is *true* can still be so
narrow that almost nothing survives it, and then the check earns nothing at all.

Measured on Ply's own scheduler: a correct precondition threw away 1025 of 1195 generated
inputs, the sampler gave up, and the verdict went from a real result to `unclaimed`. The
precondition was right; the code was the thing to change.

So when a promise needs a narrow precondition to hold, ask whether the function should
instead be **total** — handling the case rather than excluding it. That usually makes the
code better and the evidence real at the same time.

## 6. Write a promise that can fail

A promise true of every possible body earns a green verdict and tells a reader nothing.

| Write | Not | Why |
| --- | --- | --- |
| `result.len() == 64` | `result.len() >= 0` | The second holds for every body |
| `\|r\| r.bid <= r.ask` | `\|r\| r.bid >= 0` | State the relationship, not the type |
| `result.0.len() + result.1.len() == domain.len()` | `!result.0.is_empty()` | Conservation is the property; non-emptiness is a symptom |

Prefer a promise relating **inputs to output** over one about the output alone. "Returns a
number" is a type. "Returns at least what it was given" is a promise.

When unsure whether a promise has teeth, declare `mutate` beside it: that breaks the
function on purpose and reports whether anything noticed.

## 7. Prefer types the engines can build

Numbers, booleans, strings, and containers of them compose freely. Your own structs and
enums work when Ply can reach a public constructor, or when every field is public.

Refused, and worth knowing before you write the signature: `&mut` parameters, trait
objects, and generic parameters with no concrete type named in the document. Floats and
strings are sampled but never proved, so a `bounded` check on them is refused by name.

When a type genuinely cannot be built, a `routes:` entry naming a public function that
returns one is the escape — Ply then samples *that function's* inputs.

## What to do when Ply refuses

Read the refusal as a fact about the code first, and about Ply second. It names the
parameter and the reason. In order:

1. Can the logic be lifted out of a shell? (rule 1)
2. Is the signature admitting states no caller produces? (rule 2)
3. Is a type too wide, or unbuildable, and would a route fix it? (rules 4, 7)

Only after those, treat it as a Ply limitation and say so — with the parameter and reason
quoted, so the refusal can be judged rather than taken on trust.

## Change authority

| target | authority |
| --- | --- |
| new_code | may-write |
| refactor_to_separate_io | may-do |
| existing_contract | ask-first |
| existing_check | ask-first |
| weakening_a_promise_to_make_a_check_pass | never |

Reshaping code you are writing is the point of this skill. Reshaping code that already
works, to make a check pass, is a change to something a reader relies on and belongs to
the developer. And a promise is never weakened to turn a failing check green: that
converts a real finding into a result nobody can trust, which is the one outcome this
whole tool exists to prevent.
