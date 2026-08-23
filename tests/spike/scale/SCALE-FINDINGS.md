# Scale spike — findings

Follow-up to `tests/spike/FINDINGS.md` (M0), answering ADR-0003's "open risk that gates
M3": what collection-shaped code can Kani 0.67.0 (CBMC 6.8.0, same pinned toolchain)
actually verify? M0 proved the *mechanisms* on scalars and small structs; this spike
sweeps *scale* — `Vec`, arrays, `Option`/`Result`, `BTreeSet`/`HashMap`, and a recursive
tree matching the verdict kernel's own shape (`tools/kernel/src/lib.rs`).

Fixture: `tests/spike/scale/fixture`, its own `[workspace]` root. Re-run with `./run.sh`
(writes `results.csv`). Every `cargo kani` call is capped with `--harness-timeout`
(default 60s; two calls explicitly override it to answer item 7's flag question, both
still capped). This file is a first-hand run, not a re-derivation of the M0 spike or of
the kernel's own `kani_proofs` module doc comment — every number below was observed in
this session; a few lower-priority rows were not run and are marked `NOT RUN` rather than
guessed (see the note on scope at the end).

## The single most important number

**Reproducing the kernel's own shape — recursion through a `Vec<Self>` field, at the
smallest possible non-trivial size (3 nodes, one level of real recursion) — still times
out at 60s, and *still times out at 180s* even with the specific fix (`#[kani::unwind]`)
that fixes every simpler case in this sweep.** Symbolic execution alone took ~104s and
produced 64,147 verification conditions for a 3-node tree. This is not "give it more
time" territory — it is the same open risk ADR-0003 named, now measured instead of
suspected, and it means `bounded` cannot currently reach Ply's own kernel shape at all,
regardless of flags.

## Results table

Every row is one `cargo kani ... --exact --harness <name>` invocation. "Wall" is the
outer process time (includes ~1-4s of Kani/CBMC startup overhead); "Symex" is CBMC's own
reported symbolic-execution time where it printed one. `TIMEOUT@Ns` means CBMC's own
`CBMC timed out` message at that harness-timeout — **never** a genuine contract or
assertion failure; where a real assertion failure occurred it is written out in full.
Kani's plain terminal output makes both look like `VERIFICATION:- FAILED`, and
distinguishing them requires reading past that line to the reason underneath — this table
already did that reading so the reader doesn't have to.

### Item 1 — `Vec<u8>`, N = 1, 2, 4, 8, 16

| Case | N | Wall | Verdict |
|---|---|---|---|
| `.iter().map().sum()` (iterator chain) | 1, 2, 4 | ~122s | **TIMEOUT@120s** — confound, see below |
| `.iter().map().sum()` | 8, 16 + all contract variants | — | NOT RUN cleanly (build-lock artifact mid-sweep; superseded) |
| manual loop `for i in 0..v.len()` | 1, 2, 4, 8 | 60-63s | **TIMEOUT@60s** |
| manual loop, contract (`ensures`) | 1 | 61s | **TIMEOUT@60s** |
| construction only, no loop, no use | 0 | 2s | VERIFIED (252 checks) |
| construction only | 1 | 1s | VERIFIED (258 checks) |
| loop `0..1` **constant** + guard (not `0..v.len()`) | 1 | 1s | VERIFIED (303 checks) |
| manual loop **with `#[kani::unwind(N+1)]`** | 1 | 1s | VERIFIED (304 checks) |
| ″ | 4 | 2s | VERIFIED (304 checks) |
| ″ | 8 | 1s | VERIFIED (304 checks) |
| ″ | 16 | 1s | VERIFIED (304 checks) |

**Finding.** The first run (`.iter().map().sum()`, N=1) looked like a contract failure
in the CSV; the raw CBMC output showed `Unwinding loop ... iteration 1150` before timing
out — CBMC was symbolically unwinding the *generic* `Iterator::fold`/`Map::map_fold`
trait-dispatch machinery, unrelated to the real (tiny) length. That is a confound of
*this specific idiom*, not of `Vec` — corrected by switching to a manual indexed loop for
the rest of the sweep (also the kernel's own idiom: `aggregate_raw` never chains iterator
combinators).

