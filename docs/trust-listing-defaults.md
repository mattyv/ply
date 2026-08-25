# The trust listing was reading the wrong line

*2026-08-25. Written against the build in this repository; every output below is copied
from a real run rather than reconstructed.*

`docs/verify-consistency.md` fixed a defect in three places and recorded it surviving in
a fourth:

> **`audit` and `worklist` do not resolve component-default `checks:`.** They read a
> function's own list and otherwise fall back to the shape-aware default […] Same class
> of defect as #3, one command over.

A `checks:` list written on a component is the default for every function inside it,
nested components included (§5.1). `check`, `verify` and the renderer resolve it from one
shared place. The two listing commands did not: they read each function's own `checks:`
and, finding none, invented one. So a function that takes its checks from its component
looked to them like a function that declares nothing.

That is worse in a trust listing than anywhere else. `audit` exists to say what a result
rests on that nobody checked, and `worklist` to say what is owed on it. Both were saying
it about a document neither had read correctly, and the error ran in both directions:
naming an assumption `verify` never makes, and calling a function unchecked when the
document checks it.

---

## The failure, first

Four tests, written before the fix and watched fail. The messages, verbatim:

**A function that inherits `fuzz(64)` had an assumption invented for it.** Fuzzing
crosses a legacy boundary by calling the real code (§5.5), so nothing is assumed — but
reading the function's own (absent) list put the shape-aware `bounded(2)` back, and a
`bounded` proof does stand on the promise:

```
---- audit::tests::a_component_default_that_asks_for_fuzzing_leaves_nothing_assumed ----
`tiered_fee` inherits `fuzz(64)` from its component, and fuzzing runs the real
`legacy_rate` -- nothing is assumed here, so nothing may be listed as assumed: Some(
    [
        TrustItem {
            kind: "assumed_contract",
            subject: "legacy_rate",
            node_id: "demo::tiered_fee",
            statuses: [ "owed-evidence" ],
            …
```

`worklist` invented the matching debt
(`a_component_default_that_asks_for_fuzzing_owes_no_evidence`), listing
`demo::tiered_fee` as owed evidence with the same reasoning.

**A helper the document does check was reported as one nothing checks.** The helper sits
in a nested component and takes `test` from the component above it:

```
---- audit::tests::a_helper_that_inherits_its_checks_is_not_reported_as_unchecked ----
a helper that inherits its checks is checked, and saying otherwise overstates what this
contract rests on: `fee`'s contract calls `bps_ok(…)`. […] `bps_ok` has a `ply.yaml`
entry that asks for no checks, so nothing checks it. (§5.4a)
```

**The line under an unresolved marker named no check.** It says what the marker holds up,
so it has to name the check that would run:

```
---- worklist::tests::a_marker_in_a_fn_that_inherits_its_checks_names_the_check… ----
assertion `left == right` failed
  left: "§5.6 caps `demo.pricing::discount` at check `test` while this stands; it
         declares no checks of its own."
 right: "§5.6 caps `demo.pricing::discount` at check `test` while this stands; it claims
         `bounded(2)`, the default the component `demo` sets for everything inside it."
```

---

## What changed

No second resolver. The shared walk both commands already use to find every claim
(`shared::walk_fn_claims`) now carries the inherited default down the tree with it, from
the same functions `check`, `verify` and the renderer use
(`ply_core::model::{effective_checks, component_default_checks}`), and every reader of a
checks list in `audit`, `worklist` and their shared helpers goes through one accessor
(`FnClaimRef::governing_checks`). A function's own list still wins entirely — there is no
merge — an empty list still means "check nothing", and a nested component that declares
no default of its own inherits its parent's.

Five sentences changed, because a sentence that points at a `checks:` line the reader
will not find on the function sends them hunting for it. Each names the component the
list was written on:

- a helper that inherits a list: *"`bps_ok` is checked with `test`, which the component
  `pricing` declares as the default for everything inside it; whether that check passes
  is not something this command knows."*
- a helper under an inherited **empty** list: *"`bps_ok` writes no checks of its own, and
  the component `demo` declares an empty list as the default for everything inside it, so
  nothing checks it."*
- the line under a marker: *"…; it claims `bounded(2)`, the default the component `demo`
  sets for everything inside it."*, and *"…; the component `demo` declares an empty list
  as the default for everything inside it, so nothing runs against it."*
- the advice for settling a promise, where the callee's check is a component default:
  *"Its `ply.yaml` entry already asks for `fuzz(256)` — the default the component `demo`
  sets for everything inside it, which runs the real body against the promise: `cargo ply
  verify` is what settles it."*

Every one of those is pinned by an exact-string test. The wording for a function that
writes its own list is unchanged, character for character.

