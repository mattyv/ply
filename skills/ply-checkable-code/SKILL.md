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

Two words used throughout. A *document* is the `ply.yaml` that names what is checked. A
claim Ply will not attempt comes back **unclaimed** — a verdict meaning "no evidence was
gathered", which is not the same as "this code is wrong".

## 1. Separate deciding from writing

The single highest-value rule, and the one Ply helps with least — so read this part
carefully.

**Ply does not detect side effects.** It refuses a function whose signature takes a
filesystem path *type* (`&Path`, `PathBuf`) because it cannot build one, and it runs
everything else for real. So this is checkable, and Ply will happily execute it 256 times
with names it invented, leaving 256 files behind:

```rust
pub fn save(name: String, body: String) -> std::io::Result<()> {
    std::fs::write(format!("out/{name}"), body)      // Ply will run this
}
```

Nothing stops that but you. Separating the deciding from the writing is the author's job,
not a refusal to wait for:

```rust
// Before: the logic cannot be checked on its own, because the write is in the way.
pub fn write_report(dir: &Path, rows: &[Row]) -> Result<PathBuf> {
    let body = /* real logic: formatting, arithmetic, ordering */;
    fs::write(dir.join("report.txt"), body)?;
    Ok(dir.join("report.txt"))
}

// After: the logic takes data and returns data. Claim this one.
pub fn report_body(rows: &[Row]) -> String { /* the logic */ }

pub fn write_report(dir: &Path, rows: &[Row]) -> Result<PathBuf> {
    let path = dir.join("report.txt");
    fs::write(&path, report_body(rows))?;   // three lines nobody needs to check
    Ok(path)
}
```

**The shell is meant to stay unclaimed.** Not everything should be checked, and a function
that only opens a file and hands off is one to leave alone deliberately. Splitting it
further into wrappers that do nothing is worse than leaving it. What matters is that the
logic is not trapped inside it.

## 2. A lookup keyed by another argument must be total

```rust
pub fn order(
    domain: &BTreeSet<usize>,      // indices into node_ids
    node_ids: &[String],
    edges: &BTreeMap<usize, BTreeSet<usize>>,
) -> Placement
```

Nothing makes `domain` and `node_ids` agree. Real callers built both from the same list,
so they always did — until Ply generated `domain = {15}` with an empty `node_ids` and the
function panicked on an index nobody had thought about. The same bug turned up again a
week later in the layout code, keyed by a *name* rather than an index: an edge naming a
node the caller never declared, looked up in a map built only from the declared ones.

**The fix that shipped both times was to make the lookup total** — `node_ids.get(i)` with
a defined answer for the miss, and a filter that drops an edge naming an unknown node.
Ply's own `order` still takes all three arguments and earns full evidence over every input.
Restructuring the signature so the two travel as one value is the other option, and it is
the right one when the agreement is load-bearing rather than incidental.

## 3. Return values; do not write through `&mut` parameters

```rust
pub fn bump_in_place(counter: &mut u32)   // refused
pub fn bumped(counter: u32) -> u32        // checkable
```

A `&mut` parameter is a shape neither the sampling engine nor the proof engine can build
an input for, so `fuzz` and `bounded` are both refused. A `test` check with worked
examples still runs, because an example's source is spliced in as written — but that is
the concrete cases you wrote and nothing more. A function that computes something should
return it.

## 4. Keep a struct's fields public and named

Width is not a problem. **It was until 2026-09-04** — Ply folded every field into one flat
tuple and the sampling library's tuple trait stops at twelve, so a thirteen-field struct was
refused outright. That ceiling is gone: the leaves are composed in nested chunks now, so a
twenty-field struct builds exactly as a five-field one does. If you have read older advice
here telling you to design around a dozen fields, ignore it — this section said that, and it
was a limit in the tool rather than anything about your code.

What Ply actually needs to build a struct:

- **every field public** — it constructs the value with a struct literal, so a private field
  it cannot name means it cannot build one at all
