# Adversarial review — result reuse, 2026-08-25

Scope: commits `107a491` through `5b9b0b7` plus `eeb1b27` on
`claude/project-concept-eval-6soxfl`, reviewed against The-Ply-Spec.md (§5.2a, §5.5,
D14, D9), `docs/result-reuse.md`, `docs/SCHEMA.md`, `crates/ply-core/src/record.rs`,
and the `verify` orchestration in `crates/ply-cli/src/verify.rs`. Method: full read of
the record module and every reuse-touching path in the CLI; the `ply-core` unit suites
re-run (record and fingerprint tests green); four live adversarial probes through the
real release `cargo-ply` binary, each shown with its cold-run control. The two
in-range trust-listing commits (`7a4b508`, `4290243`) were skimmed for scope only and
are NOT reviewed here. The user's own two spot-checks (warm reuse at 0.141s, version
bump forcing a full re-run) are consistent with everything observed below and were not
redone.

**Bottom line first.** The stated honesty rule — *a stored result never reaches a
user's eyes without being re-hashed first* — holds mechanically: the only way to read a
recorded entry is `Record::matching(node_id, fingerprint)`, `verify` is the only
reader, and no other command, output surface, or error path shows a stored result at
all. But the rule as stated is not the guarantee that matters. The guarantee that
matters is that the hash covers what the answer depended on, and it does not: **the
fingerprint omits the bodies of every function the check actually executes or descends
into**, and it omits the `examples:` a `test` check asserts. Three live reproductions
below each show the same shape as the six defects this branch fixed — a green verdict,
`[reused]`, in under 100ms, over code that a cold run proves is in violation. The
`[reused]` legend printed on every warm run ("everything it depended on — the code …
hashes the same today") is false as printed. This must be fixed or reuse must be
narrowed before this branch's claims are true as written; everything else found is
smaller.

Why the branch's own evidence missed it: in the fixture (`tests/fixtures/resultreuse`)
every claimed function is either a leaf (`safe_increment`, `widen`) or crosses only
into a callee covered by a declared promise (`total` → `legacy_rate`) — the one callee
shape the hash does cover. All five e2e tests and all five demonstrations in
`docs/result-reuse.md` run on that fixture. Not one test in the branch has a claim
whose function calls an uncovered local body. Thirty green tests over an invisible
image, again.

---

## Defects (wrong)

### D1 — CRITICAL: a callee's body is not in the caller's fingerprint, so editing code a check runs produces a false `[reused]` pass

**Claimed** (§5.2a: "The fingerprint covers everything the answer depended on";
`docs/result-reuse.md`: "leaving any of them out means reusing a result that is about
something else"; the `[reused]` legend on every warm run): any change to what a result
depended on re-earns it.

**Actually true**: `FingerprintInputs.fn_source` is the claimed function's own token
stream and nothing else. Callee information enters the hash only via
`FingerprintInputs.assumed`, which is populated from `BoundaryPlan.stubs` — i.e. only
callees whose contract is declared in `ply.yaml` (§5.5's second branch). Everything
else the check actually runs is invisible to the hash:

- **fuzz / test / mutate**: `boundary_plan` is not even computed (`verify.rs`: the
  boundary is `BoundaryPlan::default()` unless a `bounded` check is present). The
  generated harness calls the real function, which calls its real callees — local
  helpers, transitively, and every dependency in `Cargo.lock` — and none of that is
  hashed.
- **bounded**: a local callee with its own inline contract (`CalleeStatus::Contracted`)
  is descended into — its real body is part of the proof — and neither its body nor
  its contract is hashed. Calls that resolve out of the workspace
  (`CalleeStatus::Unresolved`: std, registry crates) are likewise descended into, and
  `Cargo.lock` is not an input, so `cargo update` invalidates nothing.

The old D14 this branch rewrote listed "callee contract hashes" as a fingerprint input;
the rewrite narrowed that to "the contracts **assumed** for the callees it crosses
into" and the implementation matches the narrowed text. The spec and the code agree
with each other (commit `16c41f5`'s claim is technically true) — and both are wrong
about the world.

**How checked**: two live probes, release binary, each with a cold-run control.

*Fuzz tier.* A crate with `widen(x) = double(x)`, `double(x) = x * 2`, claim
`fuzz(64)` on `widen` with `ensures |result| *result >= x`. First run: `fuzzed(64)`,
recorded. Then the only edit is `double`'s body, `x * 2` → `x / 2`:

```
    widen — fuzzed(64)  [reused]
  [reused]  this result was not re-run: … the code … hashes the same today
real  0m0.027s
```

Delete `ply.lock`, run again: `widen — violation`, P0502, proptest shrinks a real
counterexample. The warm run showed a pass over code the cold run proves broken.

*Proof tier.* `outer(x) = inner(x)`, both carrying inline contracts
(`ensures *result <= 200`), claim `bounded(2)` on `outer` only. First run: 66s of real
Kani work, `bounded(2)`, recorded. Then the only edit is `inner`'s body, `x * 2` →
`x * 2 + 500`:

```
    outer — bounded(2)  [reused]
real  0m0.070s
```

Delete `ply.lock`, run again: 67s, `outer — violation`, K0502. A proof — the strongest
thing Ply reports — was carried forward over a broken callee the proof descends into.

**Cost to fix**: real design work, not a field addition, because the honest input set
differs by tier.

- *bounded* is the tractable half: descent is already policed call-site by call-site,
  so the fingerprint can take the token streams of every `Contracted` callee reached
  transitively (stopping at stubbed callees), plus a `Cargo.lock` hash whenever any
  descent leaves the workspace. Per-claim granularity (demonstration 3) survives.
- *fuzz/test/mutate* run the real code, and per-fn reachability is syntactically
  undecidable here: the call-site collector deliberately ignores method calls
  (`callgraph.rs`, `collects_free_calls_and_ignores_method_calls`), so `x.helper()`
  is invisible. The honest fingerprint for these tiers is a hash of the crate's whole
  source plus `Cargo.lock` — coarser invalidation (any edit in the crate re-pays every
  sampled claim) but never a false reuse. An interim narrower rule — keep per-fn
  hashing only when the function's tokens contain no calls at all — is implementable
  in a day and sound.

Until one of these lands, reuse is unsound for essentially any non-leaf function, and
the feature should not merge as-is.

### D2 — MAJOR: `examples:` are not in the fingerprint, so an edited example is never re-run

**Claimed**: same completeness claim as D1. The `test` check compiles each `examples:`
entry into an assertion (`generate_example_test`) and a failing one is a real,
reproduced violation (R0502).

**Actually true**: `FingerprintInputs` has no examples field; the per-input mutation
test in `record.rs` has no examples entry — the exact "field nobody remembered to add"
that test exists to catch, missed because the input list it walks is the struct's own.

**How checked**: live probe. Claim `checks: [test]`, `examples: ["widen(2) == 4"]` —
first run `tested`, recorded. Edit the example to `widen(2) == 5` (now false):

```
    widen — tested  [reused]
real  0m0.030s
```

Cold control: `violation`, R0502, "failed 1 of its own example/generated
direct-contract test(s)". Adding a new example is equally invisible: the claim keeps
`tested [reused]` without the new example ever running.

**Cost to fix**: an afternoon. Add the examples list to `FingerprintInputs` and its
mutation to the loop test, plus one red e2e. Unlike D1 this is a pure omission with no
design question in it.

### D3 — MINOR (fixed in this commit): `check.rs` module comment still says Ply does not write `ply.lock`

The user-facing "staleness NOT CHECKED" notice was correctly deleted (`7b96184`), but
the module comment above it still read "Staleness needs `ply.lock` (D14), which Ply
does not write" and "Two of those four tiers do not exist yet". Both false since
`107a491`. Comment-only; corrected in this review's commit.

### D4 — MINOR (fixed in this commit): the spec's crate-layout listing still names `accept`

§4's layout comment listed the cli binary's subcommands as
`check|verify|tree|worklist|audit|accept|synth|skill`. `docs/result-reuse.md` says
`accept` was removed "from the command list, the design decision that introduced them,
and the milestone" — true of §6's command list and D14, but this one survived.
One-word deletion; corrected in this review's commit.

---

## Overstatements (claimed stronger than true)

### O1 — "The fingerprint covers everything the answer depended on"

Stated in §5.2a, D14's rationale column, `docs/SCHEMA.md` ("a hash of **everything the
answer depended on**"), `docs/result-reuse.md`, and — worst — printed to the user in
the `[reused]` legend on every warm run. False until D1/D2 land (the legend's "the
code" is exactly the part that is not covered). The legend is exact-string-tested, so
rewording it is a reviewed change, not a drive-by; it should either become true (fix
D1/D2) or say what is actually hashed.

### O2 — "a hand-edited record is trusted exactly as hand-edited source is"

The disclosed gap 4 (spec §5.2a closing paragraph, `docs/result-reuse.md`). The
analogy is wrong in direction: hand-edited *source* is re-checked on the next run;
a hand-edited *verdict* beside an untouched fingerprint is the one artifact in the
repository that is believed forever with nothing ever re-deriving it. Live probe: in a
recorded `ply.lock`, change `"verdict": "fuzzed(64)"` to `"verdict": "proved"` and run:

```
    widen — proved  [reused]
```

— the strongest verdict Ply has, minted by a text editor, repeated on every future run
while the fingerprint still matches. Note the run does not even notice that the stored
verdict (`proved`) is not one its own stored `checks: [fuzz(64)]` could produce. Full
protection against a hostile editor is impossible without signing and is rightly out
of scope — but a cheap consistency check (the recorded verdict must be one the
recorded checks can earn; `fuzz` can never yield `proved`) would catch the honest
version of this mistake and costs a table lookup. The disclosure is doing work a
sentence of code could do.

### O3 — the compiler-probe fallback comment

`rustc_identity`'s comment says a probe failure "would only ever make a fingerprint
match less often". Two runs whose probes both fail hash the same `unknown`/`unknown`
regardless of the real toolchains. The safe direction still mostly holds (a record
written by a healthy probe can never match a broken one), and a broken `rustc -vV`
alongside a working cargo is exotic — but the comment claims a direction, and the
direction has an exception. Reasoned from code; NOT reproduced.

### O4 — "That is the CI run and the colleague's first checkout"

The 64ms fresh-clone demonstration is real and pinned by a test, but it holds only
when the clone's toolchain matches the recorded one to the exact `rustc -vV` line and
target triple. A colleague on macOS, or one rustc point-release ahead, re-pays
everything — and then *rewrites* `ply.lock`, dropping every entry their machine could
not reproduce (the ping-pong §5.2a itself discloses). The demonstration's framing
oversells the ordinary-team case; the mechanism is working as designed either way.

---

## Gaps (disclosed ones judged, new ones added)

**Disclosed (a) — fuzz engine version is the requirement, not the resolved version.**
Verified in code: `harness_crate::PROPTEST_REQUIREMENT = "1"` feeds both the generated
manifest and the fingerprint, with a comment linking the two. Disclosure adequate: the
risk window (a behavior-changing `1.x` proptest release) is real but bounded, the fix
(read the generated crate's resolved lockfile once it exists) is correctly described as
not free, and nothing in the disclosure stands in for a fix that is cheap. Fine as
recorded.

**Disclosed (b) — hand-editing.** Disclosure exists but its analogy understates the
exposure; see O2 for the direction error and the cheap partial hardening it skips.

**New — the callee's signature is not hashed.** `AssumedPromise` carries the callee's
path and contract text but not the `CalleeSignature` the stub is generated from. A
callee signature change that the caller's tokens absorb through inference (e.g. a
widened return type) changes the generated stub and the proof while fingerprinting
identically. Narrow window — most signature changes break the caller's tokens too.
Reasoned from code; NOT reproduced. One field to add while fixing D1.

**New — dependency identity (`Cargo.lock`) is not an input.** Subsumed by D1 for the
run-the-real-code tiers, but worth its own line because it survives a D1 fix scoped
to local callees: `bounded` descends into registry code (`Unresolved` licenses
descent, §5.5's stated verification gap) and `cargo update` invalidates nothing.

**New — build configuration beyond features is not an input.** `[profile]` settings
(overflow-checks flips fuzz outcomes for arithmetic contracts) and `RUSTFLAGS` are
unhashed. NOT CHECKED by execution; listed for the D1 design pass.

**New — invalidation is undiagnosable.** When a fingerprint mismatches, the run says
nothing about *which* input moved — the user sees a silent full-price re-run (the exact
experience the feature exists to end, now appearing without explanation whenever a
compiler or engine updates). Storing the clear-text inputs beside the hash would let
`verify` say "re-run because the compiler changed" for one comparison loop.

**Gap 3 (worklist/audit don't read the record) and gap 5 (whole-document prune)**:
verified as described — no command but `verify` touches `ply.lock` (grep over the CLI
crates), and `retain_claims` keeps exactly the reused-or-earned set. Both correctly
scoped as future work / inherent trade-offs.

---

## The committed record as an artifact

`ply.lock` in a diff shows verdict, statuses, diagnostics, `written_by`, and a hash. A
reviewer can see *that* a claim was checked and by which Ply; they cannot see when,
under which compiler, or by which engine version — those live inside the opaque
fingerprint (fuzz entries alone name their engine in the clear, in the evidence
block). Git history supplies "when"; nothing supplies the toolchain. Combined with D1,
a committed `ply.lock` can today contain entries whose fingerprints still match while
the code the checks ran has materially changed — so "this claim was checked, and here
is the fingerprint of what it was checked against" (§5.2a) is, until D1 is fixed, a
weaker statement than the diff-reading reviewer will take it for. After D1, storing
the clear-text fingerprint inputs per entry (they are small) would serve the audit
question and the diagnosability gap at once.

## What the deletion took with it

Nothing functional: `cargo ply accept` and `E0303` were never implemented (checked
against the pre-branch binary's sources — no such command), and `stale`/`W0302`
existed as a status name in one list plus a "NOT CHECKED" notice in `check`. The
replacement is strictly more honest than what the spec used to promise — a re-blessed
verdict is a human opinion with a timestamp, a re-hashed one is checked at every use —
**provided the hash is complete**, which is D1's condition. The one thing the old
design's *prose* offered that the new record does not is a human-readable "verified
under this toolchain" statement; see above for the cheap way to restore it. The
attestation carve-out (a person's word is never hashed, `audit` says so every run) is
correctly preserved and reworded.

## The version-bump question

Recommendation: **keep whole-version invalidation**, and if the upgrade cost must come
down, replace it with a key derived mechanically — never with a hand-maintained
"semantics version".

The honesty argument: the current scheme's entire value is that *nobody decides* which
release affected which result. A number a release manager bumps "when how results are
earned changes" reintroduces exactly that decision, on every release, forever — and a
single missed bump converts every user's committed record into false reuse, silently,
with no test in any user's repository able to notice. That is this feature's worst
failure class (see D1 for what it looks like). The conservative scheme's failure mode
is the opposite in kind: loud, bounded (one full re-verification per upgrade), and
one-shot — demonstrated in the write-up's own demo 5.

What it costs: hours of CI per upgrade on large codebases, paid even for a release
that only recolored a diagram. If that bites, the honest refinement is to fingerprint
the *tool's result-earning code* instead of its version string: at build time, hash
the sources of the crates that decide what a result means (harness and codegen,
engine adapters, the record module, the verify orchestration) and put that digest in
`FingerprintInputs` instead of `ply_version`. A rendering-only release then reuses
honestly with no human judgment per release; the one judgment — which crates are in
the earning set — is made once, in code, where review can see it. Do not build that
until someone actually pays the CI cost.

And in sequence: this question is second-order today. The fingerprint currently
misses inputs (D1, D2) far larger than any version-granularity refinement; settle
those first.

---

## NOT CHECKED

- The five e2e reuse tests and the no-build-output test were read, not re-run (each
  costs minutes of engine time; the user's own two spot-checks cover the same paths).
- The renderer/diagram path: `tools/render` consumes the §8 envelope, which only
  `verify` produces post-re-hash; not exercised against a reused run.
- Concurrent `verify` runs racing on one `ply.lock` (last save wins; no locking).
- The trust-listing commits in range (`7a4b508`, `4290243`, `b3c3cf8`): skimmed for
  scope only, not reviewed.
- O3's probe-failure collision and the callee-signature window: reasoned from code,
  not reproduced.

## Verdict

The mechanism is the right shape: one gated read path, absences never stored, the
record pruned to what the last run stood behind, failures loud, the terminal honest
about what was carried forward — and the unit-level engineering of `record.rs` is
careful and well-tested against the inputs it knows about. But the feature's single
load-bearing claim — the hash covers everything the answer depended on — is false, in
the way that matters most and that this project has now hit seven times: a user is
shown a green result over code nobody checked. D1 (with D2 alongside) must be fixed,
or reuse narrowed to claims it is sound for, before this branch merges. The spec, the
schema page, the write-up, and the printed legend all repeat the completeness claim
and must move in the same commit as the fix, per the repository's own rule.
