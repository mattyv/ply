# Two honesty cleanups in the verdict kernel

2026-08-25. Both items had been open since before the product existed, both live in
`tools/kernel`, and both are about the same thing: not letting the kernel claim more
evidence than it has.

1. The three Kani harnesses that never terminated are **retired** — deleted, with the
   investigation kept as a narrowed historical note, and the crate now says plainly where
   the four standing obligations are actually proved today.
2. The enumeration's reduction argument — the one CLAUDE.md says must be written "or
   `exhaustive` is overclaiming by quotient" — is **written below**. One leg holds. One
   leg did **not** hold, was measured not holding, and has been repaired.

---

## 1. Retiring the Kani harnesses

### What was there

`tools/kernel/src/lib.rs` ended in a `#[cfg(kani)] mod kani_proofs`: three
`#[kani::proof]` harnesses covering the four standing obligations, driven by a hand-rolled
`SymTree`/`SymLeaf` shim (depth 2, ≤2 children), plus a long and genuinely good module doc
comment recording two rounds of investigation into why they never returned a verdict.

### What changed since that comment was written

- `tests/spike/verus/` **proved all four obligations, unbounded, by structural induction,
  in ~2 seconds**, mutation-tested, with a differential test binding the proved shadow to
  this crate.
- `tests/spike/scale/SCALE-FINDINGS.md` established that recursive shapes are outside
  Kani's measured reach *in principle*: a 3-node tree produced 64,147 verification
  conditions after ~104s of symbolic execution and did not finish in 180s, with the unwind
  bound demonstrably in effect.
- The-Ply-Spec.md §5.4b now names **recursive or self-referential types** an explicit v1
  exclusion, and Ply's rule for an excluded shape is to refuse it with a named status,
  never to route it to an engine that will time out.

### Deleted, not gated — and why

Gating them harder (a `#[cfg(kani_someday)]`, an `#[ignore]`-equivalent) was the other
option on the table. Deleting won on three grounds:

- **They contradict the tool's own published advice.** `VerdictNode` is `Vec<Self>` — the
  exact shape §5.4b excludes. A file that tells users "we refuse this shape rather than
  route it to Kani" while carrying three Kani harnesses for that shape is not a subtle
  inconsistency; it is the tool not taking its own advice, in the one place where CLAUDE.md
  says the standard is strictest ("Ply proposes, never rewrites, applies most strictly
  where we are our own user").
- **The role they filled is now filled better.** The doc comment defended keeping them as
  "an accurate statement of what a symbolic proof of the real `aggregate` would need to
  check." That statement now exists as `tests/spike/verus/proof/shadow.rs`'s lemmas — and
  unlike the harnesses, it verifies.
- **They cost real time for guaranteed timeouts.** `cargo kani` over the workspace burns
  three 5-minute waits to print `CBMC timed out` three times.

What is preserved, in `tools/kernel/src/lib.rs`'s crate doc comment (the historical note):
the `BTreeSet<StatusKind>` root cause and how it was bisected; the `StatusSet` bitmask fix
and that it stands on its own merits; the stall moving one field over to
`Option<Vec<String>>`'s `sort`/`dedup`; everything that was tried (`--unwind 2`/`3`,
`--object-bits 8` vs 16, `kissat` vs `cadical`, `--harness-timeout 300s`, Kani 0.67.0 /
CBMC 6.8.0, ~5:03 wall per harness); why the bitmask fix was deliberately *not* repeated
for `conditional`; and the scale spike's finding that it was never going to work anyway.
The `StatusSet` doc comment's cross-reference was repointed at that note rather than left
dangling. The harness bodies are in git history.

Also removed: the two `#[cfg_attr(kani, derive(kani::Arbitrary))]` derives, which existed
only to feed those harnesses, and `tools/kernel/Cargo.toml`'s `unexpected_cfgs` allowance
for `cfg(kani)`, which had nothing left to allow. The crate now has no `kani` mention
outside the historical note.

### Where the obligations are proved today — stated precisely

The crate doc now says this, and it is worth repeating here because the tempting shorthand
is wrong:

- **`tests/enumeration.rs`** — all four, against an independent oracle, at every node of
  every tree in a bounded corpus (991,389 trees, ~2.3s). Exhaustive within its bound; see
  part 2 for exactly what "within its bound" covers.
