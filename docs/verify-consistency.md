# When `check` and `verify` disagreed: four fixes

*2026-08-25. Written against the build in this repository, every output below copied
from a real run rather than reconstructed.*

Four defects, one shape: **the command that validates your configuration and the command
that actually checks your code disagreed about what you wrote**, or the terminal hid
something the machine-readable output carried. Each one let a person believe they were
covered when they were not. They were found while writing `docs/SCHEMA.md` by running
the tool rather than reading it, and that document recorded all four as current limits.
It no longer does.

| # | The disagreement | Now |
|---|---|---|
| 1 | `verify` walked only the top level of the component tree; a claim inside a nested component produced no verdict, no diagnostic and no mention. `check` walked the whole tree and called the same claim fine. | Both walk the whole tree, and name a claim the same way (`outer.inner::safe_increment`). |
| 2 | `checks: []` was read as *no* list, so the shape-aware default fired and the function was proved anyway. `check` and the diagram read the same line as claiming nothing. | An empty list is a list: nothing runs, the verdict is `unclaimed`, and `W0515` says so. |
| 3 | A `checks:` default written on a component was resolved by `check` and ignored by `verify`. | One shared resolution for `check`, `verify` and the renderer. |
| 4 | A result resting on an assumed promise printed as a bare pass; the qualifier and the debt existed only in `--json` and in the diagnostic prose. | The node line carries `[assumed, evidence owed]`, glossed in plain words beneath the tree. |

Each landed as its own commit, each with its failing test watched first.

---

## 1 — A claim inside a nested component was silently skipped

### The failure, first

`tests/fixtures/nestedclaim` is a crate with one contracted function and one claim, and
the claim sits one component down (`outer` → `outer.inner`). Against the binary that
still had the defect:

```
---- the_nested_claim_is_named_in_the_terminal_output stdout ----
thread 'the_nested_claim_is_named_in_the_terminal_output' panicked at
tests/e2e/tests/nestedclaim_fixture.rs:64:5:
the nested claim must appear in the printed tree: workspace — unclaimed
```

`workspace — unclaimed` was the *entire* output. The envelope was worse, because it
shows there was nothing to expand and nothing to read:

```json
{"command":"verify","diagnostics":[],"ply_version":"0.1.0",
 "root":{"id":"workspace","kind":"workspace","statuses":[],"verdict":"unclaimed"}}
```

Zero diagnostics. A root with no children. `cargo ply check` on the same document
reported the same claim as pointing at real code.

### What changed

`verify` now flattens the whole component tree before it plans anything, and assembles
its verdict tree the way the document is shaped: a claim's node sits under the component
that declares it, however deep, and nested components are nodes under their parents.
Node ids match `check`'s (`outer.inner::safe_increment`), so the two commands can no
longer name the same claim differently. The same fix removed a latent bug: the component
a claim belonged to used to be recovered by splitting its node id on `::`, which is
wrong for any key that contains `::` (`rates::legacy_rate`).

After, on the same fixture:

```
workspace — bounded(2)
  outer — bounded(2)
    outer.inner — bounded(2)
      safe_increment — bounded(2)
```

### One sentence had to be rewritten to stay true

Nesting is normally written with module anchors (`anchor: ingest::book`), and `verify`
resolves a function key as a path from the crate root — so those claims are reported and
not run. The message for that case said the component was "anchored at `ingest::book`,
which is not the crate this run is verifying", which is false when `ingest` *is* the
crate being verified, and sends a reader looking for a crate that does not exist. That
case now gets its own sentence, naming the spelling that would run:

> `OrderBook::apply` is claimed under a component anchored at `ingest::book`, which is a
> module inside this crate rather than the crate itself. `cargo ply verify` reads a
> function key as a path from the crate root, so it has no way to resolve a key written
> relative to a module: this entry's `checks:` were not run and no verdict is reported
> for it. Move the claim to a component anchored at `ingest` and spell the key from the
> crate root — `book::OrderBook::apply` — and it will run. (W0303, §5.2)

Both branches are pinned by exact-string tests (`crates/ply-cli/src/verify.rs`).

### Before and after, on the real vetting document

`vetting/003-trading-system.ply.yaml` is the four-level document the grammar was vetted
against: 14 function claims, 8 of them inside nested components. Copied into a scratch
crate as its `ply.yaml` and run with `cargo ply verify .`, complete output, before:

```
workspace — unclaimed
[W0303] oms::Oms::on_fill — …
[W0303] oms::Oms::submit — …
[W0303] pnl::Positions::apply_fill — …
[W0303] risk::check_order — …
[W0303] strategy::Strategy::on_update — …
```

Five claims mentioned. Nine passed over without a word — every claim under `ingest.feed`,
`ingest.ring`, `ingest.decoder`, `ingest.book` and `strategy.signals`, plus
`gateway::Gateway::send`. After (diagnostic text trimmed to its first line here; each is
the full sentence in the real run):

```
workspace — unclaimed
[W0303] ingest.book::OrderBook::apply — …
[W0303] ingest.book::OrderBook::last_px — …
[W0303] ingest.book::OrderBook::updates — …
[W0303] ingest.decoder::decode — …
[W0303] ingest.ring::Spsc::try_pop — …
[W0303] ingest.ring::Spsc::try_push — …
[W0303] oms::Oms::on_fill — …
[W0303] oms::Oms::submit — …
[W0303] pnl::Positions::apply_fill — …
[W0303] risk::check_order — …
[W0303] strategy::Strategy::on_update — …
[W0303] strategy.signals::momentum — …
```

Twelve. The exit code is 1 in both runs — the root reads `unclaimed`, and a run that
checked nothing has never been a clean run — but the difference is what a reader is told:
seven claims that had no representation at all in the output now have one each.

The two claims still unmentioned are `ingest.feed::Feed::pump` and
`gateway::Gateway::send`, and both write `checks: []` under a component anchored to
another crate. They ask for nothing, in a component this run could not check anyway, so
there is nothing to report about them — and in a component anchored at the crate being
verified, that same `checks: []` now earns a node and a sentence of its own (defect 2).

---

## 2 — `checks: []` meant "use the default"

### The failure, first

`tests/fixtures/emptychecks` holds two identical contracted functions. One writes
`checks: []`; the other writes no `checks:` line at all. Against the binary that still
had the defect, both were proved and the run exited clean:

```
workspace — bounded(2)
  emptychecks — bounded(2)
    declared_unchecked — bounded(2)
    left_to_the_default — bounded(2)
```

A document that said "do not check this function" got a confident model-checking proof
of it.

### The decision, and why

**`checks: []` means "check nothing".** An empty list is a list — the author writing down
that this function is deliberately not checked. Nothing runs, the verdict is `unclaimed`,
and `W0515` says so. Leaving the key out is the different statement, and is how you ask
for the shape-aware default.

The alternative was to reject an empty list in the schema as ambiguous. Two things argue
against it. `vetting/003-trading-system.ply.yaml` — the real vetting document, written in
the grammar before the tool existed — uses `checks: []` twice, and in both places it
plainly means *this is not checked here*: `Feed::pump`, and `Gateway::send`, whose
correctness argument is an external certification suite recorded as a `trusted` claim.
And `check`, `audit` and the renderer already read that line as claiming nothing —
`audit` says in as many words that such an entry "asks for no checks, so nothing checks
it". Making `verify` agree was one change; making four surfaces and a vetting document
agree with a rejection would have been another.

An empty list is also a list *for inheritance*: it overrides a component default exactly
as a full list does. The two spellings could not have different scoping rules without
recreating the same trap one level up.

### What changed

The model kept no record of the difference — an absent list and an empty one both
arrived as an empty vector — so the model now carries it (`Option<Vec<String>>` on both
a function claim and a component), and every reader of a checks list reads the
difference the same way. After:

```
workspace — unclaimed
  emptychecks — unclaimed
    declared_unchecked — unclaimed
    left_to_the_default — bounded(2)
[W0515] emptychecks::declared_unchecked — `declared_unchecked` has an empty `checks:`
list, so nothing was run against it and it earned no evidence: an empty list means
"check nothing", not "use the default". Deleting the `checks:` line entirely would run
`bounded(2)`, the check Ply picks from this function's shape. Write the checks you want
to run it; leave the list empty to record a function you have deliberately not checked,
and its verdict stays `unclaimed` — Ply's word for "nothing was checked here".
(W0515, §5.4c)
```

(The diagnostic is one line in the terminal; wrapped here.)

