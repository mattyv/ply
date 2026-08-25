# Post-004 fixes — 2026-08-25

Closes the five items `docs/review-post-004-strategy.md` sequenced after vetting 004
(`vetting/004-legacy-extension.md`). Method, per CLAUDE.md: for every item the test that
fails *because of that defect* was written and watched fail first, and its message was
read to check it named the real defect. Spec amended wherever behaviour changed. Anything
not done is marked deferred or NOT RUN with its reason.

**Headline**: the boundary is now a decision Ply makes rather than an engine burn it
survives. A `bounded` check whose function calls code no contract describes refuses to
descend and names the callee, in milliseconds. Given a contract declared for that callee
in `ply.yaml`, the same function earns `bounded(2)` with status `conditional` and the
assumption listed as owed evidence. And a run that checked nothing now exits non-zero.

---


## 1 — D5's third branch: the boundary rule (spec first, then built)

**Spec.** §5.5 rewritten. D5 now splits three ways on *what the callee offers*: a callee
that passed its own Kani proof this run is stubbed with `stub_verified`; a callee with a
declared contract but weaker evidence — **including one carrying no verification at all
but a contract declared in `ply.yaml`** — is assumed, stubbed, and the caller marked
`conditional`; and a callee **no contract describes anywhere** is not descended into at
all. Three honesty conditions attach, all in §5.5: the diagnostic names the callee and
the call site; Ply never inlines an unclaimed body into a caller's proof; and an assumed
boundary contract is *owed evidence* until something exercises it against the real body.
The §2 D5 row and a new "what this rule reaches" paragraph (its limits, below) landed with
it.

**Red first.** `tests/fixtures/unclaimedcallee` — `tiered_fee` claims `bounded(2)` and
calls `legacy_rate`, which nothing describes — plus
`tests/e2e/tests/unclaimedcallee_fixture.rs`. First run, 48.58s:

```
assertion `left == right` failed: a `bounded` check that would have to descend into an
unclaimed callee earns no evidence -- it must never report a proof whose meaning includes
a body no contract describes: {"command":"verify","diagnostics":[], ... "verdict":"bounded(2)" ...}
  left: "bounded(2)"
 right: "unclaimed"
```

That is the defect in the tool's own output: Kani inlined the unclaimed body, the caller
banked the result, and the envelope carried **zero** diagnostics. The same test now passes
in 0.08s — the refusal is a call-graph decision taken before any engine starts.

For the second branch the "before" was captured literally, from a `git worktree` at
`f2c4e82`, on `tests/fixtures/boundarycontract` (whose ply.yaml declares
`legacy_rate: ensures: ["|result| *result <= 10_000"]`):

```
"root": { "verdict": "unclaimed", "children": [ { "id": "boundarycontract", ... "children": [
    { "id": "legacy_rate",  "kind": "fn", "verdict": "unclaimed" },
    { "id": "tiered_fee",   "kind": "fn", "verdict": "bounded(2)" } ] } ] },
"diagnostics": []
real 0m43.867s
exit: 0
```

Two defects in one envelope: the `bounded(2)` had inlined the real body with nothing
recording it, and the declared `ensures:` was eaten by serde so `legacy_rate` was reported
as an unclaimed *claim* — the opposite of what the file says.

**004's boundary, before and after.** `run.sh s3` (the boundary fn alone, 600s budget),
before, from `vetting/004-legacy-extension.md`:

```
"root": { "id": "workspace", "verdict": "timeout", "children": [
    { "id": "withdrawal", "verdict": "timeout", "children": [
        { "id": "tier_fee_cents", "kind": "fn", "verdict": "timeout" } ] } ] }
"code": "K0601", "title": "Kani could not finish checking `tier_fee_cents` within
         its 600s time budget -- ..."
real	11m23.094s
verify exit: 0
```

After (literal, `/tmp/ply-004-after/s3.txt`):

