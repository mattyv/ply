# Ply's own architecture

This document describes how Ply is put together, and it is not a hand-drawn picture of
what someone remembers being true. The diagram below is rendered from
[`ply.yaml`](ply.yaml) — the same file `cargo ply check` reads when Ply is pointed at its
own repository — so the shape you are looking at is the shape the checker enforces. If
someone adds a dependency this page does not show, the build says so.

<p align="center">
  <img src="docs/ply-self.svg" alt="Six dashed boxes inside a frame labelled ply.yaml: e2e, attrs, cli, render, core and kernel. Arrows run from cli to render and core, and from render to core and kernel. e2e and attrs stand alone with no arrows." width="700">
</p>

Each box is a crate. The boxes are **dashed** because none of them declares a function
contract yet: Ply's own `ply.yaml` says only how the crates may depend on one another, and
a dashed box is Ply's way of showing code that carries no claims of its own. Those four
arrows are the only dependencies permitted between components. Anything else is a
violation.

## The four crates

| Component | Crate | What it is |
|---|---|---|
| `attrs` | `ply-attrs` | The `#[ply::requires]` / `#[ply::ensures]` attribute macros. A proc-macro crate: it is compiled for the machine doing the building, not the machine the program will run on. |
| `core` | `ply-core` | The model, the schema, the call graph, the engine adapters (Kani, proptest, cargo-mutants), the result records, and the verdict kernel. Everything Ply knows how to do, with no terminal attached. |
| `cli` | `ply-cli` | The `cargo ply` commands — `check`, `verify`, `audit`, `worklist` — and every sentence a user reads. |
| `e2e` | `ply-e2e` | The end-to-end suite. It builds the real binary and drives it the way a user would. |
| `render` | `ply-render` | The §7.1 renderer: a `ply.yaml` becomes the picture, with every drawn thing carrying a tooltip that says what it is. Behind `cargo ply render`. |
| `kernel` | `ply-kernel` | The pure verdict kernel: the evidence order and the worst-of aggregation, as a model whose four invariants are checked by exhaustive enumeration over every verdict tree up to a small bound. **Not the verdict logic the binary ships** — nothing under `cli` links it yet; it is the proved model the product is converging on, and describing it otherwise would overclaim. |

## The rules, and why each one exists

**`attrs` stands alone.** It is a proc-macro crate, compiled for the host rather than the
target. A dependency from it into the library would compile perfectly well and would be
wrong: the macro would begin depending on the very thing it exists to annotate.

**`core` never reaches up into `cli`.** Everything `core` does has to be usable without a
terminal. That constraint is what makes the engine adapters testable at all — the moment a
diagnostic's wording lives in `core`, the wording and the engine can only be tested
together.

**`cli` is the top of the stack.** It is the only component allowed to depend on `core`,
and nothing may depend on `cli`. This is what makes the direction of the whole graph
unambiguous.

**`e2e` drives the built binary, not the library.** A suite that links the library instead
stops testing what ships.

**`render` may reach into `core` and `kernel`, and only `cli` may reach into `render`.** It
reads the same document model the checker reads and asks the kernel to aggregate, so the
shape it draws is the one the kernel proves rather than a second implementation that
could disagree. Both crates moved in from a separate tools workspace on 2026-08-28, so
that `cargo ply render` could exist: the development loop's own step 2 says "Ply renders
that intent", and until then that meant building a second binary and invoking it by path.
The kernel came with the renderer because the renderer calls it — leaving it behind would
have pointed the product at the tooling, the one direction this file exists to prevent.

## What happened when Ply was pointed at itself

That last rule is the interesting one, because Ply caught this codebase breaking it.

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
returns. The rule is intact, the edge is gone, and the diagram above shows one arrow
rather than two.

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
  architecture  4 real crate dependencies cross between two differently-declared components:
                4 permitted by a declared edge or by nesting, 0 not permitted (reported
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

## Regenerating the diagram

The SVG is committed, and a test fails if it stops matching the spec it was rendered from:

```console
$ cargo ply render ply.yaml -o docs/ply-self.svg
```

## Where the rest is written down

- [The-Ply-Spec.md](The-Ply-Spec.md) — the source of truth for the grammar, the evidence
  ladder, and every decision behind them.
- [README.md](README.md) — what Ply is for and what it can do today.
- [TODO.md](TODO.md) — what is agreed, what has landed, and which gaps are open on purpose.