- **`tests/spike/verus/`** — all four, unbounded, by structural induction, ~2s. **Verus
  proved a faithful *shadow*** (children as `Seq<Node>`, assumptions as an abstract
  `Set<int>`), **not this crate's literal source.** What licenses connecting the two is a
  differential test running the real `ply_kernel::aggregate` against a plain-Rust
  transcription of the shadow over thousands of generated trees, compared at every node.
  The chain is "proved for the shadow, and the shadow matches production across a large
  generated corpus" — strictly weaker than "the kernel is proved", and
  `tests/spike/verus/FINDINGS.md` is explicit that closing that gap (annotating the real
  `Vec<VerdictNode>`/`Option<Vec<String>>` types) was never attempted and remains open.

---

## 2. The enumeration's reduction argument

### The claim being argued

`tools/kernel/tests/enumeration.rs` builds every verdict tree with 1–4 nodes, depth ≤3, ≤3
children per node, each node drawn from a 21-option config space, and checks `aggregate`
against an independent oracle at every node. 991,389 trees. That much really is
exhaustive: nothing sampled, nothing skipped.

But a node's *payload* was reduced. Until this session every flagged node carried the same
single status (`Stale`) and every conditional node carried the same single assumption
string. CLAUDE.md names the two things that reduction needs, and it names the penalty for
not writing them:

> the enumeration uses a reduced configuration set (one representative status, one fixed
> assumption string), so the argument for why that reduction loses nothing — per-bit
> uniformity of `StatusSet`, content-independence of the assumption merge — must be
> written alongside the claim, or "exhaustive" is overclaiming by quotient.

Below: leg 1 holds. Leg 2 does not. Both were checked by mutation rather than by reading
alone, because the whole point of the exercise is not to write a reassuring paragraph.

### Method: mutate the kernel, see whether the corpus notices

A reduction "loses nothing" precisely when no defect can hide in the part that was reduced
away. The direct test of that is to put defects there. Six one-line breakages of the real
`aggregate`/`StatusSet` code, each run against the enumeration alone:

| # | Mutation of `tools/kernel/src/lib.rs` | Old corpus | New corpus |
|---|---|---|---|
| M1 | drop `c.sort()` in `aggregate_raw` | **survived** | killed |
| M2 | `merge_conditional`'s two-`Some` arm discards the second list | **survived** | killed |
| M3 | upward status union keeps only the first non-empty set | **survived** | killed |
| M4 | drop `c.dedup()` in `aggregate_raw` | killed | killed |
| M5 | `StatusSet::len` clamped to 1 | **survived** | killed |
| M6 | `open_items` treats a flag and a conditional as mutually exclusive | **survived** | **survives** |

M1, M2, M3 and M5 surviving is the finding. Each is a real lie the kernel could tell —
assumptions in non-canonical order, half a subtree's assumptions silently dropped, a
status flag lost on its way up, an open-item count that under-reports — and the corpus that
calls itself exhaustive did not notice any of them.

### Leg 1 — per-bit uniformity of `StatusSet`: **holds**, with one gap that was closed

**The symmetry argument.** `StatusSet` is a `u8` with one bit per `StatusKind`. Every
operation is bitwise or per-bit: `insert` is `|= 1 << k`, `contains` is a mask test,
`union` is `|`, `len` is `count_ones`, `iter` is a declaration-order filter over set bits.
`aggregate_raw` reads a node's statuses in exactly three places — copies the node's own
set, `extend`s it with each child's aggregated set (bitwise OR), and takes `len()` for
`open_items`. Nothing in `aggregate` branches on *which* status: no `match` on `StatusKind`
exists anywhere in the kernel, and the evidence fold reads only `node.kind` while the
assumption merge reads only `node.conditional`.

Two consequences, and they are different:

- **Equivariance under relabeling.** For any permutation π of `StatusKind`, applying π to
  every node's flags applies π to `aggregate`'s output flags and leaves `evidence`,
  `conditional` and `open_items` untouched (`count_ones` is permutation-invariant). The
  oracle has the same property. So a counterexample using one status kind implies a
  counterexample using any other. **One representative kind genuinely stands for all
  seven** — this leg holds, and the crate's
  `every_status_in_the_glossary_round_trips_through_the_bitset` unit test is what
  guarantees its premise, that the seven kinds occupy seven distinct bits.
- **Separability across bits.** Because the fold is a per-bit OR, the output's bit *b*
  depends only on bit *b* of the inputs, so a multi-flag assignment decomposes into
  independent single-flag problems. This is also true — but it is an argument obtained by
  *reading the code*, not a property the corpus was checking. M3 and M5 are what that
  distinction costs: the old corpus never once unioned two *different* bits (every flag
  was `Stale`) and never once had a node carrying two flags (so `len()` was never called
  on anything but 0 or 1), and both mutants walked through it. The separability argument
  was correct about the code as written and useless as a guard on the code as changed.

