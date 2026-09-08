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
claim without earned evidence may be reported as **unclaimed**, **unsupported**, or another
specific outcome. Read that outcome and its reason; a refusal is not a counterexample
showing that the code is wrong.

## Verify the first useful piece

Before changing existing claimed behavior, establish its baseline. For new code, write
one meaningful contract and a compiling implementation, then run `cargo ply check` and
`cargo ply verify` on that crate before building further behavior on it. This finds an
unsupported signature or ineffective claim while it is still cheap to address.

Repeat after coherent behavior changes, including changes to helpers that claims depend
on. Follow [Ply Verify's incremental workflow](../ply-verify/SKILL.md#verify-while-building)
for affected roots, counterexample repair, reuse, and the final checks. A passing ordinary
test or declaration check does not substitute for verification of the contract.

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
    fs::write(&path, report_body(rows))?;   // test I/O behavior separately
    Ok(path)
}
```

**The shell is meant to stay unclaimed.** Not everything should be checked, and a function
that only opens a file and hands off is one to leave alone deliberately. Splitting it
further into wrappers that do nothing is worse than leaving it. What matters is that the
logic is not trapped inside it. Leaving the shell outside Ply does not remove the need
for ordinary tests of error handling and integration behavior.

## 2. Consider a total lookup for related inputs

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

Width is not a problem: a twenty-field struct builds exactly as a five-field one does. (An
earlier version of this rule said to stay under a dozen fields. That was a ceiling in Ply's
own generator, since lifted, and never anything about your code.)

What Ply actually needs to build a struct directly:

- **every field public** — it constructs the value with a struct literal, so a private field
  it cannot name means it needs a constructor instead (below)
- **fields named** — a tuple struct has no field names to build against
- **not `#[non_exhaustive]`** — that attribute exists precisely to forbid the literal Ply
  would write

For `fuzz`, containers such as `Vec<String>` and `Option<u32>` are supported, as are
nested public-field types such as `Vec<Inner>` when every field is constructible. This
does not extend to nested types whose constructors reject inputs through `requires` or
return `Result`: those need top-level case rejection, which is unavailable inside a
container. Bounded checking has a narrower supported set. Ply's own `FingerprintInputs` was the case
that proved it (2026-09-05): twenty public fields, two of them lists of another struct, built
in full and run through the real function 256 times. That claim has since come out of Ply's
document under rule 8 — buildable and worth claiming are different questions — and the shape
stays pinned by the generator's own tests, not by the claim.

When a type has real invariants or private fields, give it a public constructor taking fewer
arguments, or declare a route naming a public function that returns one — **a route needs a
public producer that already exists**; it names one, it does not create one.

```yaml
routes: { Handle: open_handle }        # open_handle must be a real public fn
```

When no such function exists, adding one whose only caller is Ply is adding public API for
the tool's benefit, and that is the developer's call, not yours. Rule 9 is what to do
instead.

What Ply cannot do for you is know whether those public fields have a relationship between
them that nothing in the type enforces. It says so out loud rather than assuming: the run
reports that its evidence rests on there being no hidden invariant among the fields, and
that this is assumed, not proved. A type whose methods quietly keep two fields in step is
one where that assumption is wrong, and a value Ply builds may be one your program never
produces.

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

For `fuzz`, supported shapes include numbers, booleans, strings, `Vec`, slices (`&[T]`),
tuples, `BTreeSet`, `BTreeMap`, `Option`, and `Box`, subject to the nesting limits in
rule 4. Your own structs and enums need a supported public constructor or public named
fields with constructible types. These are not general guarantees for every engine: in
this build, `bounded` refuses `Vec`, `BTreeSet`, `BTreeMap`, and user types built through
constructors or fields. Inspect the selected engine's report before promising coverage.

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
statement you can make is "returns 64 characters" — a fact about the hash library, not about
the code. The test that settles it: a `fingerprint` that ignored every one of its inputs
would pass that promise. Nothing that reads a fingerprint depends on its width either; the
record compares two for equality.

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

