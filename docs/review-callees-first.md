# Adversarial review — D5's first branch (5671ab5 + 2da68f1), 2026-08-26

Scope: the callees-before-callers composition feature as landed in `5671ab5` and the
warm-reuse fix in `2da68f1`, reviewed against The-Ply-Spec.md (§1, §5.2a, §5.4c, §5.5),
CLAUDE.md, both full diffs, and the current source. Method: full read of the diffs and
the surrounding subsystems they feed (`record.rs`, `reach.rs`, `audit.rs`,
`worklist.rs`, the mutate decoration path); then seven live attack fixtures run through
the real `cargo-ply verify` binary with Kani 0.65-era toolchain installed, one of them
also run against the parent commit's binary built from `5671ab5^` for a
before/after comparison. Every defect below marked **reproduced** was demonstrated
end-to-end in this session, not inferred.

**Bottom line first**: the scheduler's core held up under attack — cycles, a
`conditional` callee, a fuzz-only callee, a timed-out callee, warm reuse, and the
deferred-fingerprint machinery all behaved exactly as §5.5 says, including two shapes
no shipped test covers. But the feature ships **one reproducible false clean verdict**
(a caller reported `bounded(2)`, exit 0, whose contract is violated by *every* real
input — the parent commit reported an honest `timeout` and failed the run), one
silent contradiction of the spec's central "always stubbed, never inlined" closure
claim, and a widened tamper net that accepts a hand-edited record the old check
refused — reproduced with the repo's own `stubverifiedminbound` fixture. This does
not merge as-is. It also does not need redoing: every fix below is local, and the
ordering/fingerprint architecture the two commits build is the right one.

---

## D1 — SEVERE, reproduced: min-composition is unsound when the callee's proof
## domain does not cover the caller's arguments

**Claimed** (§5.5): "if `f` is declared at `bounded(5)` but stands on a `g` proved
only to `bounded(2)`, the honest composed verdict is `bounded(2)`" — min of the two
bounds, presented as *the* honest composition.

**Actual**: the bound caps the *report*, not the *assumption*. `g`'s own
`bounded(k)` proof established its contract only over its harness's input domain —
for a `Vec<u8>` parameter, vectors of length ≤ k (`harness.rs:917`,
`kani::vec::any_vec::<u8, {k}>`). `#[kani::stub_verified(g)]` in `f`'s proof then
assumes that contract for whatever argument `f` actually passes, including one
outside that domain, where the contract was never established and may be false.
`crates/ply-cli/src/verify.rs:2562-2566` computes `composed_k` as a fold of
`u32::min` over the stood-on bounds and nothing anywhere inspects what arguments
cross the call.