The manual loop is not free of the same problem, though: `for i in 0..v.len()` still
timed out at N=1, and reproducibly so at N=2,4,8. The loop bound `v.len()` is itself
*symbolic* (any value 0..=N, chosen by `any_vec`/`BoundedArbitrary`), and Kani's default
unwind-bound inference cannot tell it is small — it unwinds generically until its own
internal cap, then reports "CBMC timed out". Two isolating experiments nail the cause
down precisely:

1. Replacing `0..v.len()` with the *compile-time-constant* `0..1` (plus a runtime guard)
   verifies in 1s — so it is specifically the symbolic loop bound, not indexing a `Vec`
   or constructing one, that is expensive.
2. Adding an explicit `#[kani::unwind(N+1)]` to the *unmodified* `0..v.len()` harness
   fixes it completely — VERIFIED in 1-2s at every N tried, 1 through 16.

**This is the load-bearing mechanism for the whole spike: Kani's default unwind
inference does not bound a loop whose iteration count is a symbolic (bounded) `Vec`
length, but an explicit `#[kani::unwind]` sized to the declared bound does, cheaply.**

### Item 2 — `[u8; N]`, N = 1, 2, 4, 8, 16 (manual loop, no annotation needed)

| N | Wall | Verdict |
|---|---|---|
| 1 | 1s | VERIFIED (32 checks) |
| 2 | 0s | VERIFIED (32 checks) |
| 4 | 1s | VERIFIED (32 checks) |
| 8 | 0s | VERIFIED (32 checks) |
| 16 | 1s | VERIFIED (32 checks) |
| 4, contract (`ensures`) | 4 | 3s | VERIFIED (79 checks) |
| 1, 2, 8, 16, contract | — | NOT RUN individually (pattern is uniform across the 5 plain cases + one contract case actually run; no reason to expect divergence, but not empirically confirmed) |

**Finding.** Fixed arrays need no unwind annotation at all — the loop bound `0..N` is a
compile-time constant, so CBMC never has to guess. This is the direct, decision-relevant
contrast with item 1: same computation, same element type, only the length-boundedness
differs, and it is the difference between "verifies in under a second, unannotated" and
"never verifies without an explicit flag".

### Item 3 — `Option<u32>` / `Result<u32, u8>`

| Case | Wall | Verdict |
|---|---|---|
| `Option<u32>` | 1s | VERIFIED (1 check) |
| `Result<u32, u8>` | 0s | VERIFIED (1 check) |

Trivially cheap, as expected for an enum over scalars.

### Item 4 — struct with a `Vec` field vs. struct with a fixed-array field

| Case | Wall | Verdict |
|---|---|---|
| `struct { data: [u8; 4], tag: u8 }` | 1s | VERIFIED (45 checks) |
| `struct { data: [u8; 16], tag: u8 }` | 1s | VERIFIED (45 checks) |
| `struct { #[bounded] data: Vec<u8>, tag: u8 }`, N=4 (`#[derive(kani::BoundedArbitrary)]`) | 61s | **TIMEOUT@60s** |
| ″, N=16 | — | NOT RUN (N=4 already timed out; no reason to expect N=16 differs) |

**Finding.** Wrapping the same shapes in a struct changes nothing: item 1/2's pattern
holds exactly. Also worth recording precisely: `#[derive(kani::Arbitrary)]` does **not
compile** for a struct with a `Vec` field at all (`Vec<T>` has no plain `Arbitrary` impl
in this Kani version) — the field must be marked `#[bounded]` and the struct must derive
`kani::BoundedArbitrary` instead, which is a real API difference generated code would
need to know about, not just a slowness difference.

### Item 5 — `BTreeSet<u8>` / `HashMap<u8,u8>`, N = 0, 1, 2

