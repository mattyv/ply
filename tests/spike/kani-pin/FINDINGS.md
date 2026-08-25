# Kani-pin spike — findings

Run 2026-08-25, Linux x86_64. Answers TODO.md's "Bump the Kani pin — a D13-shaped
spike, not a fork": `docs/kani-docs-sweep.md` records two stubbing blockers at the
pinned 0.67.0 and records that Kani `main` documents both as lifted. Documents are not
evidence. This spike installed a newer Kani beside the pin and ran Ply's own shapes
against both.

**The headline is not the one the TODO expected.** Blocker 1 — "`#[kani::stub]` and
`--concrete-playback` are mutually exclusive, so a stubbed failure yields no witness" —
**is already false at 0.67.0**: a stubbed harness that fails prints a witness today, and
Ply's `parse_output` would already accept it. What is true at 0.67.0 is a *different and
worse* thing, unchanged on `main`: the generated playback test **does not apply the
stub**, so replaying the witness fails on a bookkeeping panic instead of reproducing the
violation. Blocker 2 (`#[kani::stub]` over a contracted target, Kani #4591) reproduces
verbatim on `main` and is **not** fixed.

Everything below is a literal observed run. Nothing not attempted is guessed; the
NOT RUN list is at the end and is part of the finding.

---

## The two toolchains

| | A — the pin | B — the candidate |
|---|---|---|
| Identity | `cargo-kani 0.67.0` | Kani `main` @ `245709373965fcb78209135822cbafb59c08d036` (2026-08-25 03:17:54 +0000, *"Autoharness: per-parameter and trait-impl-derived generic instantiation (#4706)"*) |
| CBMC | 6.8.0 (bundled) | 6.10.0 (`kani-dependencies` on main pins this) |
| Rust | `nightly-2025-11-21` | `nightly-2026-04-01` |
| How it got here | already installed; **untouched by this spike** | `cargo build-dev` from a git checkout at `/home/user/model-checking/kani`, driven through `scripts/cargo-kani`; nothing installed into `~/.cargo/bin` or `~/.kani` |

**There is no newer release to bump to.** crates.io lists 69 versions of `kani-verifier`
and the newest is still `0.67.0` — the same seven-month-old release the docs sweep found
on 2026-08-23, still the latest today. "Bump the pin" therefore does not mean "bump a
version number"; it means "pin a git commit of an unreleased branch". That is a
materially different proposition from the one TODO.md poses, and it is the first finding.

`main` also still reports itself as **`Kani Rust Verifier 0.67.0 (cargo plugin)`**. The
version string does not distinguish the candidate from the pin. Any Ply D14 fingerprint
that records "kani 0.67.0" would be recording the same string for two engines that behave
differently — a real hazard if the pin ever moves to a commit.

### Installation friction, recorded because a bump inherits all of it

1. `cargo build-dev` fails immediately on a plain shallow clone:
   `failed to read <repo>/charon/charon/Cargo.toml`. Kani `main` now depends on the
   **`charon` submodule**; `git submodule update --init --depth 1 charon` fixes it.
   (0.67.0 arrives as a prebuilt bundle, so nothing is built and the submodule never
   arises — this cost is new with a source pin, not new with `main`.)
2. The build needs `nightly-2026-04-01` with `llvm-tools, rustc-dev, rust-src, rustfmt`.
3. `main` pins **CBMC 6.10.0**; the bundle we already have is 6.8.0. CBMC publishes no
   generic Linux tarball for 6.10.0 (`cbmc-6.10.0-Linux.tar.gz` and the `ubuntu-20.04`
   asset both 404); the working route is the `ubuntu-24.04-cbmc-6.10.0-Linux.deb` release
   asset unpacked with `dpkg-deb -x`. Note that **nothing in `kani-driver` enforces the
   CBMC version** — the pairing is a convention, not a check — so a mismatched CBMC would
   have been accepted silently. We matched it anyway.
4. Build time after the submodule landed: **2m0s**. First attempt: failed in 0.7s.
5. Both toolchains fail to `rustfmt` the source they modify under `--concrete-playback
   inplace`, and both print a stack backtrace while doing it — 0.67.0 because `rustfmt`
   is not installed for `nightly-2025-11-21` on this machine, `main` with *"Unrecognized
   option: 'unstable-features'"*. Cosmetic in both cases: the outcome is a `WARNING` and
   the generated test is still written correctly. Worth knowing only because the
   backtrace looks alarming in a log.

## Fixtures

`tests/spike/kani-pin/{stub_playback,stub_on_contracted,boundary}`, each its own
`[workspace]` root per the `tests/spike/` convention, so none can join the product
workspace. `#[cfg_attr(kani, kani::requires(..))]` is written out literally where the
product would write `#[ply::requires(..)]` — that is exactly what `ply-attrs` expands to
(`crates/ply-attrs/src/lib.rs`), so the fixtures carry no dependency on the product.

`boundary/` is `tests/fixtures/boundarycontract` transcribed: the same `legacy_rate`
callee, the same `tiered_fee` caller carrying vetting 004's `tier_fee_cents` body, and a
`src/ply_generated.rs` written in the exact shape
`crates/ply-core/src/harness.rs::generate_proof_module` emits — `use super::*`, the
`ply_stub_*` function that `kani::any()`s a return and `kani::assume`s the declared
`ensures`, and the `#[kani::proof_for_contract]` + `#[kani::stub]` attribute pair.

Re-run everything with `./run.sh` (it copies each fixture to a scratch directory first,
because `--concrete-playback inplace` edits source and the two toolchains generate
different test names). Verified idempotent: two consecutive runs exit 0, produce the same
verdict for every row, and leave the committed fixtures untouched (`git status` clean) —
the only difference between the two logs is timing.

---

## Results

Flags on every run are Ply's own: `-Z function-contracts -Z unstable-options -Z
concrete-playback -Z stubbing --harness-timeout 300s --exact --harness <h>
--concrete-playback print|inplace` (`crates/ply-core/src/engines/kani.rs`).

| # | Question | A — 0.67.0 | B — main |
|---|---|---|---|
| 1a | Stubbed harness fails: is a witness printed? | **YES** — `n = 3`, stub return `10` | **YES** — identical |
| 1b | Does replaying that witness reproduce the failure? | **NO** — panics *"there were still these concrete values left over `[[10, 0, 0, 0]]`"* | **NO** — identical |
| 1c | Control: same stub, failure on the harness's own input | replay red **at the assertion** (`n reached 3`) | identical |
| 2 | `#[kani::stub]` over a **contracted** target compiles? | **NO** — `Failed to find contract closure __kani_recursion_check_rate` | **NO** — same error, same wording |
| 3a | Ply's boundary shape, clean proof | **SUCCESSFUL**, 85 checks, **94.61s** | **SUCCESSFUL**, 110 checks, **107.70s** |
| 3b | Same shape, real violation: witness? | **YES** — `47957057, 255, 8350`, 65.23s | **YES** — `39663841, 255, 9217`, 92.40s |
| 3c | Replay of that boundary witness | leftover-values panic | leftover-values panic |
| 3d | The witness as a hand-written `#[test]` over the real code | **passes (green)** — see below | same test, same result |
| 3e | MUTATION: stub tightened to `<= 150`, violation must vanish | **SUCCESSFUL**, 52.12s | **SUCCESSFUL**, 73.65s |
| 3f | VACUITY: `kani::assume` deleted, clean proof re-run | **still SUCCESSFUL**, 86.44s | **still SUCCESSFUL**, 107.10s |
| 3g | Stub loosened to `<= 20_000`, clean proof | NOT RUN | **SUCCESSFUL**, 112.81s |

Times are CBMC's own reported `Verification Time` from one end-to-end `./run.sh`
(exit 0, every row above reproduced in it); wall clock ran 0.5–1.0s above each. Where an
earlier ad-hoc run measured the same row it agreed within CBMC's usual variance — 3a A at
96.18s and 96.67s, 3a B at 107.24s, 3b A at 67.92s, 3b B at 91.84s, 3e A 52.82s / B
73.76s, 3f A 84.98s. Row 3g was run once, ad hoc, on B only.

### 1 — stub + `--concrete-playback`: the blocker is real, but it is not the one recorded

`docs/kani-docs-sweep.md` §2 quotes the 0.67.0 book, under a heading claiming to list
*all* the limitations of stubbing: *"this feature isn't compatible with concrete
playback"*, and TODO.md turns that into "a stubbed failure yields no witness". **On the
pinned toolchain, on our own shapes, that is not what happens.** A stubbed harness that
fails prints a full concrete-playback unit test, with the stub's invented return value
recorded alongside the harness inputs. Ply's `extract_witness_bytes` keys on the same
`let concrete_vals: Vec<Vec<u8>> = vec![` marker that run prints, so nothing in the
product blocks it either. The 0.67.0 book sentence is stale about its own release.

What *is* broken, identically on both toolchains, is the replay. Kani's own generated doc
comment says so in as many words, and the code that emits it is
`kani-driver/src/concrete_playback/test_generator.rs` on `main`:

> The original harness has stubs which are not applied to this test.
> This may cause a mismatch of non-deterministic values if the stub creates any
> non-deterministic value.

Run against the fixture whose failure exists *only* because of the stub, the replay is
red — but for the wrong reason:

```
panicked at library/kani/src/concrete_playback.rs:32:9:
At the end of concrete playback, there were still these concrete values left over
`[[10, 0, 0, 0]]`. This either happened because: 1) Your code/harness changed after you
generated this concrete playback unit test. 2) There's a bug in Kani.
```

The assertion under test **passed**; the test failed because the stub's recorded value
had no consumer once the real `rate()` ran instead. The control harness — same stub, but
the failure driven by the harness's own input — replays red at the assertion itself
(`n reached 3`), which is what a reproduction looks like. So a naive "the generated test
is red, therefore the witness is real" check would be satisfied by a test that reproduces
nothing. That is a *worse* failure mode than the documented one, because it is silent.

**`main` changes none of this.** Same warning text, same leftover-values panic, same
green assertion underneath it. (The panic's file path differs — `/home/runner/work/kani/
kani/library/...` from the released build, `library/kani/src/...` from the local
checkout — which is the only visible difference between the two runs.)

`main`'s stubbing page claims the opposite —

> **Concrete playback:** Stubbing is compatible with `--concrete-playback`. When a
> stubbed harness fails, Kani can generate a concrete test case that reproduces the
> failure using the stub's behavior.

— and its own source and our runs contradict the second sentence. This is the same
docs-lead-the-driver pattern the sweep's standing rule already covers: measurement wins.

### 2 — `#[kani::stub]` over a contracted target: NOT fixed on `main`

```
error: Failed to find contract closure `__kani_recursion_check_rate` in function
       `kanipin_stub_on_contracted::rate`
```

Same error on both toolchains — that block is `main`'s wording; 0.67.0 spells the
function `rate` rather than `kanipin_stub_on_contracted::rate`, and nothing else differs.
It arrives in 0.2s (A) and 0.2s (B) as a **compile** failure, so the whole crate is dead,
not just the harness. Kani #4591 is still open and still bites. `main`'s
stubbing page documents stubbing's interaction with contracts only in the directions that
work (`stub_verified`, and plain `#[kani::stub]` *alongside* `-Z function-contracts`);
stubbing a target that *carries* a contract is not mentioned, and does not work.

This matters to Ply because vetting 004's `tier_fee_cents` calls a contracted callee
(`fee_cents`) as well as an unclaimed one. §5.5's rule reaches contracted callees the
moment a caller sits above two boundaries at once.

### 3 — the acceptance question: Ply's real boundary shape, end to end

**It verifies, on both.** The clean proof of `tiered_fee` — contracted caller, callee
replaced by its ply.yaml-declared contract — is SUCCESSFUL at 0.67.0 (94.6s, 85 checks)
and SUCCESSFUL on `main` (107.7s, 110 checks). `main` reports 25 more checks and takes
~12–14% longer on the same source across the two run pairs measured; whether that is the
extra checks or CBMC 6.10.0 was not separated.

**A violation in that configuration does yield a witness, on both.** `tiered_fee_halfclaim`
claims `result <= amount_cents / 2` — true of the real body (90 bps is 0.9%) but not
supported by the declared contract, which permits a full-rate callee. Kani reports the
contract failure and prints a three-value counterexample.

**The witness is not usable as a reproduction, on either.** Three observed steps:

1. Kani's own generated playback test panics on leftover values (3c), as in §1.
2. The witness's third value *is* the stub's invented rate (9217 bps). It has nowhere to
   go in a test of real code — the real `legacy_rate` returns 90.
3. Written out by hand at the two real inputs, D7-style, the test is **green**:
   `boundary/src/lib.rs::witness_replay` asserts `tiered_fee_halfclaim(39_663_841, 255)
   == 356_974` and that `356_974 <= 19_831_920`. It passes under plain `cargo test --lib`.

This is not a Kani defect and no version bump can fix it. **A violation that exists only
under an assumed contract has no reproduction in the real code, by construction** — the
code is not wrong; the declared boundary contract is wider than the body. §8's rule ("MUST
NOT emit a `violation` without a witness") is satisfiable here in the letter (Kani prints
values) but the D7 artifact those values are meant to become cannot be red. Worth raising
against §5.5/§8 as a design question; this spike does not change the spec.

**The mutation test (3e) is what proves the stub is really applied and really
load-bearing.** Tighten the declared `ensures` from `<= 10_000` to `<= 150` — the real
body's own range — and the identical violation disappears on both toolchains. Only the
assumed contract changed, and the verdict followed it.

**A vacuity finding about `boundarycontract` itself, not about Kani (3f/3g).** Delete the
stub's `kani::assume` entirely, so the callee returns an unconstrained `u32`, and the
*clean* `tiered_fee` proof still verifies — 86.4s at 0.67.0, 107.1s on `main`. Loosening
the bound to `<= 20_000` likewise verifies on `main` (112.81s). The reason is in the fixture's own
body: `legacy_rate(tier).min(10_000)` clamps whatever the callee returns, so `tiered_fee`
is correct for *any* callee and its proof never leans on the declared contract. The
`conditional` verdict is still the right one — Ply assumed a contract, and saying so is
honest — but `boundarycontract`'s clean harness does not, on its own, demonstrate that
the assumption is doing any work. `tiered_fee_halfclaim` (added here) is the harness that
does.

### Smaller observations

- **`main` prints which stub it applied**: `- Stub: legacy_rate -> ply_stub_legacy_rate`,
  on the "Checking harness" line. 0.67.0 prints nothing. A Ply adapter could use that to
  confirm the engine actually honoured the stub it asked for, instead of trusting it.
- Both toolchains accepted `-Z stubbing` **without** `--harness` on the two-harness
  fixture (0.67.0's book says `--harness` is required). Weak evidence: both harnesses
  there carry the *same* stub, so this does not test the "different stub configurations
  per harness" case `main` claims to have fixed. Ply always passes `--harness`, so nothing
  turns on it.

---

## What was NOT RUN

- **Any run of the Ply product itself** (`cargo ply verify`) against either toolchain.
  This spike drove `cargo kani` directly. The claim that Ply's parser would accept the
  stubbed witness is read off `extract_witness_bytes` in
  `crates/ply-core/src/engines/kani.rs` and the observed output containing its marker
  string — it is not an end-to-end product run.
- `vetting/004-legacy-extension/run.sh` against the candidate.
- **`#[kani::stub_verified]` on either toolchain.** This spike is about plain
  `#[kani::stub]`, which is what `generate_proof_module` emits. The docs sweep's finding 3
  (no soundness backstop) was not re-tested and no claim here bears on it.
- `main`'s other advertised stubbing features: `kani::stub_set!` /
  `#[kani::use_stub_set]`, `extern "C"` stubbing, trait-method and `dyn` stubbing.
- Mixed pairings (0.67.0 with CBMC 6.10.0, or `main` with 6.8.0), so the ~12–14%
  slowdown at 3a is not attributed between Kani and CBMC.
- macOS/aarch64. Everything here is Linux x86_64, where the pinned findings in
  `tests/spike/FINDINGS.md` were taken on macOS aarch64 — timings are not comparable
  across those two, only within this file.
- Kani's `autoharness`, the machine-readable output formats, and every other `main`
  capability the docs sweep lists. Out of scope for this question.

## Measured cost of a bump

| Cost | Measured |
|---|---|
| Build the candidate from source | 2m0s, plus a submodule clone and a `rustc-dev` nightly install |
| Extra moving parts a bump adds | a git submodule, a second Rust nightly, a CBMC version with no tarball release, and a version string (`0.67.0`) that does not identify the commit |
| Verification cost on Ply's boundary shape | **107.7s vs 94.6s** — `main` ~14% slower (~12% on the earlier pair), well inside §6's 300s stubbed floor and inside the run-to-run CBMC variance `docs/m3-slice-findings.md` already measured (~1s–107s on one identical harness) |
| Capability gained | one output line naming the applied stub |
| Blockers removed | **none of the two** |

---

## Recommendation: **stay put.**

Do not move the pin, and do not fork. The reasoning, in the order it was measured:

1. **There is nothing to bump *to*.** 0.67.0 is still the newest release. Moving means
   pinning a commit on an unreleased branch that reports itself as `0.67.0` — which is
   worse for D14 fingerprints than a fork would be honest about, because two different
   engines would stamp the same string.
2. **Blocker 2 is not fixed.** #4591 reproduces verbatim on today's `main`. The single
   clearest reason to bump does not survive contact.
3. **Blocker 1, as written, was never true**, and the real problem underneath it is not a
   version problem. Stubbed witnesses print at 0.67.0 today; stubbed *replays* are broken
   on both, by design (`test_generator.rs` chooses to warn rather than apply stubs), and
   for the boundary case they are unfixable in principle because a contract-gap violation
   has no counterexample in the real code.
4. **Ply's real shape already works at the pin** — verifies clean, and produces a witness
   on violation — at ~12–14% *lower* cost than the candidate.

Two things follow that cost almost nothing and are worth doing at the pin. The first is
done in this commit:

- **Corrected in place:** TODO.md's "a stubbed failure yields no witness",
  `docs/kani-docs-sweep.md` §2 item 2 and §3(iv), and
  `docs/plans/trusted-boundary.md`'s fail-side gate — all four said or assumed something
  this run falsified. What replaces them is what was measured: the witness is produced
  and carries the fabricated callee return; the *generated playback test* does not apply
  the stub. That is a different constraint on D7, and it is a live one.
- **File #4591 upstream traffic, not a fork.** §1 says we build glue, never solvers, and
  the measured cost of leaving the pin — a submodule, a second nightly, a CBMC with no
  generic release tarball, and an ambiguous version string — buys nothing today. Revisit when a release
  after 0.67.0 exists, and re-run `./run.sh` against it: this file is the baseline.

### One gate this discharges

`docs/plans/trusted-boundary.md` sequences its **fail-side** reporting behind this spike:
a `given:` region havocs the callee, and a caller that fails under havoc should be
reported as an absence whose diagnostic names *the breaking return value*. That depended
on "witness recovery through a stub", which the plan recorded as impossible at the pin.
It is not: the witness is produced and its third value **is** the fabricated callee
return — `stubbed rate = 8350` at the pin, `9217` on the candidate (3b). **The gate is open at the current pin**, and the
plan and its cost estimate have been corrected in this commit. The plan's choice of an
*absence* over a `violation` also survives — it is strengthened by 3d, since the witness
cannot be turned into a red test of the real code.

The one design question this spike hands back, which no Kani version answers: **§5.5 can
produce a violation that no test of the real code can reproduce.** §8 forbids a
witness-free `violation`; here the witness exists but is not replayable, and the D7
artifact built from it is green. That is a spec conversation, not an engine upgrade.
