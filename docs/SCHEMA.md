# `ply.yaml` — the reference

Ply is a `cargo` subcommand. You tell it, in one YAML file, what your Rust code is
supposed to do and how hard you want that checked; it drives existing checking tools
(a model checker, a property-test runner, a mutation tester), and it reports back one
answer per function — including, and this is the part most tools skip, an explicit
answer when it checked *nothing*.

This page is the reference for that file. It is written for someone who has never read
the design specification and never seen this project. Everything here was run against
the build in this repository on 2026-08-25; where the tool cannot yet do something the
file lets you write, this page says so in the same breath as describing it.

The formal definition of the file is `schema/ply.schema.json`. That file is embedded in
the Ply binary and is what actually accepts or rejects your document. If this page and
that file ever disagree, that file wins and this page has a bug.

**Contents.** [1 Ten minutes to a first file](#1-ten-minutes-to-a-first-file) ·
[2 The commands that exist](#2-the-commands-that-exist) ·
[3 The shape of the document](#3-the-shape-of-the-document) ·
[4 Components](#4-components) ·
[5 Function claims](#5-function-claims) ·
[6 Contracts, and the boundary into old code](#6-contracts-and-the-boundary-into-old-code) ·
[7 What a verdict means](#7-what-a-verdict-means) ·
[8 Architecture](#8-architecture-edges-denials-capabilities-ownership) ·
[9 Externals and entry points](#9-externals-and-entry-points) ·
[10 Trusted claims](#10-trusted-claims) ·
[11 Unresolved decisions](#11-unresolved-decisions) ·
[12 Names, paths, numbers](#12-names-paths-numbers-the-lexical-rules) ·
[13 When Ply says no](#13-when-ply-says-no) ·
[14 What this build does not do](#14-what-this-build-does-not-do)

If you are here because new code has to call old code that carries no promises,
section 6 is the whole story and the rest is reference.

---

## 1. Ten minutes to a first file

### Where the file goes

Put a file called `ply.yaml` in a crate directory, next to `Cargo.toml`. Every command
takes that directory as its argument:

```
cargo ply check .
```

One directory, one file. The design specification describes discovering many
`ply.yaml` and `*.ply.yaml` files across a workspace and merging them; **that is not
built** — Ply reads exactly the one file at the path you give it, and checks exactly
that one crate.

### The smallest useful file

Two things go together: a contract on the function, written as an attribute in the
Rust source, and a claim in `ply.yaml` saying how hard to check it.

```rust
// src/lib.rs
#[ply::requires(amount_cents <= 100_000_000 && fee_cents <= 10_000_000)]
#[ply::ensures(|result| *result >= amount_cents)]
pub fn total_debit_cents(amount_cents: u32, fee_cents: u32) -> u32 {
    amount_cents + fee_cents
}
```

```yaml
ply: 1

components:
  billing:
    anchor: billing
    fns:
      total_debit_cents:
        checks: [bounded(2)]
```

`requires` is the precondition — what a caller must guarantee before calling.
`ensures` is the postcondition — what the function guarantees about the value it
returns; `result` is that value, and `*result` because the closure receives it by
reference. `bounded(2)` asks the model checker to explore every execution, for every
possible input, with loops unrolled twice.

Those attributes come from a small crate you add as a dependency:

```toml
[dependencies]
ply = { package = "ply-attrs", path = "…" }

[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ["cfg(kani)"] }
```

Under an ordinary `cargo build` the attributes vanish and the function compiles exactly
as written; only the model checker ever sees them. The `lints.rust` block is not
optional if you want a warning-clean build — without it, `cargo build` complains about
the `kani` configuration flag the attributes leave behind.

Then:

```
cargo ply check .     # is the document well-formed, and do its claims point at real code?
cargo ply verify .    # run the checks, and report what each function actually earned
```

`verify` on this file takes about half a minute and prints:

```
workspace — bounded(2)
  billing — bounded(2)
    total_debit_cents — bounded(2)
```

Read that as: there is no pair of inputs satisfying the precondition, within a loop
bound of 2, for which `total_debit_cents` breaks its promise. Not "the tests passed" —
*every* input, inside that bound.

Expect your second function to be less obliging. Proof cost depends on what the body
does, not only on the types in the signature, and the default time budget is small on
purpose. Section 6 has the measured example and what to do about it.

---

## 2. The commands that exist

Four are built. Each takes a crate directory and supports `--json`, which emits a
machine-readable envelope with a node per claim and a list of diagnostics.

| Command | What it does | Runs engines? |
|---|---|---|
| `cargo ply check <dir>` | Validates the document, and checks that every function claim points at a function Ply can find. | No |
| `cargo ply verify <dir>` | Runs the declared checks and reports a verdict per function. | Yes |
| `cargo ply audit <dir>` | Lists the trust surface: every place your evidence rests on something Ply never checked. | No |
| `cargo ply worklist <dir>` | Lists what is owed: open decisions you recorded, and promises nothing has tested yet. | No |

`check` covers two of its four intended jobs, and says so at the bottom of its own
output: it does not compare claims against a record of past runs (that needs a
`ply.lock` file nothing writes yet), and it does not check the architecture rules
described in section 8 below.

`verify`'s useful flags:

- `--engine-timeout <seconds>` — the time budget for each check. Left off, Ply picks
  one from the shape of the check: 60s for most things, more for a check over a `Vec`,
  and a floor of 300s when a proof stands on a declared promise instead of real code
  (those are dramatically slower). Passing the flag overrides all of that.
- `--fail-on warn|evidence|error` — what makes the run fail. See section 7.
- `--seed <64 hex characters>` — replay one recorded `fuzz` run exactly.

### What you need installed

`test` and `fuzz` need nothing beyond cargo — Ply generates a small test crate and lets
cargo fetch the property-test library. `bounded` needs Kani, AWS's model checker for
Rust (`cargo kani`); this build is pinned against 0.67.0. `mutate` needs
`cargo-mutants`. If an engine you asked for is missing, Ply reports that as a missing
engine and exits 3 — it never installs anything for you.

### What `verify` writes into your crate

Worth knowing before you run it on a working tree you care about. `cargo ply verify`
generates a file `src/ply_generated.rs` holding the proof harness, and appends a
`mod ply_generated;` line to your `src/lib.rs`. Both are marked "generated by Ply — do
not edit". The harness is behind a configuration flag that only the model checker sets,
so it does not affect an ordinary build. The `test`/`fuzz` tier additionally writes a
small crate under `target/ply/`. Ply owns everything under `target/ply/` and every
generated test whose name begins with `ply_cex_`.

**These do not exist yet**, though the design specification describes them:
`cargo ply tree` (the verdict tree as a browsable, collapsible view), `cargo ply
accept` (re-bless claims after an intentional change), `cargo ply doctor` (report which
checking engines are installed), `cargo ply synth` (have a model write a body from its
contract), and `cargo ply skill` (generate an agent-facing reference). `verify`'s
`--only-changed` and `--force` flags are also not built.

---

## 3. The shape of the document

Everything below is optional except `ply: 1`. This is a complete, valid document
exercising most of the grammar; the sections that follow take it apart.

```yaml
ply: 1                            # schema version — required, and 1 is the only value

externals:                        # systems or people outside this codebase
  venue:
    note: "the exchange: sends us market data, accepts our orders"

components:                       # the architecture: named units of your code
  ingest:
    anchor: ingest                # the crate or module this component IS
    components:                   # components nest
      feed:
        anchor: ingest::feed
        uses: [net]               # the effects this component is allowed
      decoder:
        anchor: ingest::decode
        pure: true                # no effects at all
        profile: hot_path         # a named set of bans
    fns:                          # function claims, keyed by path below the anchor
      decode:
        checks: [fuzz(4096)]      # how hard to check it
        entry: [venue]            # this function is reachable from outside

  book:
    anchor: ingest::book
    strict: true                  # architecture findings here are errors, not warnings
    owns: [ingest::book::OrderBook]   # only this component may mutate that type
    checks: [bounded(2)]          # default checks for functions in this component

edges:
  - ingest.feed -> ingest.decoder           # feed may call decoder
  - "ingest.decoder ~> book : Tick"         # Ticks flow from decoder to book
  - "venue ~> ingest.feed : RawFrame"       # data crosses the system boundary

deny:
  - "* -> book except ingest.decoder"       # nothing else may call into book

profiles:
  hot_path: [no_panics, exhaustive_match]

unresolved:                       # decisions nobody has made yet
  - { id: 7, note: "backpressure policy when the ring is full: drop the frame or block" }
```

A note on YAML: strings containing `:` or starting with a character YAML treats
specially need quoting. `ingest.feed -> ingest.decoder` is fine unquoted; anything with
a `~>` and a type after a colon is not. When in doubt, quote it.

---

## 4. Components

A component is a name you give to a piece of your code, so you can then say things
about it. Nothing more mystical than that.

```yaml
ply: 1
components:
  billing:
    anchor: billing
```

`anchor:` is required, and is what makes the name mean something: it names the crate,
or a module path inside a crate, that this component *is*. A component with no anchor
is a label with nothing behind it, which is why the schema refuses one.

Component names are lower-case ASCII with underscores — `order_book`, not `OrderBook`
or `order-book`. Only siblings can collide: a nested name is qualified by its parent, so
two different parents may each have a child called `store`, but two top-level components
called `store` is an error (`E0202`).

### Nesting

Components nest, and nesting means containment: the inner component is part of the
outer one.

```yaml
ply: 1
components:
  ingest:
    anchor: ingest
    components:
      feed:
        anchor: ingest::feed
      decoder:
        anchor: ingest::decode
```

You refer to a nested component with a dot: `ingest.feed`. A bare `feed` also works as
long as no other component in the document is called `feed`; if two are, Ply refuses
the ambiguous reference and lists the candidates rather than guessing (`E0206`).

Containment already implies permission. A parent may call into its own children and
they into it, without any declared edge — that is the same relationship as one function
calling another inside a single module. Writing an edge between a component and its own
descendant is redundant, and Ply says so (`W0409`).

Both commands read the whole tree. A claim written inside a nested component is
validated by `check`, run by `verify`, and named the same way by both —
`ingest.feed::Feed::pump`, the component's dotted name and the function key.

One limit sits with nesting, and it is about `anchor:` rather than about depth. A
function key is read as a path from the **crate root**, so `verify` can only run claims
under a component anchored at the crate itself. A component anchored at a module —
`anchor: ingest::book`, while you are checking the crate `ingest` — has its claims
reported and not run (`W0303`), and the message names the spelling that would run: move
the claim under the crate-anchored component and key it `book::OrderBook::apply`.

### What a component buys you today

Honestly: less than the file suggests. `anchor:` is load-bearing — it decides which
claims belong to the crate you are checking and which describe code somewhere else. The
architecture fields (`uses`, `pure`, `owns`, `profile`, `strict`, and the `edges:` and
`deny:` lists) are validated for form and for reference resolution, and then **nothing
compares them against your code**. That whole tier is planned, not built. Section 8
says exactly what is and is not enforced, because a rule you believe is running and
which is not is worse than no rule.

---

## 5. Function claims

Under a component's `fns:` you list the functions you are making claims about. The key
is the function's path *relative to the component's anchor*.

```yaml
ply: 1
components:
  billing:
    anchor: billing
    fns:
      fee_cents:
        checks: [bounded(2)]
      fees::bps_for_tier:              # a function inside a module
        checks: [fuzz(256)]
      Invoice::total:                  # a method, written Type::method
        checks: [test]
```

Keys are plain path segments joined by `::`. Generics, lifetimes, and trait-qualified
spellings like `<T as Trait>::f` are not accepted (`E0304`).

> **What this build can actually reach.** Ply follows your crate's own structure to find
> a function: `use` imports (renames and groups included), inline `mod` blocks, file
> modules (`mod rates;` → `rates.rs` or `rates/mod.rs`), nested modules, and re-exports.
> So `fees::bps_for_tier` above works, and so does a claim written the way you would say
> it out loud for a function three modules down.
>
> Two things are still out of reach, and each says which it is rather than "no such
> function":
>
> - **A method in an `impl` block.** `Invoice::total` parses and validates, and cannot
>   be verified today.
> - **A private function below the crate root.** Ply writes its harness as a module at
>   the top of your crate, so a `fn` that is private inside `rates` — or a private `mod
>   rates` — is a name that harness cannot write. Make the function, and every module
>   between it and the crate root, `pub` or `pub(crate)`. A private function *at* the top
>   of `src/lib.rs` is fine: the generated module sits beside it and can see it.

### The checks list

`checks:` is how hard you want the function checked. Five kinds exist:

| Check | What runs | What it earns |
|---|---|---|
| `test` | Ordinary `#[test]`s: your `examples` entries, plus generated cases run through the real function with the contract asserted. | `tested` |
| `fuzz(n)` | A property-test run of `n` generated inputs, with shrinking. `1 ≤ n ≤ 1000000`. | `fuzzed(n)` |
| `bounded(k)` | A model-checking proof: every execution, every input, loops unrolled `k` times. `1 ≤ k ≤ 64`. | `bounded(k)` |
| `prove` | An unbounded deductive proof. | `proved` |
| `mutate` | Deliberately breaks the code and checks that your other checks notice. | Strengthens the verdict, or warns that the specification is weak. |

They are not alternatives so much as rungs. `test` says "these inputs work".
`fuzz(n)` says "*n* random inputs worked". `bounded(k)` says "*every* input worked, as
long as loops run at most *k* times". Two claims of the same kind are the same rung —
`fuzz(4096)` is not a stronger *kind* of evidence than `fuzz(256)`, just more of it.

A number out of range is rejected with the reason, not just a code: `bounded(0)` gets
told that a bound of 0 would prove nothing (`E0203`).

`mutate` needs something to break: it must appear alongside a `test` or `fuzz` entry in
the same list, because those are the checks a planted bug has to survive. On its own it
is an error (`E0504`).

```yaml
ply: 1
components:
  billing:
    anchor: billing
    fns:
      fee_cents:
        checks: [fuzz(4096), test, mutate]
```

**`prove` has no engine in this build.** Writing it is accepted, and `verify` reports a
missing engine rather than failing the check (`W0110`).

### Defaults, and one trap

If a function's entry has no `checks:` at all, `verify` picks one from the function's
shape: `bounded(2)` if it has a contract and a signature the model checker can build
inputs for, `fuzz(256)` if it has a contract of a shape only the property-test runner
can reach, and nothing at all if it has no contract.

**`checks: []` does not mean "check nothing".** An empty list is treated the same as no
list, so a contracted function written with `checks: []` still gets the shape-aware
default and still runs. If you want a function listed but unchecked, leave the
contract off, or give it a `requires:`/`ensures:` entry with no `checks:` — which means
something specific, and is section 6's topic.

A component may declare a default for the functions under it:

```yaml
ply: 1
components:
  book:
    anchor: ingest::book
    checks: [bounded(2)]
    fns:
      apply: {}
      last_px:
        checks: [fuzz(256)]
```

A function's own non-empty list replaces the inherited one entirely — there is no
merge. `last_px` above runs `fuzz(256)` and nothing else. Note that `cargo ply check`
honours this inheritance and `cargo ply verify` does not: `verify` reads only the
function's own list and otherwise falls back to the shape-aware default. Until that is
reconciled, write the checks you mean on the function.

### `examples`

Concrete cases, written as ordinary Rust `==` expressions, compiled as plain tests and
run by the `test` check.

```yaml
ply: 1
components:
  billing:
    anchor: billing
    fns:
      fee_cents:
        checks: [test]
        examples:
          - "fee_cents(1_000, 150) == 15"
          - "fee_cents(0, 10_000) == 0"
```

Examples are exempt from the restrictions on contract expressions — they are just Rust.
The cost of that freedom: nothing type-checks them until the generated test crate is
compiled. An entry that does not compile takes the whole harness down with it, and
because the `test` and `fuzz` checks share one harness, *neither* runs. Ply reports
that as a tool error carrying the compiler's own first message, never as a pass and
never as a failure (`X0901`) — no cases ran, so there is nothing to conclude either way.

### `check_with`

For a generic function, names one concrete type per type parameter:

```yaml
ply: 1
components:
  ring:
    anchor: disruptor::spsc
    fns:
      try_push:
        checks: [bounded(3)]
        check_with: { T: u64 }
```

**Parsed and validated; not yet used.** No instantiation happens, so a generic function
is reported as an unsupported shape today whether or not you write this.

### `mode`

`mode: check` (the default) means Ply verifies the body you wrote. `mode: synth` means
a model writes the body from the contract. Synthesis is not built; the field is accepted
and `cargo ply audit` lists any function carrying it, on the grounds that a body written
by a model is something a reviewer should know about.

---

## 6. Contracts, and the boundary into old code

This is the part most people come for: you are adding new code to a codebase that has
two years of existing code in it, and the existing code makes no promises about
anything.

### Where a contract lives

The canonical place is on the function, as attributes:

```rust
#[ply::requires(amount_cents <= 100_000_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tier_fee_cents(amount_cents: u32, tier: u8) -> u32 { … }
```

`ply.yaml` can also carry `requires:`/`ensures:` entries. They serve a different
purpose, and the difference matters:

```yaml
ply: 1
components:
  billing:
    anchor: billing
    fns:
      fee_cents:
        checks: [bounded(2)]
        requires:
          - "bps <= 10_000"
        ensures:
          - "|result| *result <= amount_cents"
```

The design intent is that these are combined with the function's own attributes. **They
are not, in this build.** A contract written in `ply.yaml` for a function that is itself
being checked is *not* included in that function's own check, and `verify` says so out
loud rather than dropping it silently (`W0510`). What such an entry *does* do is let
callers assume it — which is the whole mechanism the rest of this section is about. If
you want a clause checked against the function it describes, put it on the function.

### What you can write in a contract

Boolean Rust over the function's parameters and `result`: comparisons, `&&`/`||`/`!`,
arithmetic, field access, `.len()`, `.is_ok()`/`.is_err()`/`.is_some()`, `matches!()`,
and literals. An `ensures` clause is always a closure of the form `|result| expr`.

The design also admits calls to helper functions marked `#[ply::pure]`. That attribute
is not defined in this build, so a helper call in a contract is just an ordinary
function call as far as the engines are concerned — see section 14.

Three honest caveats:

- **Nothing validates that subset today.** Write something outside it and Ply will not
  tell you; the expression is passed straight to the engine, and you find out from the
  Rust compiler or from a tool error.
- **`old(expr)` — the value an expression had on entry — half works.** It reaches the
  model checker unchanged and is accepted there for by-value parameters. It does *not*
  work on the `test`/`fuzz` path: the generated harness fails to compile with "cannot
  find function `old` in this scope", which surfaces as a tool error. And the case
  `old()` exists for — a function that mutates something through `&mut` and returns
  nothing — does not work at all in this build. Both were run to confirm.
- **A `fuzz` check needs a postcondition.** With no `ensures` there is nothing for the
  generated inputs to be checked against, so nothing runs and Ply says so rather than
  reporting a pass. If what you have is concrete cases rather than a general property,
  use `test` with `examples:` instead.

### What the engines can build inputs for

A function can only be checked if the engine can construct its arguments. This is
narrower than you would guess, and narrower than the design specification's own list,
so here is what this build actually handles:

| Shape | `bounded` (model checker) | `fuzz` / `test` |
|---|---|---|
| `u8`–`u64`, `i8`–`i64`, `bool`, `char` | yes, cheap | yes |
| `Option<T>`, `Result<T, E>` of the above | yes | yes |
| `[T; N]` — a fixed-size array | yes, and this is the *preferred* way to express bounded data | yes |
| `Vec<u8>` | yes | yes |
| `Vec<T>` for other scalar `T`, `BTreeSet<T>` | **no** | yes |
| `&T` — a shared reference to any of the above | yes | yes |
| `&mut T`, `&[T]`, `String`, structs, enums, `HashMap`, generics, trait objects | **no** | **no** |

Type aliases resolve, so `pub type AccountId = u64;` is a `u64` here. Anything in the
last row is reported as an unsupported shape, by name, rather than attempted (`V0505`). That is deliberate: an unsupported shape is a fact Ply reports, not
a crash and not a silent skip. If a function has a contract but neither engine can build
its inputs, it gets no checks and an unsupported verdict.

Two limits narrow what *every* `bounded` verdict means, however clean it looks.
Generated arguments never point at the same thing as each other, so a bug that needs two
parameters to alias is invisible. And a type's own invariants are assumed rather than
proved, so a proof can rest on an invariant the code itself breaks.

And a supported signature is not a promise the check finishes: whether a proof completes
depends on the *body*, not just the parameters. Here is a second function in the same
crate — two scalar parameters again, one line of arithmetic:

```rust
#[ply::requires(amount_cents <= 100_000_000 && bps <= 10_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn fee_cents(amount_cents: u32, bps: u32) -> u32 {
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}
```

Two scalar parameters, a shape the table above calls cheap. At the default budget:

```
workspace — timeout
  billing — timeout
    fee_cents — timeout
[K0601] Kani could not finish checking `fee_cents` within its 60s time budget — this is
an exhausted search, not a broken promise …
```

The widened multiply-and-divide is what costs; `cargo ply verify . --engine-timeout 300`
proves the identical function in about two minutes. The same effect shows up more
sharply elsewhere: an iterator chain over a one-element `Vec` has timed out where a
hand-written indexed loop over the same data proved in seconds.

The thing to take from this is not the number. It is that a timeout is reported as a
timeout — never as a pass, and never as a failure. Nobody learned anything, and the
result says exactly that. When you hit one, raise the budget first; if that does not
finish either, the body is doing something the solver cannot get through, and `fuzz` is
the tier that will still tell you something.

### New code calling old code

Here is the case, and it is the one most people arrive with. `withdrawal` is a new
crate. `ledger` is two years old, has no contracts, and nobody is going to annotate it.

```rust
// withdrawal/src/lib.rs
#[ply::requires(amount_cents <= 100_000_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tier_fee_cents(amount_cents: u32, tier: u8) -> u32 {
    let bps = ledger::fees::bps_for_tier(tier).min(10_000);
    ((amount_cents as u64 * bps as u64) / 10_000) as u32
}
```

```yaml
ply: 1
components:
  withdrawal:
    anchor: withdrawal
    fns:
      tier_fee_cents:
        checks: [bounded(2)]

  ledger:
    anchor: ledger

edges:
  - withdrawal -> ledger
```

A proof reasons about a function's callees through their contracts. `bps_for_tier` has
none, so there is nothing to reason with. **Ply refuses to descend into it**:

```
workspace — unclaimed
  withdrawal — unclaimed
    tier_fee_cents — unclaimed
[W0512] withdrawal::tier_fee_cents — Ply did not check `tier_fee_cents`: proving it
would mean descending into `ledger::fees::bps_for_tier` (called at line 4, column 15),
and no contract anywhere describes what that code promises — not on the function
itself, and not in ply.yaml. […] So this check earned no evidence at all — the verdict
is `unclaimed`, never `bounded(2)`, and never a violation.
```

The verdict is `unclaimed` — not `bounded(2)`, and not a violation. The exit code is 1,
because a run that checked nothing is not a clean run.

Three things are deliberate here. The refusal is decided from the call graph *before*
any engine starts, so it costs milliseconds (that run took 11ms) rather than the whole
time budget. The diagnostic names **the callee and where it is called**, not just the
caller — "`tier_fee_cents` could not be checked" would tell you nothing you can act on.
And the alternative would be worse in both directions: pulling the real body into the
proof either exhausts the budget and reports nothing, or produces a clean-looking
`bounded(2)` whose meaning quietly includes code nobody vouched for.

### Writing a promise for old code

You do not have to touch the legacy source, and you do not have to annotate it. Give
the callee a contract in `ply.yaml`, with **no `checks:`**:

```yaml
ply: 1
components:
  withdrawal:
    anchor: withdrawal
    fns:
      tier_fee_cents:
        checks: [bounded(2)]

  ledger:
    anchor: ledger
    fns:
      fees::bps_for_tier:
        ensures:
          - "|result| *result <= 10_000"

edges:
  - withdrawal -> ledger
```

Note the second component: `anchor: ledger` names the *other crate*, and the function
key is the path below that anchor. An entry with a contract and no `checks:` is a
**boundary contract declaration** — it exists so callers can assume something, not so
this function gets verified. It earns no node of its own in the results; reporting it as
an unchecked claim would say the opposite of what you wrote.

Now the proof runs. It replaces the call with the promise, and the verdict says so:

```
workspace — bounded(2)
  withdrawal — bounded(2)
    tier_fee_cents — bounded(2)
[W0511] withdrawal::tier_fee_cents — `tier_fee_cents` earned bounded(2), but
conditionally: the proof used the contract declared in ply.yaml for each callee it
crosses into, instead of that callee's real body. Assumed:
`ledger::fees::bps_for_tier`: ensures |result| *result <= 10_000. That is what
`conditional` means here — the result holds if those promises do. Nothing has checked
them against the real code yet, so each one is owed evidence rather than settled: an
assumed contract nobody exercises is green paint.
```

Two facts sit on that node, and they are different facts:

- **`conditional`** — the verdict rests on an assumed contract. If the promise is wrong,
  the verdict is wrong with it.
- **`owed-evidence`** — nothing has yet checked that promise against the real code.
  Trust that is never checked is green paint.

### The promise has to say something

Before it runs a proof that stands on a promise, Ply checks that the promise is not
empty. It asks two questions about each clause you wrote, over the clause on its own:
*can any value satisfy it*, and *can any value break it*. Both are answered exhaustively
over the type, and both together cost about a second.

- **Nothing can satisfy it** — say you wrote `|result| *result > 10_000 && *result < 5`.
  Assuming something impossible lets a proof conclude anything at all, so the proof would
  come back green no matter what your function does. Ply refuses to run it: `E0502`, the
  function earns no evidence, and the message quotes the clause back to you.
- **Nothing can break it** — say you wrote `|result| *result >= 0` about a function
  returning `u32`. That is true of every `u32`, so it constrains nothing: inside the
  proof the callee was replaced by *any* value, not by the promise. Your function's
  verdict still stands and is honest — it holds whatever that callee returns — but the
  clause is not an assumption and nothing is owed on it. `E0503` says so, and the
  `conditional` message stops listing it as a debt.
- **Ply could not tell** — a precondition over a parameter type the model checker cannot
  build a value for, a clause it cannot parse, or a solver that ran out of time. `W0514`,
  and the promise is reported as unchecked rather than as fine.

Both `E0502` and `E0503` are errors, so a run carrying either does not pass. What they
cannot tell you is whether a promise is *strong enough* — one that rules out a single
value out of four billion is neither impossible nor trivial — nor whether your real code
actually keeps the promise. That second one is what `owed-evidence` is about, and
fuzzing the callee is what closes it.

In a codebase that is mostly legacy, `conditional` is the *normal* state, not an alarm.
`owed-evidence` is the half that is supposed to close.

Two practical notes. Proofs that stand on a promise are much slower than proofs that
stand on real code — the promise hands the solver a symbolic value where the body
returned one of four concrete ones — which is why the default budget has a 300s floor
when a stub is involved. The run above took about two minutes. And the human-readable
output prints the verdict plus this diagnostic; the `conditional` and `owed-evidence`
statuses themselves appear as a `statuses` list in `--json`.

### Seeing and settling the debt

`cargo ply audit` lists what your evidence permanently rests on. `cargo ply worklist`
lists the same boundary from the other side — the part somebody means to finish:

```
  owed evidence (1)
    `withdrawal::tier_fee_cents` (at line 4, column 15)
      `tier_fee_cents`'s proof stands on a promise `ply.yaml` makes for
      `ledger::fees::bps_for_tier` — ensures |result| *result <= 10_000 — and nothing
      has run the real `ledger::fees::bps_for_tier` against it. […] To close it, add
      `checks: [fuzz(256)]` to its `ply.yaml` entry — fuzzing crosses a legacy boundary
      by simply calling the code, so it tests the promise against the real
      `ledger::fees::bps_for_tier`.
      blocks: `withdrawal::tier_fee_cents` keeps a `conditional` verdict until the
              promise made for `ledger::fees::bps_for_tier` is checked against the real
              body.
```

That is the discharge route, and it works because the property-test tier needs no
contracts on anything: it simply *runs* the code. Adding `checks: [fuzz(256)]` to the
callee's entry tests your promise against the real legacy body.

One wrinkle here. `verify` checks one crate at a time, so when the callee lives in
another crate — as it does above — that `checks:` entry will not run from the caller's
directory, and Ply says so rather than pretending (`W0303`). Run `cargo ply verify` in
the `ledger` crate to settle it there. When caller and callee are in the *same* crate,
adding the check to the same document is all it takes.

### Four things to know before you trust a boundary contract

Each of these is a way a green result can mean less than it looks. They are real gaps in
this build, not hypotheticals.

1. **A promise that cannot be satisfied proves everything.** Writing
   `ensures: ["|result| false"]` makes the assumption unsatisfiable, and the caller's
   proof then passes vacuously. Nothing detects this yet. Write promises that are true
   and non-trivial, and prefer to discharge them with a `fuzz` check rather than leaving
   them standing.
2. **A promise does not go stale.** If the legacy code changes under a standing
   assumption, nothing notices. (Attestations have the same gap — see section 11.)
3. **The refusal only inspects the claimed function's own body.** If your function calls
   a *contracted* callee, whatever *that* callee calls still travels into the proof
   unnamed.
4. **Calls into `std`, `core`, or a registry crate are left alone**, because Ply cannot
   read their source. A `bounded` verdict can still include a body Ply never examined.
   Calls Ply's reader genuinely cannot see — generated by a macro, made through a
   function pointer or a trait method — are likewise not call sites for this rule.
   Method calls on a receiver (`x.min(10_000)`, `v.len()`) are excluded on purpose:
   they are overwhelmingly `std`, and flagging them would fire on every ordinary line
   of Rust.

If Ply follows a path into first-party source and *cannot read it* — a module whose file
is missing, a path dependency that will not open — it refuses rather than descending
(`W0513`). Not being able to look is not the same as there being nothing there.

One last thing that will bite if it happens to you: a boundary promise is matched by the
path the caller writes. If the dependency is renamed in `Cargo.toml`
(`ledger = { package = "real-name", … }`), the anchor will not match and the callee
classifies as unclaimed. That fails loudly — you get the refusal above for a callee
whose contract you just wrote — rather than quietly.

---

## 7. What a verdict means

Every function claim ends with exactly one verdict. Six of them, weakest to strongest:

| Verdict | In plain words |
|---|---|
| `violation` | A check found a concrete input for which the promise does not hold. There is a failing witness. |
| `unclaimed` | **Nothing was checked.** No claim, or a claim Ply refused to run — most often the legacy-boundary refusal above. |
| `tested` | The examples and generated cases ran and passed. |
| `fuzzed(n)` | *n* generated inputs ran and passed. |
| `bounded(k)` | Every input passed, for every execution in which loops run at most *k* times. Says nothing beyond that bound. |
| `proved` | Proved for all inputs with no bound. (No engine in this build.) |

Alongside the verdict, a node can carry **statuses**. These are not weaker verdicts;
they are different kinds of fact, and they travel upward as flags:

| Status | In plain words |
|---|---|
| `conditional` | The verdict rests on a contract that was assumed, not checked. |
| `owed-evidence` | One of those assumed contracts has never been checked against the real code. |
| `timeout` | The engine ran out of time. **Not a failure of the code** — nobody learned anything. |
| `unsupported` | Ply cannot build inputs of this shape, so no check was attempted. |
| `engine-missing` | The tool that would run this check is not installed. |
| `tool_error` | The check did not run — usually the generated harness failed to compile. Zero cases ran, so this is never a pass and never a violation. |
| `inconclusive` | A check ran and settled nothing. |
| `weak-spec` | A `mutate` run planted bugs your checks did not catch. |
| `stale` | The code changed since the evidence was recorded. (Needs `ply.lock`; not produced yet.) |

The distinction worth internalising is between three answers that look similar and are
not: **checked and fine** (`tested`/`fuzzed`/`bounded`/`proved`), **checked nothing**
(`unclaimed`, `timeout`, `unsupported`, `tool_error`, `engine-missing`,
`inconclusive`), and **fine, assuming something nobody verified** (any verdict carrying
`conditional`). Most tools collapse the middle group into the first. Ply does not, and
that is the point of the whole exercise.

### Exit codes

For `cargo ply verify`:

- `0` — clean.
- `1` — a violation, or **any node with an absence of evidence**. A run that checked
  nothing is not a clean run.
- `2` — Ply itself broke.
- `3` — an engine you explicitly asked for is not installed.

`cargo ply check` exits 0 when it is clean or has only advisory findings, 1 on any
error-severity finding, and 2 if it could not run. `audit` and `worklist` exit 0 with
findings to report; only a document that will not load fails them.

`--fail-on` is a `verify` flag. It relaxes the default; it never tightens past it:

| `--fail-on` | The run fails when |
|---|---|
| `warn` | Any warning at all was emitted. |
| `evidence` *(default)* | Any node carries an absence of evidence, or any error was emitted. |
| `error` | Only an error was emitted — violations, unresolvable anchors, tool errors. |

`error` is the opt-out for a codebase mid-adoption where absences are expected and
tracked elsewhere. Choosing it is a statement that this run's green means less than the
default's.

`cargo ply worklist` always exits 0, whether or not it has items. An open item is work
somebody recorded, not a failure — a command that failed a build for containing a note
would make deleting the note the cheapest fix.

---

## 8. Architecture: edges, denials, capabilities, ownership

> **Read this first.** None of the rules in this section is enforced against your code
> in this build. `cargo ply check` validates that these lines are well-formed, that
> every name resolves, and that references are unambiguous; it does **not** compare them
> against what your code actually calls, touches, or mutates. The output says so on
> every run. Write them if you want the intent recorded and validated — do not write
> them believing a violation will be caught today.

### Edges

```yaml
ply: 1
components:
  feed:
    anchor: ingest::feed
  decoder:
    anchor: ingest::decode
  book:
    anchor: ingest::book
edges:
  - feed -> decoder
  - "decoder ~> book : Tick"
```

`a -> b` declares that `a` may call `b`. The intent is default-deny: once components are
declared, a call between two of them with no edge is a finding.

`a ~> b : Type` declares that data of that type flows from `a` to `b`. Flows are
**never** checked, by design — they are documentation of intent that the tooling can
draw, and nothing more.

Edges constrain *direct* calls only. `a -> b` and `b -> c` neither grant nor require
`a -> c`.

### Denials

```yaml
ply: 1
components:
  book:
    anchor: ingest::book
  decoder:
    anchor: ingest::decode
  strategy:
    anchor: strat
deny:
  - "* -> book except decoder, strategy"
```

The pattern on each side is a component name or `*`. `except` lists the components the
rule does not apply to.

### Two tiers, and `strict`

The intended enforcement has two tiers, and they differ in how much you can trust them:

- **Crate tier** — derived from cargo's own dependency graph, which is exact. Findings
  here are errors.
- **Item tier** — derived from parsing your source without type inference or macro
  expansion. It resolves calls, capability use, and mutation *approximately*: it can
  miss a call it cannot place, and it can misattribute one. Findings here are
  **warnings by default**.

`strict: true` on a component turns that component's item-tier findings into errors.
That is an opt-in precisely because the underlying data is approximate — turning
advisory findings into build failures is a promise you make about your own code, not one
the tool can make for you.

```yaml
ply: 1
components:
  book:
    anchor: ingest::book
    strict: true
```

The intended escape hatch for a false positive is a per-item attribute,
`#[ply::allow(no_panics, reason = "…")]`, which suppresses one finding on one item and
is recorded in `cargo ply audit`. **That attribute does not exist yet** — `ply-attrs`
does not define it, so writing one fails to compile.

### Capabilities

A coarse statement of what a component is allowed to touch. The permitted values are
`net`, `fs`, `db`, `time`, `rand`, `proc`, and `unsafe`.

```yaml
ply: 1
components:
  feed:
    anchor: ingest::feed
    uses: [net, time]
  decoder:
    anchor: ingest::decode
    pure: true
```

`pure: true` means no capabilities at all. The intended rules: a pure component that
touches any capability is a finding; a component that reaches a capability outside its
`uses` set through its own code — rather than by calling into a component that has it —
is a finding.

### Ownership

```yaml
ply: 1
components:
  book:
    anchor: ingest::book
    owns: [ingest::book::OrderBook]
```

`owns` names types that only this component may mutate. It is the "who is allowed to
change this" question, written down. Mutation from anywhere else is the intended
finding.

### Profiles

A named set of bans a component opts into:

```yaml
ply: 1
components:
  decoder:
    anchor: ingest::decode
    profile: hot_path
profiles:
  hot_path: [no_panics, exhaustive_match]
```

The available bans are `no_unsafe`, `no_trait_objects`, `no_interior_mut`, `no_panics`,
`no_async`, and `exhaustive_match`. These are purely syntactic checks over a
component's items — reliable, and intended to be errors regardless of `strict`.

---

## 9. Externals and entry points

An external is a system or a person outside this codebase: an exchange, a payment
processor, a human operator. Something you talk to and can never verify.

```yaml
ply: 1
externals:
  venue:
    note: "the exchange: accepts orders, returns fills"
components:
  gateway:
    anchor: gw
    fns:
      send:
        checks: [test]
        entry: [venue]
edges:
  - "gateway ~> venue : FixMessage"
```

`note:` is required. A bare name tells a reader nothing, and this construct exists
precisely so a reader can see where the system ends.

Externals are top-level only — they have no interior and cannot nest — and they share
the same name space as components, so a collision is an error. An external carries no
verdict and never appears on the evidence scale, not even as `unclaimed`: it will never
be claimed, and that is correct rather than pending.

Three rules follow from "Ply can never check this", and each is enforced today:

- An external may appear as an endpoint of a `~>` flow, or in a function's `entry:`
  list. Naming one in a `->` call edge or a `deny` pattern is an error, and the message
  points you at the two forms that do work (`E0207`). Ply cannot verify a call into code
  it cannot see, and cannot enforce a ban on a system it cannot observe.
- A `~>` flow needs at least one endpoint inside your workspace. `external ~> external`
  describes the outside world talking to itself, which is not this codebase's business
  to declare (`E0208`).
- An external nothing refers to is a warning: the document declares a boundary and then
  never says how it connects (`W0410`).

`entry: [venue]` on a function claim says that external can reach this function
directly. Each name must resolve to a declared external (`E0209`). The consequence is
about honesty rather than checking: nothing inside your workspace calls that function,
so no caller ever establishes its preconditions. Those `requires` clauses become
*environmental assumptions* — things you are trusting the outside world to satisfy.
`cargo ply audit` lists them permanently. They never change the function's verdict and
are never counted as owed work, because nobody can discharge them; counting them would
pressure you into deleting an honest declaration.

---

## 10. Trusted claims

Some load-bearing properties live outside any checker's reach: cross-thread safety
established by a specialised test, a paper proof, an external audit.

```yaml
ply: 1
components:
  ring:
    anchor: disruptor::spsc
    fns:
      try_push:
        checks: [bounded(3), fuzz(1024)]
        trusted:
          - claim: "SPSC cross-thread safety (happens-before between cursors)"
            evidence: "loom test tests/loom_spsc.rs"
```

Both fields are required. `claim` is what is being asserted; `evidence` is the named
artifact a reviewer can go and read — not "we checked", but *where*.

A trusted claim changes no verdict and runs no engine. It exists so the picture is
honest: without it, a function whose real correctness argument lives outside the tool
looks identical to one nothing supports. `cargo ply audit` lists every one with its
evidence.

Two things to know. Attestation is a human act — a claim about what a person has
verified should be added by that person, not on an agent's judgment. And a trusted claim
is *supposed* to go stale when the code it vouches for changes, so that `audit` asks for
re-attestation; **that comparison is not implemented**, so an attestation signed off
against a function that has since been rewritten looks exactly like one signed off this
morning. `audit` says so on every run.

---

## 11. Unresolved decisions

A marker for a decision nobody has made yet. Two places to write one.

In code, where the missing decision would go:

```rust
pub fn discount_bps(employee: bool) -> u32 {
    if employee {
        ply::unresolved!(147, "employee discount rate is undecided");
    }
    0
}
```

That expands to `unimplemented!("unresolved #147: employee discount rate is undecided")`
in *every* build, dev and production alike. Reaching it panics — deliberately. Simple,
honest, and greppable, and it cannot ship quietly.

In `ply.yaml`, for a decision with no code behind it yet — either against a function:

```yaml
ply: 1
components:
  billing:
    anchor: billing
    fns:
      fee_cents:
        checks: [test]
        unresolved:
          - { id: 12, note: "rounding on the half-cent: down, or to even?" }
```

…or in a top-level registry, for a decision that belongs to no particular function:

```yaml
ply: 1
components:
  billing:
    anchor: billing
unresolved:
  - { id: 151, note: "settlement rounding rule is not decided" }
```

Ids are positive integers, unique across the whole document — registry entries and
function entries share one number space, and a duplicate is an error (`E0205`). That is
what lets `cargo ply worklist` merge a marker in the code with its registry entry into
one item.

`worklist` lists every marker with its file, line and column, the function it sits in,
and what it blocks. One caveat it prints itself: the intended rule caps a function
containing a marker at the `test` check — since a contract cannot be complete while a
decision inside it is open — and **that cap is not enforced**. `verify` will still run
whatever the claim asks for, against a body that panics when it reaches the marker.

---

## 12. Names, paths, numbers: the lexical rules

- **Unknown keys are errors.** Every object in the schema refuses keys it does not know,
  with a suggestion for the nearest one it does (`E0204`). A typo has to be caught: an
  ignored key is a contract you think you wrote and Ply never read. This is the reason
  `ensure:` will not silently become nothing.
- **Component, external and profile names** are `[a-z][a-z0-9_]*` — lower-case ASCII,
  digits and underscores, starting with a letter.
- **Anchors and function keys** are plain segment paths: `ident` or `ident::ident::…`,
  where a segment may be a type name in `Type::method` position. No generics, no
  lifetimes, no `<T as Trait>::f` (`E0304`).
- **Numbers.** `fuzz(n)` takes `1 ≤ n ≤ 1000000`; `bounded(k)` takes `1 ≤ k ≤ 64`.
  Out of range is an error that says why (`E0203`).
- **Unresolved ids** are positive integers, unique across the document (`E0205`).
- **Component references** in `edges:` and `deny:` use component names, not Rust paths.
  A bare name resolves only if it is unique in the whole tree; otherwise write the
  dotted form `parent.child`. An ambiguous reference is an error listing the candidates
  (`E0206`).
- **Whitespace** in edge and deny strings: tokens are separated by one or more spaces.
  Any run of whitespace parses; single spaces are the canonical form.

---

## 13. When Ply says no

Every diagnostic leads with a plain sentence; the code follows it so scripts can match
on something stable. These are the ones this build emits.

**Reading the document**

| Code | What happened |
|---|---|
| `E0201` | The document does not match the schema, or `ply:` names a version this build does not speak. |
| `E0202` | Two components or externals share a name. |
| `E0203` | A check, edge or deny string is malformed, or a number is out of range. |
| `E0204` | A key Ply does not know — with the nearest key it does. |
| `E0205` | Two unresolved entries use the same id. |
| `E0206` | A bare component name is ambiguous; the message lists the candidates. |
| `E0207` | A `->` edge or a `deny` pattern names an external. |
| `E0208` | A `~>` flow has externals on both ends. |
| `E0209` | A function's `entry:` names something that is not a declared external. |
| `W0409` | An edge between a component and its own descendant — already implied. |
| `W0410` | An external is declared but nothing connects it. |

**Finding the code**

| Code | What happened |
|---|---|
| `E0301` | A claim points at a function Ply cannot find — or one it found and cannot verify from, because the function or a module above it is private. The message says which. |
| `E0304` | An anchor or function key is not a plain path. |
| `W0303` | This claim's component is anchored somewhere this run cannot check from — another crate, or a module of this one — so its checks did not run. The message says which, and what would run. |

**Running the checks**

| Code | What happened |
|---|---|
| `E0504` | `mutate` with no `test` or `fuzz` beside it — nothing to catch the planted bugs. |
| `E0501` | A contract expression could not be parsed. |
| `E0502` | A promise declared for a callee is satisfiable by no value at all. Assuming it would make any proof standing on it hold for nothing, so the proof is not run. |
| `E0503` | A promise declared for a callee is true of every value, so it constrains nothing and is not an assumption. |
| `K0502` | The model checker found an input that breaks the postcondition. This is a real violation, with a witness. |
| `V0505` | The signature is a shape Ply cannot build inputs for. Reported, not attempted. |
| `K0601` / `M0601` | The proof, or the mutation run, ran out of time. Not a failure of the code. |
| `X0901` | The generated harness never ran — usually a compile error, and the compiler's own message is quoted. Zero cases, so no verdict. |
| `W0110` | A check was declared whose engine does not exist in this build (`prove`). |
| `W0502` | A `mutate` run found bugs your checks did not catch. Note that an *equivalent* mutant — a change that cannot alter behaviour — survives any specification, so not every survivor is a gap. |
| `W0503` | The `requires` filter rejected so much of the generated input that the spread was narrow, or the run was abandoned entirely. |
| `W0510` | A contract written in `ply.yaml` for a checked function was used at the boundary but not merged into that function's own check. |
| `W0511` | The verdict is conditional: it used a declared contract instead of a callee's real body, and names what it assumed. |
| `W0512` | Ply refused to descend into a callee no contract describes, and names the callee and the call site. |
| `W0513` | Ply followed a path into first-party source and could not read it, so it refused rather than descending. |
| `W0514` | Ply could not tell whether a declared promise says anything, and reports it as unchecked rather than as fine. |
| `W0541` | A failing input was found but cannot be written out as runnable Rust, so the engine's own rendering is reported instead. Inputs are never invented. |

---

## 14. What this build does not do

Collected in one place, so nothing here has to be discovered at minute eleven.

**Not built at all**

- The architecture tier. `uses`, `pure`, `owns`, `profile`, `strict`, `edges:` and
  `deny:` are validated and then compared against nothing.
- `cargo ply tree`, `accept`, `doctor`, `synth`, `skill`; `verify --only-changed` and
  `--force`.
- The `prove` check — no engine.
- `ply.lock`, and everything that needs it: staleness of any kind, and skipping checks
  whose result is already recorded. Every run re-pays full engine cost.
- The attributes `#[ply::allow]`, `#[ply::pure]` and `#[ply::derived]`. They are read
  by `audit` if present in source, but `ply-attrs` does not define them, so writing one
  does not compile.
- Discovery and merging of multiple `ply.yaml` files. One file, one crate, per run.

**Built, with a limit worth knowing**

- Only top-level functions in `src/lib.rs` can be verified. Functions in modules and
  methods in `impl` blocks validate but cannot be checked, and a crate with no
  `src/lib.rs` at all (a binary-only crate) has nothing for Ply to resolve claims
  against. *(Actively changing.)*
- `check_with` is parsed and unused; generic functions are unsupported shapes.
- `ply.yaml` `requires`/`ensures` are not merged into the described function's own check
  (`W0510`). They work as boundary promises for callers.
- Component-level default `checks:` are honoured by `check` and ignored by `verify`.
- `checks: []` means "use the default", not "check nothing".
- `old()` works on the model-checking path for by-value parameters, fails to compile on
  the `test`/`fuzz` path, and does not work for the mutating case it exists for.
- The contract expression subset is documented but not validated; an expression outside
  it fails later, in the compiler or the engine.
- A boundary promise that cannot be satisfied makes the caller's proof pass vacuously,
  and nothing detects that.
- A claim under a component anchored at a *module* (`anchor: ingest::book`) cannot be
  run: function keys are read as paths from the crate root, not relative to the anchor.
  Such a claim is reported (`W0303`) with the crate-root spelling that would run.
- A boundary promise is matched by the callee's path as written, so a dependency renamed
  in `Cargo.toml` (`ledger = { package = "real-name", … }`) will not match. It fails
  loudly — you get the refusal for an unclaimed callee — rather than quietly.

**By design, and not planned**

- Data flows (`~>`) are never checked. They are declared intent, drawn but not verified.
- Concurrency, async functions in verified components, and the internals of crates
  outside your workspace.
- Specifying the inside of a function body. Ply verifies below a function's signature
  and contract; it never tries to express algorithms declaratively.

---

*This document describes Ply as of 2026-08-25. `schema/ply.schema.json` is the
normative definition of the format; The-Ply-Spec.md is the design specification behind
it. Every YAML example on this page was validated by running `cargo ply check` against
it.*
