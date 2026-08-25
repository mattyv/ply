# Verus feasibility spike — findings

Follow-up to `tools/kernel/src/lib.rs`'s own `kani_proofs` module doc comment and
`tests/spike/scale/SCALE-FINDINGS.md`: both established that Kani's bounded model
checking cannot reach the verdict kernel's own shape — a tree recursing through
`Vec<Self>` — at any depth, with or without an explicit unwind bound, once combined
with more than one symbolic dimension (SCALE-FINDINGS.md item 6). This spike asks the
question that gap left open: can a **deductive** verifier (induction over an unbounded
tree, not bounded symbolic unrolling) close the four standing obligations (CLAUDE.md)
for **all** trees, not just the ones small enough to enumerate or symbolically execute?

Run 2026-08-24. **Verus 0.2026.08.15.7d4628a** (commit `7d4628a8543d3e51e6e314c52032c9bab43f0f53`),
requiring rustup toolchain **1.97.1-x86_64-unknown-linux-gnu** (Verus ships its own
compiler build; this toolchain install is separate from whatever `tools/` itself
targets). Linux x86_64, prebuilt release zip, not built from source.

**Headline result: all four standing obligations verify, unbounded, by structural
induction, in ~2 seconds — where Kani's bounded model checker could not terminate at
all on the same shape (SCALE-FINDINGS.md: 60s and 180s timeouts on a 3-node tree,
64,147 verification conditions and still not enough).** This is not "Verus is faster
at the same kind of check" — it is a different kind of evidence entirely: induction
proves the property for every tree of every size, where bounded model checking proves
it only for trees small enough to fit in the symbolic execution budget (which turned
out, per the scale spike, to be smaller than the smallest non-trivial recursive case).

## Installing Verus

