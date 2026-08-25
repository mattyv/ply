# Phase 1b — the enforcement loop stops being an IOU

Run 2026-08-25, branch `claude/project-concept-eval-6soxfl`. Two slices and one fix:
`cargo ply audit` (`662adfa`), `cargo ply worklist` (`f46edd8`), and two honesty fixes to
the assumed-contract list — a double-counted assumption (`cb4027d`) and one that no verdict
could ever rest on (`abff37b`), both in §5 below. No engine work, no
`ply.lock`, no architecture tier.

The adversarial review's verdict on §5.5's boundary rule was that `conditional` is

> not yet a silent pass — visible three ways, unlaunderable by aggregation — but it is a
> *quiet* one, CI-green with an enforcement loop that is entirely IOU; it turns silent the
> day users skim W0511.

The loop was an IOU for one concrete reason: **§5.5 described `audit` and `worklist`
listing owed evidence in the present tense, and neither command existed**. `owed-evidence`
was a status the kernel could carry, `W0511` was a diagnostic one run printed and the next
run forgot, and nothing anywhere accumulated. Both commands exist now.

---

## 1. What `audit` lists

§6's one line — "trust surface: profile escapes, assumed contracts, derived fns" —
predates most of the surface it now has to cover. The command lists six tiers, in this
fixed order, so two runs on a growing codebase read as the same document with more lines
in it:

| tier | source | statuses |
|---|---|---|
| assumed contracts (§5.5) | the call graph — the same set `verify` stubs, from the same code | `owed-evidence` |
| environmental assumptions (§5.1 `entry:`) | the fn's own `requires`, from either place a contract can be written | none, ever |
| trusted claims (§5.4d) | `trusted: [{claim, evidence}]` | `staleness-unknown` |
| helpers called from contracts (§5.4a) | every free call in a contract expression | none |
| profile escapes (§5.3) | `#[ply::allow(name, reason = "…")]` in `src/**/*.rs` | none |
| derived bodies (§5.7) | `mode: synth`, and `#[ply::derived(spec_hash = "…")]` | none |

**An assumed contract names all four things a reader needs**: the callee, the promise
`ply.yaml` makes for it, the caller whose verdict rests on it, and what would discharge it
— which is different advice depending on whether that callee's own entry already declares
a check. Both wordings are exact-string tested, because "add `checks: [fuzz(256)]`" told to
somebody who added it last week is how a listing loses its reader.

**Three of the six tiers can never carry an owed state, and that is the design.** The
review was explicit that counting an environmental assumption as owed would pressure users
into deleting honest declarations; the same argument covers an escape and an attestation.
`audit` therefore exits 0 with a surface to report. Only a document that will not load
fails it (1); a missing one is a tool error (2).

### The distinction between the two commands

> `audit` lists what a codebase rests on **permanently**; `worklist` lists what somebody
> recorded and **means to finish**.

One item appears on both, and it is the one this phase exists for. §5.5's assumed contract
is *permanent trust surface* (the assumption is a real thing this codebase stands on) and
*owed evidence* (unlike an escape or an environmental assumption, it closes — cheaply, with
one line of `ply.yaml`). So `audit` carries the assumption and `worklist` carries the
evidence owed on it. Both read it from one function (`shared::assumed_contracts`), so the
two commands cannot disagree about what this codebase is assuming; §5.5 now says this in
place of its "M5 — neither command is built yet".

## 2. What `worklist` lists

Two tiers of §6's three:

- **unresolved markers** (§5.6) — `ply::unresolved!` in the code and the `ply.yaml`
  registry (top-level and per-fn), **merged by id**, so one decision written in both places
  is one item rather than two. Each carries its span, its enclosing function, and its
  blocking status: `§5.6 caps demo::discount at check test while this stands; it claims
  bounded(2)` for a claimed fn, `Nothing: no claim in ply.yaml names helper` for an
  unclaimed one, `Nothing: there is no code behind this entry to cap` for a registry-only
  entry.
- **owed evidence** (§5.5) — see above.

`worklist` exits 0 whether or not it has items. A command that failed a build for
containing a `TODO` would make deleting the `TODO` the cheapest fix, which is the same
failure mode the review named for environmental assumptions.

### One thing was added that the brief did not list

