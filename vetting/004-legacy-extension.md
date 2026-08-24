# Vetting 004 — a new feature inside an old codebase

Scenario: an ordinary two-year-old ledger module, and one new feature added beside it,
written **inside The-Ply-Spec.md §5.4b's checkable fragment from the first line** rather
than designed freely and audited afterwards. This is the first vetting scenario of both
kinds: the first designed fragment-first, and the first **executed against the real
`cargo ply verify`** (Kani 0.67.0, cargo-mutants 27.1.0, proptest) rather than reasoned
about on paper. Run 2026-08-24.

Vetting 001–003 all designed a system naturally and then asked whether Ply could check
it; 001's own feasibility pass found its flagship methods `unsupported`. 004 inverts
that, to test the sharpened target: Ply is for *new and modified* code, where the new
code is written inside the subset and the surrounding code is not. If that thesis holds,
a fragment-first design should feel like a design, not like a contortion, and the
boundary between the two should produce an honest verdict.

Everything below that is quoted as tool output is literal, copied from
`vetting/004-legacy-extension/run.sh`'s own stage logs; the two places where a long
envelope is cut down to its verdict spine say so on the line above. Anything not run is
marked **NOT RUN**.

## The design under test

`vetting/004-legacy-extension/` is two crates, each its own workspace root (the
`tests/spike` and `tests/fixtures` convention), so the scenario never joins the product
workspace.

**`legacy/` — `ledger`.** Written as if it had been there for two years, and
deliberately *not* a strawman: nothing in it is unusual, hostile, or contrived. No
`ply::` attributes anywhere, no claims on any of its functions.

```rust
pub type AccountId = u64;

pub struct Ledger {                       // private fields: the two must move together
    balances: BTreeMap<AccountId, i64>,
    journal: Vec<Entry>,
    next_seq: u64,
}
impl Ledger {
    pub fn post(&mut self, account: AccountId, amount_cents: i64, kind: EntryKind) -> u64;
    pub fn balance(&self, account: AccountId) -> i64;         // BTreeMap lookup
    pub fn entries_for(&self, account: AccountId) -> Vec<Entry>;   // Vec-returning query
}
pub fn total_by<T, F>(items: &[T], amount: F) -> i64 where F: Fn(&T) -> i64;  // generic

pub mod fees {                            // table-driven since the pricing rework
    pub const STANDARD_BPS: u32 = 150;
    fn schedule() -> &'static BTreeMap<u8, u32>;               // OnceLock
    pub fn bps_for_tier(tier: u8) -> u32;                      // ← the boundary point
}
```

**`feature/` — `withdrawal`.** The new feature: what a withdrawal costs and whether the
account can afford it. Every claimed function is a top-level free function over scalars,
because that is what the fragment admits.

```rust
#[ply::requires(amount_cents <= 100_000_000 && bps <= 10_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn fee_cents(amount_cents: u32, bps: u32) -> u32 { amount_cents * bps / 10_000 }

#[ply::requires(amount_cents <= 100_000_000 && bps <= 10_000)]
#[ply::ensures(|result| *result >= amount_cents)]
pub fn total_debit_cents(amount_cents: u32, bps: u32) -> u32;

// THE BOUNDARY: fragment-clean signature, body that calls unclaimed legacy code.
#[ply::requires(amount_cents <= 100_000_000)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn tier_fee_cents(amount_cents: u32, tier: u8) -> u32 {
    let bps = ledger::fees::bps_for_tier(tier).min(10_000);
    fee_cents(amount_cents, bps)
}

#[ply::requires(amount_cents <= 100_000_000)]
#[ply::ensures(|result| !*result || balance_cents > 0)]
pub fn approve_withdrawal(amount_cents: u32, balance_cents: i64, tier: u8) -> bool;

// THE SHELL: where the feature really meets the ledger. Outside the fragment by
// construction — see finding 5.
pub fn withdraw(accounts: &mut ledger::Ledger, account: ledger::AccountId,
                amount_cents: u32, tier: u8) -> bool;
```