One more rule arrived with the shared resolution, because it is part of the same answer:
**an empty list is a list.** A function that writes `checks: []` is checked by nothing,
so it proves nothing, so it assumes nothing. These two commands used to test that list
for emptiness rather than for presence — the defect `docs/verify-consistency.md` fixed in
`verify` — and put the shape-aware `bounded(2)` back, so a function that asked for no
checks was listed as resting on a promise. On the `boundarycontract` fixture with
`tiered_fee`'s list emptied, that is one invented assumed contract before and none after.

---

## Before and after, on a real document

`tests/fixtures/boundarycontract` is a real crate with a real trust surface: `tiered_fee`
has a contract, calls `legacy_rate`, and `ply.yaml` declares a promise for `legacy_rate`
that nothing has tested. Written the way §5.1 invites — the check every function in the
component runs, written once on the component, and fuzzing chosen because it crosses a
legacy boundary by running the real code:

```yaml
components:
  boundarycontract:
    anchor: ply_fixture_boundarycontract
    checks: [fuzz(256)]
    fns:
      legacy_rate:
        checks: []
        ensures:
          - "|result| *result <= 10_000"
      tiered_fee: {}
```

`cargo ply audit` on that crate, before:

```
  call graph  … 1 of them stands on a contract declared for a callee Ply does not read.

The trust surface — what this codebase's evidence rests on, and Ply does not check:

  assumed contracts (1)
    `legacy_rate` — assumed by `boundarycontract::tiered_fee` (at line 26, column 15)  [owed-evidence]
      `tiered_fee`'s proof never reads `legacy_rate`'s code. Ply replaces the call with the
      promise `ply.yaml` declares for that function — ensures |result| *result <= 10_000
      — and proves `tiered_fee` against the promise instead of the body …
```

After:

```
  call graph  … 0 of them stand on a contract declared for a callee Ply does not read.

Nothing in this crate rests on trust that Ply can see: no assumed contract, no attestation,
no escape, no derived body, and no contract that calls a helper. That is a fact about what
is declared, not a verdict about the code — see what this command could not look at,
below.
```

`cargo ply worklist` on the same crate loses the matching owed-evidence item and prints
"Nothing is owed that Ply can see".

**That is the intended change, and it is what `verify` does.** Only a `bounded` check
stubs a callee — `verify` builds a boundary plan for one and skips it for every other
tier, because proptest simply runs the code (`verify.rs`, §5.5's three-way split). With
`fuzz(256)` inherited from the component, the real `legacy_rate` runs, nothing is
assumed, and there was nothing for either command to list. The old output was work
invented by the tool.

The other direction, on a two-level document where the helper a contract calls sits in a
nested component and takes `test` from the component above:

```
- `bps_ok` has a `ply.yaml` entry that asks for no checks, so nothing checks it. (§5.4a)
+ `bps_ok` is checked with `test`, which the component `pricing` declares as the default
+ for everything inside it; whether that check passes is not something this command knows.
```

### The trading-system document, and every fixture: unchanged

`vetting/003-trading-system.ply.yaml` writes a `checks:` list on every one of its 14
function claims and declares no component default anywhere, so there is no inheritance in
it to resolve. `cargo ply audit` and `cargo ply worklist` on it are byte-identical before
and after — confirmed by diffing the two binaries' output, not by inspection. The same
sweep over all 25 fixtures in `tests/fixtures/` (50 runs, two commands each) found no
output change either. `componentdefault` is the only fixture with a component default, and it has no markers,
no contract helpers and no boundary promises, so it has nothing for these two commands to
report.

---

## Tests

Red first for each of the four, with the failure read before the fix; the messages above
are those failures. Eight tests added in all — the four above, plus exact-string tests for
the advice that names where a callee's check was written, for an inherited empty list, for
the two spellings of "check nothing", and for a caller that asks for no checks at all.

Suites: **263 passed / 0 failed** in the product workspace (`cargo test --workspace`, 257
before this change and another session's work landed alongside it), **118 passed / 0
failed** in `tools` (`cd tools && cargo test --release`, unchanged — the renderer already
resolved inheritance). `cargo fmt --check` and `cargo clippy --all-targets` clean in both
workspaces.

## Documents corrected

`docs/SCHEMA.md` §5 still told a reader that `check` honours component-default
inheritance and `verify` does not, and to "write the checks you mean on the function"
until that is reconciled. That stopped being true when `73242ba` landed. It now says what
is true: every command resolves it in one shared place, nested components included.
The-Ply-Spec.md §5.4c's paragraph on the shared resolution names `audit` and `worklist`
alongside the other three.

## TODO deltas

Done:

- [x] **`audit` and `worklist` resolve a component's default `checks:`** — through the
      same shared resolution the other three readers use, nesting included, with a
      function's own list still overriding entirely.
- [x] **`docs/SCHEMA.md` §5 no longer describes the inheritance split as a live limit.**

Still open, and untouched here:

- [ ] A claim under a module anchor still cannot run (`W0303`).
- [ ] `docs/SCHEMA.md` §14's two stale limits — "only top-level functions in `src/lib.rs`
      can be verified" and the vacuous-boundary-promise line — belong to the work that
      fixed them.