| Case | N | Wall | Verdict |
|---|---|---|---|
| `BTreeSet<u8>` via `any_vec::<u8,N>().collect()` | 0 | 64s | **TIMEOUT@60s** |
| ″ | 1, 2 | — | NOT RUN (N=0 already timed out) |
| `BTreeSet<u8>` via a **constant**-bound insert loop (`for _ in 0..N { if any() { insert } }`) | 0 | 3s | VERIFIED (1408 checks) |
| ″ | 2 | 62s | **TIMEOUT@60s** |
| `HashMap<u8,u8,BuildHasherDefault<DefaultHasher>>` via `kani::bounded_any` (Kani's own impl) | 0 | 0.3s | VERIFIED (2167 checks) |
| ″ | 1, 2 | — | NOT RUN (N=0 already establishes the contrast with BTreeSet; time did not permit the rest) |
| `HashMap<u8,u8>` (default hasher, `RandomState`) | any | — | **does not compile** — no `Arbitrary`/`BoundedArbitrary` impl exists for `RandomState` anywhere in this Kani version (`RandomState` is seeded from OS randomness, not deterministically constructible) |

**Finding, in three parts.**

1. **`BTreeSet`/`BTreeMap` have no `BoundedArbitrary` impl at all** in Kani 0.67.0's
   library (checked directly: `library/kani/src/bounded_arbitrary.rs` implements it only
   for `Box<[T]>`, `Vec<T>`, `String`, `HashMap`, `HashSet`). Building one by hand via
   `any_vec().collect()` inherits item 1's "symbolic loop bound" cost inside `.collect()`
   — confirmed by rebuilding it the way Kani's own `HashMap`/`HashSet` impl does instead
   (a **constant**-bound `for _ in 0..N` loop with a per-element coin flip): that version
   verifies in 3s at N=0, where the `any_vec`-based version times out at the same N=0.

2. **But `BTreeSet` carries a second, independent cost beyond the loop-bound issue**:
   even the constant-bound-loop version times out once N=2 — `BTreeSet::insert`'s own
   generic B-tree algorithm becomes expensive once a real second element is possible,
   with no symbolic-loop-bound trick to blame this time. This independently reproduces
   the exact diagnosis in `tools/kernel/src/lib.rs`'s `StatusSet` doc comment (CBMC
   unwinds `BTreeSet`'s generic insert/clone algorithm regardless of bound) — now
   demonstrated in isolation, at N as small as 2, outside the kernel's own code.

3. **`HashMap`'s default hasher is not merely slower — it is categorically unsupported.**
   `RandomState` has no `Arbitrary` impl because it cannot be deterministically
   constructed at all; this is a compile error Ply's codegen must route around
   (`BuildHasherDefault<DefaultHasher>` is the swap Kani's own library ships support
   for), not a timeout to budget for.

### Item 6 — a recursive tree (the kernel's actual shape)

Shim mirrors `tools/kernel/src/lib.rs`'s own `kani_proofs::SymTree`/`SymLeaf` hand-rolled
symbolic-tree pattern (bool flags choosing whether each child slot is present), at three
depths, `depth1` being the kernel's own already-reported-unprovable shape (root + ≤2
children, ≤3 nodes total).