`ply::unresolved!` **did not exist**. `ply-attrs` shipped `requires` and `ensures` only, so
no code containing a §5.6 marker could compile, and the tier §5.6 describes had nothing it
could ever list on a codebase that builds. The macro is now there, expanding to
`unimplemented!("unresolved #<id>: <note>")` unconditionally, exactly as §5.6 states — 25
lines, one exact-string test on the expansion. `#[ply::allow]` and `#[ply::derived]` are
deliberately **not** added: they belong to the always-on architecture tier (M2) and to
`synth` (M6), both explicitly out of scope here. The scanner reads them anyway, so the
listing is ready when the macro lands, and until then both tiers honestly report zero.

---

## 3. Against vetting 004 — the case this phase exists for

004 as checked in declares **no** contract for the legacy callee (that is stage `s5`'s
edit, not the committed document), so `tier_fee_cents` is in §5.5's *third* branch — the
refusal — and there is nothing being trusted. `audit` says exactly that, and it is the
right answer:

```
$ cargo-ply audit vetting/004-legacy-extension/feature

  call graph  Every call site written in a `bounded` claim's own body, classified the same
              way `cargo ply verify` classifies it. 0 of them stand on a contract declared
              for a callee Ply does not read.

Nothing in this crate rests on trust that Ply can see: no assumed contract, no attestation,
no escape, no derived body, and no contract that calls a helper. That is a fact about what
is declared, not a verdict about the code — see what this command could not look at,
below.
```

Run `run.sh`'s `s5` transformation — the `requires`/`ensures` declared in `ply.yaml` for
the unclaimed legacy callee — and the assumed-contract case appears, across a real crate
boundary (`ledger` is a path dependency, and the promise is keyed by the path the caller
writes):

```
$ cargo-ply audit <s5 copy>/feature

  document    6 fn claims across 2 top-level components, read from
              <scratch>/feature/ply.yaml.
              Trusted claims, environmental assumptions and `mode: synth` claims come from
              here.
  call graph  Every call site written in a `bounded` claim's own body, classified the same
              way `cargo ply verify` classifies it. 1 of them stands on a contract declared
              for a callee Ply does not read.
  source      Every `.rs` file under `src/`, read for `#[ply::allow]` escapes (0),
              `#[ply::derived]` bodies (0), and the helper functions contracts call.

The trust surface — what this codebase's evidence rests on, and Ply does not check:

  assumed contracts (1)
    `ledger::fees::bps_for_tier` — assumed by `withdrawal::tier_fee_cents` (at line 44, column 15)  [owed-evidence]
      `tier_fee_cents`'s proof never reads `ledger::fees::bps_for_tier`'s code. Ply replaces
      the call with the promise `ply.yaml` declares for that function — ensures |result|
      *result <= 10_000 — and proves `tier_fee_cents` against the promise instead of the
      body, which is why `tier_fee_cents`'s verdict reads `conditional`. If the promise is
      wrong, the verdict is wrong with it. Nothing has run `ledger::fees::bps_for_tier`
      against that promise yet, and that is what `owed-evidence` means here. To settle it,
      add `checks: [fuzz(256)]` to its `ply.yaml` entry — fuzzing crosses a legacy
      boundary by simply calling the code, so it tests the promise against the real
      `ledger::fees::bps_for_tier`. (§5.5)
```

and the same fact, from the side that says it is work:

```
$ cargo-ply worklist <s5 copy>/feature

  markers        `ply::unresolved!` in every `.rs` file under `src/`, and the registry in
                 <scratch>/feature/ply.yaml,
                 merged by id: 0 in total.
  owed evidence  Every assumed boundary contract, read from the call graph the same way
                 `cargo ply audit` reads it: 1 waiting on evidence.

What is owed — recorded by somebody, and expected to close:

  owed evidence (1)
    `withdrawal::tier_fee_cents` (at line 44, column 15)
      `tier_fee_cents`'s proof stands on a promise `ply.yaml` makes for
      `ledger::fees::bps_for_tier` — ensures |result| *result <= 10_000 — and nothing
      has run the real `ledger::fees::bps_for_tier` against it. That is what `owed-evidence`
      means: trust that is never checked is green paint. Unlike the rest of the trust
      surface this one closes, and cheaply. To close it, add `checks: [fuzz(256)]` to its
      `ply.yaml` entry — fuzzing crosses a legacy boundary by simply calling the code, so
      it tests the promise against the real `ledger::fees::bps_for_tier`. (§5.5)
      blocks: `withdrawal::tier_fee_cents` keeps a `conditional` verdict until the promise
              made for `ledger::fees::bps_for_tier` is checked against the real body.