Canonical YAML: [004-legacy-extension/feature/ply.yaml](004-legacy-extension/feature/ply.yaml).
One document, read by three tools with nothing duplicated between them — `cargo ply
verify` takes `anchor`/`checks`/`examples` and ignores the rest, `ply-check` and
`ply-render` read the same file in the full §5 grammar. `ply-check` passes it clean
(exit 0).

## Question 1 — is a fragment-clean design still a natural design?

**For the arithmetic core: yes, and the contracts improved it. For everything else: the
tool made the design decisions, not the designer.** Both halves are real, and the second
half is much larger than §5.4b's prose suggests, because **the fragment as implemented is
narrower than the fragment as specified**.

What genuinely helped:

- The fee and limit rules really are scalar-in, scalar-out. Left to myself I would have
  written them as private methods on a `FeeSchedule` struct; free functions with
  `requires`/`ensures` are a better shape for this logic, and the preconditions
  (`amount_cents <= 100_000_000 && bps <= 10_000`) are documentation I would otherwise
  have left in a comment.
- 002's guidance — *state you want checked should be constructible state* — applied
  deliberately, worked: `approve_withdrawal` takes `balance_cents: i64`, not `&Ledger`,
  and is checkable because of it.
- The first real run found a real bug — the `fee_cents` overflow below — which is the
  return on all of it.

What the fragment forced, each one a thing I wanted to write and could not:

1. **No domain types at all.** `amount_cents: u32`, not a `Cents` newtype; `u64`, not
   `AccountId`. This is not a guess — Ply's own diagnostic says so about the one place a
   legacy alias does appear: `account: Unsupported("ledger :: AccountId")` (finding 5).
   A newtype and a type alias are one line of ordinary Rust each, and either one moves a
   function out of the checkable set.
2. **No methods, one file.** Ply's extractor walks top-level `Item::Fn` in
   `<crate>/src/lib.rs` and nothing else (`crates/ply-core/src/harness.rs::discover_fn`),
   so a claimed feature is a flat module of free functions in a single file. That suited
   this feature and would not suit most.