Note also that the old oracle computed its status union with `StatusSet::extend` — the same
bitwise code the implementation uses. An oracle that reaches its answer by running the
implementation's own set algebra cannot disagree with it about that algebra.

**Closed by:** flagged nodes now draw their flag set from a period-2 cycle over pre-order
position — `{Stale}` at even positions, `{WeakSpec, Timeout}` at odd — so the corpus
contains distinct bits unioning across nodes, and nodes carrying two flags at once (which
is also the first time §7's own sentence, "a node carrying two statuses contributes 2", is
exercised at all). The oracle now accumulates into a `std::collections::BTreeSet`, so it no
longer computes the union with the code under test. M3 and M5 now die.

### Leg 2 — content-independence of the assumption merge: **does not hold**

This is the leg the brief warned would need a real argument, and it does not survive one.

**What the merge actually does.** `merge_conditional` concatenates two `Option<Vec<String>>`
(`None` is the identity; two `Some`s extend one into the other), and `aggregate_raw` then
runs `c.sort(); c.dedup();` on the result at every node. The observable aggregated value at
a node is therefore `Some(the sorted, deduplicated set of every assumption string in the
subtree)`, or `None` when no node in the subtree is conditional.

**Where content genuinely doesn't matter.** Two observables are content-blind, and for
these the reduction is fine:

- `evidence` and `statuses` never read `conditional` at all.
- `open_items` reads it as `if node.conditional.is_some() { 1 } else { 0 }` — presence
  only, not content and not even cardinality. A node with five assumptions contributes one
  open item.

**Where content decides the answer.** The aggregated `conditional` field is *itself* a
content-carrying value, and both operations applied to it are content-sensitive by
construction:

- **`sort` is a no-op on a list whose elements are all equal.** With one fixed string, any
  concatenation is already sorted, so the corpus cannot distinguish a correct `sort()` from
  no sort at all. Measured: M1.
- **`extend` and "keep only the first list" are indistinguishable when the lists are
  equal.** With one fixed string, dropping half of every merge produces a byte-identical
  result. Measured: M2 — and M2 is precisely standing obligation 2's failure mode,
  assumptions disappearing during aggregation, which is the obligation this corpus is most
  often cited as covering.

So the honest verdict: **one fixed assumption string does not stand for all strings.** What
it stands for is the *presence* half of obligation 2 — `Some`/`None` propagating correctly
and `open_items` counting conditionals — and nothing about content. Dedup was the one
content-sensitive operation the old corpus did reach (M4 died), because two conditional
nodes in one subtree produce a genuine duplicate even when the string is fixed.

There is no symmetry argument available to rescue this the way equivariance rescued leg 1.
Relabeling assumption strings is *not* a symmetry of `aggregate`: `sort` orders by string
content, so a non-monotone renaming does not commute with the fold. The aggregated list is
equivariant only as a *set*, and set-equivariance is exactly what a single-element corpus
cannot observe.

**Closed by:** conditional nodes now draw their assumption text from a period-2 cycle over
pre-order position, `"zeta: assumed contract"` / `"alpha: assumed contract"` — chosen in
*descending* lexicographic order so that concatenation order and sorted order disagree
whenever an even-positioned node precedes an odd-positioned one. Over trees of up to 4
nodes a period of 2 yields both same-text pairs (positions 0 and 2, keeping M4's dedup
coverage) and different-text pairs (0 and 1, in parent-child and sibling-sibling roles).
The oracle collects into a `BTreeSet<&str>` — sorting and deduplicating by a different
mechanism than the code under test — and the expected value is the exact sorted list, not
"non-empty". M1 and M2 now die.

The reduction that remains is honest and much smaller: the corpus covers 2 distinct
assumption texts, not all strings. Nothing in the kernel inspects assumption text beyond
`Ord` and `Eq`, so what a third distinct text could reach that the second cannot is a
three-way comparison — a `sort` bug that is correct on all pairs and wrong on a triple.
That is a `core::slice::sort` defect, not a kernel defect, and is not what this corpus is
for.

### What the reduction still does not cover

Stated plainly, because these are the things a reader should not assume from the word
"exhaustive":

1. **A single node carrying both a status flag and a conditional.** The three status shapes
   are mutually exclusive, so no node in the corpus contributes 2 open items *by that
   route*, and no node exercises the `+` in `node.statuses.len() + if conditional {1}`.
   M6 is the exact witness: replacing that `+` with an either/or leaves the suite green.
   Closing it means enumerating the payload rather than cycling it — a fourth status shape
   takes the config space from 21 to 28 and the corpus from 991,389 to **3,117,996 trees**
   (3.14×, since the count is dominated by `5 · configs⁴`), with the runtime and the
   resident memory of holding them all to match. **Recorded, not done** — and note it would
   also invalidate the 991,389 figure that CLAUDE.md, `demos/fault-injection.md` and
   `docs/plans/trusted-boundary.md` all quote.
