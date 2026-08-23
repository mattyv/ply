# cargo-mutants feasibility spike — findings

Run 2026-08-23. Toolchain: **rustc/cargo 1.90.0**, **cargo-mutants 27.1.0**, macOS
aarch64. `cargo-mutants` was not preinstalled; `cargo install cargo-mutants` (unlocked)
failed — the unlocked resolve pulls `cargo-platform` v0.3.3, which requires rustc 1.91,
newer than this machine's 1.90.0. `cargo install cargo-mutants --locked` (using the
versions in cargo-mutants' own checked-in `Cargo.lock`) built and installed cleanly in
under a minute. Re-run everything with `./run.sh`.

This spike closes the one M0 feasibility item the earlier spike (`tests/spike/FINDINGS.md`,
ADR-0003) never touched: §10's M0 list names "cargo-mutants with a custom test command
running a generated harness," and §5.4c asserts that mechanism is "confirmed in the M0
spike" — a claim that was never actually checked. It's checked here.

Fixtures, both throwaway and outside `tools/`'s workspace (own `[workspace]` table or own
workspace root, per the existing spike's convention):

- `colocated/` — one crate, two functions with identical bodies (`strong_target`,
  `weak_target`; comparison, `&&`, and `+`/`-` all present so mutants are obviously
  plantable), each with its own `#[cfg(test)]` checks in the same crate.
- `scoped/` — a `lib` crate with the same two functions and **no local tests at all**,
  plus a separate `harness` crate (real Ply-shaped: a proptest property test + explicit
  boundary examples for `strong_target`, one vacuous smoke test for `weak_target`) that
  depends on `lib` by path. A second copy of the harness, `harness-genloc`, sits at
  `lib/target/ply/fuzz/` — reproducing §5.4c's own words ("generated harness crate under
  `target/ply/fuzz/`") literally, including that this path is matched by the repo's root
  `.gitignore`'s bare `target/` pattern (confirmed with `git check-ignore`).

## Verdicts

| # | Item | Verdict |
|---|---|---|
| 1 | Does `cargo mutants` run at all, what does it report by default? | works; plain-English MISSED/caught report, one line per mutant |
| 2 | Does `--re <fn>` scope mutation to one function? | works, exactly |
| 3 | **Does a custom test command work?** | **no such flag exists** — the real mechanism is `-p <mutated-pkg> --test-package <harness-pkg>` plus a cargo-test name filter; verified end to end |
| 4 | Strong spec catches mutants, weak spec lets them survive | works — 13/14 vs 0/14 |
| 5 | Timing for a scoped run on a trivial function | ~1.2s/mutant warm, ~8s cold baseline; ~18s total for 14 mutants |
| 6 | Generated harness in a `target/ply/fuzz/`-style location | works by default; **breaks outright** if `--gitignore true` is ever turned on |

## The three that matter

### 1. There is no custom-test-command flag — §5.4c's phrase describes a mechanism cargo-mutants doesn't have

`cargo mutants --help` and `cargo mutants --emit-schema config` were checked directly
against the installed 27.1.0 binary, not against docs that might be stale. `--test-tool`
is a two-value enum, full stop:

```
"TestTool": {
  "oneOf": [
    { "const": "cargo", "description": "Use `cargo test`, the default." },
    { "const": "nextest", "description": "Use `cargo nextest`." }
  ]
}
```

There is no "run this arbitrary command/binary instead" option anywhere in the CLI or
config schema. cargo-mutants **always** runs `cargo test` (or `cargo nextest run`) in the
tree it built; the only thing an adapter can steer is *which package's* copy of that
command runs, and what gets passed after it.