The function that wrote no list still earns `bounded(2)`; a test holds it there, because
the new rule must not swallow the default. The run now exits 1 rather than 0, which is
the rule this tool already applies to every other absence of evidence.

Two test documents spelled "no checks of its own" as `checks: []` — one for the
component-default inheritance rule in `tools/check`, one for the renderer's glyph row.
Both were rewritten to leave the key out, which is what they meant; both still test what
they were written to test. The three committed vetting diagrams are byte-identical,
confirmed by the test that pins them rather than by inspection.

---

## 3 — A component's default was resolved by one command and ignored by the other

### The failure, first

`tests/fixtures/componentdefault` declares `checks: [fuzz(64)]` on a component. Two of
its three functions write no list of their own; one is inside a nested component. Against
the binary that still had the defect, all three were model-checked instead:

```
assertion `left == right` failed: the component asked for `fuzz(64)` and that is what
must have run -- a proof here means the declared default was ignored
  left: "bounded(2)"
 right: "fuzzed(64)"
```

```
workspace — bounded(2)
  outer — bounded(2)
    takes_the_default — bounded(2)
    writes_its_own — bounded(2)
    outer.inner — bounded(2)
      nested_takes_the_default — bounded(2)
```

Not a weaker answer than the declared one — a *different* one, arrived at by ignoring the
document. `cargo ply check` resolved the same line correctly, including for the nested
component.

### What changed

`verify` now resolves each claim's governing list with the same shared functions `check`
and the renderer already used (`ply_core::model::{effective_checks,
component_default_checks}`), carried down the tree walk from defect 1. After:

```
workspace — fuzzed(64)
  outer — fuzzed(64)
    takes_the_default — fuzzed(64)
    writes_its_own — bounded(2)
    outer.inner — fuzzed(64)
      nested_takes_the_default — fuzzed(64)
```

A function's own list still wins entirely — there is no merge — and a nested component
that declares no default of its own inherits its parent's.

An inherited *empty* default gets its own wording, because telling a reader that
`quote` "has an empty `checks:` list" when the empty list is on the component above it
points at a line that is not there:

> `quote` writes no `checks:` of its own and the component `pricing` declares an empty
> list as the default for everything inside it, so nothing was run against it …

### One consequence worth knowing

The default applies to every entry in scope, including an entry listed only to give a
function a contract for its callers. If you do not want the default to reach such an
entry, write `checks: []` on it — which, since defect 2, means exactly that.
`docs/SCHEMA.md` §5 says so.

---

## 4 — An assumed promise was invisible on the line people read

### The failure, first

The rendering had both facts in hand and dropped them:

```
---- tests::a_result_resting_on_an_unchecked_promise_says_so_on_the_node_line stdout ----
the node line must carry both marks: workspace — bounded(2)
  f — bounded(2)
```

The node carried `conditional` and `owed-evidence` in its `statuses`, and printed as a
bare pass — indistinguishable from a result standing on code somebody actually checked.

### What changed

`§7.1` gives statuses their own visual channel on the diagram: corner markers beside the
fill, never a change to the fill. The terminal now has the same channel. Real output,
from `cargo ply verify` on `tests/fixtures/boundarycontract` (no flags):

```
workspace — bounded(2)  [assumed, evidence owed]
  boundarycontract — bounded(2)  [assumed, evidence owed]
    tiered_fee — bounded(2)  [assumed, evidence owed]

  [assumed]        this result rests on a promise Ply was handed and did not check — if the promise is wrong, the result is wrong with it
  [evidence owed]  nothing has run the real code against that promise yet; the lines below name it and say what would settle it

[W0511] boundarycontract::tiered_fee — `tiered_fee` earned bounded(2), but conditionally: …
```

Three properties, each deliberate:

- **Plain words, not the internal names.** `conditional` and `owed-evidence` mean nothing
  to a reader who has not read the specification; `[assumed]` and `[evidence owed]` carry
  their own gloss, per CLAUDE.md's newbie bar.
- **The marks travel upward**, exactly as the statuses do, so a qualified leaf is legible
  at the root without expanding anything.
- **The gloss prints once, and only when the tree carries a mark.** A run with nothing to
  qualify prints exactly what it printed before — pinned character for character by a
  test, so this cannot become noise on every run.

---

