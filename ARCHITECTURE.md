# Ply's own architecture

This document describes how Ply is put together, and it is not a hand-drawn picture of
what someone remembers being true. Both diagrams below are rendered from the same files
`cargo ply check` reads when Ply is pointed at its own repository, so the shapes you are
looking at are the shapes the checker enforces. If someone adds a dependency this page
does not show, the build says so.

There are two of them because they answer two different questions. The first says **what
the parts are and who may depend on whom**. The second says **what the library actually
promises about itself**, which is a claim of a different kind and lives in a different
file.

## What the parts are

Rendered from [`ply.yaml`](ply.yaml) at the repository root.

<p align="center">
  <img src="docs/ply-self.svg" alt="A frame labelled ply.yaml, with a line reading 13 components, 0 functions, 0 promise nothing. Inside are six top-level boxes: e2e, attrs, core, cli, render and check. Every box is filled with diagonal hatching, meaning nothing inside promises anything yet, and every box that holds nothing else inside it is also dashed. Most boxes carry a line saying what that part holds, followed by a row per field with a small shape beside it and its type to the right: core holds an Envelope, kernel a VerdictNode, engines a TimedOutput, harness a ContractFn, visual a VisualEnvelope, record a RecordEntry, cli holds a Cli (its parsed command-line arguments), render likewise holds a Cli of its own, and inside cli, verify holds a VerificationResult and report an AuditReport. core contains five smaller boxes, kernel, engines, harness, visual and record; cli contains two, verify and report. e2e, attrs and check carry no such line. Three arrows run into core: one from cli, one from render, one from check. e2e and attrs stand alone with no arrows." width="760">
</p>

The six outer boxes are the crates. The smaller boxes inside `core` and `cli` are modules
within those crates: they carry no rules of their own yet, and they are there so that a
reader can go one level in rather than being told that `core` does everything.

Every box is filled with **diagonal hatching**, which is how Ply draws code that carries
no claims about its own behaviour. That is honest and it is the point of the second
diagram further down: this file says only how the crates may depend on one another, and
saying nothing about behaviour has to look like saying nothing. Those three arrows are the
only dependencies permitted between components. Anything else is a violation.

## The six components

| Component | Crate | What it is |
|---|---|---|
| `attrs` | `ply-attrs` | The `#[ply::requires]` / `#[ply::ensures]` attribute macros. A proc-macro crate: it is compiled for the machine doing the building, not the machine the program will run on. |
| `core` | `ply-core` | The model, the schema, the call graph, the engine adapters (Kani, proptest, cargo-mutants), the result records, and the verdict kernel. Everything Ply knows how to do, with no terminal attached. Its five modules are drawn inside it: `kernel` decides what a pile of results adds up to, `engines` drives each outside checking tool, `harness` works out whether a function's inputs can be built at all, `visual` produces the drawing and the prose, and `record` stores what a result depended on. |
| `cli` | `ply-cli` | The `cargo ply` commands — `check`, `verify`, `audit`, `worklist` — and every sentence a user reads. `verify` is the one command that produces evidence; `report` says what this codebase's evidence rests on that Ply never checks. |
| `e2e` | `ply-e2e` | The end-to-end suite. It builds the real binary and drives it the way a user would. |
| `render` | `ply-render` | Draws a document before any code exists, and writes the same facts as prose. |
| `check` | `ply-check` | The standalone spec validator, which predates the installed `cargo ply check` command and overlaps it. Retiring one of the two is recorded work, not a decision taken here. |

## The rules, and why each one exists

**`attrs` stands alone.** It is a proc-macro crate, compiled for the host rather than the
target. A dependency from it into the library would compile perfectly well and would be
wrong: the macro would begin depending on the very thing it exists to annotate.

**`core` never reaches up into `cli`.** Everything `core` does has to be usable without a
terminal. That constraint is what makes the engine adapters testable at all — the moment a
diagnostic's wording lives in `core`, the wording and the engine can only be tested
together.

**Nothing may depend on `cli`.** It is a top of the stack, and so are `render` and
`check`: all three are allowed to depend on `core` and nothing is allowed to depend on
them. That is what makes the direction of the whole graph unambiguous — everything points
at the library, and the library points at nothing.

**`e2e` drives the built binary, not the library.** A suite that links the library instead
stops testing what ships.

## What the library promises about itself

Rendered from [`crates/ply-core/ply.yaml`](crates/ply-core/ply.yaml).