**Reproduction** (fixture preserved in this session's scratchpad as `vecmin2`):

```rust
#[ply::ensures(|result| *result <= 10)]
pub fn g(v: Vec<u8>) -> u32 {
    if v.len() > 2 { 99 } else { 0 }     // honours its contract only to len 2
}

#[ply::ensures(|result| *result <= 10)]
pub fn f(x: u32) -> u32 {
    let mut w = Vec::new();
    w.push(x as u8); w.push(0); w.push(0);
    g(w)                                  // always length 3
}
```

with both claimed `bounded(2)`. Result on these commits: `g — bounded(2)` (true),
`f — bounded(2)`, **no statuses, W0517 info only, exit 0**. The real `f` returns 99
for *every* input — `assert!(f(x) <= 10)` fails at `x = 0` — so the clean verdict is
false throughout its own reported bound, not merely beyond it. On the parent commit
(`5671ab5^`, same fixture): `f — timeout` (Kani inlining `g`'s real body), `K0601`,
run fails. The feature converted an honest absence into false evidence — the one
failure §1 exists to prevent, in the first commit whose soundness §5.5's own limits
admit "rests entirely on Ply's scheduler, never on Kani".

**Why min looked right and is not.** For a callee whose every parameter gets a
full-domain `kani::any()` (all scalars — every one of the six shipped fixtures),
the callee's contract is proved over the *entire* argument space and its `k` never
constrained anything; composition is sound there and min is actually an
*under*claim (`f` could honestly keep its own declared bound). For a callee with a
`Vec` parameter, `k` is a genuine domain restriction and **no arithmetic on the two
bounds makes the composition sound**, because the unsoundness is about argument
containment, not depth. Min is a plausible rule that is simultaneously too strong
where it is sound and no defense where it is not — and the fixture suite is
scalar-only, so it cannot see this.

**Fix that matches the design**: branch one qualifies only when every parameter of
the callee is one the bounded codegen covers at full domain (today: not `Vec`) —
a `Vec`-parameter callee falls back to branch two (`conditional`, the contract named
as assumed), exactly as a fuzz-only callee does, until a real argument-containment
argument exists. State the domain-coverage condition in §5.5's limits either way.
Keep min for the report if you like it as a conservatism, but the spec should stop
presenting it as what makes the composition honest — D1 is the proof it is not the
load-bearing part.

## D2 — SEVERE, reproduced: a same-crate contracted callee the stub builder cannot
## handle is silently inlined, contradicting the closure claim added in this commit

**Claimed** (§5.5, honesty condition 2, lines 1063-1065, added by `5671ab5`): "a
same-crate `g` is now always stubbed, never inlined, whichever branch reached it, so
whatever `g` itself calls never travels into the caller's proof at all."

**Actual**: `boundary_plan`'s `CalleeStatus::Contracted` arm
(`crates/ply-cli/src/verify.rs:1211-1237`) adds the callee to the contracted list
only when the lookup returns `Found` **and** `found.local` **and**
`found.unnameable.is_none()` **and** `build_contract_fn` succeeds. Every failing
case except the deliberate cross-crate one falls through with no `else`: no stub, no
refusal, no diagnostic — Kani inlines the contracted callee's real body and
everything below it. `build_contract_fn` (`crates/ply-core/src/harness.rs`) fails on
a `self` parameter, a non-identifier parameter pattern, or a contract attribute it
cannot parse.

**Reproduction** (`tuparg`): `g((a, b): (u32, u32))` carrying
`#[ply::ensures(|result| *result <= 10)]`, calling an entirely unclaimed `deep`;
`f` claimed `bounded(2)` calls `g((x, x))`. Result: `f — bounded(2)`, **zero
diagnostics, zero statuses, no stub of any kind in the generated harness**, exit 0.
`g`'s body and the unclaimed `deep` both traveled into `f`'s proof — the exact
"proves the caller against a body nobody claimed" outcome the third branch's
honesty condition says cannot happen any more, on the honest side of the verdict
this time, but silent and spec-contradicting. The parallel path for a
`ply.yaml`-declared contract does refuse (`unstubbable` → `unclaimed`); this new
path has no equivalent.

**Fix**: the fallthrough cases (unnameable, unbuildable) must land in a refusal
list like `unstubbable` does — verdict `unclaimed`, a diagnostic naming the callee
and the shape that blocked the stub — or the spec sentence must be retracted to name
the exception. Given the sentence was added in this same commit, refusing is the
option that keeps it true.

## D3 — reproduced: the widened earnability check accepts a hand-edited overclaim
## the old check refused, and the exact honest value was computable at refusal time

**Claimed** (§5.2a): `W0516` catches "the honest version" of record tampering — "a
stored verdict must be one the checks recorded beside it could actually have
earned"; the D5 exception lets `bounded(k)` earn any `bounded(j)`, `j <= k`
(`crates/ply-core/src/record.rs:496-498`).

**Actual**: the widening is broader than the fact it encodes. **Reproduction**: run
the repo's own `stubverifiedminbound` fixture (declared `bounded(5)`, genuinely
composed to `bounded(2)`), then hand-edit `ply.lock`'s verdict for `f` from
`bounded(2)` to `bounded(4)`. Second run: `f — bounded(4) [reused]`, exit 0, **no
W0516** — while the carried-forward W0517 in the same envelope still says "`f`
earned bounded(2) ... capped at the weakest of the two". The envelope contradicts
itself, and the stale-bound overclaim that commit `5671ab5` fixed (defect 2 of the
review it records) is resurrected through the exact channel `W0516` was built to
close. Before the widening this edit was refused as impossible.

The hole did not have to open. At lookup time the claim's *finalised* fingerprint
has already matched, which pins the recorded `verified_bounds` to the current ones —
so the only honestly earnable value is exactly
`min(declared k, min over verified_bounds)`, computable on the spot. Passing
`verified_bounds` into the earnability check and demanding equality (per bounded
check in the list) restores the full tamper net; "any `j <= k`" was the easy
superset, not the true statement. Lowering (`bounded(1)`) is likewise accepted
today and is at least an honest-direction lie, but the same equality closes it too.

Related, and widened by this feature rather than introduced: `verdict_is_earnable`
never looks at **statuses**, so hand-deleting `"conditional"` from a callee's entry
was always possible — but before branch one it only mislabeled that callee's own
node, and now it upgrades every *caller* (the reuse-path gate at
`verify.rs:838-845` reads `entry.statuses` to decide whether the callee counts as
clean). A hand-edit on `g`'s entry can now launder debt out of `f`'s verdict. Worth
a line in §5.2a's tamper paragraph even if unfixed.

## D4 — reproduced: the new inline-contract assumption class is invisible to
## `audit` and `worklist`, breaking §5.5's own enforcement loop

Branch two reached through a same-crate *inline*-contracted callee (a cycle, a
fuzz-only callee, an unclean callee) is a new kind of `conditional`/`owed-evidence`
verdict this feature mints: the assumed contract lives in source attributes, not in
`ply.yaml`. Both trust-listing commands collect assumptions solely from
`CalleeStatus::Assumed` — the `ply.yaml` route (`crates/ply-cli/src/shared.rs:355`,
consumed by `audit.rs:380` and `worklist.rs:347`).

**Reproduction** (`privmod`): `f` claimed `bounded(2)` calling an unclaimed
`inner::g` that carries an inline contract. `verify`: `f — bounded(2)
[conditional, owed-evidence]`, W0511. Then `cargo ply audit`: **`trust_surface:
[]`**; `cargo ply worklist`: **"0 waiting on evidence"**. §5.5's honesty condition 3
— "trust that is never checked is green paint ... `cargo ply audit` lists it" — is
the stated reason `conditional` is allowed to exit 0, and for this class the
listing side of that bargain does not exist. This is precisely the pattern hunted:
the feature's output feeding a subsystem built before it existed. Fix in
`shared.rs` (teach `assumed_contracts` the `Contracted` status) or state the
exclusion in §5.5; the former keeps condition 3 true.

## D5 — reproduced: the promise-content gate fires on branch one's *proved* callee,
## against the spec's explicit exclusion, with prose that misattributes the contract

**Claimed** (§5.5, line 1181): the emptiness gate "does not look at a verified
function's own inline `#[ply::ensures]`, where a weak spec is the `mutate` tier's
question (`W0502`) rather than this one's."

**Actual**: promise probes are planned for every stub regardless of `StubKind`
(`promise.rs` receives the full stub list), so a branch-one callee — proved this
run, owed nothing — still gets the E0502/E0503 treatment on its inline contract.
**Reproduction** (`taut`): `g` proved `bounded(2)` with the trivially-true
`#[ply::ensures(|result| *result >= 0)]` on a `u32`; `f` stands on it cleanly
(W0517, no `conditional`) and the run **fails, exit 1**, with an error-severity
E0503 reading "The promise **declared in ply.yaml** for `g` says nothing" — there
is no ply.yaml promise; the clause is `g`'s inline contract, which `g`'s own proof
just passed. Same misattribution in W0511's text for any inline-contract assumption
("the contract declared in ply.yaml for each callee", `verify.rs:1610`, observed on
the `privmod` and cycle-gate runs). Under CLAUDE.md's newbie bar these sentences
are reviewed like code, and both now state a falsehood about where the words a user
must go fix actually live. Either exempt `StubKind::Contracted` from the gate per
the spec, or amend the spec and fix the prose to name the inline attribute — but
the current state contradicts both the spec and the wording rule at once.

## D6 — latent, code-cited: a callee that strengthens its evidence with `mutate`
## silently drops out of branch one

`apply_mutate_outcome` decorates a fully-passing claim's verdict in place:
`verdict.push_str("·spec-strong")` (`verify.rs:3501`), so a callee claimed
`[bounded(2), fuzz(64), mutate]` that kills every mutant records the verdict string
`bounded(2)·spec-strong`. Both places that admit a callee into `known_bounded`
parse the verdict with `parse_bound` (`verify.rs:1664`), whose
`strip_suffix(')')` fails on the decorated string — fresh path `verify.rs:862-868`
and reuse path `verify.rs:838-845` alike. Result: the callee vanishes from branch
one and every caller silently downgrades to `conditional`/`owed-evidence` — because
the callee's evidence got *stronger*. Conservative direction, but undocumented,
silent, and §5.5's branch-one condition ("passed its own Kani contract proof this
run") is plainly met. I attempted the end-to-end run; the natural fixture kept one
genuinely equivalent mutant alive (`weak-spec`, no decoration), so this is
demonstrated at the string level, not the envelope level — the two string shapes
are both real in shipped code paths, and their meet is unhandled. Strip the
decoration before parsing (the record's own `verdict_is_earnable` already does
exactly that at `record.rs:477`).

## D7 — spec honesty: branch one's insulation does not reach the record, and the
## composition's remaining safety rides on that accident undeclared

§5.2a input 3 says the fingerprint covers "the code the check actually runs or
descends into". Under branch one the check never runs or descends into `g` — yet
the reach walk still queues a `Contracted` callee like any other
(`crates/ply-core/src/reach.rs:369`) and hashes its whole token stream into the
caller. Two consequences, opposite signs:

- Every edit to `g`'s *implementation* re-proves every caller at full engine cost,
  though their proofs read only `g`'s contract and bound. The ~70s-per-run waste
  that `2da68f1`'s message celebrates ending is still paid on every callee edit;
  the spec sentence describing input 3 is now false for this branch; and D5's
  "insulated from the callee's body" story silently does not apply to reuse.
- That same over-hash is currently the **only** thing that invalidates a caller
  when `g`'s contract text changes, or when the body of a helper function named
  inside `g`'s contract changes (`verified_bounds` carries only path + bound;
  contract text is hashed nowhere else for this branch — I traced both through
  `mentioned_paths`' contract-attribute parsing at `reach.rs:435-452`). Anyone who
  later "fixes" the first bullet by stopping the walk at stood-on callees, without
  first adding the contract text and its helpers as declared fingerprint inputs,
  reopens the exact class of defect 2 for contract meaning instead of bound. This
  load-bearing accident should be written down — in §5.2a, and ideally as a
  comment on the `Contracted` fall-through in `reach.rs` — before someone
  optimizes it away.

---

## The scheduler attacks that did NOT land

Run live unless marked otherwise: a two-function **cycle** falls back to branch two
on both sides, no W0517, no hang (shipped test, confirmed). A **fuzz-only callee**
never qualifies (shipped test). A **timed-out callee** never enters
`known_bounded`, and because the caller's fingerprint is finalised only after
composition, its stored branch-one entry cannot match either — it honestly re-runs
as conditional (traced; the deferred-lookup design gets this right for free). A
**`conditional` callee** is refused as a foundation: the clean-gate fixture
(`condgate`: `g` conditional on a `ply.yaml` promise for legacy `h`, `f` calling
`g`) came back with `f` conditional and no W0517 — the launder-prevention gate
works, though **no shipped test covers it** (see tests, below). A **record from a
different configuration** cannot match (engine, toolchain, Ply version are
fingerprint inputs). **Diamond and transitive** shapes compose correctly: a
callee's recorded verdict is already its *composed* bound, so the min propagates
through chains, and a stale intermediate invalidates its callers transitively
through `verified_bounds`. The **mixed `bounded`+`fuzz` list** behaves; its cost
gap (harness built before the deferred reuse decision) is honestly recorded as a
KNOWN GAP in TODO.md. `topological_order` is deterministic and correctly
domain-restricted.

## Are the six tests load-bearing?

Each one fails if the specific behaviour it names regresses — none is decorative,
and `stubverifiedwarmreuse` is a genuinely good test (it pins reuse, the absence of
W0516, *and* that no proof module is written, in one run). Three coverage
judgments against the repo's own standard:

- **All six are scalar-only**, which is why D1 is invisible to them *by
  construction*: for scalar claims the bound never restricts the proof's domain, so
  min-composition cannot be caught overclaiming there. The suite pins the label
  arithmetic, not the soundness of the composition. A `Vec`-parameter fixture in
  the D1 shape is the missing red test.
- **The clean-gate has no test** (`condgate` above): the "never merely
  bounded-shaped" rule — the one §5.5 calls out as preventing debt-laundering — is
  enforced at two `statuses` checks in `verify.rs` and nothing in the suite goes
  red if either is deleted.
- No invariant-style test in the house style: e.g. *every fn node whose verdict's
  bound is shallower than its declared `bounded(k)` carries a W0517 naming a
  stood-on callee with exactly that bound, and no node with a W0517 carries
  `conditional`*. That walk would have been the cheap tripwire for D3's
  self-contradicting envelope and D6's silent downgrade alike.

## Is the spec honest?

§5.5's new opening, the limits subsection, and §5.2a's twelfth input match the
build in every place I could execute — with the exceptions already filed: the
"always stubbed, never inlined" closure claim is false (D2), the promise-gate
exclusion for verified functions' inline ensures is false (D5), input 3's "code the
check actually runs" is false for stood-on callees (D7), and the limits list omits
the domain-coverage condition that D1 shows is the actual soundness boundary of
branch one. One small addition: §5.5 says branch one's ordering runs over "claimed
functions" — the implementation orders only bounded-eligible claims, which is the
right call and worth one clarifying word. The two KNOWN GAP entries in TODO.md
(mixed checks list; multi-hop conditional composition) are accurate and honestly
scoped.

## Verdict

**Merge blocked on D1 and D2; D3 strongly recommended before merge; the rest can
follow.** D1 is a reproducible false clean verdict inside the reported bound — the
project's defining failure — and its minimal fix (exclude `Vec`-parameter callees
from branch one until domain containment is argued) is a few lines plus a spec
sentence. D2 is a silent contradiction of a claim this same commit added to the
spec, fixed by refusing where the code now falls through. D3 reopens W0516's
tamper net where the exact honest value was already in hand. None of this
indicts the architecture: ordering, the deferred fingerprint, and the clean gate
survived every other attack I could construct, including two nobody had tested.