3. **Fixed-size arrays — §5.4b's own *preferred* bounded shape — are not implemented.**
   The fragment-first way to keep the legacy table out of the proof is to pass the rate
   card in as data (`card_bps: [u32; 4]`) instead of looking it up. §5.4b explicitly
   recommends this ("this is v1's **preferred** bounded shape, and generated harnesses
   should reach for it first"). It is refused — see finding 5.
4. **The property I most wanted was inexpressible.** What `withdraw` owes its caller is
   two-state: on `true`, the balance falls by exactly the amount plus the fee. §5.4a
   admits `old()`, but only over its closed expression subset, and `old(accounts.balance(
   account))` is a method call on a struct receiver. The contract in the source is the
   weak one that fits.
5. **The feature's entry point is unclaimable by construction.** Keeping state out of the
   checked signatures does not delete the code that touches state; it relocates it. Every
   legacy contact ends up in `withdraw`, which no engine can build inputs for, so the one
   function a reviewer would most want checked is the one that cannot be.

Net: writing the *core* inside the fragment felt like ordinary good practice. Writing a
*feature* inside it did not — after the pure arithmetic there was nothing left that the
fragment could hold, and the boundary the scenario is about ends up living entirely in
unclaimed code.

## Question 2 — what actually happens at the boundary

Run as written, with the natural `u32` fee arithmetic
(`run.sh s1`, `--engine-timeout 120`, 6m51s wall clock, exit 1):

(the envelope's verdict spine and diagnostic codes, summarised; `$PLY_004_OUT/s1.txt`
has the full JSON)

```
withdrawal::fee_cents          violation     K0502, counterexample {amount_cents: 3589630, bps: 9568}
withdrawal::total_debit_cents  violation     K0502, counterexample {amount_cents: 59365685, bps: 6929}
withdrawal::tier_fee_cents     timeout       K0601   ← the boundary
withdrawal::approve_withdrawal fuzzed(256)
withdrawal::withdraw           unsupported   V0505
```

The two violations are a real bug, not a contrived one: `amount_cents * bps` overflows
`u32` well inside the declared precondition (3,589,630 × 9,568 ≈ 3.4 × 10¹⁰). I wrote
that line the way anyone writes it. Widening the product to `u64` is the one-line fix
(`run.sh s2` applies it), after which `fee_cents` earns `bounded(2)`.

The boundary function's literal verdict, on every run of it at a 120s budget:

```json
{
  "code": "K0601", "severity": "warning", "engine": "kani", "check": "bounded(2)",
  "node_id": "withdrawal::tier_fee_cents",
  "title": "Kani could not finish checking `tier_fee_cents` within its 120s time budget -- this is an exhausted search, not a broken promise: Kani never got far enough to say whether the contract holds or not, so this is reported as `timeout`, never as a violation. (K0601)",
  "open_item": "timeout"
}
```

**So the answer is `timeout`, not `conditional`, and not a false pass.** Three things
follow, each with its own finding below: the verdict is honest but uninformative
(finding 2), the boundary really is the cause (finding 3), and the run that contains it
exits 0 (finding 1).

The control settles causation. `run.sh s4` verifies the *identical* function, identical
contract, identical arithmetic, with only the legacy call replaced by an in-fragment
`match tier { 0 => 150, 1 => 90, 2 => 45, 3 => 0, _ => 150 }`:

```
=== s4: control — same fn, same contract, no call across the boundary ===
{ ... "root": { "id": "workspace", "verdict": "bounded(2)", "children": [
      { "id": "withdrawal", "verdict": "bounded(2)", "children": [
          { "id": "tier_fee_cents", "kind": "fn", "verdict": "bounded(2)" } ] } ] },
  "diagnostics": [] }

real	1m20.552s
verify exit: 0
```
(the envelope elided to the verdict spine; `$PLY_004_OUT/s4.txt` has it in full,
with a 600s budget available and no diagnostics at all)

One `BTreeMap` lookup behind a `OnceLock`, in code the feature merely calls, is the
difference between a proof in seconds and an exhausted search.

The other direction of the same boundary is fine: `approve_withdrawal` calls
`tier_fee_cents`, which calls `ledger::fees::bps_for_tier`, and it earns `fuzzed(256)` —
proptest simply *runs* the legacy code, so nothing about the boundary troubles it.
That is the good news, and finding 4 is why it is smaller than it looks.

## Question 3 — can the delta be scoped?

**No. Neither mechanism exists** (`run.sh s6`, literal):

```
--- cargo-ply --help
Commands:
  verify  Run checks via engines and write cex artifacts (§6)
  help    Print this message or the help of the given subcommand(s)
--- cargo-ply check .
error: unrecognized subcommand 'check'
exit: 2
--- cargo-ply verify . --only-changed
error: unexpected argument '--only-changed' found
exit: 2
```

`cargo ply check` and the global `--only-changed` are both specified in §6; only `verify`
and `--json` are built. Recorded, not built — see finding 9 for why `--only-changed` is
not a convenience flag in this particular thesis.

## What held

- **Ply never reported evidence it did not have.** The boundary function was checked in
  five separate `verify` runs (s1, s2, s3, s5, and the first exploratory run) and came
  back `timeout` in every one — never a pass, never a witness-free violation. §5.4c's
  "a timeout is not a violation" MUST survived contact with a case it was not designed
  for (an unclaimed callee), which is the property this project exists to protect.
- **The fragment paid for itself on the first run.** `fee_cents` was written the way
  anyone writes a basis-points fee, and Kani produced a concrete counterexample
  (`amount_cents: 3589630, bps: 9568`) inside the declared precondition. No unit test in
  this scenario would have found it; the `examples` entries certainly did not.
- **`unsupported` is reported as a fact, not as a failure.** `withdraw` — the shell that
  actually touches the ledger — comes back `unsupported` with V0505 naming the offending
  parameter types, and the run continues. That is the honest answer for a signature no
  engine can construct, and it arrived in milliseconds rather than after a timeout.
- **One document served three tools with nothing duplicated.** `feature/ply.yaml` is read
  by `cargo ply verify` (M3 subset), `ply-check` (clean, exit 0) and `ply-render` (the
  committed SVG). The two grammars coexisted in one file without a second copy.
- **The picture reads correctly at the boundary**: a solid, claimed call edge
  from a green component into a dashed, unclaimed one. A reader can see where the
  evidence stops (though not, per finding 8, that it also stopped inside the green box).

## Findings

1. **A run in which nothing was checked exits 0.** `run.sh s2` — the scenario after the
   overflow fix — ends with `tier_fee_cents` and `total_debit_cents` both `timeout`, the
   root verdict `timeout`, and:

   ```
   "root": { "verdict": "timeout", ... }
   real	7m13.803s
   verify exit: 0
   ```

   `main::exit_code_for` returns non-zero only for an `error`-severity diagnostic, and
   `K0601 timeout` is a warning. §6's exit table has rows for clean, violations, tool
   error and missing engine, but none for *checked nothing*, and its `--fail-on=warn|error`
   flag is unimplemented. In CI this run is green while two of the five claims produced no
   evidence at all (a third, `withdraw`, was never attempted) — and one of those two is
   the function that crosses the boundary this scenario exists to test. → Direction: `timeout`, `unsupported` and `tool_error` at any
   node should fail the run by default (they are absences of evidence, which is what §1
   says a Ply run is *for*), with `--fail-on` as the opt-out rather than the opt-in.

2. **The boundary answer is `timeout`, and the spec has no branch for an unclaimed
   callee.** D5 splits on what the callee earned — `stub_verified` when it passed its own
   Kani proof, `conditional` (`W0511`) for "anything else, listing each assumed
   contract". Both branches assume the callee *has* a contract. Legacy code has none, and
   there is nothing to assume, so neither branch applies: Kani inlines the real
   `BTreeMap`-and-`OnceLock` body and symbolically executes it until the budget runs out.
   None of D5 is implemented in the product crates either — `stub_verified`,
   `conditional` and `W0511` appear nowhere in `crates/`, and `ply-schedule` (which owns
   the callee-before-caller planning) lives in the `tools/` workspace and is not linked
   into `cargo ply verify`. So today the boundary's behaviour is not a decision Ply makes;
   it is whatever the engine does with the inlined body. The verdict is honest —
   §5.4c's "a timeout is not a violation" MUST holds under real conditions, which is the
   single most important thing this scenario confirms — but it is uninformative: the
   diagnostic names `tier_fee_cents` and never mentions `ledger::fees::bps_for_tier`, so
   nothing tells the user that the cost came from across the boundary, let alone which
   call it was. → Direction: D5 needs an explicit third branch for an *unclaimed* callee,
   and whatever it decides (refuse to descend and report `unclaimed-callee`; or admit a
   declared boundary assumption and go `conditional`), the diagnostic must name the callee
   that was descended into.

3. **The boundary is decisively the cause here — but a fragment-clean signature is still
   no promise of checkability.** The control (s4) proves causation: identical function,
   identical contract, identical arithmetic, the legacy lookup replaced by a `match`,
   verifies `bounded(2)` in 1m20s of total `verify` wall clock, compile included. With
   the legacy call in place and a **600s**
   budget (`run.sh s3`), the same function still yields:

   ```
   "root": { "id": "workspace", "verdict": "timeout", "children": [
       { "id": "withdrawal", "verdict": "timeout", "children": [
           { "id": "tier_fee_cents", "kind": "fn", "verdict": "timeout" } ] } ] }
   "code": "K0601", "title": "Kani could not finish checking `tier_fee_cents` within
            its 600s time budget -- this is an exhausted search, not a broken promise ..."

   real	11m23.094s
   verify exit: 0
   ```

   That is roughly 7½× the control's *entire* run, spent inside the engine alone, with
   no verdict. And separately, `total_debit_cents` — which touches no legacy code at all,
   only `amount_cents + fee_cents(amount_cents, bps)` — also timed out at 120s in the same
   run in which `fee_cents` passed. §5.4c already says "checkability is a property of the
   body, not just the signature"; this scenario measures it twice, and the second case has
   nothing to do with legacy code. → Direction: §5.4b gates on parameter *types*, which is
   the wrong axis for the fragment-first pitch. A design guide that says "write inside the
   fragment" has to say something about bodies — here, widened multiply-then-divide on
   symbolic integers — or a user who follows it exactly still gets timeouts.

4. **The one tier that crosses the boundary happily gives a different answer each run.**
   `approve_withdrawal` earns `fuzzed(256)` through the boundary — proptest just runs the
   legacy code — but that verdict is a coin flip. `run.sh s8` runs `verify` six times on
   six fresh copies of the *same, unmodified* source (the version that still contains the
   overflow), same command line:

   ```
   === s8: the same fuzz check on the same (unfixed) code, six fresh runs ===
   run 1: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
   run 2: tool_error [('approve_withdrawal', 'tool_error')] ['X0901']
   run 3: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
   run 4: fuzzed(256) [('approve_withdrawal', 'fuzzed(256)')] []
   run 5: tool_error [('approve_withdrawal', 'tool_error')] ['X0901']
   run 6: tool_error [('approve_withdrawal', 'tool_error')] ['X0901']
   ```

   Three of six runs report a clean pass on code that panics; three report the panic. The
   generated harness builds its runner with
   `Config { cases, ..Config::default() }` (`crates/ply-core/src/fuzz_gen.rs`), so the
   seed comes from entropy, and the envelope records no seed anywhere — the run that found
   the panic cannot be replayed, and the run that missed it cannot be distinguished from a
   real pass. The exit code flips with it: 0 for a `fuzzed(256)`, 1 for the `X0901`.
   → Direction: the `fuzz(n)` verdict must carry its seed in the §8 envelope, and a
   `--seed` (or a recorded-and-replayed seed in `ply.lock`) must make a fuzz run
   reproducible. Until then `fuzzed(n)` sits low in D6's evidence order *and* is the only
   kind of evidence that is not repeatable — a bad combination for the tier §5.4c makes
   the default for every Kani-excluded shape, and the only tier that works at a boundary.

5. **The implemented fragment is scalars, and it is much narrower than §5.4b.** §5.4b
   names fixed-size arrays as "v1's **preferred** bounded shape", and the fragment-first
   way to keep the legacy table out of the proof is exactly that: pass the rate card in as
   data. `run.sh s7` adds that function and claims it:

   ```
   "code": "V0505", "node_id": "withdrawal::carded_fee_cents",
   "title": "Ply cannot check `carded_fee_cents`: parameter(s) card_bps: Unsupported(\"[u32 ; 4]\") use a type neither the bounded (Kani) nor the fuzz (proptest) codegen builds inputs for. ..."
   ```

   `crates/ply-core/src/harness.rs::rust_type_from_syn` has no `Type::Array` arm at all.
   Nor does it resolve type aliases — the shell's `account: ledger::AccountId` (an alias
   for `u64`) is reported as `Unsupported("ledger :: AccountId")` in the same breath as the
   struct beside it (s1). So the *real* fragment today is: integers, `bool`, `Vec<u8>`,
   and (fuzz only) `Vec`/`BTreeSet` of scalars — no arrays, no structs, no aliases, no
   methods, one file. → Direction: this gap decides whether "design inside the fragment"
   is advice or a joke, and it is mostly cheap to close (arrays, alias resolution). Until
   it is closed, §5.4b describes a fragment nobody can actually write against, and this
   scenario had to be designed around the implementation rather than the spec.