<p align="center">
  <img src="docs/ply-core-self.svg" alt="A frame labelled ply.yaml, with a line reading 5 components, 6 functions, 0 promise nothing. Inside is a solid box named core, filled mid-grey rather than hatched. Under its name a line reads state Envelope, 4 of 8 shown, followed by four rows: command, root, diagnostics and coverage, each with a small shape beside it and its type to the right. Nested inside core are four smaller boxes, one per module — kernel, harness, visual and record — each with its own state line and field rows, and each holding the chips for its own functions: StatusSet::len and StatusSet::union under kernel, last_two_segments and rust_type_from_source under harness, stable_element_id under visual, fingerprint under record. Every chip reads fuzz: 256 cases. The kernel box has no field rows; its state line reads state StatusSet, promises 1 thing about itself. There are no arrows." width="440">
</p>

This is the first drawing of Ply's own code that is not hatched. Each chip is a function
that states something that must be true of what it returns, and how that statement is to
be tested — 256 generated inputs each. A box is filled mid-grey because grey depth is how
strongly a thing promises to be checked, and a box is never shown as stronger than the
weakest function inside it.

The kernel's box says its structure promises something about itself: a set of statuses can
never hold more than the seven kinds of status that exist. That is not a comment — Ply
builds a set through its own constructor, inserts and merges through its own operations,
and checks the claim again after every one of them, 256 histories deep.

Each module is its own box, holding the structure it keeps and the functions that promise
things about it. That became possible on 2026-09-04: a function key is now read relative
to the box it is written in, so `StatusSet::len` written inside the box for `kernel` names
the function in that module and is checked there. Until then a key was read from the crate
root whatever the box said, so a function claimed inside a module box was drawn and never
checked — which is why this file used to be one crate-wide box with the module carried in
each function's name.

**The four rows under the name are what this crate holds.** The shape beside each one
says what kind of thing it is — a filled block for a single value, stacked bars for a
list, key-beside-value for a lookup table, a dashed outline for something that might not
be there — and the text beside it is the type as the code spells it. None of that is
written in the document: the document names the type and four of its fields, and Ply
reads the rest out of the source. Name a field the code does not have and the build
fails, listing the ones it does have; name a type that lives in a different module and it
fails too, because a component's state is resolved under its own anchor. Three of these
four are drawn with fine diagonal hatching, which means Ply has no way to make one of
them; that is usually the reason functions taking such a value come back unchecked.

Four boxes on the workspace diagram carry no such line, and that is not an omission: the
attribute macros and the end-to-end tests hold no state worth naming, the renderer's
types are the library's, and the checker is a binary with none of its own.

**Grey is not green.** Nothing on either page is green, and that is deliberate: green is
kept for evidence a run has actually earned. These drawings are made from the documents
alone, without running anything, so a picture full of promises must not look like a
picture full of results. `cargo ply verify` is what turns one into the other.

Two of these promises are load-bearing rather than decorative. `StatusSet` is the type the
verdict machinery carries flags in, and the two claims about it — that it never holds more
than seven, and that combining two sets never loses one — are the properties the rest of
the machinery quietly assumes. The other four are the smaller kind: a function that splits
a path, one that reads a type out of source text, one that mints an identifier for a
drawn element, and one that hashes what a result depended on.

Why they live in a second file rather than the one above: Ply resolves a function claim
against a single crate's `src/lib.rs`, and a workspace root has none. Pointed at the root,
it says so rather than reporting no problems — "this is not a count of zero problems, it
is a count of zero searches."

One caveat that belongs here rather than in a footnote: a promise written in this file is
**not yet folded into the function's own checks**. It is drawn, it is counted, and the
plumbing that would make a failing promise fail the run is a recorded gap in
[TODO.md](TODO.md), not a claim being made on this page.

## What happened when Ply was pointed at itself

The `e2e` rule above is the interesting one, because Ply caught this codebase breaking it.

`e2e` has no outgoing edge. Running `cargo ply check` on this repository while writing this
document reported that `ply_e2e` had grown a dependency on `ply_core` anyway, and that
nothing in the spec allowed it:

```
A0401 crate `ply_e2e` depends on crate `ply_core`. `ply_e2e` belongs to the `e2e`
  component and `ply_core` belongs to `core`, and no `->` edge in this document says
  `e2e` may depend on `core` — so this dependency crosses a boundary nothing allows.
  Add "e2e -> core" under `edges:` if this is intended, or remove the dependency.
```