```
"code": "W0512", "severity": "warning", "check": "bounded(2)",
"node_id": "withdrawal::tier_fee_cents",
"title": "Ply did not check `tier_fee_cents`: proving it would mean descending into
 `ledger::fees::bps_for_tier` (called at line 44, column 15), and no contract anywhere
 describes what that code promises -- not on the function itself, and not in ply.yaml. ...
 So this check earned no evidence at all -- the verdict is `unclaimed`, never `bounded(2)`,
 and never a violation. (W0512, §5.5)"
"open_item": "unclaimed_callee"

real	0m0.005s
```

**11m23.094s → 0m0.005s**, and the callee `ledger::fees::bps_for_tier` — never once
mentioned by the old diagnostic — is named with its call site.

**004's boundary with a declared contract.** `run.sh s5` declares
`ensures: ["|result| *result <= 10_000"]` for `ledger::fees::bps_for_tier` in `ply.yaml`.
Before, this died with a misleading `E0301` ("could not find fn `fees::bps_for_tier` in
./src/lib.rs") because `anchor:` was ignored and every component's fns were looked for in
the *feature* crate. After (literal, `/tmp/ply-004-after/s5.txt`):

```
"id": "tier_fee_cents", "kind": "fn", "verdict": "bounded(2)",
"statuses": [ "conditional", "owed-evidence" ]

"code": "W0511", "title": "`tier_fee_cents` earned bounded(2), but conditionally: the proof
 used the contract declared in ply.yaml for each callee it crosses into, instead of that
 callee's real body. Assumed: `ledger::fees::bps_for_tier`: ensures |result| *result <=
 10_000. ... Nothing has checked them against the real code yet, so each one is owed
 evidence rather than settled: an assumed contract nobody exercises is green paint.
 (W0511, §5.5)"
"assumptions": [ { "kind": "assumed_contract", "fn": "ledger::fees::bps_for_tier",
                   "verdict": "unclaimed", "contract": "..." } ]

real	3m15.948s
verify exit: 0
```

The generated harness, verbatim from the same run — the stub is real, and cross-crate:

```rust
#[cfg(kani)]
#[allow(dead_code, unused_variables)]
fn ply_stub_ledger_fees_bps_for_tier(tier: u8) -> u32 {
    let __ply_result: u32 = kani::any();
    kani::assume((|result: &u32| * result <= 10_000)(&__ply_result));
    __ply_result
}

#[cfg(kani)]
#[kani::proof_for_contract(tier_fee_cents)]
#[kani::stub(ledger::fees::bps_for_tier, ply_stub_ledger_fees_bps_for_tier)]
fn ply_proof_tier_fee_cents() { ... }
```

**Measured, and the review's estimate corrected.** The review predicted "`conditional
bounded(2)` in about a minute". It is **not** about a minute: Kani reports
`Verification Time: 201.77356s` for this harness (hand-written stub spike, 2026-08-25),
and the full `verify` stage is 3m15.9s wall including compile. `run.sh s5`'s budget was
raised from 120s to 600s for that reason, with the change annotated in `run.sh`; at 120s
the stage reports `timeout` and says nothing about the assumption (that run is preserved
above as the reason). The cost is explicable: the control (s4) replaces the call with
`match tier { 0 => 150, ... }` — four concrete values — while the stub returns a
*symbolic* `u32` constrained only by `<= 10_000`. The assumption is weaker than the code,
which is what makes it an assumption, and it costs more to reason about.

**What the rule reaches (limits, recorded in §5.5).** It applies to `bounded` only. It
fires for callees Ply resolves: free-function calls naming a `fn` in the caller's own
file, or in a **path dependency's** `src/lib.rs` (walking inline `mod`s, which is how
`ledger::fees::bps_for_tier` is found). Method calls on a receiver are not call sites for
this rule — flagging `x.min(10_000)` and `v.len()` would fire on every ordinary line of
Rust. Calls into `std`, `core`, or a registry crate resolve to no source Ply can read and
are left alone, so a `bounded` verdict can still include a body Ply never examined.
**This is a real gap**, stated in the spec rather than left to be found.

> **RETRACTED 2026-08-25** (adversarial review, D1; closed in
> `docs/post-004-review-closure.md`). This paragraph ended "just never a first-party
> one", and that was false when written: the resolver never read `use` declarations, so
> `use rates::legacy_rate;` plus a bare-name call classified *unresolved* -- and
> unresolved meant descend. The most idiomatic spelling in Rust bought a clean
> `bounded(2)`, zero diagnostics, exit 0, over an unclaimed first-party body. The
> resolver now follows `use` declarations (renames, groups, globs), inline and file
> modules and re-exports, and refuses (`W0513`) any first-party source it was pointed at
> and could not read. The gap that remains is stated in §5.5's own limits.

**Also landed here, because branch 2 could not work without it**: `anchor:` is consumed
(a component anchored elsewhere is a boundary component — contracts read, `checks` not
run, `W0303` saying so, no node); `FnClaim` gained `requires`/`ensures`; a fn entry that
declares a contract and asks for no checks is a boundary contract declaration and earns no
node; `Diagnostic` gained §8's `assumptions` array; `conditional` propagates upward as a
status (D6) via `union_statuses`.

**Deferred, with reasons.** D5's *first* branch (`stub_verified` for a callee that passed
its own proof this run) is still not implemented — it needs the callees-first scheduling
ADR-0003 calls the entire soundness guarantee, which the review sequences as the next
tranche. Concretely: 004's `total_debit_cents` still has `fee_cents` inlined rather than
stubbed, and still times out at 120s. `ply.yaml` `requires`/`ensures` are still not ANDed
into the fn's *own* check (§5.4) — `W0510` now says so out loud instead of dropping them.
Exercising a boundary assumption (fuzzing the callee against its declared contract) is not
built; the assumption is reported as `owed-evidence`, and `cargo ply audit`/`worklist`,
which §5.5 says should list it, do not exist yet — **NOT BUILT**, not NOT RUN.

## 2 — Absence of evidence fails the run by default

**Spec.** §1 gains the principle, stated where the evidence rules live: *a run succeeds
only if every claim earned its declared evidence*; `timeout`, `unsupported`, `tool_error`,
`unclaimed` and `engine-missing` are absences, and absence of evidence fails the run by
default. §6's exit table gains the missing row and a `--fail-on` table
(`warn` | `evidence` (default) | `error`), with `error` named as the documented opt-out
that reproduces the old behaviour. §6's exit codes 2 (tool error) and 3 (missing engine)
are now actually returned, which they never were.

The review's principle names five absences; the brief named three. The review wins:
`unclaimed` and `engine-missing` are in, which matters because §5.5's refusal verdict is
`unclaimed` — the boundary rule and this principle are the same fix seen twice.

**Red first.** An assertion added to `tests/e2e/tests/timeout_fixture.rs`. First run:

```
assertion `left == right` failed: a run whose only check timed out has no evidence in it,
and must not exit 0: {"command":"verify","diagnostics":[{"code":"K0601", ... "severity":"warning" ...}],
 "root":{ ... "verdict":"timeout"}}
  left: Some(0)
 right: Some(1)
```

Root verdict `timeout`, one warning, exit 0 — vetting 004's finding 1 in miniature. A
second e2e pins the opt-out (`--fail-on error` on the same fixture still exits 0), and
four unit tests in `ply-cli` pin the table itself, including that real evidence still
exits 0.

## 3 — Seed and record the fuzz run, and recover the panic witness

**Spec.** §5.4c gains "a `fuzz(n)` verdict names the run that produced it": the harness's
RNG is built from a seed Ply chooses and records, proptest's own persisted-failure replay
is switched off, `--seed <hex>` replays exactly, and the honesty caveat is written into
the rule itself — *it buys replay and auditability, not detection power*. §5.4c's
witness-recovery sentence is corrected: a panicking body no longer counts as a failure
whose witness "cannot be recovered", because proptest prints it. §8 documents the node's
`evidence: { engine, seed, cases }`; §6 documents `--seed`.

**Red first, the escalation half.** The panicbug fixture, rewritten to demand what a real
crash bug deserves. First run:

```
assertion `left == right` failed: a body that panics on a legal input has broken its
promise, and Ply can now show the input: {... "code":"X0901" ... "title":"proptest reported
a failing case for `halves`, but Ply could not find the line its own generated harness
prints to record the failing input ..." ... "verdict":"tool_error"}
  left: String("tool_error")
 right: "violation"
```

That is the defect exactly: a function that panics on an ordinary input, correctly found
by proptest, reported as *Ply's* problem. After (literal, `cargo-ply verify` on the same
fixture):

```
"id": "halves", "kind": "fn", "verdict": "violation",
"evidence": { "engine": "proptest",
              "seed": "093eb922b294a5d02253fb7a16389a1297d2dcd3e5c334bf1879a87a29d0a7cc",
              "cases": 256 }

code: P0502
title: `halves` does not return at all for this input -- it panicked before its
 postcondition `|result|*result *2 == x` could even be evaluated. proptest shrank the
 failing case to the smallest input that still crashes, and it is below. A function that
 panics inside its own declared precondition has broken its promise as surely as one that
 returns a wrong answer, so this is a violation, with a witness. (P0502)
cex: { "inputs": { "x": "13" }, "cargo_test": "src/ply_generated_cex.rs" }
exit: 1
```

§5.4c's MUST is untouched: no witness, no violation. What changed is that the witness was
there all along, in proptest's own `minimal failing input:` report, and the adapter was
throwing it away.

**Red first, the seeding half.** A `ply-core` unit test asserts the generated harness
builds its runner from `TestRng::from_seed` (not `TestRunner::new`) and switches
`failure_persistence` off, and that the same source derives the same seed while a changed
contract or a different fn derives a different one. Seeding's *observable* regression is
004's own `run.sh s8`, below — a randomised e2e would have been a flaky red, which is not
a red.

**s8, before and after.** Before (`vetting/004-legacy-extension.md`, six fresh copies of
identical unfixed source):

```
run 1: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 2: tool_error  [('approve_withdrawal', 'tool_error')]  ['X0901']
run 3: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 4: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 5: tool_error  [('approve_withdrawal', 'tool_error')]  ['X0901']
run 6: tool_error  [('approve_withdrawal', 'tool_error')]  ['X0901']
```

After (literal, `/tmp/ply-004-after/s8.txt`):

```
run 1: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 2: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 3: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 4: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 5: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
run 6: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
```

**Six for six identical — and it misses the bug every time.** This is the honesty caveat
made concrete, and it should be read before anyone celebrates the six identical lines: the
derived seed for `approve_withdrawal` is one that does not draw an overflowing input, so a
run that used to find the real panic half the time now never does. Nothing was lost —
that half was never reliable — but nothing was gained in *detection* either. The arithmetic
explains it exactly: the overflow needs `amount_cents` in (28.6M, 100M], the strategy draws
from `any::<u32>()` a quarter of the time, and `requires(amount_cents <= 100_000_000)`
rejects ~97.7% of those, so only about two of 256 accepted cases come from the wide arm and
roughly one of those overflows — a coin flip, which is what the six-run split measured.
Seed plus `mutate`, not seed alone, is the reliability story.

**And the replay path works.** Six explicit seeds on the same unfixed source:

```
5feceb66ffc86f38...  fuzzed(256) []
6b86b273ff34fce1...  fuzzed(256) []
d4735e3a265e16ee...  violation ['P0502']
4e07408562bedb8b...  fuzzed(256) []
4b227777d4dd1fc6...  fuzzed(256) []
ef2d127de37b942b...  violation ['P0502']
```

and the finding seed replays exactly, three times running:

```
run 1 violation P0502 {'amount_cents': '28633116', 'balance_cents': '1', 'tier': '4'}
run 2 violation P0502 {'amount_cents': '28633116', 'balance_cents': '1', 'tier': '4'}
run 3 violation P0502 {'amount_cents': '28633116', 'balance_cents': '1', 'tier': '4'}
```

`28633116 × 150 = 4,294,967,400`, which is `u32::MAX + 105` — the shrunk witness is the
first input that overflows, and it is now a `violation` with an input rather than an
`X0901` about Ply's harness. That is the whole of finding 4 closed: reproducible, and
reportable.

**Recorded, not fixed.** The rendered cex test for a panicking body fails with the
*function's own* panic message rather than the contract message, because the call sits
outside the test's `catch_unwind` (deliberately — it is the call that crashes). §9's cex
oracle clause "with failure output that states the contract, pinned by substring" therefore
does not hold for this shape; the contract is named in the generated test's own comment
above it. The oracle test itself (`clamp_oracle.rs`) is unaffected and green.

## 4 — Unknown-key parity on the verify path

**Spec.** §5.1a rule 1 gains the sentence that was missing: the rule **binds every tool
that reads a `ply.yaml`, not only `check`**, and — the converse, which matters just as
much — a tool must accept every key §5 defines even where it acts on none of them. One
document, three readers; a reader that refuses the keys it ignores breaks that outright,
and a reader that silently drops the keys it does not know is how vetting 004's finding 7
happened.

**Red first.** `tests/e2e/tests/unknown_key_fixture.rs`, `ensure:` for `ensures:` on the
clamp fixture's own claim. First run:

```
an unknown key must be refused with E0204, not silently dropped: 
```

The assertion message is followed by nothing, because **stderr was empty**: the typo
produced no error, no warning, and no note. The run simply proceeded without the contract.

**After** (literal, `cargo-ply verify . --json`, exit 1):

```
Error: E0204: `ensure:` is not a key Ply knows. The keys a fn claim accepts are: `checks`,
`mode`, `requires`, `ensures`, `examples`, `check_with`, `trusted`, `unresolved`, `entry`.
Did you mean `ensures`? A key Ply does not know is almost always a typo, and a typo has to
be caught rather than ignored (§5.1a rule 1) -- an ignored key is a contract you think you
wrote and Ply never read. Found at `components.clamp.fns.clamp.ensure` in ply.yaml.
```

and `ply-check` on the same file, for parity:

```
error: /tmp/e0204/ply.yaml did not parse as ply.yaml: components.clamp.fns.clamp: unknown
field `ensure`, expected one of `checks`, `mode`, `requires`, `ensures`, `examples`,
`check_with`, `trusted`, `unresolved`, `entry` at line 8 column 9
ply-check exit: 2
```

Both refuse it now. `verify`'s message additionally carries the nearest-key suggestion
E0204 calls for, which `tools/model`'s own doc comment says it leaves "to the full
`ply check` implementation".

**The validator is the whole §5 grammar, deliberately.** `config::validate_keys` walks the
document against key sets mirrored from `tools/model` — document, component, fn claim,
external, `trusted` entry, `unresolved` entry — not the subset `verify` acts on. A second
e2e pins that: `pure:`, `strict:`, `edges:`, `deny:`, `profiles:` on a fixture `verify`
ignores entirely must still run clean. 004's own `ply.yaml`, which carries `pure: true` and
`edges:`, re-runs unchanged (`run.sh s3`, no `E0204`).

**`anchor:` — the other half of finding 7 — landed in item 1**, because D5's second branch
could not work without it: a component anchored at another crate is a boundary component,
its contracts are read, its `checks` are not run here, and `W0303` says so rather than the
misleading `E0301` s5 used to produce.

**Deferred, recorded.** `schema/ply.schema.json` still does not exist; §5/D3 call it
normative and the two key sets now agree by mirroring rather than by construction. That
reconciliation (promote one model, delete the other — the TODO at the top of
`ply-core/src/config.rs`) is unchanged in scope by this fix and stays on TODO.md.

## 5 — The implemented fragment reaches §5.4b: arrays first

**Measured before implemented**, on a scratch crate with trivial bodies so the number is
the *construction* cost and nothing else (Kani 0.67.0, `Verification Time` as Kani reports
it):

| shape | Kani verification time |
|---|---|
| `u32` (baseline) | 0.0287 s |
| `char` | 0.0638 s |
| `Option<u32>` | 0.0360 s |
| `Result<u32, u8>` | 0.0399 s |
| `[u32; 4]` | 0.0358 s |
| `[u32; 16]` | 0.0405 s |
| `type Bps = u32` (alias) | 0.0282 s |

Every one is within a factor of ~2 of a bare `u32`, and `[u32; 16]` costs the same as
`[u32; 4]` — which is §5.4b's claim ("cheap with no annotation, because the bound is a
compile-time constant") measured rather than inherited. Nothing here needed an unwind
annotation, and the generated harness emits none: the `Vec` rule is a `Vec` rule.

One measurement worth keeping, because it is the trap: an `Option<u32>` parameter whose
body does the same widened multiply-then-divide as 004's `fee_cents` took **60.3 s**, over
1500× the 0.036 s of the same parameter with a trivial body. §5.4c's "checkability is a
property of the body, not just the signature" measured a third time. The fragment
widening buys *reachability*, not speed.

**Red first.** Three behavioural tests, run against the pre-change extractor:

```
§5.4b calls a fixed-size array v1's *preferred* bounded shape, and it must reach the
Kani gate: got Unsupported("[u32 ; 4]")
an alias is transparent in Rust: got Unsupported("AccountId")
§5.4b lists char/Option/Result as cheap unconditionally:
got [Unsupported("char"), Unsupported("Option < u32 >"), Unsupported("Result < u32 , u8 >")]
```

**What landed.** `RustType` gains `Char`, `Option(T)`, `Result(T, E)` and `Array(T, N)`;
`rust_type_from_syn` gains a `Type::Array` arm, `Option`/`Result`/`char` paths, and
top-level type-alias resolution (depth-capped at 8, because this reader is not a compiler
and must not hang on a cycle that would not compile anyway). Composite shapes are admitted
only down to leaves the engines really build — `[BTreeSet<u8>; 2]` stays `Unsupported`,
pinned by its own test, because widening the fragment must not widen it past the engines.
The Kani harness emits `let card_bps: [u32; 4] = kani::any();` and **no** unwind
annotation; the proptest strategy is `any::<T>()` for these shapes (no small-magnitude
bias: what is interesting about an `Option` is the variant, not the size), and the
failing-input marker prints them with `Debug` rather than `Display`.

**e2e**: `tests/fixtures/arraycard` is 004's own `carded_fee_cents` — the fragment-first
rate-card idiom — with a `[Bps; 4]` parameter, exercising the array shape and alias
resolution in one signature. It passes in 138.6s; the cost is the body, not the array.

**004's s7, before and after.** Before, from `vetting/004-legacy-extension.md`:

```
"code": "V0505", "node_id": "withdrawal::carded_fee_cents",
"title": "Ply cannot check `carded_fee_cents`: parameter(s) card_bps: Unsupported(\"[u32 ; 4]\")
 use a type neither the bounded (Kani) nor the fuzz (proptest) codegen builds inputs for. ..."
```

After (literal, `/tmp/ply-004-after/s7.txt`):

```
"root": { "id": "workspace", "verdict": "bounded(2)", "children": [
    { "id": "withdrawal", "verdict": "bounded(2)", "children": [
        { "id": "carded_fee_cents", "kind": "fn", "verdict": "bounded(2)" } ] } ] },
"diagnostics": []

real	2m50.087s
verify exit: 0
```

The fragment-first idiom §5.4b recommends first — pass the rate card in as data instead of
looking it up across the boundary — now works. `run.sh s7`'s budget was raised from 120s to
600s and annotated: the *shape* is 0.036s, the body is the rest.

**Honest limit, and the code that keeps it honest.** `WitnessValue` cannot spell any of
the new shapes, so a *failure* on one has no rendered counterexample. That gap is now
handled rather than crashed into: a Kani violation whose witness cannot be decoded is
reported as `X0901`/`tool_error` naming the parameter and its type, never as a witness-free
`violation` (§5.4c's MUST), and never by propagating a decode error out of `verify` and
killing the whole run — which is what the pre-change `?` would have done the first time
anyone claimed an array and got a violation. On the fuzz side the same case lands on the
existing `W0541` witness-only path. Decoders for these shapes are **not built**, and the
diagnostic says so in words a user can act on.

---

## Gates

- `cargo test --workspace -- --test-threads=1`: **102 passed, 0 failed** (was 72 before
  this round). 70 `ply-core` unit, 11 `ply-cli` unit, 21 e2e. New fixtures:
  `unclaimedcallee`, `boundarycontract`, `arraycard`; new e2e:
  `unclaimedcallee_fixture`, `boundarycontract_fixture`, `unknown_key_fixture` (2 tests),
  `arraycard_fixture`, plus one added to `timeout_fixture`; `panicbug_fixture` rewritten.
- `cargo fmt --all --check` and `cargo clippy --all-targets -- -D warnings`: clean.
- `cd tools && cargo test --release`: green (unchanged by this round — nothing in `tools/`
  was touched); `cargo fmt --check` and `cargo clippy --release --all-targets -D warnings`
  clean.

## Spec changes, in one place

§1 (the absence-of-evidence principle), §2's D5 row, §5.1a rule 1 (the rule binds every
reader, and so does its converse), §5.4c (the `fuzz(n)` seed rule; the corrected
witness-recovery sentence), §5.5 (rewritten: D5's three branches, three honesty
conditions, stated limits, boundary contract declarations), §6 (exit table, `--fail-on`
table, `--seed`), §8 (`evidence` on a node), §10's M5 bullet (what landed early and what
is still M5's).

One ordering blemish, recorded rather than tidied: item 2's §1/§6 text was written before
item 1 was committed and therefore landed inside item 1's commit (`2cf09c2`). The
behaviour landed in `d73558f`.

## What is not done

Everything in this section is on TODO.md with the same wording.

- **D5's first branch** (`stub_verified` for a callee that passed its own proof this run)
  needs callees-first scheduling, which lives unlinked in `tools/schedule`. 004's
  `total_debit_cents` still times out at 120s with `fee_cents` inlined.
- **§5.5's rule does not reach `std`/`core`/registry callees.** A `bounded` verdict can
  still include a body Ply never examined. (The trailing clause "just never a first-party
  one" was retracted 2026-08-25 -- see the review's D1 and
  `docs/post-004-review-closure.md`. The `use`-import hole it hid is closed; two narrower
  first-party gaps -- transitive callees, and calls Ply's reader cannot see -- are stated
  in §5.5 and on TODO.md.)
- **Nothing exercises a boundary assumption.** `owed-evidence` and `W0511` are built;
  `cargo ply audit` and `cargo ply worklist`, which §5.5 says list it, are **not built**,
  and fuzz-checking a declared contract against the real legacy body is not built.
- **`ply.yaml` contracts are not ANDed into the fn's own check** (§5.4). `W0510` says so.
- **No witness decoder for `char`/`Option`/`Result`/`[T; N]`.** A Kani violation on one is
  `X0901`/`tool_error` naming the parameter; a fuzz violation lands on `W0541`.
- **Cross-crate type aliases** are not resolved (004's `withdraw` stays `unsupported`
  either way, because of the `&mut ledger::Ledger` beside the alias). Structs of scalars,
  `--only-changed`, `cargo ply check`, `schema/ply.schema.json` and the renderer's
  earned-vs-declared split (004's finding 8) are untouched.

## NOT RUN

- **The full `run.sh` sweep.** Stages s3, s5, s7 and s8 were re-run and are quoted above;
  s0, s1, s2, s4 and s6 were not re-run in this session. s1/s2 are 7-minute Kani stages
  whose behaviour is unchanged except where s3's rule applies, s4 is the control, s6 is a
  CLI-surface probe whose two answers (`check` absent, `--only-changed` absent) are
  unchanged.
- **`mutate` and `prove`** in 004: still no fn in that scenario declares either.
- **A boundary callee with a non-scalar signature.** `ledger::fees::bps_for_tier` is
  `u8 -> u32`; the stub generator's behaviour on a callee returning a struct, or returning
  `()` (which it refuses with `W0512`, by construction), was not exercised against a real
  crate.
- **Any budget above 600s** at the boundary, and any bound other than `bounded(2)`.