| Case | Depth | Wall | Verdict |
|---|---|---|---|
| `count_nodes` (trivial walk, `BTreeSet`-tagged nodes, `Vec<Self>` children) | 1 | 63s | **TIMEOUT@60s** |
| ″ | 2, 3 | — | NOT RUN (depth1 already timed out) |
| `collect_tags_btreeset` (the kernel's exact clone+extend pattern) | 1, 2, 3 | — | NOT RUN (the simpler `count_nodes` at the same depth already timed out, so this strictly-more-expensive case was not separately spent on) |
| `collect_tags_bitmask` (the kernel's actual shipped fix: `u8` bitmask, not `BTreeSet`) | 1 | 62s | **TIMEOUT@60s** |
| ″ | 2, 3 | — | NOT RUN |
| `count_nodes_fixed`, children in `[Option<Box<Self>>; 2]` **not** `Vec<Self>`, depth 0 (type is recursive but never actually recurses) | 0 | 1s | VERIFIED (130 checks) |
| ″, depth 1 (one real level of `Box<Self>` recursion, 3 nodes, no unwind annotation) | 1 | 60s | **TIMEOUT@60s** — CBMC's own trace: `Unwinding recursion count_nodes_fixed iteration 307...308` before giving up, i.e. it treats the recursive *call* as an unbounded unwind target with no relation to the real depth |
| ″ **with `#[kani::unwind(3)]`** | 1 | 6.3s | VERIFIED (143 checks) |
| `collect_tags_bitmask` depth1 **with `#[kani::unwind(4)]`** (recursion + `Vec<Self>` children, both bounded now) | 1 | 60s, then 180s | **TIMEOUT at both caps.** CBMC's trace confirms the unwind bound worked — recursion/loop iteration counts dropped from 300+ to 4-5 — but symbolic execution alone still took ~104s and produced 931,188 program-expression steps / 64,147 verification conditions (32,219 after simplification) for a 3-node tree, and 180s still was not enough. |

**Finding, the headline of the spike.** Depth alone is not the story — a `FixedNode`
whose recursive type never actually resolves (depth 0) is trivial (1s); the *very first*
level of real recursion (depth 1, 3 nodes) already times out, with or without `BTreeSet`,
with or without `Vec`-shaped children, purely from `Box<Self>` recursion with no explicit
unwind bound. An explicit `#[kani::unwind(N)]` fixes *that* completely (6.3s). But once
the harness combines heap-allocated recursion (`Box`, whose `Drop` glue is itself
recursive and needs its own unwinding) with a `Vec<Self>` children field (symbolic
length) and several independent per-node symbolic choices (three tag bits plus two
presence flags per node in this shim), the same fix that rescues each ingredient alone
does not rescue the combination — the state space itself, not the unwind bound, is what
is now too large. **This is exactly the kernel's own shape**, and it is exactly where
`tools/kernel/src/lib.rs`'s own `#[cfg(kani)]` proofs currently stand: none of them
terminate, with no unwind bound, solver, or object-bits setting tried changing that.

### Item 7 — flag variance

| Flag | Case | Result |
|---|---|---|
| `#[kani::unwind(N)]` (code-level) | item 1's `0..v.len()` loop, N=1,4,8,16 | **fixes it completely** — TIMEOUT→VERIFIED at every N tried |
| `#[kani::unwind(3)]` | item 6's pure-recursion case (`FixedNode`, depth1) | **fixes it completely** — TIMEOUT→VERIFIED (6.3s) |
| `#[kani::unwind(4)]` | item 6's combined recursion+`Vec` case (`BitmaskNode`, depth1) | does not fix it — unwinding itself now terminates correctly (4-5 iterations, confirmed in the trace) but the resulting SAT problem is still too large within 60s, or 180s |
| `--solver kissat` (CLI, no code change) | item 6's pure-recursion case, **without** an unwind annotation | still `CBMC timed out` — solver choice is irrelevant when the unwinding itself never terminates; it only matters once the unwind bound is already fixed and the bottleneck has shifted to SAT solving |
| `--harness-timeout 180s` vs. 60s | item 6's combined case | does not resolve it (Symex alone was ~104s and it still failed); this is a genuine size problem, not a "just needs a bit more time" one |
| `--object-bits`, `--cbmc-args` variants | — | NOT RUN — item 7's flag budget went to the two decisive questions above (does an explicit unwind bound fix the Vec case, and does it fix the kernel's actual combined shape); this is the one line item from the original brief not exercised |

## What §5.4b may honestly claim

The current text says v1 supports, recursively: integers, `bool`, `char`, structs/enums
with derivable `Arbitrary`, "`Option`/`Result`/`Vec` of supported types", and `&T`/`&[T]`.
Evidence-backed replacement:

- **Integers, `bool`, `char`, `Option<T>`, `Result<T,E>` of supported types** — confirmed
  cheap (item 3), unconditionally supported, no changes needed.
- **Fixed-size arrays `[T; N]`** — confirmed cheap at every N tried up to 16, with **no**
  unwind annotation required, because the loop/copy bound is a compile-time constant.
  This should be v1's *preferred* bounded-collection shape, not merely "an alternative".
- **`Vec<T>` (and any type built by iterating a length-bounded `Vec`, e.g. `.collect()`
  into a `BTreeSet`)** — supported **only if** Ply's own harness codegen emits an
  explicit `#[kani::unwind(N+1)]` (or an equivalent CLI `--unwind`) sized to the declared
  bound. Without it, `Vec` is **not** supported in any practical sense — it times out at
  every N tried, including N=1. §5.4b currently promises `Vec` support with no mention of
  this; that promise is not honest without the annotation requirement attached to it.
- **`BTreeSet`/`BTreeMap`** — **not supported**, full stop, beyond a single element. Even
  with the unwind-bound fix applied, `BTreeSet::insert`'s own generic algorithm becomes
  intractable at 2 elements. §5.4b does not currently mention these types by name; it
  should explicitly exclude them (or gate them behind a much smaller declared bound and a
  loud warning) rather than let them fall under an implied "any supported-element
  collection" umbrella.
- **`HashMap`/`HashSet` with the default hasher** — **not supported**; not a timeout, a
  compile error (`RandomState` has no `Arbitrary`). With an explicit
  `BuildHasherDefault<DefaultHasher>` swap, small bounds (confirmed only at N=0 in this
  session) are cheap. §5.4b should say plainly that Ply's codegen must perform this
  hasher swap itself for any `HashMap`/`HashSet`-typed parameter — a user cannot be
  expected to know this.
- **Recursive/self-referential types** (`Vec<Self>` or `Box<Self>`-shaped fields,
  i.e. any tree or linked structure) — **not supported in v1**, even at the smallest
  non-trivial size (one level of real recursion), *even with the unwind-bound fix
  applied*, once combined with more than one symbolic dimension. This needs to be an
  explicit, named exclusion in §5.4b, not something a user discovers by timing out. It is
  also the exact shape of Ply's own verdict tree, which is what makes this the spike's
  headline finding rather than a footnote.

## What this means for M3

1. **`bounded` must never be routed to Kani for a recursive/tree-shaped signature.**
   `VerdictNode`-shaped code — Ply's own kernel included — cannot currently get a
   `bounded` verdict from Kani at all, regardless of flags tried. M3 must either refuse
   this shape up front with a clear `unsupported`/`V0505`-style diagnostic (naming the
   recursive type, per §5.4b's existing pattern for unsupported types), or defer it to a
   future engine. Silently attempting it and reporting whatever Kani says risks a
   multi-minute hang per `ply check` invocation for a construct that will never resolve.

2. **Ply's harness codegen must always emit an explicit unwind bound for any
   `Vec`/collection-typed parameter, sized to the check's declared `bounded(k)`.**
   Relying on Kani's default unwind inference is not merely suboptimal — it makes `Vec`
   support non-functional even at the smallest possible bound. This is a concrete,
   mechanical codegen requirement, not a tuning suggestion: `#[kani::unwind(k+1)]` (or the
   `--unwind`/`--default-unwind` CLI equivalent scoped correctly) on every generated
   harness that takes a bounded `Vec`/`BoundedArbitrary`-derived input.

3. **Prefer fixed-size arrays over `Vec` in any codegen path that has the choice.** Where
   a check's shape allows it (e.g. a user-supplied generator, or a `check_with`
   instantiation that fixes a length), array-shaped harnesses need no unwind annotation
   at all and verify in about a second regardless of size tried (1 through 16) — a
   strictly better default than `Vec` plus an unwind annotation, not just an equally-good
   alternative.

4. **`BTreeSet`/`BTreeMap` should not be offered as `bounded`-checkable types at all** in
   v1, independent of the unwind-annotation fix — cap `bounded`'s declared-size limit
   for these types at 1, or exclude them and point users at `fuzz`/`test` instead, which
   don't share this failure mode.

5. **`HashMap`/`HashSet` need a codegen-level hasher substitution**, not a user-facing
   requirement — generating `BuildHasherDefault<DefaultHasher>` in place of whatever
   hasher a `HashMap`/`HashSet` parameter was declared with (Ply already rewrites
   parameter types into harness-local `Arbitrary`-generating code, so this is one more
   substitution in the same codegen pass, not new machinery).

6. **`bounded(k)`'s existing `1 ≤ k ≤ 64` range (§5.1a) is not itself the safety net** —
   this spike found genuine timeouts at k as small as 1 (item 1's `Vec`) and k=2 (item
   5's `BTreeSet`). The signature *shape*, not just the declared bound, has to gate
   whether `bounded` is offered at all.

## Scope note: what this session did not get to

Marked `NOT RUN` throughout rather than guessed, per the brief's own instruction that a
partial evidence table beats a fabricated one. In priority order, most to least likely to
change a conclusion above if run:

- `collect_tags_btreeset`/`collect_tags_bitmask` at depth 2 and 3 (expected: still
  TIMEOUT, since depth 1 already exceeds the cap for both, but not confirmed)
- `--object-bits` and alternate `--cbmc-args` sweeps on the hardest combined case (item 7
  asked for this explicitly; time went to the two more decisive unwind-bound questions
  instead)
- `HashMap` at N=1, N=2 (only N=0 was run; the BTreeSet-vs-HashMap contrast is already
  established at N=0, but the point where HashMap itself starts costing something, if
  any, is unmeasured)
- Array and Vec-with-unwind sweeps at every N individually for every variant (a
  representative subset was run per case; the pattern was uniform in every case actually
  checked)

`run.sh` re-runs everything that *was* run, in the same order, so re-running it (and
extending it for the items above) is the natural way to close these gaps.