Nobody had decided to break the rule. One measurement test — the count of which Rust types
Ply can actually handle — read `core`'s type classifier directly, so that the published
number could never drift into a second, hand-maintained idea of the same fact. A good
reason for the dependency to exist, and no reason at all for it to live in the end-to-end
suite.

The diagnostic offers two ways out, and it matters which one you take. Adding
`e2e -> core` would have made the run green by widening the rule to the entire suite in
order to excuse one file — and Ply's architecture checking works crate by crate today, so
there is no narrower edge to write. Every other test in `e2e` would have kept driving the
binary on convention alone, with the checker no longer able to say otherwise. That is a
rule quietly abandoned, wearing the green tick of a rule enforced.

So the test moved instead. It now lives in `ply-core`'s own tests, next to the classifier
it measures, where the dependency is native and needs no exception. Nothing about that
measurement ever wanted to be end-to-end: it reads one function and counts what it
returns. The rule is intact, the edge is gone, and no arrow leaves `e2e` in the diagram
above.

The general lesson is the one Ply is built around. A checker earns its keep at the moment
it disagrees with you — and the useful response is usually to fix the thing it found, not
to widen the rule until the complaint goes away.

## Checking it yourself

From the repository root:

```console
$ cargo ply check .
cargo ply check — ./ply.yaml

  schema        The document against schema/ply.schema.json, then every rule that can be
                settled from the document alone.
  anchors       This document declares no fn claims, so there was nothing for this tier to
                resolve.
  architecture  3 real crate dependencies cross between two differently-declared components:
                3 permitted by a declared edge or by nesting, 0 not permitted (reported
                below). 6 of 6 crates in this workspace belong to a declared component.

  No problems found in the document.

What this command did NOT check:
  item-level    NOT CHECKED. Ply now checks whether one crate depends on another crate
                across a boundary no `edges:` line allows. It does not yet look inside your
                functions: a call from one function to another, use of a capability like the
                filesystem or the network, or a change to a type another component owns can
                still cross that same boundary with nothing here noticing.
```

Two things in that output are deliberate. The dependency count is reported whether or not
anything was wrong, so a clean run is a count you can check rather than a silence you have
to trust. And the run states what it did **not** look at, so "no problems found" cannot be
mistaken for "this architecture is fully enforced" — the checking that happens inside
functions does not exist yet, and the tool says so itself rather than letting you assume
otherwise.

Point it at the library document and it discloses the gap named above in its own words,
rather than leaving it to this page:

```console
$ cargo ply check crates/ply-core
cargo ply check — crates/ply-core/ply.yaml

  anchors       6 of 6 fn claims in this crate point at a function Ply can find. 6 of them
                also write a `requires:`/`ensures:` contract directly in ply.yaml. A
                ply.yaml contract is only used one way -- a caller of those functions may
                assume it, but it is not added to their own checks, so `verify` does not
                check them against it. Move the contract onto those functions as
                `#[ply::requires]`/`#[ply::ensures]` attributes if you want them checked.
```

## Regenerating the diagrams

Both SVGs are committed, and so is the prose form of each — the same facts written out in
full, including every sentence the drawing only shows on hover. A test fails if any of the
four stops matching the document it was rendered from, which is what stops this page
quietly describing a shape the repository no longer has.

From `tools/`:

```console
$ cargo run --release -p ply-render -- ../ply.yaml                   -o ../docs/ply-self.svg
$ cargo run --release -p ply-render -- ../ply.yaml            --text -o ../docs/ply-self.txt
$ cargo run --release -p ply-render -- ../crates/ply-core/ply.yaml   -o ../docs/ply-core-self.svg
$ cargo run --release -p ply-render -- ../crates/ply-core/ply.yaml --text -o ../docs/ply-core-self.txt
```

The text forms are the ones to read when you want the detail: [`docs/ply-self.txt`](docs/ply-self.txt)
and [`docs/ply-core-self.txt`](docs/ply-core-self.txt).

## Where the rest is written down

- [The-Ply-Spec.md](The-Ply-Spec.md) — the source of truth for the grammar, the evidence
  ladder, and every decision behind them.
- [README.md](README.md) — what Ply is for and what it can do today.
- [TODO.md](TODO.md) — what is agreed, what has landed, and which gaps are open on purpose.