## Tests

Red first for each of the four, with the failure read before the fix — the messages above
are those failures.

Added: three fixtures (`nestedclaim`, `emptychecks`, `componentdefault`), three e2e test
files, one new e2e test on the existing `boundarycontract` fixture (the terminal
surface), and unit tests for the two `W0303` sentences, the inherited-empty `W0515`
sentence, the marked node line and the unmarked one.

Nothing was removed. Two test *documents* changed spelling (`checks: []` → the key left
out) for the reason given under defect 2, and both still assert what they always did.

Full runs with all four fixes in: **257 passed / 0 failed** in the product workspace
(`cargo test --workspace`, was 238 before this session's work and another session's
landed alongside it), **118 passed / 0 failed** in `tools` (`cargo test --release`, and
that number is unchanged — the shared model change reached the renderer and the
validator, and neither's behaviour moved). `cargo fmt --check` and `cargo clippy
--all-targets` are clean in both workspaces.

---

## TODO deltas

Done, this session:

- [x] **A claim inside a nested component is checked and reported** — `verify` walks the
      whole component tree, node ids agree with `check`, and `W0303`'s message no longer
      claims a module of this crate is another crate.
- [x] **`checks: []` means "check nothing"** — decided, implemented in the model so every
      reader agrees, documented in `docs/SCHEMA.md` §5 and The-Ply-Spec.md §5.4c, and
      reported with `W0515` rather than left as a node nobody expands.
- [x] **A component's default `checks:` is what runs** — `verify` reads the same shared
      resolution `check` and the renderer use.
- [x] **The terminal shows `[assumed]` and `[evidence owed]`** on the node line, glossed
      beneath the tree.

Open, found while doing the above and deliberately not fixed here:

- [ ] **`audit` and `worklist` do not resolve component-default `checks:`.** They read a
      function's own list and otherwise fall back to the shape-aware default, so for a
      function whose component default is `fuzz(64)` they still reason as though a proof
      would run — which can overstate the trust surface (listing an assumed contract that
      `verify` never assumes). Same class of defect as #3, one command over; the shared
      walk (`walk_fn_claims`) would need to carry the inherited default the way
      `verify`'s does.
- [ ] **A claim under a module anchor still cannot run.** Function keys are read as paths
      from the crate root rather than relative to the component's anchor, so the ordinary
      way of writing a nested component (`anchor: ingest::book`) yields a reported,
      unrun claim. `W0303` now names the crate-root spelling that would run; anchor-
      relative resolution is not built, and `docs/SCHEMA.md` §4 and §14 say so.
- [ ] **`docs/SCHEMA.md` §14 still lists two limits that appear to have been fixed
      earlier** — "only top-level functions in `src/lib.rs` can be verified" (the module
      anchor work landed in `b8a445c`) and "a boundary promise that cannot be satisfied
      makes the caller's proof pass vacuously, and nothing detects that" (`E0502` landed
      in `802b862`). Not touched here because they belong to that work, not this one.

---

## The three questions, answered

**Is a nested claim checked now?** Yes. Literally, on a fixture whose only claim sits one
component down:

```
before:  workspace — unclaimed

after:   workspace — bounded(2)
           outer — bounded(2)
             outer.inner — bounded(2)
               safe_increment — bounded(2)
```

And on the real trading-system vetting document, the count of claims the run says
anything at all about went from 5 of 14 to 12 of 14 — the remaining two ask for no checks
at all, in a component anchored to another crate.

**What does an empty checks list mean now, and why?** `checks: []` means *check nothing*:
nothing runs, the verdict is `unclaimed`, and a sentence (`W0515`) says so and names the
default you gave up. It reads that way to every person who has ever written it, it is
what `check`, `audit` and the diagram already made of it, and it is what the one real
document written in this grammar uses it for. Leaving the key out is how you ask for the
shape-aware default, and an empty list overrides a component default exactly as a full
list does.

**What does the terminal show for a result leaning on an unverified promise?**

```
    tiered_fee — bounded(2)  [assumed, evidence owed]
```

with, printed once beneath the tree:

```
  [assumed]        this result rests on a promise Ply was handed and did not check — if the promise is wrong, the result is wrong with it
  [evidence owed]  nothing has run the real code against that promise yet; the lines below name it and say what would settle it
```