```

Both runs exit 0. `audit` takes **0.057s** and `worklist` **0.051s** on this scenario, with
no engine installed or started — the property §6 asserts when it calls these commands fast.

For comparison, `verify` on that same stage needs ~202s of Kani time to reach the `W0511`
these two lines report (measured in the post-004 tranche, quoted in §6) — and then forgets
it. That is the loop the review called IOU: the fact existed only inside a run that
scrolls away.

---

## 4. What each command cannot see

Both carry it the way `check` does — as `coverage.not_checked` in the envelope and a
"What this command did NOT look at" block in the human run, each entry saying what the
user is therefore *not* being told.

**`audit`** — five entries:

- **trusted-claim staleness**: §5.4d's stale marker needs the fingerprint in `ply.lock`
  (Phase 1c). Every attestation is listed undated, and "one signed off against a function
  that has since been rewritten looks exactly like one signed off this morning".
- **assumption discharge**: the same file. Every assumption is reported owed, "including one
  whose callee declares a check that has been passing for months".
- **helper evidence**: §5.4a says a helper without a passing check of its own makes
  dependent verdicts `conditional`. `audit` lists which helpers a contract trusts, never
  whether they earned it — that is a verdict, and this command produces none.
- **unreadable call sites**: §5.5's own gaps. Macro-generated calls, function pointers,
  trait methods, and calls written inside a callee rather than in the claim itself.
- **architecture bans**: M2. An escape is listed as the declaration it is; today it
  switches nothing off, because nothing reports the findings it would suppress.

**`worklist`** — three:

- **weak specs (W0502)**: a `mutate` run's finding, plus a record of past runs. Neither
  exists here.
- **stale claims (W0302)**: `ply.lock`, Phase 1c.
- **check cap (W0521)**: marked `NOT ENFORCED` rather than `NOT CHECKED`, because it is a
  promise made on every marker line above it. §5.6 caps a marked fn at check `test`; this
  build does not apply the cap, so `verify` still runs whatever the claim asks for against
  a body that panics at the marker. Each marker's own detail repeats it, so a reader who
  skips the block still gets it.

Both commands also carry `check`'s no-verdicts sentence: every node in their envelopes
reads `unclaimed`, and that is the command reporting no evidence of its own.

---

## 5. The red-first failures, verbatim

Every slice went red first. Honest accounting: for the wording tests the sentence went
into the test before the implementation and the implementation then matched it, so most
of them passed on their first real run. The failures worth recording are the structural
ones (cheap, but they are what "red first" means for a new module) and four that changed
the code or the test.

**The scanner, before it existed:**

```
error[E0432]: unresolved import `ply_core::surface`
  --> crates/ply-core/tests/surface.rs:11:5
   |
11 | use ply_core::surface;
   |     ^^^^^^^^^^^^^^^^^ no `surface` in the root
```

**Each command, before it existed** (`audit` shown; `worklist` was the same shape):

```
error[E0425]: cannot find type `AuditReport` in this scope
error[E0425]: cannot find type `TrustItem` in module `ply_core::diag`
error[E0425]: cannot find value `STALENESS_GAP` in this scope
```

**The macro, before it existed:**

```
error[E0425]: cannot find function `expand_unresolved` in this scope
```

**The one where the test lost the argument.** The environmental-assumption test demanded
the sentence §5.1 itself uses — "Nothing inside this codebase calls `quote`, so no caller
here ever checks it":

```
thread '…an_escape_and_an_environmental_assumption_are_listed_and_neither_is_owed' panicked
`quote` is declared as an entry point for `venue` — an outside party Ply never verifies …
```

`audit` does not walk the call graph looking for a fn's *callers*, so it cannot know that.
The sentence became "the code that satisfies it is on the other side of the boundary, where
no engine of Ply's can reach", which is what this command actually established, and the
test now carries a comment saying why the stronger claim was dropped.

**The one that caught what every structural test was blind to.** All forty `ply-cli` tests
were green, the JSON envelope was right, and the human run said:

```
  document    fn claims across top-level component, read from …
  call graph  … 1 call site stand on a contract declared for a callee Ply does not read.