2. **`conditional: Some(vec![])`.** Every conditional node in the corpus carries exactly one
   assumption, so a conditional with an *empty* list is never aggregated. This matters more
   than it looks: `VerdictNode::conditional`'s doc says "a conditional status without an
   assumptions list is unrepresentable in the kernel, not validated against" — true of
   `Some` vs `None`, but an empty `Vec` is representable and would aggregate to
   `Some([])`, a conditional carrying no assumptions. Whether that is a real hole or a
   garbage-in case worth a type change (`Vec1`, or a private constructor) is a design
   question for the spec, raised here rather than closed here.
3. **Trees beyond the structural bound** — more than 4 nodes, deeper than 3, wider than 3.
   Unchanged by this session, and this is the gap `tests/spike/verus/` exists to fill:
   induction over every tree of every size, subject to its own shadow-not-source caveat
   above.
4. **Anything outside the kernel.** `open_items` here counts status flags and conditionals
   only; unresolved-marker registry entries (§5.6) have no representation in this node type.
5. **`StatusSet::iter` and `insert` themselves.** The oracle reads a node's flags through
   `iter`, and the corpus builds them through `insert`, so a defect in *those two*
   primitives would be shared by both sides. `every_status_in_the_glossary_round_trips_through_the_bitset`
   is the check that covers them; the enumeration is not it. (Union and `len` are no longer
   shared — that was the point of the `BTreeSet` oracle.)

### What the enumeration may now honestly claim

> For every verdict tree of 1–4 nodes, depth ≤3, ≤3 children per node, over all 7 node
> kinds × 3 status shapes at every position — 991,389 trees, checked at every node against
> an independently-computed oracle — `aggregate` satisfies all four standing obligations.
> Status flags and assumption texts are *represented*, not enumerated: three of the seven
> status kinds and two assumption texts, positioned so that the union combines distinct
> flags, a node carries two flags, and the assumption merge must both order and deduplicate
> across distinct values. Per-bit uniformity of `StatusSet` (§ leg 1) extends the status
> result to all seven kinds by relabeling and to all flag combinations by per-bit
> separability. **No such extension is available for assumption content** — the claim there
> is what the corpus directly exercises and no more. Not covered: a node carrying a flag
> and a conditional at once, an empty assumption list, and every tree larger than the
> bound.

That is a weaker sentence than "exhaustive over every verdict tree up to a small bound",
and it is the true one. The strong version now lives where it is earned:
`tests/spike/verus/`, unbounded, subject to its own shadow-not-source caveat.

---

## TODO deltas

`TODO.md` is held by another agent this session, so these are listed rather than applied:

- **DONE** — retire the three non-terminating Kani harnesses in `tools/kernel/src/lib.rs`;
  investigation preserved as a historical note in the crate doc; crate now names Verus and
  the enumeration as where the four obligations are proved, with the shadow-not-source
  caveat stated.
- **DONE** — write the enumeration's reduction argument (this document). Leg 1 (per-bit
  uniformity) holds; leg 2 (content-independence of the assumption merge) does not, was
  measured not holding via four surviving mutants, and the corpus was repaired at the same
  991,389 trees and the same ~2.3s runtime.
- **KNOWN GAP (open, deliberate)** — the corpus cannot see a node carrying both a status
  flag and a conditional (mutant M6 survives). Closing it costs 3,117,996 trees and
  invalidates the widely-quoted 991,389 figure. Left open on purpose.
- **OPEN, spec question** — `VerdictNode::conditional: Some(vec![])` is representable and
  aggregates to a conditional with no assumptions, which the field's own doc comment says
  should be unrepresentable. Needs a decision (non-empty-vec type, private constructor, or
  an explicit "garbage in" stance), not a test.
- **STALE REFERENCES, outside this session's lane** — three files still describe the
  deleted `kani_proofs` module as present: `tests/spike/scale/SCALE-FINDINGS.md:189`,
  `tests/spike/FINDINGS.md:115`, and `tests/spike/verus/FINDINGS.md` (several places).
  They are dated findings documents, so a one-line "since retired, see
  `docs/kernel-honesty-cleanups.md`" is probably the right fix rather than a rewrite.
- **NOTE** — CLAUDE.md's kernel paragraph still says the Kani harnesses "exist and are
  `#[cfg(kani)]`-gated" and that "none of them terminate". The second half is still true;
  the first half is not, as of this commit.
