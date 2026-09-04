# Handoff — 2026-09-04, late session

Five branches pushed, none merged. `main` is clean. Nothing is running.

**TODO.md stays the source of truth.** This file is the narrative it cannot carry: what
the session was for, and the four traps that cost real time, so tomorrow does not pay again.

---

## What landed

Continued the previous handoff's ranked list. Three of its four items are done.

**1. The container fix — `record::fingerprint` earns `fuzzed(256)`.** All 44 of Ply's own
promises now earn evidence; there is no unchecked claim left in the library. Three faults,
each confirmed by running:
- the resolver behind a field, a variant field, a constructor argument and a route argument
  never walked into a container, so `Vec<Item>` came out unsupported although `Item` builds
- the generator bound a field's raw draw straight to the field name — right for a leaf,
  wrong when the field is a container of a user type. **This was the handoff's
  "believed, not confirmed" item: confirmed, and not where the handoff pointed** — it is the
  top-level struct path, not the nested one. Proven by compiling the generated crate and
  letting the compiler name it (`E0308`), which is the only way this class is provable.
- the generator still called the buildability gate through `unwrap_or_else(panic!)`, the
  exact shape that lost a whole run to exit 101 once. It returns a named error now.

**2. Six more claims**, all earning: `registry::all` at `tested`, the rest `fuzzed(256)`.
On three of them the disclosure showed one branch deciding all 256 cases, so worked
`examples:` were declared to exercise the rare branch rather than reword the promise to hide
the disclosure.

**3. The writing skill's rule 4 corrected.** It still told authors to keep structs under a
dozen fields — a limit lifted that morning. Real constraints: fields public and named, type
not `#[non_exhaustive]`.

**4. Not started, and not scoped.** Deriving a component's invariant from its operations'
promises. The previous handoff says "a full design plan is in TODO.md" and **it is not
there** — searched for the plan's own distinctive phrases, nothing. Either it was never
written down or it is under wording nobody has guessed. Do not invent one and attribute it.

## Also landed, from the maintainer's own observation

The two documents (`ply.yaml`, `crates/ply-core/ply.yaml`) could not be navigated between —
no semantic zoom from a crate box into that crate's modules.

A design pass rejected an explicit `include:` grammar key **for now** in favour of deriving
the link, on two grounds worth keeping: derived facts already draw in this grammar (findings,
`entry` edges, hollow, worst-descendant fill), so derive and declare produce the *identical
picture*; and no new glyph is needed because the collapsed stacked card already means
"folded content, zoom here".

`core` now draws as one stacked card reading `21 components · 44 fns —
crates/ply-core/ply.yaml`. **The root drawing went from 1510px tall to 630px**, because the
five hand-copied module boxes collapsed into one card — row budget repaid, not spent.

**The half that mattered more than the feature:** the root was hand-declaring five modules
inside `core` while the real document declares twenty-one. Already drifted, so the committed
drawing was lying to every reader. That copy is deleted; the link supplies the interior.

---

## Open, ranked — from the review of that work

1. **The silent shadow (weakest joint).** Local content suppresses a resolvable link with
   **no diagnostic**, so the exact five-of-twenty-one drift this feature exists to kill can
   quietly return; only a comment in `ply.yaml` defends it. One rule and one sentence: "this
   box declares its own interior while its crate's document declares N components; one of the
   two is stale." Related: a non-hollow component still claims its target in the dedup map,
   so it can block a hollow sibling while its own link never draws.
2. **An owed §7.1 row.** All four new codes cite §7.1, and §7.1 has no derived-link row — the
   visual table's totality claim now has a drawn form it does not list. Spec amendment,
   maintainer's to write, and TODO.md does not record the debt.
3. **`check`'s summary still says nothing about a resolved link.** The drawing and transcript
   carry the pointer; the command that started the whole thread does not. One sentence in the
   anchors tier closes the original observation completely.
4. Two small holes in the drift rule: a target that parses with zero components produces no
   `W0532`, and the message says "that document's own top-level anchor" while naming only the
   first of several.
5. **`cargo clippy --workspace --all-targets` never ran on `claude/container-resolution`** —
   the host disk filled and took the shell out. Run it before merging.

## Branches

| branch | what |
|---|---|
| `claude/container-resolution` | the container fix; **clippy not run** |
| `claude/six-more-claims` | six promises |
| `claude/skill-rule-4` | the stale skill rule |
| `claude/derive-document-links` | cross-document zoom + the drift deletion |

---

## Traps — read this part

**`cargo install` is global, and an agent running it silently replaces the maintainer's
binary.** Several agents were briefed to reinstall as part of verification. One installed
from its own worktree, on a branch predating the `--json` state fix, and the maintainer then
spent time looking at a viewer fed by that binary and reasonably concluded the glyphs were
broken. They were not. **Never brief an agent to run `cargo install`.** Build and test with
`./target/release/cargo-ply` in the worktree.

**A stacked PR merges into a dead branch.** A PR based on another PR's branch was merged
after its base merged, so its work went into a branch that no longer led anywhere and never
reached `main`. GitHub reported it merged. Verify with `git merge-base --is-ancestor <commit>
origin/main`, not with the PR's badge. Retarget the moment the base lands.

**Two green branches can make `main` red between them.** One reworded a sentence; the other
made the document a test asserted that sentence against stop being an example of the case.
Each was green against a `main` that did not contain the other. Neither review could have
caught it. A test that borrows a repo document to demonstrate a case should own its fixture
instead.

**`qlmanage` silently truncates.** It forces a square viewport, so a drawing taller than it
is wide is checked from the waist up and reports success. `docs/ply-self.svg` was 1283x1510
and came back 1100x1100 — the bottom quarter never rendered. CLAUDE.md recommended it until
today and several drawings were signed off on a partial view. Rasterise at the drawing's own
size and check the PNG came back that size.

**One working tree, many agents.** Creating a branch in a shared checkout while an agent
worked there destroyed its finished-but-uncommitted work. Give every agent its own worktree,
and tell them to commit early — the disk filling later took another agent's shell out
entirely (`ENOSPC`, even `echo` failed) after it had finished but before it could commit.

**Disk.** The host hit 100% with 1.5 GiB left. Freed ~9 GB: build artifacts from finished
worktrees, a stale `tools/target`, and (with permission) the Ollama and Hugging Face model
weights. Ollama's keypair was kept; `phi4-mini` and `pplx-embed-v1-4b` need re-pulling.