**What actually works**, proven with the `scoped/` fixture (`lib` mutated, zero local
tests — the honest stand-in for a real Ply target, where §5.4c says the checks live in a
generated harness crate, not in the target crate's own tests):

- Mutating `lib` with no extra flags: **all 14 `strong_target` mutants MISSED**, even
  though `strong_target` has a strong property spec — because `cargo-mutants` defaults to
  running only the *mutated package's* own tests, and `lib` has none. This is the exact
  failure §5.4c is worried about, reproduced directly.
- Adding `-p ply-spike-mutants-lib --test-package ply-spike-mutants-harness`: **13/14
  caught**, identical to the colocated result. `-p` selects what to *mutate*;
  `--test-package` selects what tests to *run*; they can name different packages.
- Adding a trailing cargo-test name filter (`-- strong_target_harness`) on top of that:
  same 13/14 result, and the harness crate's log shows `1 filtered out` — proof the
  filter is a real exclusion (the sibling `weak_target_harness` module's one test was
  excluded), not just `--re` picking the right mutant while the whole suite runs as
  unfiltered noise.

So the mechanism Ply's `mutate` adapter needs is real and works — it just isn't a
"custom test command." It's: generate the harness as a proper workspace-member crate
(not loose files under some arbitrary path), then invoke cargo-mutants with
`-p <target-crate> --test-package <harness-crate> --re <fn-regex> -- <fn-name-filter>`.

### 2. `--gitignore` defaults to *off*, and turning it *on* breaks §5.4c's own placement

Two independent, surprising results, checked with `--leak-dirs` to inspect the actual
scratch build directory cargo-mutants built and tested against:

- **The runtime default does not respect `.gitignore` at all.** The config schema
  describes `gitignore` as "When copying the tree, exclude patterns in `.gitignore`" and
  gives its default as `null`, implying "on." Empirically: a loose non-package file
  (`junk/filler.txt`) matched by a local `.gitignore` rule was copied into the scratch
  tree anyway, with no flag passed. Only passing `--gitignore true` explicitly excluded
  it. So the wording in the schema is aspirational; the observed *behaviour* of the
  installed binary is "copy everything unless told otherwise."
- **This accidentally means §5.4c's placement (`target/ply/fuzz/`) works today** — the
  `harness-genloc` copy at `lib/target/ply/fuzz/` (git-ignored by the repo's plain
  `target/` rule) mutation-tests identically to the non-ignored `harness` copy: 13/14
  caught. It works *only because gitignore isn't actually being consulted by default*.
- **Turning `--gitignore true` on breaks it outright**, and this is not a hypothetical: a
  real target crate — the exact case `mutate` exists for — has a real `target/` full of
  gigabytes of build cache that a competent operator would *want* excluded from every
  mutant's tree-copy, for speed. Passed explicitly:

  ```
  error: failed to load manifest for workspace member
  `.../lib/target/ply/fuzz`
  referenced by workspace at `.../Cargo.toml`
  Caused by: failed to read `.../lib/target/ply/fuzz/Cargo.toml`
  Caused by: No such file or directory (os error 2)
  ERROR cargo build failed in an unmutated tree, so no mutants were tested
  ```

  A loud, total, immediate failure — not a silent false-pass, which is the one mercy
  here — but the entire `mutate` check for that crate goes dark. A one-line local
  `.gitignore` negation (`!target/` then `target/*` then `!target/ply/`, the standard
  layered-negation idiom) was tried as a fix and did **not** reliably restore the
  intended split (it also un-ignored real build noise like `target/debug`) — gitignore
  negation-under-an-excluded-parent is a known-fragile corner of the format, and this
  spike doesn't recommend relying on it.

  **The straightforward fix**: never place `mutate`'s harness crate under a path any
  applicable `.gitignore` would match, or have the adapter always pass `--gitignore
  false` explicitly rather than trusting either the current default or a future default
  change. Given `target/ply/` is also where D2/§5.5's *other* generated artifacts live
  (Kani proof modules, playback JSON), this is a real placement decision, not a detail —
  see amendment 3 below.

### 3. Timing is affordable for a per-function budget (item 5)

Scoped run, `strong_target` alone, warm build cache: baseline + 14 mutants in **17.6s
real** (`/usr/bin/time -p`), i.e. ~1.2s/mutant once the crate is warm. Per-mutant
`outcomes.json` phase timings: ~0.2s build, ~0.8s test, on this trivial two-branch
function. A cold first run (fresh `Cargo.lock`, dependencies not yet built) took ~30s
for the same 28-mutant colocated batch, dominated by proptest's own compile time (a
one-time cost per crate, not per mutant). For a real fn with a non-trivial dependency
tree, expect the per-mutant marginal cost to still be dominated by incremental
build+test, not by cargo-mutants' own overhead — which this spike does not have the
scale to characterise further (see the toy-type caveat in `tests/spike/FINDINGS.md`,
item 4, which applies here too: nothing here says anything about a real crate-sized
dependency graph).