**A claim whose only honest promise says nothing about the function's job — a fact about
the type, or about a library it calls — should not be declared at all.** It takes up a row in
the document, earns a verdict, and tells the reader nothing. That is rule 6 applied one level
up: the fix for a promise that cannot fail is sometimes to delete the claim rather than to
reword it.

Ply's own document carried exactly that claim for two days. It earned a green verdict the
moment the generator could build the input, and it came out anyway (2026-09-06): a green
that a do-nothing body would also have earned is the thing this rule is about, not evidence
against it.

## 9. Methods, and how a type's own state gets checked

A method taking `&self` is checkable like any function. A method taking `&mut self` — one
that changes the object it is called on — is checkable too: Ply builds the value through
the type's own constructor, runs a random sequence of the type's own operations on it, then
calls the checked method, and the promise says what that call changed in terms of the
value's own readings before and after:

```rust
#[ply::ensures(|result| !*result
    || self.available() as u64 + tokens as u64 == old(self.available()) as u64)]
pub fn try_take(&mut self, tokens: u32) -> bool { /* ... */ }
```

`old(self.available())` is the reading before the call, `self.available()` the reading
after — both ordinary calls to the type's own `&self` methods, nothing new. See
`tests/fixtures/tokenbucket/src/lib.rs` for the worked example this is drawn from.

A method that **consumes** `self` (takes it by value) is still not checkable — it fits the
same shape but needs codegen that does not exist yet. And a promise about a `&mut self`
method has two shapes of its own that Ply refuses by name: taking the "before" reading
through one of the type's own `&mut self` methods (that call would itself change the very
thing the promise is about — read a field, or a `&self` method, instead), and `old(self)`
asking for a copy of the whole object (that would need `Clone`, which nothing here
provides).

A constructor that returns `Result<Self, _>` is recognised: Ply calls it and discards any
generated arguments it rejects, so every value checked is one the constructor accepted —
but a type built that way cannot sit inside a container (`Vec<Inner>`), because a rejection
partway through a list has nowhere to go.

A method promise says what one operation *does*; it is not a replacement for stating what
must always be true of the value. For that, use the component's `state:` and `holds:`
clauses — Ply builds a value through the type's own constructor, calls its public
operations in generated sequences, and checks every `holds:` clause after each one. The two
are complementary: `holds:` catches an invariant broken by *any* sequence of operations,
including ones nobody wrote a method promise for; a method promise catches a bug that
leaves every invariant true but still does the wrong thing (round 3 of the vetting rounds
measured this directly — four of six planted bugs left the whole-value rule perfectly true
and only a promise about the transition saw them). Read the report either way: it names the
operations it could not call, and a promise checked without the one mutator that would
break it is worth much less than the number beside it suggests.

Only the random-sampling tier (`fuzz`) can check a method with a receiver at all — the
exhaustive/proof tier (`bounded`) refuses any receiver, checked or not.

## What to do when Ply refuses

Read the refusal as a fact about the code first, and about Ply second. It names the
parameter and the reason. In order:

1. Can the logic be lifted out of a shell? (rule 1)
2. Is the signature admitting states no caller produces? (rule 2)
3. Is a type unbuildable — private or positional fields, `#[non_exhaustive]`, a shape from
   the rule 7 table — and is there an existing public function a route could name?
   (rules 4, 7)
4. Is the property somewhere you cannot claim it, so an ordinary test is the answer?
   (rule 8)

A refusal may simply be a Ply limitation. Consider these alternatives without changing
required behavior or widening public APIs just to suit the tool. Report the parameter
and reason when leaving verification unresolved; ordinary tests may be the right way
to check the property.

## Change authority

The table states defaults when the task has not already authorized the change. Honor
existing user authorization; do not ask again for work already approved. A request to
review alone does not authorize changing requirements.

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