6. **The fix Ply offers for an unsupported shape names a mechanism that does not exist.**
   Both V0505s above carry: *"add a `pure`-marked generator hook for `…`'s parameter type
   (§5.4b)"*. There is no `#[ply::pure]` macro (`crates/ply-attrs` exports `requires` and
   `ensures`, nothing else), and no `ply.yaml` key names a generator hook. The one piece
   of advice given to a user who has just been told their function cannot be checked sends
   them after something unbuildable — a newbie-bar failure of the specific kind §8 exists
   to prevent ("Ply proposes, never rewrites" still requires the proposal to be real).

7. **`verify` is single-crate, so the two-crate shape this scenario is about cannot be
   modelled.** `verify_crate` reads `<crate_dir>/ply.yaml` and resolves *every* component's
   fn claims against `<crate_dir>/src/lib.rs`; `anchor:` is parsed and never used. §5.4
   says contracts may be declared in `ply.yaml` "for teams that prefer external specs", so
   `run.sh s5` declares one for the legacy callee — the documented way to give an
   unclaimed callee a contract without touching its source. Result:

   ```
   "code": "E0301", "node_id": "ledger::fees::bps_for_tier",
   "title": "Ply could not find the function `fees::bps_for_tier` this claim anchors to.
             E0301: could not find fn `fees::bps_for_tier` in ./src/lib.rs (unresolvable anchor)"
   ...
   withdrawal::tier_fee_cents  timeout   (unchanged)
   ```

   Two defects in one: the claim is looked for in the *feature* crate because anchors are
   ignored, and the `ensures:` key itself is silently dropped — `ply-core`'s `FnClaim` has
   only `checks` and `examples`, and serde ignores the rest, so a team writing external
   specs today gets no contract and no warning. Note the asymmetry: the *same file* read by
   `ply-check` enforces §5.1a rule 1 (`additionalProperties: false`, "a typo must be
   caught, never ignored"); the verify path enforces nothing. → Direction: verify must
   validate against the same schema, and the ply.yaml contract-merge path (already listed
   as unimplemented in TODO.md) is the mechanism the whole legacy-boundary story depends
   on — it is how an unclaimed callee ever gets a contract.

8. **The picture draws declared ceilings as if they were earned.**
   [004-legacy-extension.svg](004-legacy-extension.svg) shows `tier_fee_cents B2` and
   `withdraw B2` in the same green as `fee_cents B2`. `fee_cents` earned `bounded(2)`;
   `tier_fee_cents` has never once finished; `withdraw` is `unsupported` and no engine can
   even build its inputs. The drawing is identical in all three cases, and it is identical
   before and after any run. This is already recorded in TODO.md as "separate declared
   ceilings from earned verdicts"; 004 is the first scenario where a real run makes the
   difference concrete — a reader of this diagram would conclude the boundary function is
   bounded-proved. → Direction: the renderer needs the verdict envelope as an input, and a
   declared-but-unearned ceiling must draw differently from an earned one.

9. **`--only-changed` is not a convenience for this thesis; it is the mechanism.** §6
   specifies it, and it does not exist (s6). The whole pitch of "Ply is for new and
   modified code" is that the checked set is the delta: this scenario's `verify` run has
   no way to say "check `withdrawal`, not `ledger`" other than by which crate directory it
   is pointed at — which works here only because the feature happens to be its own crate.
   A feature added as a *module* inside an existing crate has no expressible scope at all.
   → Direction: `--only-changed` (and `cargo ply check`) should be sequenced with the
   fragment-first pitch, not after it.

10. **`verify` writes into the crate under test.** It generates `src/ply_generated_*.rs`,
    adds `mod` declarations to `src/lib.rs`, and appends its harness crate to the target's
    `[workspace] members` (`harness_crate::ensure_workspace_member`, which also *requires*
    the target crate to have a `[workspace]` table — i.e. to be a workspace root, which an
    ordinary crate inside a larger workspace is not). That is why `run.sh` copies the
    scenario to a scratch directory before every run, exactly as `tests/e2e` does for the
    fixtures. For the adoption story this scenario is about — add Ply to a codebase that
    already exists — a tool that edits your `Cargo.toml` and your `src/` on every run is a
    harder sell than the milestone plan currently accounts for. Already on TODO.md as
    "where the harness crate should live"; 004 is a second vote for it.

## Rendered

[![the legacy boundary drawn from this scenario](004-legacy-extension.svg)](004-legacy-extension.svg)

Produced by `ply-render vetting/004-legacy-extension/feature/ply.yaml`
(`run.sh s0`). The feature component is green with its per-fn check badges
(`B2`, `F256 T e×2`); `ledger` is a dashed, empty box — declared, claimed by nothing —
and the call edge into it is solid, because the *edge* is a checked claim even though
the box it points at carries none.

Render-pass notes:

- Nothing overlaps, nothing is clipped, every badge is legible; checked by rasterising
  the committed SVG (CairoSVG, 1100px wide) and looking at it, since `qlmanage` and
  headless Chromium are both absent from this environment.
- **This document is not covered by the renderer's invariant sweep.**
  `tools/render/tests/render.rs` walks a hardcoded list of vetting documents (001, 002,
  003) plus its own fixtures; 004's YAML lives at a different path and is not in it.
  Adding it would mean editing `tools/`, which this session is not permitted to do —
  recorded here and in TODO.md for whoever does.
- Finding 8 (declared ceilings drawn as earned) is a render finding as much as a
  verify one.

## Confirmed walls (deliberate, unchanged)

- `withdraw`'s two-state promise (balance falls by exactly amount + fee) stays outside
  §5.4a: `old()` is admitted, method calls on a struct receiver are not. Same wall 001
  hit; no new information, no proposal.
- `ledger` itself is honestly unclaimed and stays that way. Nothing in this scenario
  tries to retrofit contracts onto two-year-old code, which is the whole premise.
- `total_by` (generic, closure parameter) and `entries_for` (`Vec`-returning query) were
  never claimed: `check_with` covers one concrete instantiation per fn (§5.4b) and
  nothing in the feature needs them checked.

## NOT RUN

- **`mutate` and `prove`.** No fn in this scenario declares either; the `·spec-strong`
  suffix and D12's kill-signal machinery are untouched here.
- **`conditional` / `W0511` / `stub_verified`.** Never observed, because none of D5 is
  implemented in `crates/` (finding 2). This scenario therefore says nothing about
  whether the `conditional` propagation is right — only that it never fires.
- **A boundary call with a non-scalar signature.** `ledger::fees::bps_for_tier` is
  `u8 -> u32`; a legacy callee taking or returning a struct was never tried, so all that
  is measured is the cheapest possible boundary crossing.
- **Whether the boundary ever finishes.** 600s is the largest budget tried (s3). No run
  established an upper bound, only that 600s is not enough.
- **Bounds other than `bounded(2)`.** The K0601 fix text suggests lowering the bound;
  `bounded(1)` at the boundary was not measured.
- **The delta thesis end to end.** `--only-changed` does not exist (finding 9), so
  "check only what changed" was never exercised — the scoping in this scenario comes
  from pointing `verify` at one crate directory.
- **`cargo ply check`, `tree`, `audit`, `worklist`, `accept`, `doctor`.** Only `verify`
  exists (s6).

## Where this leaves the thesis

The sharpened target — *Ply is for new and modified code, written inside the subset,
beside code that is not* — survived this scenario at the level of honesty and did not
survive it at the level of usefulness.

Honesty: every verdict Ply produced was defensible. The overflow was real and came with
the input that triggers it. The boundary said `timeout` and never pretended otherwise.
The shell said `unsupported` and said which parameter. No run claimed evidence it did
not have, which is the one thing this project cannot get wrong.

Usefulness: the part of the feature that ended up inside the fragment is the pure fee
arithmetic — a calculator. Every point where the feature touches the codebase it was
added to is outside the evidence: the shell is `unsupported` because a legacy struct
cannot be constructed, and the one function that *does* call across the boundary with a
fragment-clean signature cannot be proved at all — 600s of Kani against 1m20s for the
identical function with the call removed. What remains for the boundary is the fuzz
tier, which crosses it happily and reports a different verdict every other run
(finding 4).

So the single most direction-affecting result is findings 2 and 3 together: **`bounded`
cannot cross into unclaimed code, and the spec has no rule saying what should happen when
it tries.** That pushes the whole fragment-first pitch onto `fuzz`, which makes finding 4
— an unreproducible, unseeded `fuzzed(n)` — not a papercut but a load-bearing defect.