## The equivalent-mutant caveat (item 4, worth stating precisely)

`strong_target`'s one surviving mutant is `replace > with >=` on `y > 0`
(`x > 0 && y > 0` → `x > 0 && y >= 0`). This is not a gap in the property test: the two
conditions differ only when `x > 0 && y == 0`, and in that exact case
`x + y == x - y == x` regardless of which branch fires — so **no test oracle, however
strong, can distinguish this mutant by output**. It is a textbook equivalent mutant,
confirmed algebraically, not empirically patched around. `strong_target_matches_reference`
(the proptest property) also independently demonstrated the opposite failure mode: with a
uniform `-1_000_000..1_000_000` sampling range and no boundary-focused strategy, it
*missed* both `>`→`>=` mutants at first (256 random cases essentially never land exactly
on `x=0` or `y=0`) until explicit boundary examples (`strong_target_boundary_cases`) were
added alongside it — which is D12's own model: `test` (examples) and `fuzz` together are
the kill signal, not `fuzz` alone.

**Consequence for §5.4c/D12's `W0502 weak spec (N surviving mutants)` wording:** N is not
purely a measure of spec weakness. It can include equivalent mutants that no spec, however
complete, could kill. `W0502`'s phrasing and any documentation of it should say so, or
users chasing "N surviving mutants" to zero will burn time on mutants nothing can catch.

## Spec amendments this forces

1. **§5.4c** — drop or rewrite "(Mechanism confirmed in the M0 spike.)"; it was never
   checked until this spike. Replace the claim with the real mechanism: cargo-mutants has
   no custom-test-command flag; scoping is achieved via `-p <target> --test-package
   <harness> --re <fn> -- <name-filter>`, which requires the generated harness to be a
   proper workspace-member crate (not loose files), and requires the adapter to know both
   the target crate's package name and the harness crate's package name.
2. **§5.4c / D2 / §5.5 (housekeeping, "Ply owns everything under `target/ply/`")** — the
   `target/ply/fuzz/` placement is currently *safe only because cargo-mutants doesn't
   actually respect gitignore by default*, which is an accident of this tool version, not
   a designed-in guarantee. The adapter must pin `--gitignore false` explicitly on every
   `mutate` invocation (never rely on the ambient default), and the spec should record
   *why*: a real crate's genuine `target/` directory is enormous, an operator turning on
   `--gitignore true` for speed is a foreseeable, reasonable action, and it currently
   causes total, if loud, failure of `mutate` for any crate using this placement.
3. **D12 / W0502** — state explicitly that a surviving-mutant count can include equivalent
   mutants (demonstrated here with an algebraic proof, not a hand-wave), so `N` in
   "`W0502 weak spec (N surviving mutants)`" is an upper bound on spec weakness, not an
   exact measure. Whether M3/M4 should attempt any equivalent-mutant suppression, or
   simply document the caveat in the diagnostic's wording, is an open call for whoever
   implements `W0502` — flagged here, not decided here (out of this spike's scope).
4. **§10 M0** — this item is now discharged. All of items 1–6 from the task brief have a
   recorded verdict above with exact commands/output in `run.sh`'s transcript.