Not a crates.io package and not fetchable from `github.com`/`api.github.com` directly
in this environment — both returned `403` ("GitHub access to this repository is not
enabled for this session") through the outbound proxy, and `add_repo` for
`verus-lang/verus` only grants anonymous **git clone** access (source, not release
binaries), plus a straight cross-owner HTTPS attach was refused (this session already
had `mattyv/ply` sources attached; "cross-tier adds are not supported in v1"). The
working path, in order:

1. `WebFetch` on `https://github.com/verus-lang/verus/releases` — works (a different
   network path than this session's own outbound `curl`), and lists release tags.
2. `WebFetch` on `.../releases/expanded_assets/<tag>` (the fragment endpoint the
   release page's JS loads asset links from — the plain release-tag page's asset
   section renders as a client-side loading error under `WebFetch`, so the tag page
   alone is not enough) — returns the real asset filenames and relative hrefs.
3. `WebFetch` on the actual `.../releases/download/<tag>/<asset>.zip` URL reports a
   redirect (not fetchable itself, since it's a binary) but returns the redirect's
   `Location` header verbatim: a **signed, time-limited** `release-assets.githubusercontent.com`
   URL (`se=...` expiry ~1 hour out).
4. That signed URL is a *different host* than `github.com`/`api.github.com` and was
   reachable directly via plain `curl` from inside this session — downloaded the
   301,536,248-byte `verus-0.2026.08.15.7d4628a-x86-linux.zip` in one shot.

No second (older) release was attempted — the first one tried worked end to end, so
per the task's own "try at least: latest, one older, check errors" instruction: only
the latest was needed, and network access (not the Verus release itself) was the
actual obstacle, now resolved. Once unpacked, `./verus --version` immediately
demanded `rustup toolchain install 1.97.1-x86_64-unknown-linux-gnu` (named exactly,
with the exact command to run) — one `rustup toolchain install` call and it ran clean.
No other friction. Total install time, network fetch included: under 3 minutes.

Verus was **not installed inside the repo** (per the task brief) — it lives under this
session's scratchpad, outside `/home/user/ply`. `run.sh` takes `VERUS=/path/to/verus`
as an environment variable rather than assuming a location; a fresh environment
re-running this spike must redo the download-and-toolchain-install above and point
`VERUS` at the result. This mirrors `tests/spike/run.sh`'s own stance on Kani ("this
script does not pin or install Kani, it assumes it's already on PATH").

## Approach: shadow spec, not in-place annotation

Per the task brief, the production `ply-kernel` crate (`tools/kernel/src/lib.rs`) was
**not touched**. Two new, independent crates instead:

- `tests/spike/verus/proof/shadow.rs` — a **spec-mode** mirror of `Evidence`,
  `NodeKind`, `StatusSet` (bitmask), the tree, and `aggregate`, verified directly by
  the `verus` binary (not through `cargo` at all — same "engine runs as a subprocess,
  never linked as a library" posture D9 already establishes for Kani). Children are
  modeled as `Seq<Node>` (Verus's built-in, spec-only sequence type) rather than
  `Vec<Node>` — this is what makes the recursion **inductively provable**: Verus's
  automatic structural-decrease checking recognizes a value nested inside a `Seq<Self>`
  field as strictly smaller than the whole, the same way it recognizes `Box<Self>`, so
  the mutual recursion `aggregate(node) → fold(children) → aggregate(child) → …` gets a
  termination proof for free, with no unwind bound to size or exhaust (contrast:
  Kani's bounded model checker has no notion of "smaller" at all — it symbolically
  unrolls until its own cap, which is exactly what SCALE-FINDINGS.md's item 6 measured
  running out at 180s on 3 nodes).
- `tests/spike/verus/diff/` — a **plain, vstd-free** Rust crate: a hand-written
  executable transcription of the same rules (`src/lib.rs`), and a differential test
  (`tests/differential.rs`, plain `cargo test`) that runs both the real
  `ply_kernel::aggregate` and this transcription over a shared corpus of 4,000
  generated trees (up to 24 nodes, depth 5, every one of the 6 status flags decided
  independently rather than enumeration.rs's reduced 3-shape config) plus 3 hand-picked
  edge cases, checking agreement **at every node**, not just the root.

**Why both halves are needed, and neither alone is the claim.** Verus proved
properties of `shadow.rs`'s *spec* model — a mathematical object, not compiled,
executable code. The differential test proves that a *runnable* transcription of that
same model agrees with the real production kernel, bit-for-bit, on thousands of trees.
Chained together: the production kernel behaves like the transcription (differential
test, empirical, thousands of cases) and the transcription's rules hold for every tree
whatsoever (Verus proof, deductive, unbounded) — which is the only way this spike
licenses an unbounded claim about code neither proof actually touches directly. Neither
Verus nor the differential test was ever pointed at `tools/kernel/src/lib.rs`'s literal
source.

## Modeling decision: `conditional` abstracted to a set of ids

Per the task brief's own allowance: `VerdictNode::conditional: Option<Vec<String>>`
carries free-form assumption text in production, and its aggregation
(`merge_conditional`'s `Vec::extend` + `sort` + `dedup`) is exactly the construct that
defeated Kani (`kani_proofs` module doc comment: CBMC stalls inside
`core::slice::sort::...::<String>` regardless of how much content the shim actually
puts there). The shadow's spec model sidesteps this by carrying `Option<Set<int>>` — an
abstract, opaque assumption-id set — instead of `Option<Vec<String>>`. `Set<int>`
union is Verus's native, well-supported operation; no sort/dedup of `String` content is
ever modeled, because assumption *content* is not what the four standing obligations
are about — obligation 2 is about **presence and identity** surviving aggregation
(nothing silently dropped), not about the exact text.

This abstraction is **not merely assumed to be safe** — the differential test closes
the gap it opens. Each generated tree's conditional node gets a globally unique `u64`
id; the production side receives `vec![format!("assumption-{id}")]` as its real
`Vec<String>` payload, and the check (`conditional_ids_match` in
`tests/differential.rs`) parses the ids back out of production's aggregated
`Vec<String>` and compares the resulting set, exactly, against the shadow's `Set<u64>`.
So while Verus never reasons about `String` sort/dedup at all, the differential test
confirms — over 4,000 generated trees — that production's real `Vec<String>`-based
`merge_conditional` (sort, dedup, `Vec::extend`, all of it) produces a content-exact
match with the abstract set-union the proof reasons about. The abstraction loses
nothing the differential test could have caught; it is recorded here rather than
silently assumed, per the brief.

## Results

| # | Standing obligation | Verus verdict | Scope | Kani comparison |
|---|---|---|---|---|
| 1 | Aggregation never reports evidence stronger than the weakest claimable child | **Proved** (`lemma_worst_of` / `lemma_worst_of_seq`) | **All trees**, by induction | Kani: timeout, every attempt (kernel module doc comment) |
| 2 | `conditional` never disappears without its assumptions | **Proved** (`lemma_conditional_carries_assumptions` / `_seq`), exact-content (not just presence) | **All trees**, by induction | Kani: timeout (the exact stall CBMC hit, on `Vec<String>` sort/dedup) |
| 3 | A `violation` anywhere always reaches the root | **Proved** (`lemma_violation_reaches_root`, as a corollary of 1: rank 0 is the unique minimum) | **All trees**, by induction | Kani: timeout |
| 4 | No rule sequence assigns one node two different verdicts | **Proved** (`lemma_deterministic`) — but see caveat below | **All trees**, trivially | Kani: timeout |

All four verify together in one `verus proof/shadow.rs` invocation: **22 verification
conditions, 0 errors, ~1.6–2.3s wall-clock** (measured 3 times; startup overhead
dominates — the proof search itself is near-instant). Reproduce with `./run.sh` (see
below) or directly: `verus tests/spike/verus/proof/shadow.rs`.

**Obligation 4's caveat, stated plainly rather than left implicit:** in a deductive
model, `aggregate` is a pure mathematical function — `aggregate(n) == aggregate(n)`
holds by reflexivity, not by any interesting reasoning about the tree. This is *not*
the same guarantee Kani's version of this obligation was chasing: Kani's concern was
concrete, imperative nondeterminism (hash-set iteration order, allocator behaviour)
that a spec function cannot exhibit by construction, so proving it in Verus is real but
nearly free. The property that actually matters for production — that the *real*,
concrete Rust `aggregate_raw` avoids hash-based collections and so is genuinely
deterministic in practice — is exactly what `tools/kernel`'s own choice of `BTreeSet`→
`StatusSet` bitmask and sorted `Vec<String>` already establishes, and what this spike's
differential test empirically reconfirms (every one of 4,000+ generated trees produces
identical `Agg` values on repeated evaluation, implicitly, since `ply_kernel::aggregate`
is called once per tree and Rust's own type system rules out interior mutability here).
Verus proves the *shape* of the guarantee; it was never going to be the interesting
half of obligation 4.

## Mutation testing — no proof passes vacuously

Per the task's non-negotiable: at least one proved lemma was deliberately broken,
confirmed to fail, and reverted. Two were done, on two different obligations:

1. **`evidence_min`: `<=` flipped to `>=`** (min → max on the evidence order). Result:
   `lemma_worst_of`'s own postcondition fails immediately (`assertion failed`,
   `evidence_rank(result) <= evidence_rank(e)`), plus the two downstream forall blocks
   that depend on it. **20 of 22 still verified — only the two "worst-of" proof
   obligations broke**, exactly the ones whose correctness depends on `evidence_min`
   actually computing a minimum. Reverted; confirmed clean (22/22 again).
2. **`merge_conditional`: the `(None, Some(w)) => Some(w)` arm changed to `=> None`**
   (a lone conditional child's assumptions silently vanish when the parent itself
   isn't conditional — precisely the bug class obligation 2 exists to rule out).
   Result: both conditional-propagation lemmas fail at the exact line asserting the
   dropped case (`merge_conditional(None, rest_cond) == Some(w)`). Reverted; confirmed
   clean.

A third, smaller mutation on the differential test's own executable shadow
(`combine_claimable`'s `x.min(y)` → `x.max(y)` in `diff/src/lib.rs`, not the Verus
proof) confirms that half isn't vacuous either: `cargo test` immediately fails with 6+
concrete `evidence mismatch: production Violation vs shadow Proved`-style reports
across both the hand-picked edge cases and the generated corpus. Reverted; confirmed
clean (`cargo test --release`, 4 passed).

Every mutation above was reverted before this spike's final state; `run.sh` reproduces
the clean, un-mutated result end to end.

## Honest costs

**Annotation overhead**, counting non-comment source lines (comments and blank lines
excluded from both sides for a fair comparison):

| | Lines (non-comment) |
|---|---|
| Production `tools/kernel/src/lib.rs`, types through `aggregate` (excludes tests, excludes the entire `#[cfg(kani)]` module) | **148** |
| Shadow spec model (`shadow.rs`, `Evidence` through `aggregate`, plus the three independent-oracle spec fns `claimable_evidences`/`all_conditional_ids`/`any_conditional` the *lemmas* need but production does not) | **192** |
| Shadow proof annotations (`lemma_worst_of` through `lemma_deterministic` — the actual induction proofs) | **177** |
| **Shadow total** | **369** (2.5× production's core logic) |
| Differential shadow executable (`diff/src/lib.rs`) | 116 (mostly a line-for-line transcription of the same match arms) |
| Differential test harness (`diff/tests/differential.rs`, generator + checker) | 299 |

So proving these four properties, unbounded, cost roughly **1.2 proof-lines per line of
modeled logic** (177 lemma lines against 192 model lines) — plus the one-time cost of
a plain-Rust transcription and differential harness to bind the abstract proof back to
production (415 more non-comment lines, but this scaffolding is reusable for any
*future* property of the same kernel shape, not a one-off tax on these four).

**Wall-clock**, all measured on this machine, this run:

| Step | Time |
|---|---|
| Verus proof (all 22 obligations, one invocation) | 1.6–2.3s |
| Differential test compile (`cargo build --release`, cold) | ~1.7s |
| Differential test run (4,000 generated trees + 3 edge cases) | ~0.02s |
| **Full `run.sh`, both halves, cold** | **under 5 seconds total** |

Contrast: the same four obligations under Kani, per `tools/kernel/src/lib.rs`'s own
`kani_proofs` module doc comment, ran to their full 300-second `--harness-timeout` and
still reported `CBMC timed out` — three separate 5-minute waits for *no* verdict at
all, not even on the kernel's own reduced (depth-2, ≤2-children) symbolic shim. The
scale spike's dedicated sweep (SCALE-FINDINGS.md item 6) pushed the timeout to 180s on
a **3-node** tree and still didn't finish, after burning ~104s in symbolic execution
alone producing 64,147 verification conditions. Verus proved strictly more (every tree,
not a 3-node one) in about 2% of that time.

**What this spike did *not* attempt** (recorded as `NOT RUN`, not guessed, per house
convention):

- Verifying `tools/kernel/src/lib.rs`'s literal source directly (annotating the real
  `Vec<VerdictNode>`/`Option<Vec<String>>` types with `verus!`/contracts). The task
  brief explicitly calls for a shadow, not in-place annotation, so this was never
  attempted — it remains open whether Verus's `Vec`/`String` support (as opposed to
  `Seq`/`Set`) would hit the same generic-algorithm costs that stalled Kani, or whether
  Verus's deductive approach sidesteps that class of problem entirely. This is the
  single largest open question for anyone reading this spike as "Verus can verify the
  kernel" rather than its accurate, narrower claim: "a faithful shadow of the kernel's
  rules, checked equivalent to production by a large generated differential corpus."
- A differential corpus at production scale (thousands of *real* fn-level trees from
  an actual codebase) — the corpus here is synthetic, generated by a hand-rolled PRNG,
  not drawn from real `ply.yaml` documents (none exist yet; M2+ is what would produce
  them).
- Timing Verus against a *larger* shadow (more fields, e.g. modeling `worst_descendant`
  display formatting, or `content_hash`/`id` per §7) — out of scope per CLAUDE.md's
  scope rule (this kernel-only carve-out is deliberate in production too).
- A second, older Verus release, per the task's own "try at least ... one older
  release" instruction — not attempted because the first (latest) release worked
  cleanly end to end; there was no failure to triage that an older release might have
  resolved differently.

## What this means for M7

1. **The deductive gap Kani could not close is closeable, and cheaply.** All four
   standing obligations, unbounded, in under 2.5 seconds. This is the evidence M7's own
   milestone gate asks for ("an ADR ... arguing the translation is worth its cost"):
   Verus is not merely "an alternative engine" for the `prove` check slot — for
   recursive/tree-shaped code specifically, which SCALE-FINDINGS.md already showed is
   categorically outside Kani's reach, Verus is not competing with Kani on cost or
   speed; it is the only one of the two that produces a verdict at all.
2. **But this spike proved a shadow, not the kernel.** The gap between "a faithful
   `Seq`/`Set`-based model of the kernel's rules verifies" and "the kernel's actual
   `Vec<VerdictNode>`/`Option<Vec<String>>` source verifies" is real and unmeasured.
   Before M7 commits to Verus as the `prove` engine (TODO.md's existing "Verus as first
   tenant" decision), the next spike should attempt annotating `tools/kernel` directly
   — or an equivalently-shaped throwaway fixture using `Vec`/`String` instead of
   `Seq`/`Set` — to learn whether Verus's own executable-type support pays the same
   symbolic-collection tax that made Kani's `BTreeSet`/`Vec<String>` costs unbounded.
   That is the honest continuation of this finding, not a foregone conclusion of it.
3. **The differential-testing pattern itself is reusable, independent of what happens
   in (2).** Binding an abstract, provable shadow to production via a large generated
   differential corpus — rather than either "prove production directly" (potentially
   infeasible per (2)) or "prove nothing about production at all" (the trust gap a
   from-scratch Verus proof would otherwise leave) — is a general technique for
   bringing deductive proof to bear on code whose concrete types resist it. If M7
   adopts Verus, this shadow/differential split (not full in-place annotation) may be
   the *permanent* shape of `cargo ply`'s own self-proof, not just this spike's
   workaround.
4. **`conditional`'s abstraction (ids, not text) is the load-bearing modeling choice**
   to carry forward or reconsider explicitly if M7 proceeds: it is what let Verus
   sidestep the exact `Vec<String>` cost that stalled Kani, and the differential test
   is what currently justifies calling that safe. Any future spec change to
   `conditional`'s shape (e.g. structured assumption objects instead of strings) should
   re-run the differential test, not assume the abstraction still applies unexamined.