```

A pluralisation helper had swallowed the counts. This is exactly CLAUDE.md's renderer
story in miniature — correctly-shaped output nobody had *read* — so it earned its own
exact-string test before the fix:

```
assertion `left == right` failed
  left: "fn claims across top-level component, read from /tmp/.tmpf0fQYl/ply.yaml. …"
 right: "2 fn claims across 1 top-level component, read from /tmp/.tmpf0fQYl/ply.yaml. …"
```

**The one that was a real defect, found the same way.** A contract clause can be written
twice — as a `#[ply::requires]` attribute and in `ply.yaml` — and `audit` deduplicated the
two by string equality. It should not have: the attribute comes back from the parser
token-spaced, so `bps_ok (bps)` and `bps_ok(bps)` are the same clause and did not compare
equal, and one environmental assumption was listed as two. A trust surface that
double-counts is a trust surface overstating itself. The comparison is now over parsed
tokens (`surface::same_expression`), which keeps a difference *inside* a string literal a
real difference, and the spelling that survives is the one the user typed in `ply.yaml`.
Nothing in the test suite had caught it; reading the output did.

**And one more defect of the same family, caught by asking what `verify` would actually
do.** A contract declared for a callee that returns nothing was listed as an assumed
contract carrying `owed-evidence`, with a sentence saying the caller's verdict therefore
reads `conditional`. All of that was false: Ply stands in for a callee by producing a
value for it, so a `-> ()` promise cannot be encoded at all — `verify` refuses it with
`W0512` and the caller earns no evidence. Nobody is trusting anything there, so there is
nothing to list and nothing owed. `shared::assumed_contracts` now skips it, which also
makes "classified exactly as `verify` classifies it" true rather than nearly true: the
set `audit` lists is now precisely the set `verify` stubs.

**Two more of the same kind**, both cosmetic, both found by reading rather than asserting:
a `worklist` line that led with a bare em dash where an id would have been
(`— \`demo::tiered_fee\``), and my *guess* at a fixture's column number in an exact-string
test:

```
  left: "`legacy_rate` — assumed by `demo::tiered_fee` (at line 4, column 51)  [owed-evidence]"
 right: "`legacy_rate` — assumed by `demo::tiered_fee` (at line 4, column 43)  [owed-evidence]"
```

The fixture won; the expectation was wrong.

---

## 6. What else moved

**`crates/ply-cli/src/shared.rs`** is new, and is the one copy of what every engine-free
command does to a `ply.yaml`: load-and-schema-validate, the §7 tree of declared shape, the
local-anchor test, the declared-contract map, the fn-claim walk, and the 92-column wrap.
`local_anchor_names` existed twice before a third command wanted it — `check`'s copy and
`verify`'s — which is Phase 1a's "two readers of one document" defect at a smaller scale.
`check` and `verify` now read it from there; their behaviour is unchanged, and their tests
are the evidence.

**`crates/ply-core/src/surface.rs`** is new: one walk over `src/**/*.rs` recovering
markers, escapes and derived bodies, plus `contract_helpers`, which pulls the helper calls
out of a contract expression. Two names that look like helper calls are deliberately not
helpers: `old(expr)` is §5.4a's own two-state primitive, and a capitalised path is a type
or enum-variant constructor — §5.5 draws that same line for call sites, for the same
reason.

**§8's envelope grew two optional fields**, `trust_surface` and `open_items` — additive,
which is what §8's stability rule permits. Absent is not the same as empty: an empty
`trust_surface` means "this crate rests on nothing Ply can see", while an absent one means
the command never got to look.

**Test counts.** Product unit tests 142 → 182: `audit` 17, `worklist` 12, the source
scanner 10 (`crates/ply-core/tests/surface.rs`), the macro's expansion 1, and nothing
removed. E2e 28 → 34 (`audit_command` 3, `worklist_command` 3), counted as `#[test]`
functions in `tests/e2e/tests/`. `tools` unchanged at 118. `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` are clean in both workspaces.