- **fields named** — a tuple struct has no field names to build against
- **not `#[non_exhaustive]`** — that attribute exists precisely to forbid the literal Ply
  would write

A container of a plain type is fine (`Vec<String>`, `Option<u32>`). A container of *your own*
struct — `Vec<Inner>` — is the one shape still refused as of this writing; Ply reports it
rather than guessing. Ply's own `FingerprintInputs` is the example: twenty public fields, and
refused for that reason alone, not for its width.

When a type has real invariants or private fields, give it a public constructor taking fewer
arguments, or declare a route naming a public function that returns one — **a route needs a
public producer that already exists**; it names one, it does not create one.

```yaml
routes: { Handle: open_handle }        # open_handle must be a real public fn
```

When no such function exists, adding one whose only caller is Ply is adding public API for
the tool's benefit, and that is the developer's call, not yours. Ply's own twenty-field
`FingerprintInputs` is in exactly this position and is left refused on purpose. Rule 9 is
what to do instead.

## 5. Watch what a precondition throws away

A `requires` clause is a filter on generated inputs. A precondition that is *true* can
still be so narrow that almost nothing survives it, and then the check earns nothing at
all.

Measured on Ply's own scheduler: a correct precondition threw away 1025 of 1195 generated
inputs, the sampler gave up, and the verdict went from a real result to **unclaimed** — no
evidence, not a failure. The precondition was right; the code was the thing to change.

So when a promise needs a narrow precondition to hold, ask whether the function should
instead be **total** — handling the case rather than excluding it.

**But do not make it total by inventing an answer.** The test is whether the handled case
has a meaning a caller would accept. Ply's scheduler passed it: the missing name is
consulted only to break a tie, so falling back to the empty string provably changes no
valid input's behaviour. Returning a plausible-looking value for an input that is a caller
bug fails it, and swallowing an error to raise the accepted-input count is the same
mistake wearing a different hat.

Related, and the reason this rule is not just about counts: **a panic is a finding, an
`Err` is a handled case.** If a generated input makes the function panic, Ply reports a
broken promise with the input that did it. If the same input comes back as `Err`, the
function handled it and the promise still has to hold. Choosing between them is a design
decision, not an implementation detail.

## 6. Write a promise that can fail

A promise true of every possible body earns a green verdict and tells a reader nothing.

| Write | Not | Why |
| --- | --- | --- |
| `result.len() == 64` | `result.len() >= 0` | The second holds for every body |
| `\|r\| r.bid <= r.ask` | `\|r\| r.bid >= 0` | State the relationship, not the type |

Prefer a promise relating **inputs to output** over one about the output alone. "Returns a
number" is a type. "Returns at least what it was given" is a promise. So
`result.0.len() + result.1.len() == domain.len()` — everything given back, nothing
invented — beats `!result.0.is_empty()`, which is not vacuous but is a symptom rather than
the property.

Two mechanical points that decide whether a promise is worth anything:

- **A `fuzz` check needs a promise to check.** Without one there is nothing to assert and
  the claim is refused.
- **Watch which half of an `||` does the work.** Ply reports the split. On its own document,
  `result.is_err() || !s.is_empty()` was decided by the first half in 256 of 256 cases —
  random text is never valid input, so the interesting half never ran. Rewriting it to
  "a rejection always quotes the text it rejected" moved all 256 onto the half that says
  something. Where the interesting case is rare rather than reachable, declare `test`
  alongside `fuzz` and write the cases out.

When unsure whether a promise has teeth, declare `mutate` beside it: that breaks the
function on purpose and reports whether anything noticed.

## 7. Prefer types the engines can build

Numbers, booleans, strings, `Vec`, `BTreeSet`, `BTreeMap`, `Option` and `Box` of them
compose freely. Your own structs and enums work when Ply can reach a public constructor,
or when every field is public **and named**.

Refused, and worth knowing before you write the signature:

| Shape | What happens |
| --- | --- |
| `&mut` parameters | Refused for `fuzz` and `bounded` (rule 3) |
| `HashMap`, `HashSet` | Not recognised — use the `BTree` versions where you can |
| Tuple structs, tuple enum variants | Refused by name |
| Trait objects, `impl Trait`, closures | Refused |
| Generic parameters | Refused. Naming a concrete type in the document is described in the spec but **not built** — do not plan around it |
| Filesystem paths (`&Path`, `PathBuf`) | Refused (rule 1) |
| Floats and strings under `bounded` | Sampled, never proved — a proof check on them is refused by name |

When a type genuinely cannot be built, a route naming an **existing** public function that
returns one is the escape — Ply then samples *that function's* inputs (see rule 4 for what
to do when there is no such function).

## 8. Some functions should be checked by an ordinary test instead

A refusal is sometimes telling you the function you picked is not where the property lives.

Ply's own `fingerprint` is one line: it hashes a canonical byte encoding of a twenty-field
struct. The encoding is the part worth checking — it length-prefixes every value so that a
contract containing a newline cannot be arranged to hash the same as two different fields.
That encoding is a **private** helper. So the claim sits on the wrapper, where the only
statement you can make is "returns 64 characters", which is a fact about the type rather
than about the code.

The honest answer is not to widen the API until the checker can reach it. It is a plain
Rust test. Ply's has one: it mutates each of the twenty inputs in turn and asserts the hash
moves, naming which input stopped counting when it fails. That is better coverage than any
promise about the wrapper, and it needs nothing from Ply at all.

So, before contorting a signature to make a claim possible, ask which of these is true:

| The property lives... | Do this |
| --- | --- |
| in the function being claimed | Claim it |
| in a private helper it calls | Write an ordinary test; leave the wrapper unclaimed |
| in a public helper it calls | Claim the helper instead |

**A claim whose only honest promise is a type-level fact should not be declared at all** —
it takes up a row in the document, earns a verdict, and tells the reader nothing. That is
rule 6 applied one level up: the fix for a promise that cannot fail is sometimes to delete
the claim rather than to reword it.

## 9. Methods, and how a type's own state gets checked

A method taking `&self` is checkable like any function. `&mut self` and methods that
consume `self` are not, and a constructor that returns `Result<Self, _>` is not recognised
as a way to build one.

The way to check a type that changes is not to claim its mutating methods one by one. It is
to state what must always be true of the value, under the component's `state:`; Ply then
builds one through the type's own constructor, calls its public operations in generated
sequences, and checks every clause after each one. Read the report: it names the operations
it could not call, and a promise checked without the one mutator that would break it is
worth much less than the number beside it suggests.

## What to do when Ply refuses

Read the refusal as a fact about the code first, and about Ply second. It names the
parameter and the reason. In order:

1. Can the logic be lifted out of a shell? (rule 1)
2. Is the signature admitting states no caller produces? (rule 2)
3. Is a type too wide, or unbuildable, and is there an existing public function a route
   could name? (rules 4, 7)
4. Is the property somewhere you cannot claim it, so an ordinary test is the answer?
   (rule 8)

Only after those, treat it as a Ply limitation and say so — with the parameter and reason
quoted, so the refusal can be judged rather than taken on trust.

## Change authority

| target | authority |
| --- | --- |
| new_code | may-write |
| refactor_new_code_to_separate_io | may-do |
| refactor_existing_io | ask-first |
| existing_contract | ask-first |
| existing_check | ask-first |
| weakening_a_promise_to_make_a_check_pass | never |

Reshaping code you are writing is the point of this skill. Splitting a function that
already works — even to make a check possible — is a change to something a reader relies
on, and belongs to the developer: propose it, name the function, and wait.

And a promise is never weakened to turn a failing check green: that converts a real finding
into a result nobody can trust, which is the one outcome this whole tool exists to prevent.