The two new e2e files are engine-free by construction and run in **0.2s** — they use the
`boundarycontract`, `unclaimedcallee` and `clamp` fixtures, which exist already, so the
commands are exercised against the same crates `verify`'s Kani-driven tests use.

---

## 7. NOT RUN / NOT DONE

- **`audit` and `worklist` walk nested components; `verify` does not.** `verify`'s own loop
  reads top-level components only, so a fn claim inside a nested component gets no verdict
  from it at all. The listing commands report those claims (a claim a user wrote is a claim
  they wrote), which means `audit` can name a caller `verify` never checks. That is a
  `verify` gap, recorded below, not something the listing commands should hide.
- **A trust surface is per-crate.** Both commands take a crate directory, like `check` and
  `verify`, and `assumed_contracts` resolves against that crate's own `src/lib.rs`.
  Multi-file `ply.yaml` discovery (§5) is still not implemented, so a workspace-wide trust
  surface is not available.
- **`--json` only; no `--format`, no filtering.** `worklist --only-changed` and
  `audit --kind=…` would both be reasonable and neither was asked for.
- **No golden files.** Both commands' output is pinned by exact-string unit tests on the
  sentences and by substring e2e tests on the human surface, in the style `check` set. A
  `tests/ui` golden of a whole run would pin the wrap width too, which is not a contract
  anybody should be held to.
- **The full suite** was run once at the end, as briefed: another agent is running
  Kani-heavy experiments on this machine, so development used the engine-free subset.

---

## 8. TODO.md deltas — for the owner to apply

I did not edit `TODO.md`. These are the deltas:

**Tick as landed:**

- `cargo ply audit` — six-tier trust surface, `--json`, exit 0/1/2, engine-free — done,
  `662adfa`.
- `cargo ply worklist` — markers (source + registry, merged by id) and owed evidence,
  `--json`, exit 0/1/2, engine-free — done, `f46edd8`.
- `ply::unresolved!` exists in `ply-attrs` and expands as §5.6 states — done, `f46edd8`.
- **KNOWN GAP (review G2), part 3 — "no accumulating surface"** — closed. Parts (1) no
  vacuity check and (2) no staleness for a declared boundary contract remain open; the
  entry should be re-scoped to those two rather than ticked whole.
- **KNOWN GAP — "a boundary assumption is reported as owed, and nothing exercises it"** —
  half closed. `audit` and `worklist` now list it; *fuzz-checking a declared contract
  against the real legacy body is still not built*, so the assumption still cannot be
  discharged by any command. The entry should be re-scoped to that.

**Add as new, open:**

- `audit`'s trusted-claim staleness and assumption-discharge tiers — blocked on `ply.lock`
  (Phase 1c). Both absences are declared in `coverage.not_checked`; that text comes out
  when the tiers land.
- `worklist`'s `W0502` weak-spec tier and `W0302` stale-claim tier — same shape, same file.
- **§5.6's check cap is not enforced.** A fn containing `ply::unresolved!` should be capped
  at check `test` with `W0521`; `verify` applies no cap. `worklist` says so on every marker
  line and in `coverage.not_checked`, which makes it visible, not fixed.
- `#[ply::allow(...)]` and `#[ply::derived(...)]` have no macro behind them, so code
  carrying one does not compile. The `audit` scanner reads both already; the macros belong
  to M2 and M6 respectively.
- **`verify` reads top-level components only.** Nested-component fn claims get no verdict.
  `check`, `audit` and `worklist` all walk them, so the three listing commands and the
  verifying one disagree about which claims exist.
- `audit` and `worklist` should accept a loose `*.ply.yaml` path, alongside the same open
  item already recorded for `check`.
- A helper called from a contract is listed as trusted, but §5.4a's *consequence* — a
  helper without a passing check makes dependent verdicts `conditional` — is not
  implemented in `verify`. `audit` names the trust; nothing enforces the downgrade.

**KNOWN GAP, left open on purpose:**

- `audit` finds assumed contracts by reading the call sites in a claimed function's own
  body, so it inherits every limit §5.5 states: macro-generated calls, function pointers,
  trait methods, and an assumption reached through a *contracted* callee one level down.
  The `unreadable call sites` entry in `coverage.not_checked` says this in the output, so
  the limit is visible rather than silent — but it is still there.
