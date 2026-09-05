# TODO

**Picking this up fresh?** `docs/handoff-2026-09-04.md` is the narrative and the traps; this
file is the state. Read that one first, then this.


## Landed: the last unchecked promise in Ply's own library now earns evidence — 2026-09-05

`record::fingerprint` was the one claim in `crates/ply-core/ply.yaml` that earned nothing,
refused because its parameter has two fields that are lists of another struct
(`assumed: Vec<AssumedPromise>`, `engines: Vec<EngineId>`). All 44 claims in the library now
earn real evidence -- confirmed by running, not by reading: `cargo ply verify crates/ply-core`
exits clean with zero refusals of any kind, and the same is still true of `crates/ply-cli`.

Two separate, real bugs, not one -- found by tracing exactly why a shape that should have
worked (once yesterday's crash fix made it stop being refused outright) still would not
actually build:

- [x] **A struct's own field, one container deep, was never resolved at all.** The function
      that turns a bare type name into a real, buildable one only ever handled a field
      whose type *is* that bare name directly -- never one sitting inside a `Vec`, `Option`,
      a tuple, or a map. A top-level *parameter* of the same shape already worked, because
      the code path for parameters walks containers and the code path for fields did not;
      two implementations of the same idea, one of them incomplete. Fixed by making the
      field path recurse through every container shape the parameter path already knows,
      reusing the exact same per-leaf resolution either way so the two cannot drift apart
      again.

- [x] **Once resolved, the generated code still built the wrong value.** A field whose type
      is `Vec<AnotherStruct>` fell into the generic "just bind whatever proptest drew" path,
      so it ended up holding a list of raw tuples where the real field expects a list of
      real struct values -- "mismatched types" in the generated file, not a refusal. The
      exact conversion already existed for a *parameter* of this shape; applying the same
      conversion to a field's own generated code, which previously skipped it, was the whole
      fix.

  Both proved rather than assumed. A focused test for each: one asserting the resolver
  accepts the shape at all, a second asserting the *generated code itself* -- not just
  whether the function was accepted -- actually constructs real values, because
  accepted-but-uncompilable is exactly the failure that slipped through before. Both
  confirmed red without their respective fix. And the real case, not just a synthetic one:
  `record::fingerprint`'s own promise was broken on purpose (truncating the hash it
  returns) and the check caught it as a genuine violation, naming both struct-typed fields
  in the failing input it reported, before being restored.

## Landed: CI now runs Ply against its own two documents — 2026-09-05

Until now, every claim built up over the last two days -- 44 promises about ply-core, 6
about ply-cli, all earning real evidence -- had only ever been checked by hand, on one
machine, never by CI. Nothing stopped either document quietly drifting out of truth the
next time the code under it changed; the whole "Ply proves itself" story rested on someone
remembering to run it.

- [x] **`cargo ply verify` can now write the real, evidence-coloured drawing to a file.**
      New `--svg <path>` flag. The colouring code already existed (`render_svg_with_evidence`)
      and was already computed on every `--publish-view` run, but only ever wrapped in a
      JSON envelope for an editor to poll -- nobody could get a plain `.svg` file out of a
      real run at all before this. `cargo ply render` still only ever draws from the
      document, deliberately grey, never green; this is the other half.

      Proved with a real end-to-end test, not a shape check: the promise is that the
      written file carries the `fn-chip-box-earned` class (a real green fill, applied only
      when a chip has actual `DisplayState::Earned` evidence attached) and the plain
      declared render never does. Confirmed red without the flag (clap rejects it) and red
      again on a first draft that asserted the wrong signal (the check-kind label like
      "fuzz: 64 cases" turned out to be part of the *declared* chip too, present whether or
      not anything ever ran -- an assumption worth recording since it looked right at a
      glance and was not).

- [x] **A new required CI job, `ply-self-check`**, added to the one gate `main` actually
      requires. It builds the tool, runs it against both of Ply's own documents with
      `--fail-on error` (the looser mode: only a real regression -- a broken promise, a
      harness that stops compiling -- fails the build), and uploads both verified drawings
      as a downloadable build artifact on every run, pass or fail.

      `--fail-on error` was chosen by checking, not assumed: `cargo ply explain` confirms a
      real violation and a real tool error are both error-severity, and the fingerprint
      gap's own diagnostic is warning-severity, so this is strict where it needs to be and
      tolerant only of the one gap already written down elsewhere in this file.

      **Retracted the same day, by the section above this one.** `--fail-on error` was
      picked to tolerate exactly one gap -- `record::fingerprint`'s refusal -- and that gap
      was closed hours later. The setting is now looser than anything justifies it being.
      Tightening it is carried as an open item below rather than left implied here.

      Simulated the exact commands CI will run before committing: both crates verify clean
      at exit 0, and the two drawings were opened and read, not just size-checked -- every
      chip in both is genuinely filled and checkmarked, with the header line itself now
      reading "6 earned" rather than the declared form's plain function count.

## Landed: the evidence is published where a person can look at it — 2026-09-05

The job above proved both documents on every change and then put the result somewhere
almost nobody would ever go: a zip attached to a build, which expires after 90 days, which
the forge will not render an SVG out of, and which costs a download and an unzip to read.
Evidence that expensive to look at is evidence nobody looks at, and "Ply proves itself" was
still, in practice, a sentence in a commit message.

- [x] **A small published page, written by the build and only by a build that passed.**
      New `publish-evidence` job and `.github/scripts/build-evidence-page.sh`. Both
      drawings, plus three short paragraphs telling a stranger how to read them, at a
      permanent address rather than an expiring one. Linked from the top of the README.

      From CI and only from CI, on the same reasoning that already keeps `ply.lock` out of
      commits: Ply's evidence about itself must never be something one developer's machine
      can put in front of a reader. A drawing committed to the repository could be
      regenerated and committed by hand; a page only a passing run can write cannot.

      Deliberately not added to `ci-gate`. Publishing is not checking, and a page that
      cannot deploy should not turn `main` red. The risk that buys -- a publish quietly
      breaking, leaving a stale page under an address people trust as current -- is handled
      where it can actually be seen: the page prints the commit it was built from and when,
      so staleness is visible to the reader rather than silent.

      The page generation is a script, not inline YAML, specifically so it can be built and
      *looked at* before deploying. That is not decoration -- it was used: the page was
      generated locally from the real drawings CI produced (downloaded from the actual run,
      not regenerated), rasterised at its true full height of 4,638px after a first capture
      at 4,400px silently cut the second drawing in half, and read. Both drawings render,
      all 50 chips are filled and checkmarked, and the header lines read "44 earned" and
      "6 earned". The truncated first capture is exactly the failure CLAUDE.md warns about
      one level up, and it happened here on the first try.

- [x] **The self-check now fails on a refusal, not just on a broken promise** — 2026-09-05.
      It ran `--fail-on error`, chosen to tolerate `record::fingerprint`'s refusal, which
      was fixed hours later; the looser setting then had nothing justifying it.

      Answered by running, which is what the open item asked for. Both documents were
      verified under `--fail-on evidence` first: **both exit 0**, so the stricter setting
      costs nothing today. The worry that prompted the delay -- that `evidence` would also
      reject the evidence-quality disclosures -- turned out to be wrong, and measurably so:
      ply-core's run reports 74 "generated text excludes control characters" and 32 "one
      side of this either/or decided every case" disclosures and still exits clean. Those
      describe what a *passing* check did and did not cover; they are not absences.
      `--fail-on warn` is the setting that rejects them, and it is not wanted.

      What the tightening actually buys, confirmed with `cargo ply explain`: a function Ply
      refuses to check reports at **warning** severity, so `error` waved it through. That is
      precisely how `record::fingerprint` sat unchecked for days while every build stayed
      green. The next such refusal now fails the build instead of being discovered by
      someone reading a report.

- [x] **The build's actions moved off a runtime that is being removed** — 2026-09-05.
      GitHub removes Node 20 entirely on 2026-09-16 and had already started force-running
      these actions on Node 24, which is a warning today and a broken build shortly.

      Version numbers picked by reading each action's declared runtime rather than its
      release notes, which was not paranoia: `upload-artifact@v5`'s notes announce Node 24
      support, and its manifest still says `node20` -- "preliminary support" that a reader
      would reasonably take for the fix. Each action moved to the lowest major that
      actually declares `node24`: checkout v4→v5, upload-artifact v4→v6, download-artifact
      v4→v7, deploy-pages v4→v5. Deliberately not "latest of everything": `download-artifact`
      v8 changes when it unzips a download, and that is the exact hand-off `publish-evidence`
      depends on and the one path a pull request cannot exercise.

      `upload-pages-artifact@v3` and `actions/cache@v4` are untouched and were never in the
      warning -- the first is a composite action with no Node runtime at all.

## Landed: the CLI's own library gets its first six claims — 2026-09-05

`crates/ply-cli/ply.yaml` is new: 6 claims, all in `shared.rs` plus one in `lib.rs`, all
earning evidence. This is 6 of roughly 180 public functions in the crate -- the fast
first cut of pure helpers, not the whole crate. `verify.rs` (9,100 lines, the code that
decides every verdict) is entirely unclaimed still; that is the next, much bigger step.

Three real defects found and fixed along the way, all confirmed by running rather than
by reading:

- [x] **`wrap` could abort the whole process.** Every real call site passes a small
      literal indent (0, 4, 6, 14 ...); nothing stopped a pathological one, and Ply's own
      first fuzz run against its own CLI crate found it in seconds -- an indent near
      `usize::MAX` makes `" ".repeat(indent)` try to allocate that many bytes. Fixed by
      clamping the indent to just under the line width, since an indent at or past the
      width already makes wrapping meaningless -- a fact about what the width means, not
      an invented answer for an input nobody meant.

- [x] **The fuzz harness never imported the checked function's own containing module.**
      A promise may call a sibling function defined right next to it in the same file --
      no `use` statement needed, since same-module items need none. That compiled fine in
      the real crate and failed in the generated check with "cannot find function", because
      the harness only ever imported the checked function's own bare path plus a
      crate-root glob, never the specific module it actually lives in. This is a second,
      independent instance of the class of bug fixed 2026-09-04 for the counterexample
      replay test (`contract_rt::render_cex_test`'s `module_import`) -- that fix covered
      only the reactive replay path; the *initial* check, which is what actually finds a
      violation in the first place, still had the gap. Fixed the same way, in
      `fuzz_gen::wrap_fn_harness_module`, reusing the same `import_path()` split so the two
      fixes cannot again disagree about how many segments to drop.

- [ ] **OPEN, recorded rather than fixed: `check` accepts a function that `verify` can
      never actually check.** `bounded(k)` (Kani) runs inside the target crate's own
      source, where `pub(crate)` is visible; `fuzz`/`test`/`mutate` run in a genuinely
      separate crate that depends on the target as an external dependency, where
      `pub(crate)` is invisible. Ply's own resolver only distinguishes "private to its
      module" from everything else, so `check` reports a `pub(crate)` function as
      perfectly resolvable and `verify` then fails opaquely with a raw compiler error
      naming some unrelated function first in line -- never explaining that the real
      cause is visibility crossing a crate boundary. Worked around here by promoting the
      six claimed helpers to full `pub`, which is a safe, reversible visibility widening
      and not new API surface. The underlying disagreement between `check` and `verify`
      is untouched and worth its own session: `check` would need to know which checks a
      claim declares before it can say whether `pub(crate)` is actually sufficient.

- [x] Rendered and reviewed (`docs/ply-cli-self.svg`/`.txt`), added to ARCHITECTURE.md.
      **Note for future review, not a Ply defect:** the first rasterisation clipped the
      bottom chip even though the window size exactly matched the SVG's own declared
      width and height -- CLAUDE.md's own prescribed check. Headless Chrome's rendering
      of a bare SVG file can carry a few pixels of margin the declared size doesn't
      account for. Re-rendering with ~40-60px of headroom beyond the declared size, then
      confirming the content doesn't reach that margin, is the more reliable check.

## Open: the CLI crate is claimed 6 of ~180 — 2026-09-05

The obvious next batch is `shared.rs`'s remaining pure surface (`declared_contracts`,
`assumed_contracts`, `FnClaimRef`'s methods) and `verify.rs`'s standalone helpers
(`default_engine_timeout_secs` is a clean, already-pure candidate: real properties like
"a stubbed harness never gets less than the stubbed floor" and "the vec-param cost grows
with the bound"). Both take `Document`/`ContractFn`/`FnClaimRef` as parameters in several
cases, which are not Ply-buildable types -- those functions stay unclaimed until routed
around or reduced to their buildable inputs, same as everywhere else in this codebase.


## Landed: a name shadowed across documents is reported, not silently preferred — 2026-09-05

Expanding a linked document made a new kind of collision reachable: names arrive from a
file the author of this one never edited, and can shadow theirs without either file
changing. The fix that unblocked the drawing resolved a top-level name in favour of the
top-level component -- correctly, but *silently*, which is its own trap.

- [x] **`W0419`, a warning: "this name means the top-level component, and something nested
      shares its short name."** Names the reading Ply took and the path that would reach the
      other one. A warning rather than an error because the name does resolve, to exactly
      one thing, by a rule that does not depend on what else exists -- there is nothing here
      Ply had to guess. Genuine ambiguity between two *nested* components is still the hard
      error it was.

      It fires on this repository: `check` the crate against `core.check` the module. That
      warning is correct and is staying -- the edge really does mean the crate, and the
      shadow really does exist.

- [x] **The rule can see the case it exists for.** `run_checks` only ever saw this
      document's own tree, so the shadow -- which arrives from the *linked* file -- was
      invisible to it. It now takes the resolved links and widens its name index with them.

      Only the name index: a linked document's own rules stay that document's business, and
      running them here would report the same problem twice against a file whose author may
      not be able to edit this one.

## Landed: a linked component draws the other document's interior, not a pointer to it — 2026-09-05

Follows the section below, which stopped the root document *copying* `core`'s interior but
replaced the copy with a single folded box reading "look in that file". The maintainer's
report was that the root drawing still showed a subset of `core` -- and it did: five parts
before, then none at all, against a real twenty-one.

- [x] **A linked box now draws the linked document's whole interior, in place.** The root
      drawing goes from one folded box to twenty-nine boxes and forty-four promises, none of
      them written down twice. Folding is a reader's choice again (`--depth`, `--focus`, the
      viewer's own control) rather than the only thing on offer.

      The box keeps its provenance on the anchor line -- `ply_core — crates/ply-core/ply.yaml`
      -- because a reader looking at forty-four promises that are declared in a different
      file needs to know which file to open, and without it the drawing would silently
      present another document's content as this one's.

- [x] **Three things that would have quietly disagreed with the picture, fixed with it.**
      Each was found by looking at the output rather than by a failing test:

      - The *text form* still said "they live in a different file" while the drawing showed
        them. Its own contract is that it states everything the drawing shows, so it now
        walks the linked interior too: 70 lines to 494.
      - The *summary strip* counted only what this file spells out -- "8 components ·
        0 functions" above a drawing of twenty-nine boxes and forty-four chips. Both views
        now read `29 components · 44 functions`, byte-identical, from the one shared walk.
      - That shared walk is `document_counts`, whose own comment records what a second
        independent walk cost the last time one existed. A first draft of this change added
        exactly that second walk; it was removed rather than left to rot.

- [x] **A real resolution bug, surfaced by expanding.** The root document has a top-level
      `check` (the standalone validator crate) and `core` has a `check` module. The moment
      core's interior was drawn, the edge `check -> core` was reported ambiguous and the
      whole drawing failed to render -- and the advice attached to it could not be followed,
      because the "dotted form" of a top-level component is the bare name just rejected.

      A token that already names a component outright is now that component, and the
      leaf-name search never runs for it; the search exists to turn a short name into a
      path, and there is nothing to search for when the token *is* the path. Fixed in all
      three copies of the rule (the renderer, and both halves of `check.rs`). Genuine
      ambiguity between two nested components is still a hard error -- the test that pins
      that was checked, not assumed.

      Covered by a test written before the fix, confirmed red for the right reason.

- [x] **The link invariant tightened from "the counts match" to "everything is drawn."**
      The sweep re-opens every file a drawing says it took content from and asserts each
      declared component and promise actually appears. A count can match while the wrong
      things are drawn, and a box quietly showing *some* of a file is precisely the failure
      this change exists to fix. Both rewritten tests confirmed red when the expansion is
      reverted, green when restored.

## Landed: a component links to another document instead of copying its interior by hand — 2026-09-04 (`cf2fc2e`, branch `claude/derive-document-links`)

`ply.yaml` at the repository root used to hand-declare `core`'s five modules and a
`state:` block as a copy of what `crates/ply-core/ply.yaml` says about itself, and the
two had already drifted (five modules here, twenty-one there) before either file noticed.
No `include:` key was added — a component now links to another document when that
document's own top-level anchor equals, or sits under, the linking component's anchor,
derived from real crate directories the same way anchor resolution already works
(`ply_core::config::derive_links`, `crates/ply-core/src/config.rs`). The linked box draws
with the existing collapsed-component stack (no new glyph), with the target file's path
riding in the text tier after the usual `N components · M fns` count.

- [x] **Four named ways a candidate fails to link, each tested**: the target exists but
      cannot be read or does not parse (`A0417`, error); its top-level anchor no longer
      sits under the linking anchor (`W0532`, "drifted", warning); a chain of documents
      would lead back into itself (`W0534`, warning — real in this repo only as the
      two-file fixture the unit tests build directly, since discovery is a real crate
      directory per hop and today's two documents are one hop apart); another component
      in the same document already claimed the same target (`W0533`, warning). A crate
      with no `ply.yaml` of its own produces neither a link nor a finding — the ordinary
      case for four of `core`'s five siblings. All four codes registered in
      `crates/ply-core/src/registry.rs`. `cargo ply check` reports all four; both real
      `check` runs (`.` and `crates/ply-core`) stay clean, since the one real link
      resolves cleanly and the self-reference `crates/ply-core/ply.yaml`'s own top
      component would otherwise "link to itself" (a document naming its own crate, not a
      link to "another" document) is refused silently rather than as a finding.
- [x] **The ordering trap held**: a derived-link box with no declared interior of its own
      ranks above the hollow rule (checked first in `render_component_dispatch`), so it
      draws the collapsed stack rather than a dashed hollow box — verified by temporarily
      swapping the check order and watching the regression tests fail for the right
      reason, then restoring it. Gated the other way too: a component that already
      declares a real fn or nested component never consults a link at all, even a
      resolvable one — a link stands in for an interior nobody wrote, never overrides one
      the document did write.
- [x] **The invariant the design pass asked for**: every cross-document link's drawn
      counts match the target document, checked as a sweep over the real rendered SVG
      markup against an independent from-scratch recount of the target file
      (`tools/render/tests/derive_links.rs`), not a spot-check on one pair.
- [x] **A real bug caught along the way, not by the docs regeneration itself but by
      updating `self_architecture.rs`'s comparison to resolve links the same way**:
      `target_path` was stripping a literal `"./"` prefix rather than the actual root path
      handed to `derive_links`, so a caller that resolved an absolute root first (as a
      test comparing against a fixed checkout must) would have leaked that host's own
      filesystem layout into a committed drawing. Fixed by stripping `root` as a real path
      prefix; pinned by a regression test asserting the exact relative string with an
      absolute tempdir root.
- [x] Deleted `core`'s hand-declared interior from the root `ply.yaml`, with a comment
      explaining why so nobody restores it. Regenerated `docs/ply-self.svg`/`.txt` (now
      roughly a quarter of the former height — five drawn boxes became one stacked card);
      `docs/ply-core-self.*` unchanged. Updated `ARCHITECTURE.md`'s alt text, component
      table, and "why a second file" paragraph.

**KNOWN GAP, left open on purpose.** Link derivation only ever considers a document's
**top-level** components, and only ever resolves **one hop**: a linked box's drawn counts
come from the target's own file taken at face value, never further expanded through any
link *that* document might itself declare. Both restrictions are deliberate (a nested
component's anchor almost always shares its crate with the document it already lives in,
which would make every module "discover" its own document; and nothing in this
repository's two real documents needs more than one hop), not something a future chain of
three or more real documents is guaranteed to want. The cycle guard (`would_cycle` in
`config.rs`) is already written generically over an arbitrary chain, so extending
resolution past one hop would not need a new safety mechanism, only a decision about what
"the target's counts" should mean once the target itself links onward.

## Landed: a timed-out engine run now kills the whole process tree, not just cargo — 2026-09-04 (`9037b83`)

`run_with_timeout` (`crates/ply-core/src/engines/mod.rs`) only ever killed the one process
it spawned directly. Every command Ply budgets is `cargo ...`, and cargo always spawns the
real prover or test binary as a child of its own — so on timeout, cargo died and the actual
hung process (the thing the budget exists to stop) kept running forever, invisibly. A
regression against the pre-macOS-fix code, which shelled out to GNU `timeout` and killed
the whole process group by default.

- [x] **Fixed by walking the process tree on expiry, not by changing where the child
      runs.** Two designs were weighed: putting the child in its own process group and
      forwarding Ctrl+C to it via a signal handler, versus leaving the child exactly where
      it is (so Ctrl+C keeps working exactly as today, untouched) and separately killing
      every descendant (via `ps`-based tree walking, since macOS has no `/proc`) before
      killing the child itself. Took the second: no signal handler, no process-group
      change, no new global state, at the cost of the tree walk being racy — a process
      forked fast enough right up to and past each of the three re-listing sweeps could
      still escape. Judged acceptable since this only runs on the rare timeout path
      against non-adversarial tooling. Regression test spawns `sh` (direct child)
      backgrounding real `/bin/sleep` (the grandchild) and asserts by pid liveness
      (`kill(pid, 0)`, not log text) that the grandchild is dead after the budget expires
      — red before the fix (the `sleep` survived), green after. `libc` promoted from
      transitive to direct dependency of `ply-core` (already pinned in `Cargo.lock`, so no
      new code enters the build) to send `SIGKILL` to a pid the crate did not spawn
      itself.

## Agreed 2026-09-04, not started: `--json render`'s envelope must resolve declared state too

- [ ] **`build_declared_visual_envelope`'s state rows must go through
      `ply_core::visual::state_shapes::rows_for`, the same as the plain SVG/text renderers
      do, so a declared-only document (no crate on disk, `show:` written as a mapping)
      shows its declared rows to a visual client reading `--json`, not only to a terminal
      user asking for the SVG file.** Declared state shapes (The-Ply-Spec.md's `state:`
      section, "A document may declare a field's shape") landed on
      `claude/declared-state-shapes` in `crates/ply-core/src/model.rs` (the `ShowField`/
      `DeclaredShape` parse), `crates/ply-core/src/visual/state_shapes.rs` (`rows_for`,
      the one shared rows decision), `crates/ply-core/src/visual/svg.rs` and
      `transcript.rs` (both consume `rows_for`), and `crates/ply-cli/src/check.rs`
      (`A0416`, the declared/real shape comparison). None of that touched
      `crates/ply-core/src/visual/mod.rs`'s `build_declared_visual_envelope` or
      `crates/ply-cli/src/main.rs`'s `--json` arm on purpose — a second, concurrent branch
      owns exactly that path (teaching it to resolve state fields at all, tracked in its
      own `render_json_outcome.rs` tests). Whichever of the two lands second owns this one
      integration task: thread `rows_for` into the JSON envelope's row-building the same
      way `svg.rs`'s `state_rows` and `transcript.rs`'s `write_component` already do, so
      the three views (SVG file, transcript, JSON envelope) cannot quietly disagree about
      which rows a declared-only document draws.

## Agreed 2026-09-03, not started: hand-written tests as evidence on a claim

- [ ] **A claim may name existing `#[test]`s as its `test` evidence.** Today the `test`
      check runs only what Ply generates from `examples:`; a hand-written test is not a
      claim source at all. Tests reach what the engines cannot — calls through traits,
      closures and macros; sequences of state; collections at real sizes; a boundary
      promise on legacy code; anything behind a fake — so a claim gains a list of test
      names beside its checks, Ply runs exactly those, records their bodies in the
      fingerprint, and the verdict stays `tested`. No new engine.
      **Honesty condition:** a hand test is opaque, so Ply can say "these passed", never
      "this promise was checked" — either the test calls the contract helper so the
      promise is visibly asserted, or the claim is an attestation with automated evidence
      (hollow shield, audit row). `mutate` already takes the `test` tier as its kill
      signal, and that is the meter: a named test that kills no mutants is `W0502`, not
      evidence. Post-code tier only; it does not close the "trust beyond the bound"
      gap on collections, it makes that trust cheaper to believe.
      **First step:** one fixture with a hand test that kills a mutant the generated
      examples miss, red before the feature and green after.

## Landed: a skill for writing code Ply can check at all — 2026-09-04

The maintainer's observation, and it closes the loop: the side-effect scan finds a bad
shape *after* the code exists, and nothing was stopping an agent writing that shape in the
first place. `skills/ply-checkable-code/` is the generative counterpart.

Seven rules, and **every one has a real incident behind it in this repository** rather than
being a style preference:

1. Separate deciding from writing (`write_harness_lib_rs`, split this session).
2. Do not take an index into a separate argument (`schedule::order`, panicked on
   `domain = {15}`).
3. Return values rather than writing through `&mut` (a shape neither engine builds).
4. Keep a public struct under about a dozen fields (`FingerprintInputs` has 20, and is the
   one claim in Ply's own library still earning nothing).
5. Watch what a precondition throws away (1025 of 1195 inputs rejected, verdict fell to
   `unclaimed`).
6. Write a promise that can fail.
7. Prefer types the engines can build, and reach for `routes:` when one cannot be.

Plus the ordering that matters when Ply refuses: read it as a fact about the code first and
about Ply second, and only call it a limitation after the three questions above.

Four contract tests, including the one rule that is `never` rather than `ask-first` --
weakening a promise to make a check pass, which converts a real finding into a result
nobody can trust. Confirmed by relaxing it to `ask-first` and watching the test go red.

## Landed: fifteen claims on Ply's own library, and what they found — 2026-09-04

The standing programme, started. Measured first rather than guessed: of 165 public
functions in `ply-core`, **63 are checkable today with no refactoring at all** -- they only
lack a promise. Nine were claimed this sitting, chosen for promises worth making rather
than promises easy to satisfy.

- [x] **Eight earn `fuzzed(256)`**: the harness package name always ends in the suffix; its
      path always starts under `target/ply/fuzz/`; a seed always renders as 64 characters;
      stripping terminal colour never makes text longer; an empty string is never an
      identifier; a suggestion is never made from an empty list of keys; a string is always
      the same expression as itself; and one span per generated module.
- [x] **`schedule::order` found a real undocumented precondition.** It indexes `node_ids` by
      the values in `domain`, and Ply panicked it with `domain = {15}`, `node_ids = []`.
      Real callers build both from the same list so they always agree; nothing said so.
      Now written down.

**And writing it down made the function unfuzzable, which is the honest outcome and worth
recording as a worked example.** The precondition threw away 1025 of 1195 generated inputs,
proptest gave up, and the verdict is `unclaimed` -- not a thin green. That is exactly the
trap the README's new contracts section warns about, hit on Ply's own code within an hour
of the warning being written.

- [x] **FIXED same day.** `node_ids` is consulted for exactly one thing -- a deterministic
      tie-break key so two independent nodes always place in the same order -- so the index
      never needed to be fatal. It is now a checked lookup with an empty-string fallback,
      which leaves every existing input's behaviour byte-for-byte and makes the function
      total. The precondition came back out of the document: it was true and it cost the
      function all of its evidence, while every node being accounted for is the property
      that actually matters. `order` earns `fuzzed(256)` over every input now, and the
      exhaustive scheduler enumeration is still green.

**All fifteen claims now earn evidence** except `record::fingerprint`, which is refused by
name for a documented Ply limitation (20 public fields, past what the sampling engine's
tuple strategy reaches).

**The remaining checkable-today functions are the worklist** (54 at the time of writing;
eight of them claimed in the section above). Each needs a promise
worth writing, which is the slow part and the only part that matters -- a promise that
cannot fail would turn all 54 green and mean nothing.

## Open: claims an agent found that are not in the document yet — 2026-09-04

An agent worked the remaining ply-core functions in an isolated copy, but its copy was
branched from a **stale point** -- 6 claims, not the 44 already landed -- so most of its 35
claims duplicate work already here, and it independently rediscovered two bugs fixed
earlier the same day (the `assign_ranks` panic and the unescaped braces in an `examples:`
entry). Its branch is not merged: rebasing 35 mostly-duplicate claims onto a document that
has since gained state blocks everywhere would cost more than rewriting the handful that
are genuinely new.

These are the ones that are **not** in `crates/ply-core/ply.yaml` and are worth adding:

- [ ] `registry::all` -- no two rows share a diagnostic code. A duplicate would make
      `cargo ply explain` ambiguous about which rule a reader is looking at.
- [ ] `schema::known_keys` -- every key it returns satisfies the schema's own identifier
      grammar, so the vocabulary and the validator cannot drift apart.
- [ ] `engines::kani::classify_probe` and `parse_output` -- never conflate a timeout with a
      real counterexample. This is §5.4c's structural rule written as a promise.
- [ ] `visual::state_shapes::glyph_svg` -- always draws the hatch mark when a field could
      not be built. The hatch is the only thing telling a reader that shape is a guess.
- [ ] `kernel::StatusSet::is_empty` -- agrees with `len`, checked against the other's
      independent code path rather than restating either.
- [ ] `fuzz_gen::classify_seedable_wrap` and the two `extract_examples_seed_strings`
      functions -- never invent a shape or a seed the source text did not contain.

Refusals it confirmed by running, worth not re-attempting: `StatusSet::contains` (a
by-value enum parameter cannot be read after the call consumes it, even when `Copy`);
`contract_rt::wrap_test_module` and `engines::fuzz::attribute_build_errors` (a slice of a
plain struct breaks the shared harness build, taking every other claim in the crate with
it -- the same nested-container gap recorded above); `promise`'s three accessors (only a
derived `Default`, no real constructor); `visual::svg::ceiling_class` (two different types
both named `Evidence`, and Ply will not guess -- the same ambiguity the `A0414` fix now
reports properly for state types, still unreported for a parameter type).

## Landed: what each part holds, drawn everywhere — 2026-09-04

Every component in both documents that owns a type now declares it, so the field shapes are
drawn across the whole picture rather than on six boxes out of twenty-two. 16 components
gained a `state:` block.

- [x] **Six are still without one, and each says why in the document itself** -- an absence
      a reader can mistake for an oversight is worth a line: a proc-macro crate defines
      attributes rather than holding a value; the standalone validator is a binary with no
      library; the render facade owns no struct; the scheduler's result is a plain pair of
      collections rather than a named struct, so there are no field names to draw; and
      `config` declares no type at all.

- [x] **`check` is the sixth, and its reason found a bug.** Declaring state on it and on
      `diag` was refused with "declares no type called that" -- for `Diagnostic`, which both
      modules plainly declare. The scanner records a duplicate name by storing "ambiguous",
      and the one place that reads it collapsed ambiguous and absent into the same answer.
      So a reader was sent hunting for a type sitting right there, twice.

      `A0414` now has three sentences instead of one: not declared anywhere; declared, but
      under a different anchor, and here is which; declared more than once, and here are the
      modules. Found by using the feature, not by looking for it.

- [x] Private fields resolve fine, which was an open question -- `reach::FirstParty` and
      `fuzz_gen::ParamSeedPlan` are drawn from fields no caller can touch. That is correct:
      `state:` answers "what does this hold", not "what can Ply build".

## Landed: Ply crashed on a list-of-struct field — 2026-09-04

Found by planning the wide-struct question, not by looking for it. **Exit 101, no JSON, and
every claim in the crate lost -- not one honest refusal, the whole run.** Reproduced in a
four-line crate before anything was changed:

```rust
pub struct Inner { pub name: String, pub n: u32 }
pub struct Outer { pub items: Vec<Inner> }      // Ply panicked here
```

`is_fuzz_supported` answered `true` for any struct or enum without looking inside it, so a
field Ply cannot build passed the gate, reached codegen, and hit a panic codegen writes
*because* it trusts that gate ("safe: every caller gated on `is_fuzz_supported`").

- [x] **The gate looks inside now** -- a struct's fields, an enum's variant fields, and a
      constructor's arguments, each recursively. That crate reports `unsupported` with the
      parameter named, which is the honest answer. Three tests: two red before the fix, one
      guarding against newly refusing types that work today.

      This is Ply failing to take its own advice at the sharpest possible point: the project
      rule is to refuse with a named status rather than crash, and the crash was in the code
      that decides whether to refuse.

- [ ] **KNOWN GAP, deliberately not closed here.** This makes the shape refuse honestly; it
      does not make it *work*. The plan's step 0 also wants the four sites that resolve a
      field/variant/constructor/route type to walk containers the way a top-level parameter
      already does, and the codegen panic turned into a reported tool error so a future
      gate/codegen disagreement cannot take a run down at all. Both still open.

## Open: the twelve-field ceiling is a tool artifact, and the plan to lift it — 2026-09-04

A plan came back on why `record::fingerprint` cannot be checked. **The hypothesis in the
brief was half right, and the wrong half mattered.**

Right: the ceiling is an artifact of folding every leaf into one flat tuple, and the
sampling library's tuple trait stops at 12. Confirmed by running it rather than reading the
docs -- a flat 13-tuple does not compile; a nested `((12),(8))` compiles, runs 256 cases,
and finds a bug planted on leaf 19 with shrinking intact.

Wrong: `default()`-then-assign buys nothing. Ply already assembles the struct with an
ordinary struct literal, so *construction* never had a ceiling -- only *generation* did.
Assignment would additionally require `Default` and public fields, a strict subset of what
the literal already does. Nested tuples chunked at 12 is the fix.

Also found, and neither of us knew it: **the guard counts fields, the tuple counts leaves.**
A 2-field struct holding two 7-field structs is 14 leaves, sails past the guard, and dies as
raw compiler output.

- [x] **Done.** `nest_tuple` folds the parts in chunks of twelve, applied only past twelve
      so every existing harness stays byte-identical; the constant and its refusal arm are
      gone; the `fiveshapes` e2e test that asserted the refusal by name now asserts the
      opposite.

      Proved rather than assumed: a 20-field struct whose promise is false only on the
      **19th** field is caught, with that leaf shrinking to exactly the planted boundary and
      every other leaf to zero.

      **The first version of the fixture's planted bug was the wrong instrument, and an
      agent caught it rather than patching around it.** The threshold was large, so the
      promise was false in only the top few percent of the range -- which the sampler
      reaches rarely enough that the fixed seed missed it every run. The threshold is small
      now, and that is the trick: a defaulted `u32` is 0, so a codegen that quietly left the
      13th leaf at its default would make the promise HOLD. It fails only when the field is
      really drawn.

      Mutation-checked: keeping only the first chunk (fields past 12 never drawn) turns the
      e2e test red.
- [x] **`record::fingerprint` is fixed** -- see "Landed: the last unchecked promise in
      Ply's own library now earns evidence" (2026-09-05). The rest of the container fix
      this note asked for is done: fields resolve through the same container-walking path
      parameters already did, and the generated code that builds one now constructs real
      values instead of leaving a raw tuple in a field that expects a struct.
- [ ] Rewrite `skills/ply-checkable-code` rule 4: wide structs are fine, and the real
      constraints are public, named, and not `#[non_exhaustive]`. Still open -- the skill
      currently tells authors to design around a limit that no longer exists.

**An agent-written producer was considered and rejected for this type**, under the
maintainer's steer that LLM help is acceptable. It is strictly weaker here and for a
non-obvious reason: a producer's own arguments hit the same 12-limit, so it would have to
fabricate 20 fields from at most 12 inputs -- a narrowing Ply cannot see and has no code to
report, and a field added later but not to the producer would silently never be varied.
Routes stay the right answer for types with real invariants and private fields.

## Landed: what the last refusal taught the writing skill — 2026-09-04

`record::fingerprint` is the one claim in Ply's own library that earns nothing, and the
question "can the route mechanism fix it" turned out to be a question about the skill.

**It cannot, and the skill said it could.** A route names an existing public function that
returns the type; it does not create one. Nothing anywhere returns a `FingerprintInputs`
except a private test helper, so there is nothing to name. Worse, the skill's own worked
example was `routes: { FingerprintInputs: fingerprint_inputs_for }` -- a function that does
not exist, invented for the one type in this codebase that has no producer. That is the
same defect the 2026-09-04 review found in the generics rule, introduced while fixing it.

- [x] **The route rule now states its precondition** and says what to do when no producer
      exists: adding a public function whose only caller is Ply is adding API for the
      tool's benefit, and belongs to the developer.

- [x] **New rule 8: some functions should be checked by an ordinary test instead.**
      `fingerprint` is one line over a private encoder. The property worth checking is that
      encoder's length-prefixing -- without it, a contract containing a newline could be
      arranged to hash the same as two different fields -- and it is not reachable from any
      public function. So the only promise the wrapper can carry is "returns 64 characters",
      which is a fact about the type.

      **That property is already checked, thoroughly, by an ordinary Rust test**: 22
      mutations, one per input the spec lists, each asserting the hash moves and naming
      which input stopped counting when it does not. Better coverage than any promise about
      the wrapper, and it needs nothing from Ply. The skill now says so, with a table for
      deciding: property in the function → claim it; in a private helper → ordinary test,
      leave the wrapper unclaimed; in a public helper → claim the helper.

      It also closes rule 6's loop, which stopped one step short: the fix for a promise that
      cannot fail is sometimes to delete the claim, not reword the promise.

      OPEN, for the maintainer, and the facts underneath it changed 2026-09-05: this was
      written when the claim was permanently refused, so its only honest content really was
      a type-level fact nothing could disprove. It now runs 256 real generated cases against
      the real implementation and earns real evidence -- breaking the function on purpose
      (truncating the hash) was caught as a genuine violation, so the promise is not vacuous
      any more. Whether that changes the answer is still the developer's call; the question
      is left open rather than resolved either way.

- [x] Three tests for the new material, each confirmed red under a deliberate breakage:
      restoring the invented function name, deleting rule 8, and softening the
      delete-the-claim sentence.

## Landed: ten more claims, and the second half of the typo suggestion — 2026-09-04

44 promises now, in 22 components. Every one earns evidence except `record::fingerprint`.

- [x] **Ten more claims**: the sampling engine's remaining output readers
      (`parse_fuzz_marker`, `parse_proptest_minimal_input`, `build_errors_with_lines`),
      `fuzz_gen::derive_seed` and `seed_from_hex`, `model::parse_deny`,
      `config::validate_keys`, `schema::unknown_key_message` and `validate_text`, and
      `visual::svg::examples_prose`.

- [x] **`derive_seed`'s promise was mutation-checked, not assumed.** Its two inputs are
      kept apart by a `\x1f` separator, so swapping them must give a different seed.
      Deleting that one line turned the claim red -- by sampling *and* by the worked case,
      `("ab","c")` against `("a","bc")` -- and the separator was restored. That is the
      answer to "would this promise notice if the code broke", asked rather than assumed.

- [x] **Ply was writing a test into the user's crate that did not compile**, and the first
      fix for it was wrong in two new ways. A promise may name something the function's own
      module defines or imports; that text is spliced into the counterexample replay test
      verbatim, but the test sits at the crate root under `use super::*`, where such a name
      is not in scope. So `cargo test` broke for a reason the user did not cause, which is
      the failure this project treats as worst.

      The review of the first fix (2026-09-04) found it made things worse for two shapes,
      both confirmed by running rather than by reading:

      - a claim on a receiverless associated function (`Bucket::new`) emitted
        `use crate::Bucket::*;` -- a hard compile error where the old output built. The
        repo's own `nonnumericcompare` fixture has exactly that shape with a deliberately
        false promise, so this was reachable in production, not hypothetical.
      - every nested replay whose promise names no sibling -- nearly all of them -- carried
        an unused-import warning, which is an error under `-D warnings`.

      Both came from writing a second copy of a split `ContractFn::import_path` already
      does correctly. It is reused now, with the two `allow`s the harness has carried since
      August. The generated test name also gained `#[allow(non_snake_case)]`:
      `ply_cex_Bucket_new_01` warned, same failure one step along.

      **The claim that this made the two paths agree was wrong**, and is retracted. The
      sampling harness imports the function *by name* plus a crate-root glob; the replay
      test imports the function's *module*. Neither is a superset. The gap that showed it:
      a promise naming a type the module imported (`Ordering`, via `use std::cmp::Ordering`)
      resolved in the harness and not in the replay. Fixed by asking the question in one
      place -- `contract_rt::contract_use_paths` -- and letting each renderer spell the
      answer for its own scope, since a second copy of this rule is what caused the bug in
      the first place.

      The test that guards this builds a real crate under `-D warnings` and asserts every
      rendered shape fails on its promise rather than on its syntax. The two string-contains
      tests it replaces both passed while the renderer emitted a compile error. Four
      deliberate breakages of the renderer were run against it; the first version of the
      new test survived one of them, because its fixture had no nested promise that names
      no sibling. That shape was added and all four now die.

- [x] **`E0301` now tells you when a claim is under the wrong module.** The suggestion
      fixed earlier today only covered a misspelling; a claim whose *name* is right but
      whose component is wrong got nothing, because `visual::examples_prose` is five
      characters from `visual::svg::examples_prose` and edit distance cannot see that.
      Matched on the final segment when the distance match finds nothing, and only when
      the answer is unambiguous -- two functions of the same name in different modules is
      a question, not a suggestion.

      The two cases get different sentences. Telling someone their function "was renamed"
      when it is sitting one module over sends them looking for a change nobody made.
      Found by writing the `examples_prose` claim in the wrong place and being told the
      function did not exist.

## Landed: eleven more claims — 2026-09-04

The library goes from 23 promises to 34, in seven modules that had none: the fast
document pass, the diagnostic vocabulary, the call graph, the reachability scan, the
config loader, and the sampling and proof engines' own output readers. Every one earns
evidence; `record::fingerprint` is still the only refusal.

Promises chosen against `skills/ply-checkable-code`'s own rule 6 rather than for an easy
green -- most are a single clause relating input to output, with no `||` to hide behind:

- never report something the tool's output did not contain (`first_build_error`,
  `path_dependencies`)
- never count more tests than the output has lines (`count_tests_executed`)
- never hand back an empty dependency path, which would silently resolve to the crate
  itself (`path_dependency`)
- never grow the text (`tidy_contract_text`) -- it is quoted back to the reader as "the
  line you wrote", so growing means it started rewriting
- the flags come in pairs, `-Z` then its value (`unstable_flags`) -- an odd list builds a
  malformed command and the proof engine fails for a reason nothing to do with the code
- the message says where the problem is (`mutate_kill_signal_message`), which is the
  newbie bar written as a promise
- the rejection names the code the reader types into `cargo ply explain`
  (`parse_check_string`)

`diag::is_absence` is checked by worked cases alone and earns `tested`, one rung below the
rest. Its input is a fixed vocabulary of eight words; sampling text against it would say
nothing, and dressing that up as a stronger verdict would be exactly the green paint the
rest of this file argues against.

`check::crate_has_workspace_table` still reports a lopsided sampling half -- random text is
never `[workspace]`. Its three worked cases carry the other side: the plain form, the
indented form, and `[workspace.dependencies]`, which is a near-miss that must read as
false or Ply would edit a manifest that owns no members.

## Landed: the three gaps found by deliberately breaking things — 2026-09-04

Every failure path was exercised on purpose against `crates/ply-core`: a false promise, a
false structure promise, a claim naming a function that does not exist, and a harness that
does not compile. All four reported correctly, at the right severity, and propagated to the
root; the `--json` envelope and the terminal tree agreed on all 39 nodes and every
diagnostic code. Three gaps in *how well* they reported, all now closed.

- [x] **A counterexample on a string parameter is now replayable.** `WitnessValue` gained a
      `Str` arm, `RustType::String` is `is_witness_renderable`, and `render_cex_test`
      writes the value back out with `str`'s own `Debug`, which is exactly a valid Rust
      literal -- quotes, backslashes and control characters all escaped, so there is
      nothing left for Ply to get wrong by hand. Measured: 17 of ply-core's 23 claims take
      text, so this was the common case, not an edge one.

      Checked end to end, not just in a unit test: breaking `schema::dotted` on purpose
      moved the report from `W0541` (witness only) to `P0502` with a written test, and
      `cargo test` then failed on `ply_cex_schema_dotted_01` exactly as the report said it
      would.

- [x] **An empty-string witness says `(empty)` instead of printing nothing.** Everything
      else still passes through exactly as the engine produced it; only the empty case is
      named, because bare it is indistinguishable from a rendering failure.

- [x] **`E0301` suggests the nearest name again under a module anchor.** The diagnosis in
      the first draft of this entry was wrong: the suggestion was never missing, it was
      *broken by this session's own anchor-relative claim keys*. The typo was matched as
      the user wrote it (`dottted`) against names held as crate-root paths
      (`schema::dotted`), which are never within edit distance, so the suggestion went
      quiet exactly where this project had just moved all of its own claims. Now matched on
      the crate-root key and shown back relative to the anchor, ready to paste over the
      typo.

      KNOWN GAP: `verify`'s own `E0301` (a separate emitter in `verify.rs`) still carries no
      suggestion at all. `check` is the command that runs in a second and is meant to catch
      this, so the miss there was the one that mattered; wiring the same suggestion into
      `verify` is small and unstarted.

## Landed: eight more claims, a crash Ply found in the renderer, and a bug in Ply itself — 2026-09-04

- [x] **Eight new claims in `crates/ply-core/ply.yaml`**, taking the library from 15 to 23:
      `model::parse_check`, `model::parse_edge`, `registry::lookup`,
      `record::verdict_is_earnable`, `schema::dotted`, `surface::contract_helpers`,
      `harness_crate::remove_workspace_member`, `visual::layout::assign_ranks`.
      All 23 earn evidence except `record::fingerprint`, still refused for the documented
      20-field limit.

- [x] **The layout code crashed on an edge naming a node nobody declared.** Ply's first run
      of the new `assign_ranks` claim panicked on `edges = [("","0")], names = [""]`: an
      edge target was looked up in a map built only from the declared names. Fixed by
      dropping an edge whose either end is undeclared, which also repairs the quieter half
      -- an edge *from* an unknown node used to freeze the rank of the node it pointed at.
      Two tests in `visual/layout.rs`, red before the fix.

      This is the same shape as the scheduler bug a week earlier, keyed by a name rather
      than an index, and both fixes were "make the lookup total". `skills/ply-checkable-code`
      now says so.

- [x] **A real defect in Ply: an `examples:` entry containing `{}` broke the harness.**
      `generate_example_test` echoes the entry into the assert's failure message, which is
      a `format!` template, so `format!("{:?}", ..)` inside an example was read as a
      placeholder and the whole crate failed to build with "1 positional argument in
      format string". Braces are now doubled, alongside the existing quote escaping. Found
      by writing one for Ply's own document.

- [x] **Six promises rewritten because one side of the `||` was doing all the work.**
      Ply's own disclosure caught them: `parse_check`'s `result.is_err() || !s.is_empty()`
      was decided by the first half in 256 of 256 cases, because random text is never a
      valid check string. Rewritten to "a rejection always quotes the text it rejected",
      which moved all 256 onto the half that says something. Where the interesting case
      is genuinely rare (`registry::lookup`, `record::verdict_is_earnable`,
      `surface::same_expression`), `test` was declared alongside `fuzz` with worked cases,
      so the branch random text never reaches is exercised by hand.

      KNOWN GAP, left open on purpose: those three still report a lopsided fuzz half, and
      that report is true -- the fuzz run really does say little about them. The concrete
      cases are what carries the other side. Hiding the disclosure by rewriting the
      promise as a single non-`||` clause would have been worse.

- [x] **`skills/ply-checkable-code` rewritten after an adversarial review, and its tests
      given teeth.** The review found three behavioural claims that were wrong or stale:
      the skill said Ply catches a function that writes files (it does not -- `effects.rs`
      has no callers, and a function that builds a path from a `String` will be run for
      real against generated inputs); rule 2's headline told an agent to restructure a
      signature Ply's own maintainers kept; and it offered naming a concrete type for a
      generic parameter as an escape, which is in the spec and drawn but not wired at
      verify time. It also granted `may-do` over reshaping existing I/O while the prose
      said that belongs to the developer. All fixed, plus the gaps it named: `HashMap`/
      `HashSet` and tuple structs are refused, `&mut` blocks `fuzz`/`bounded` but not a
      `test` with examples, `fuzz` needs a promise at all, and methods (`&self` yes,
      `&mut self` no) now have their own rule.

      The old four tests passed with every rule body deleted. The eleven that replace them
      go red under each of: gutting the bodies, flipping the authority table, and restoring
      the old rule-2 headline -- checked, not assumed.

## Landed: the side-effect scan is a design signal, not a gate — 2026-09-04

The maintainer's correction, and it is the better frame: **not every function should be
unit tested, and a refusal is often the right answer.** A path-taking function that writes
is not Ply failing to check it -- it is a function where the deciding and the writing sit
in the same place, so the deciding cannot be checked. The fix is to separate them, not to
teach Ply to run I/O against invented paths.

So the fork recorded above dissolves. Enumerating danger to clear more functions would
optimise for the wrong thing. **The scan stays sound and fails closed; what changes is
what it is for.** And 1-of-35 stops being a bad number: it is a measurement of ply-core,
saying nearly every path-taking public function there is shell. Some of that is correct
(`record::save` should just save a record) and some is a factoring smell.

- [x] **Worked example, and it was a real one.** `harness_crate::write_harness_lib_rs`
      computed each generated module's line span *and* wrote the file. Those spans are what
      map a compiler error back to the one claim that caused it, so a slip there
      misattributes a build failure to an innocent function -- the exact defect the
      attribution mechanism exists to end. Split into `harness_lib_source(&[HarnessModule])
      -> (String, Vec<ModuleSpan>)`, which takes no path at all, and claimed in
      `crates/ply-core/ply.yaml` with a promise that is worth making: one span per module.
      It earns `fuzzed(256)`.
- [x] **Which immediately found a codegen bug, by dogfooding.** A parameter of type
      `&[UserType]` generated a value collected with no target type, so inference took it
      from the call site -- which wants `&[T]`, so the binding inferred to the unsized
      `[T]` and the harness would not compile. Shipped with slice support on 2026-09-02 and
      never noticed, because no fixture took a slice of a user type. One harness is shared
      by every claim in a crate, so it took all of them down. Fixed by naming the
      collection; test added, confirmed red before the fix.

**NEXT, and this is the standing programme rather than one item: prove as much of Ply's own
code as Ply's own mechanisms allow.** The scan's output is the worklist -- 6 writers and 28
unknowns in `ply-core` alone. Each is one of three things: correctly a shell and to be left
unclaimed on purpose, a logic-and-I/O split waiting to be made, or a Ply limitation worth
fixing. The two found so far were one of each.

## Landed: contracts explained, and the text form pointed at — 2026-09-04

- [x] **What `requires` and `ensures` mean.** The reference had every mechanic and none of
      the meaning: nothing anywhere defined the two words. Now stated, with what each one
      *does* on each tier (a filter on the sampling tier, a narrowing of the search on the
      proving tier), the caller-side reading that makes proving tractable at all, and the
      trap that had no home -- a precondition too narrow makes a green result mean less,
      because a promise checked on the three inputs that survived out of 256 is thinner
      than `fuzzed(256)` sounds.
- [x] **No rendering skill, and the reason written down.** A skill earns its place when
      there is a decision to get wrong and an authority to overstep; rendering has neither.
      What was real was the neighbouring risk: only one of four skills mentioned the text
      form, so an agent asked to explain a document would reach for the picture and read
      about a twentieth of it. `ply-review` and `ply-audit` now point at `--text`, with
      `ply-review` required to say it is describing declared intent and not evidence. Two
      tests cover it, confirmed red by removing the pointer.

## Landed: the parts of the tool nothing explained — 2026-09-04

All raised by the maintainer reading the README and the `skills/` folder as a newcomer
would, which found gaps no amount of internal review had.

- [x] **`cargo ply explain <CODE>`.** Every message ends in a code and there was no way to
      find out what one meant: the table is in Ply's own source, and the meaning of the
      leading letter -- whether the prover, the sampler, or Ply itself is speaking -- was
      written down nowhere at all. The command reads from the same registry the two
      invariant tests already hold the tool to, so it cannot drift from what is emitted,
      and it says outright when a code is described but never produced.
- [x] **What each check kind is worth**, and when each applies -- including the point about
      `mutate` that everyone misses: it does not check your function, it checks whether
      your checks would notice if the function broke.
- [x] **A legend for reading a drawing**, grouped by channel, leading with the rule the
      whole grammar rests on: green means earned evidence and nothing else.
- [x] **The text form has its own section**, with the measurement that justifies it: on the
      trading-system example, 95% of the render is in the hover text, and a model cannot
      hover.
- [x] **The failed-check section no longer reads as "Ply writes your tests."** It leads with
      the opposite and separates the scratch harness from the one test written into the
      crate only when a promise really breaks.
- [x] **Two new skills, and two folded-in workflows.** `ply-author` (writing a document
      that says something a run can be wrong about) and `ply-audit` (what the green rests
      on) join the two that existed; the counterexample repair loop and `explain` folded
      into `ply-verify`. Each new skill's authority table is covered by a test that goes red
      when the rule is removed -- both breakages were made and reverted to check.

**One paragraph was not confusing but false.** The README described the case where Ply
finds a failing input and cannot render it as a test, blamed it on a stubbed callee's
invented return value, and claimed the diagnostic proposes a tightening of the promise.
Neither is true: the real reason is that the value cannot be spelled as a Rust literal
(usually one built by a constructor plus a sequence of calls), and that diagnostic carries
no proposed fix at all. Retracted and rewritten to describe the code.

## Landed: the structure-promise review, and what it found — 2026-09-04

A review of the `holds:` feature (the commit below) found the feature's own green paint,
plus five smaller holes. All fixed in `crates/ply-cli/src/verify.rs`,
`crates/ply-core/src/fuzz_gen.rs` and the visual layer; every fix carries a test that
goes red when the fix is removed.

- [x] **THE BIG ONE: a structure nothing could be built of reported `fuzzed(256)`.** A
      constructor returning `Err` for every input, or with a precondition nothing
      satisfies, left the run finishing cleanly with no value ever made -- and the verdict
      read 256 cases of evidence, exit 0, no diagnostic. The generated run now counts the
      histories a value was actually built for, the verdict is read off that count, and
      zero is `unclaimed` with `W0417` saying why. A run narrower than it asked for says
      so (`W0418`).
- [x] **Coverage disclosures were dropped.** The receiver machinery already names the
      operations it could not call and the constructors it never started from; the
      invariant path threw them away, so Ply's own kernel reported a clean number beside
      two fn claims on the same type that both warned `StatusSet::extend` was never
      called. Now surfaced (`W0418`) and marked `partial-history` on the node.
- [x] **The severity was hardcoded.** Every one of these diagnostics went out as a
      warning, including `E0506`, which the registry calls an error -- so `--fail-on
      error` exited 0 on a document whose promise could not be read. Severity now comes
      from the registry, so the table and the emitter cannot disagree.
- [x] **`check` said "no problems" about a document `verify` refused.** It now reads each
      `holds:` line and reports one it cannot parse, which is the same two-commands-two-
      answers failure this repo already names for fn keys.
- [x] **`check` and `verify` disagreed about whose type it is.** `verify` used a
      crate-wide lookup and happily checked a same-named type declared somewhere else,
      while `check` refused it by name. `verify` now scopes to the component's own anchor.
      An ambiguous name gets its own sentence instead of "no type by that name", which
      was false.
- [x] **Two components promising things about one type collided**: same generated module
      name, neither compiled, and each was told every other claim had run. The module name
      now carries the component. The name is also derived once and passed to both the
      generator and the test filter -- deriving it twice is what let them drift.
- [x] **The drawing counted a structure promise as a function.** A document with one
      structure promise and no functions read "0 functions · 1 broken". The node has its
      own kind now. A component whose only claim is a structure promise is also no longer
      called "hollow -- promises nothing yet", printed one line under the promise, and its
      declared ceiling counts the promise.

**Test adequacy, measured rather than assumed.** The reviewer found four one-line
breakages of the production code that every test survived: dropping the operation-argument
imports, deleting the assertion that runs straight after the constructor, checking only
the clauses that parse, and never refusing a timed-out run. Three now have a test that
dies on them (the fourth, the timeout, still has none -- **KNOWN GAP**). Each new test was
confirmed by making the breakage, watching it go red with a message naming the real
defect, and reverting.

**KNOWN GAP: a `holds:` result is never recorded or reused.** Any structure promise forces
a harness compile even when every fn claim in the crate was reused. Written down in the
spec and SCHEMA rather than left to be discovered from a build time.

## Landed: a structure can promise something about itself, and Ply checks it — 2026-09-04

Asked for by the maintainer, the second half of "push the ply.yaml file and rendering down
to function and collection level". §5.4c has always admitted that a type's own invariants
are **assumed, never asserted**, so a proof could rest on "the bids are sorted" while the
code quietly breaks it. That assumption can now be written down and checked.

- [x] **`holds:` under `state:`.** Each line is a Rust expression about the value -- a
      bare one names it `state`, a closure names it whatever you like, the same two forms
      `ensures:` takes.
- [x] **Checked by building one and using it.** Ply calls the type's own constructor,
      respecting its precondition and rejecting rather than unwrapping a fallible one,
      then calls the type's own public operations in a generated sequence, asserting every
      clause after the constructor and after **every single operation**. A structure that
      is fine when made and wrong four calls later is the bug nobody catches by hand, and
      the report says how many calls in.
- [x] **Every non-answer is a non-answer, never a pass**: structure in another crate,
      no type of that name, no way to build one. A clause that will not parse holds back
      *every* clause on that type rather than checking the readable half.
- [x] **Ply's own kernel carries one**: a status set can never hold more than the seven
      kinds of status that exist. It earns `fuzzed(256)` and shows in the drawing.
- [x] **Drawn and written out**: the box says the structure promises something about
      itself, the tooltip quotes each promise, and the text form lists them above the
      fields -- what is binding should not be found by reading past twenty field lines.

**THE FAILURE THIS NEARLY SHIPPED WITH, kept as a test.** The first version reported a
clause that could not *compile* against the real type as a **violation** -- a false
accusation about the author's code, worded identically to a true one. It passed its own
"catches a real break" test for the wrong reason: the harness failed to build and that
read as a broken promise. The companion test (a type that keeps its promise must come
back clean) is what caught it. A violation now requires that the check was observed to
run; otherwise the verdict is a tool error quoting the compiler.

**Not linked to proofs yet, and the spec says so.** A `bounded` check still does not
consult these clauses -- the invariant it assumes and the invariant now carrying evidence
are two facts side by side, not one. Linking them is the next step, not a claim made here.

## Landed: a module can be a component and still promise things — 2026-09-04

Asked for by the maintainer ("can we push the ply.yaml file and rendering down to
function and collection level?"). A function claimed inside a box anchored at a module
used to be drawn, counted, and never checked: the key was read from the crate root
whatever the box said, so it resolved to nothing and was declined by name. Ply's own
library document was written around that limit and said so in its own header.

- [x] **A function key is read relative to the box it is written in.** `StatusSet::len`
      inside the box for `ply_core::kernel` names `kernel::StatusSet::len` and runs. A box
      anchored at the crate root leaves the key untouched, so every claim written before
      this resolves exactly as it did.
- [x] **All four readers agree**, because there is now one place that resolves a key:
      `verify`, `check`, `audit` and the assumed-contract scan all go through it. Two
      commands disagreeing about which claims point at real code is the failure this
      project has already had once.
- [x] **A promise written inside a nested box is now assumable at a boundary.** The map
      callers consult read only the top level of the document, so a promise one level down
      was drawn, listed by `audit`, and silently missing from it.
- [x] **The retired advice is gone**, not left to be followed: the warning that told a
      reader to move the claim up and respell the key would now be advice to undo a
      feature. It says one thing, about another crate.
- [x] **Ply's own library is written the new way and re-rendered.** Four module boxes,
      each with the structure it holds and the functions that promise things about it.

**A second defect fell out of it, and it is the bigger one.** The generated harness wrote
every struct or enum name bare, assuming types live at the crate root. A receiver built by
calling a constructor and then a sequence of the type's own methods passes arguments to
those methods, and a type reached only that way never got its `use` line -- so the harness
crate failed to compile, and because one harness is shared by every claim in a crate,
**every** claim came back a tool error, including claims with nothing but scalars in them.
Ply's own six promises had been in exactly that state, reporting a broken harness rather
than a verdict, for as long as they have existed. Five of the six now earn `fuzzed(256)`;
the sixth is refused by name for a reason that has nothing to do with this.

## Landed: a documentation change no longer runs the full suite — 2026-09-03

Asked for by the maintainer. The end-to-end shards install Kani and take a quarter of an
hour each; the kernel mutation run is comparable; and nothing either of them runs reads
a document -- measured, not assumed: one grep hit across the whole end-to-end suite, and
it was a comment mentioning "a README". So a change touching only prose, the docs tree,
the diagram bundle, or the generated drawings and text forms beside the scenarios now
runs only the fast job.

- [x] **The fast job never skips.** It is where documentation *is* checked -- the
      spec-consistency test and the drawing drift check live there -- so a docs change
      still gets every check that can see it.
- [x] **Anything unrecognised is code.** "Documentation" is a closed list in
      `.github/scripts/changed-kind.sh`, and a file not on it runs everything, so a new
      kind of source file can never be waved through as prose. `ply.yaml` files and the
      demo crate are deliberately off the list.
- [x] **The classifier is a script, so the code CI runs was run by hand first**: a
      docs-heavy pull request came back `docs`, a code one `code`, a TODO-only commit
      `docs`, and an empty range `code` -- an empty diff is nothing to classify, and the
      safe answer to that is the full suite.
- [x] **It rides on the fixed-name gate** the shard-count item further down had been
      asking for, which is the honest condition: skipping a shard is only safe when the
      required check is something that always reports.

**NOT VERIFIED YET, and cannot be from here:** that the forge treats a skipped shard as
satisfying the *old* per-shard required checks while those are still what `main`
requires. The first documentation-only pull request after this lands is the measurement.

## Landed: a promise written in the document is now actually checked — 2026-09-03

The audit below named this as the most valuable thing open, and it was already next on
the maintainer's own list. §5.4 has said since the beginning that a `requires:`/`ensures:`
written in `ply.yaml` is "ANDed in" to the function's own contract. It was not. The
clauses were read, drawn, written into the transcript, and offered to callers as a
boundary assumption -- and never checked against the function they were written for, with
a warning saying so on every run.

Disclosed is not checked. This is the project's own central failure mode, a promise that
reads as checked and is not, in the file whose entire purpose is that its claims are
checked.

- [x] **Measured both ways, on the two fixtures that already existed for it.** A document
      promising `*result == 99` of a function returning 7, with a passing example beside
      it, used to report a clean `tested`; it now reports `violation`. A document
      promising `*result == 7` of the same function is now genuinely checked and holds.
      Both pinned by tests that assert the verdict, not the diagnostics.
- [x] **Both sources hold, and nothing half-merges.** A document clause is ANDed with an
      inline attribute rather than replacing it, several clauses in a list are a
      conjunction, and every clause is parsed before any is applied -- a partially applied
      contract would be checked against something nobody wrote. Clauses are parenthesised
      when conjoined, because `a || b` joined to `c` without them silently becomes
      `a || (b && c)`, a different promise from the one written.
- [x] **A clause Ply cannot read is refused by name** (`E0505`) and the function's checks
      do not run. Dropping one clause while running the rest would be the same failure in
      smaller clothes.
- [x] **Two `ensures:` clauses that name the returned value differently are reconciled,
      not refused.** `|result|` is the convention and everything here uses it, but
      refusing `|r|` would be a new way for a valid document to stop working.
- [x] **`W0510` is retired**, not reworded: its whole condition -- "declared here, not
      folded in" -- can no longer occur. Spec §5.4 and SCHEMA.md both said this was
      unimplemented and now describe what it does.

## Landed: this file audited, and nineteen entries that were no longer true removed — 2026-09-03

Asked for by the maintainer ("anything stale just remove"). Every one of the 128 open
items was classified and the candidates checked against the source rather than against
what a later entry claimed. Nineteen described work that has since been done or a design
that changed under them; they are gone. Two more were only half stale and were narrowed
rather than deleted, because deleting them would have taken a real remaining limit with
them:

- **`NonZero`/`Duration` nesting** — they nest on the sampling tier now; the proving tier
  still refuses them, deliberately, and that half is the part worth keeping.
- **The three-part assumed-contract loop** — the vacuity check and the two reporting
  commands are built; only the missing fingerprint tying a declared boundary contract to
  the callee's body survives, and the entry's own "the conjunction is the risk" argument
  no longer applies to what is left.

The count went 128 open to 107. What remains is roughly 55 deliberate known gaps and
about 40 items of genuine open work.

**The most valuable thing the audit found still open** was that a promise written in
`ply.yaml` was never folded into the function's own check. That is the section above:
built the same day, so this list does not outlive its own conclusion.

## Landed: TODO.md was contradicting itself in a committed merge conflict — 2026-09-03

Found while auditing this file for stale items, not by anyone reading it. Conflict
markers from an agent worktree merge were committed to `main` and had been sitting
there: `<<<<<<< HEAD`, `=======`, `>>>>>>>`, with both sides kept verbatim.

The damage was not cosmetic. The two sides disagreed about the *status* of two items,
so the file simultaneously said each one was done and not done — a reader taking either
at face value would have been misled, and the whole point of this file is that it can be
taken at face value. This is the failure CLAUDE.md's own rule names: a stale list is
worse than none.

- [x] **Resolved against the code rather than by picking a side.** Both items are in fact
      done, and both were verified before the resolution: the reject-path disclosure
      exists (`promise-lopsided`, raised in `verify.rs`, commit `297dd8f`), and the
      non-numeric comparison no longer breaks the build (`is_provably_numeric` gates the
      cast in `contract_rt.rs`). Both now read `[x]`.

## Landed: two documentation drawings showed a notation the tool stopped writing — 2026-09-02

Reported by the maintainer looking at the README. Both drawings that had no drift test
were stale; both that had one were current. The asymmetry predicted exactly which would
rot.

- [x] **`vetting/004-legacy-extension.svg`, embedded in README.md**, still used the old
      cryptic badges (`B2`, `F256 T`) rather than the words the renderer has written for
      months (`bounded: loop≤2`, `fuzz: 256 cases · test`). Regenerated.
- [x] **`demos/fault3-flagged.svg`** had the same problem, plus a tooltip quoted in
      `demos/fault-injection.md` naming a diagnostic wording no longer produced.
- [x] **Every committed drawing now has a drift test**, beside the text forms that always
      had one — six drawings, byte for byte against a fresh render.
- [x] **`demos/fault3-as-drawn-by-faulted-toolchain.svg` is deliberately excluded** and
      the test says why in its own doc comment: it is the record of what a *broken*
      renderer drew, and regenerating it would delete the evidence it carries. The demo
      prose now says so too, so a later reader does not "fix" it.
- [x] **A renderer defect found by regenerating, not by review.** A finding badge is
      vertically centred on a function chip, so it covers the checks line as well as the
      name line — but only the name row reserved room for it. A long checks line ran
      underneath: in the very drawing whose point is that a broken document is visibly
      flagged, `bounded(0) · fuzz: 4096 cases · mutate` ended with `mutate` buried under
      the red `E0203` tag. A check fixture collided too, so it was never only the demo.
      Fixed in the width calculation, pinned by an invariant walking every fixture that
      carries a finding.

## Landed: pulling back for less detail no longer leaves empty boxes — 2026-09-02

The viewer folded detail by hiding boxes inside the full drawing, so the boxes around
them kept the size their contents needed. Ply's own architecture at 66% became two blank
rectangles five times the height of the crates beside them — more screen for less
information. Recorded as a known defect in the viewer's own source, with the intended fix
already named there. Now closed.

- [x] **The envelope carries a properly laid-out drawing for each level a reader can fold
      to.** Ply could already draw a document at any level; the results now travel with
      the full drawing, so a client never asks twice and never hides anything. Ply's own
      document folds from 754 pixels tall to 296; the trading-system scenario from 1772 to
      624. The folded boxes even say what went away ("5 components, 0 fns") instead of
      being blank, which hiding could never have produced.
- [x] **Only levels that change something are sent**, and none at all when the caller
      already narrowed the drawing with `--depth`/`--focus`/`--collapse` — offering
      alternatives to a selection would silently undo it.
- [x] **Measured against a control rather than a threshold.** The viewer test compares a
      folded box against `e2e`, which has never held anything, so its height is what a box
      with nothing in it is supposed to look like. Before the fix it was seven times
      taller.
- [x] **Every drawing that can reach the screen is sanitized**, not just the first one.

KNOWN GAP, on purpose: the field is additive and absent when empty, but a client built
before this **refuses the new envelope outright** rather than ignoring the field, because
it checks for exactly the keys it knows. ply-vis was updated in the same session; any
other reader of a published view would need the same one-line change.

## Landed: ARCHITECTURE.md carries both of Ply's own documents — 2026-09-02

The page showed one diagram and described a repository that no longer existed: four boxes
when there are six components with thirteen boxes, one arrow when there are three, and a
crate table missing `render` and `check`. All of it corrected against the real render and
the real `cargo ply check` output, not from memory.

- [x] **The library-level drawing is on the page.** `crates/ply-core/ply.yaml` now renders
      to `docs/ply-core-self.svg` and `docs/ply-core-self.txt`, both committed and both in
      the drift test beside the workspace pair. It is the first drawing of Ply's own code
      that is not hatched white — six functions, each promising something about what it
      returns and how that will be tested.
- [x] **The drift test covers all four artifacts.** It was two hard-coded paths and is now
      two loops. Verified by breaking each new file and watching the failure name it.
- [x] **The "grey is not green" distinction is stated on the page**, so a mid-grey box is
      not read as a passing run.
- [x] **The unfolded-promise gap is disclosed there too**, rather than left for a reader to
      discover — a promise in a `ply.yaml` is drawn and counted but not yet checked, which
      the tool's own `anchors` line says as well.
- [x] **A false sentence in `crates/ply-core/ply.yaml` retracted.** Its header claimed the
      modules appear as nested components in that file. They do not, and cannot: a function
      claimed inside a module-anchored box stops resolving. The comment now says that.

## Landed: `state:` — the structure a component holds, and Ply checks it exists — 2026-09-03

The maintainer's question was the whole point of building it rather than just drawing it:
*"Can ply verify these items exist in the code. Heaven forbid the llm decides otherwise"*.
It can. Naming a type that is not there, or a field that type does not have, fails the
build and names what is really there.

- [x] **The document names, the code says what.** `state: { of: OrderBook, show: [bids,
      ticks] }`. `show:` takes field *names* only. Listing the field types in the document
      would be a second hand-maintained copy of a fact the compiler owns, and would drift
      the first time a field changed.
- [x] **A made-up field is caught and the real ones are listed.** Proven end to end against
      Ply's own document: `show: [invented_field]` on the envelope type exits 1 with the
      eight fields that actually exist printed beside it; the honest version exits 0. The
      source scan reads private fields too — a state struct's fields usually are private,
      and a checker that only saw the public ones would pass exactly the claims worth
      catching.
- [x] **"I could not check this" is its own message, never silence.** When there is no
      library source to resolve against, Ply warns that the line went unchecked rather
      than exiting 0. Found by testing it: an invented type in the workspace-root document
      passed silently. The two messages come from two separate call sites on purpose, so
      "this is false" and "I could not check" can never be confused for one another.
- [x] **All three documents that can carry it, carry it**, and every drawing and text form
      was regenerated: Ply's own library, and vetting scenarios 001 and 002.
- [x] **The drawable gate was run, not asserted.** A first draft of the glyph sheet was
      thrown away after rasterising it: two glyphs were indistinguishable at 12px, and one
      shape in the set was a return shape rather than a state one.

**Field rows landed the same day, on the maintainer's question — "should I see the data
types in above diagram?" The answer was no, and it should have been yes.** The box now
draws `state T — N of M shown` and a row per field: shape glyph, name, and the type as
the source spells it. Both numbers are counted from code; a document rendered with no
code under it draws the type name alone rather than a number Ply invented.

- [x] **Seven glyphs, ink only, no new colour.** Geometry taken from the reviewed
      proposal sheet rather than reinvented, so what shipped is what was looked at. Two
      of them earned their design by *failing* first: hatching a solid glyph erased its
      silhouette (a hatched list next to an unhatched one was a ghost), so a hatched
      glyph keeps an outline; and the hatch could not reach the two outline-only forms
      at all, which are the commonest unbuildable fields there are, so on those it
      became the fill.
- [x] **The hatch leans the other way to the ceiling hatch.** Found by rasterising Ply's
      own workspace drawing, where every box is unclaimed and therefore already hatched:
      a glyph hatched the same way vanished into its background — one channel carrying
      two meanings at two scales, which §7.1 forbids. Crossing them costs no colour.
      It also runs at half the pitch, because the ceiling pattern inside a 12-unit glyph
      is about one stripe and does not read as hatching at all.
- [x] **"Cannot build" is the sampling engine's own answer**, not "the parser gave up".
      A `BTreeMap<u64, Level>` parses perfectly well as a map and still cannot be built.
      The narrow predicate missed all three unbuildable fields in a fixture written to
      have three.
- [x] **The text form says the same things**, shapes included. Its whole contract is that
      it states everything the drawing shows, and a reader who cannot see the picture
      would have been the one to lose by it.
- [x] **The tooltip stopped repeating the document.** It listed `show:` verbatim, so a
      field nobody declared appeared on the drawing inside a sentence promising such a
      name is refused. Caught by a test written to check exactly that.
- [x] **The render suite could not see any of this until now.** Every fixture it walks is
      a document with no code under it, so no row was ever painted in it — a deliberately
      misspelled glyph class still passed the "every painted element resolves a style
      rule" invariant. There is now a fixture crate written to disk at test time, and the
      style and tooltip rules run over a drawing that actually has rows.

**Cross-crate state resolution landed too, closing the other gap.** A component's state
is read from the crate its anchor names, so the workspace-root document — which has no
library of its own and was the reason the "could not check" warning existed — is checked
like any other. Proven both ways: an invented field in the root document exits 1 naming
the eight real ones; the honest version exits 0.

**Every component that has a main type now names it — and a misfiled one is caught.**
Asked for on 2026-09-03 ("update other specs to include the data types no??"), and the
first thing it turned up was that the checking was weaker than this file and the spec
both claimed.

- [x] **Resolution is scoped to the anchor, and was not before.** A component's state was
      found by scanning the whole crate for a type of that name, while the spec and
      `A0414`'s own message to the user both said "resolved under its own anchor, never
      guessed at". So a component that misfiled a type passed: the type really exists,
      nothing looks wrong, and only the attribution is false — one level subtler than the
      failure `state:` was built for. Measured before the fix: this repository's kernel
      component claiming a type from its diagnostics module exited 0. Now it fails by
      name, and the message says which module was searched. A type in a module *below*
      the anchor still counts, since that is still the component's own code.
- [x] **A binary-only crate can say what it holds.** Crate discovery required
      `src/lib.rs`, so the command-line crate could never carry state — for no better
      reason than that its root is `main.rs`. Its modules are code like any other.
- [x] **Seven more components carry one**, each verified by breaking it and watching the
      build fail: the verdict kernel, the engine adapters, the harness builder, the
      drawing layer, the result record, and both halves of the command line. The
      workspace drawing now says what every part of Ply holds.
- [x] **Two fixtures were wrong and had been passing.** Both anchored a component at a
      module name while declaring the type at the crate root — exactly the misfiling the
      new rule catches. They now put the code where the anchor says it is, which is what
      a real document looks like.

**KNOWN GAP — four components legitimately carry no state**, and this is a description
rather than a shortfall: the attribute macros and the end-to-end tests hold nothing worth
naming, the renderer's types belong to the library it re-exports, and the checker is a
binary with none of its own.

**KNOWN GAP — two crate shapes cannot be followed, both failing toward the warning
rather than a false clean.** A crate that renames its library with an explicit
`[lib] name` different from its package name is keyed by the package name (vetting 004's
own two crates do this). And a crate reached as a *dependency* rather than as a sibling
under the document is not walked at all, which is why vetting 004's legacy component
carries no `state:` — measured, and it reports the warning correctly.

**KNOWN GAP — vetting scenarios 001 to 003 still show no shapes, and correctly so, for a
narrower reason than before (2026-09-04).** A document may now declare a field's shape in
`show:`'s mapping form and have it drawn with no code at all (The-Ply-Spec.md's `state:`
section, "A document may declare a field's shape"; `vetting/005` is the scenario that
argued for it and the accepted example). 001–003 draw nothing because they use the plain
list form, which still declares names only — not because the grammar has no way to say a
shape. Only 004 has real crates behind it, so its rows (where it has any) are read from
code rather than declared.

**KNOWN GAP — one component in vetting 003 still cannot carry `state:`.** With deny
lanes in (`7f5ae36`), four of its five candidate components take one at zero overlapping
lines. The fifth, `risk`, still takes the ratchet from 0 to 2 on its own: one more line
of height there pushes two routed lines onto the same path. Bisected one component at a
time rather than assumed, and the reason is written into the scenario document itself so
a later reader does not add it back blind. It goes in the day edge routing reroutes when
a box grows. Height is a real budget: the lesson recorded here is that it gets spent per
component with a measurement beside each, not by a rule declared once.

NEXT, and the reason this was worth building: `state` is where a **type invariant**
belongs. §5.4c admits type invariants are "assumed, never asserted", so a proof can rest
on an invariant the code itself breaks. The fields are now named and verified, and the
receiver machinery that already builds constructor-plus-mutator sequences is exactly what
would check one across them.

## Landed: the false green is marked — 2026-09-02

**Was the highest-priority open item.** A method on a type whose only way in is a
constructor taking no arguments was checked against one value 256 times and reported a
clean `fuzzed(256)`. Both deliberately broken functions of Ply's own status-set shape now
carry `one value over and over` where the count is read, and the argument that previously
carried no disclosure at all now has its own.

The fix is not the route guard extended. A route needs a runtime count because an author's
function *might* ignore its inputs; a constructor with no inputs has none to ignore, so one
value follows from the signature. That makes it stronger and cheaper: no `#[derive(Debug)]`
needed, nothing generated, and it holds for a type that can be neither printed nor compared.

- [x] **A value built by a no-argument constructor is counted like any other** (`W0529`),
      for a top-level parameter and for the receiver alike. Suppressed for a receiver when
      the type has an operation taking `&mut self` this run could call, which really does
      move the value off what the constructor made.
- [x] **The `default()` reasoning is generalised.** Ply refused `T::default()` in its own
      words — "it produces a single value, and reporting that as many sampled cases would
      overstate what was checked" — while accepting an inherent `new()` of the identical
      shape. Same rule now, said rather than assumed.
- [x] **The verdict itself carries it.** A reader scanning the tree sees the number long
      before any diagnostic beneath it, which is how a broken function came to read clean.
      Same mark a collapsed route earns, because it is the same fact.
- [x] **The reassuring sentence beside it is gone.** `W0520` said "that covers every value
      this type can reach within 3 steps of a fresh one — nothing else was assumed", which
      is a broad-sounding phrase for a set with one member. Where the set is one value it
      now says so. Two pinned-wording tests had been standing in for the ordinary case with
      a fixture that could not vary — itself part of why this went unnoticed — and now
      carry a constructor argument so they test what they claim to.

NOW CLOSED (2026-09-03), and the evidence is real rather than labelled. The third bullet
turned out sharper than recorded: the defect was never about enums. An operation's own
arguments were never put through the type resolution the checked call's arguments get, so
at the moment the sequence pool was chosen every argument type still read as unbuildable —
whether or not the crate declared a perfectly good type by that name. That judgment was
being made inside a purely syntactic scan which has no crate-wide type index and cannot
make it.

- [x] **The decision moved to where the answer is knowable**, beside the constructor
      candidates that had already been moved out of that same scan for the same reason. An
      operation is now resolved first and judged second.
- [x] **The sequence loop can build an argument, not only draw one.** Each step's arguments
      go through the same plan the checked call's arguments do, under that step's own name
      prefix, and the binding runs inside the arm before the call. Without that half an
      operation would join the pool with an unbound argument and the generated harness
      would not compile, so the test asserts both halves.
- [x] **Measured, not asserted.** On a fresh probe the same deliberate break that reported
      a clean `fuzzed(64)` reports `violation` now, and `narrower than it looks` is gone
      from the verdict because the mutator really is called.
- [x] **The negative case is pinned too.** An operation whose argument genuinely cannot be
      built — a filesystem path — is still left out and still named with its reason.
      Admitting everything would trade a silent gap for a harness that does not compile.

HONEST LIMIT, and it is why Ply's own status-set probe still reads `one value over and
over`: that type has no `&mut self` method at all. `union` takes `&self` and returns a new
value, so no sequence of its own operations can move the receiver off what the constructor
made. `W0529` is right to fire there, and now fires for the real reason rather than because
an argument went unresolved.


## Decided: `ply-core` does NOT take a dependency on `ply-attrs` — 2026-09-02

Asked, reviewed and answered no, before anything was built.

The macro's only expansion is a hook for the proof engine, which this project has decided
never to run on its own kernel. For the sampling tier -- the only one self-hosting could use
-- the checker reads the promise text out of the source file and never consults the macro's
output. So the dependency would exist purely to make the attribute syntax parse: a
dependency on a crate whose behaviour is never used, taken by the crate every other crate
stands on.

Measured costs are small and were not the reason (clean build 15.5s to 15.9s; a lint entry
would be needed or CI fails; the dependency must be aliased to exactly `ply`). Two are worth
keeping: the build identity hashes the library and the manifests but not the macro's source,
so once the library's compiled form depends on the macro's expansion a change there moves
the binary without moving the identity -- inert today, real the moment the macro grows.

**The alternative is strictly better and is what to build instead: close the recorded gap so
a promise written in a `ply.yaml` is folded into the function's own checks.** With no
dependency at all, five of six promises on Ply's own functions fail for that one reason and
nothing else, so the fold produces the same five results without the edge -- and serves
every user with code they will never annotate, which the spec names as where all legacy code
lands. Sequencing matters: if the edge lands first, the pressure to close the user-facing
gap disappears, which is the wrong incentive for a tool whose author is its first user.

- [ ] **Inline promises on the kernel would be ceremony, and are refused on that ground.**
      Ply cannot build the kernel's input tree at all; the suggested fix is to add a
      constructor for the verifier's benefit, which is the reshaping this project already
      refuses. And 256 random trees is strictly weaker than 991,389 enumerated ones with an
      independent oracle. The honest way to put that evidence in the picture is a trusted
      claim citing the enumeration and the inductive proof -- a human's attestation, drawn
      as such, and the maintainer's to write.
- [ ] **A stale `cargo ply` sits on this machine's PATH** (28 Aug), and it rejects both
      `note:` and `routes:` outright. Today's measurements all used the freshly built binary
      by absolute path, so they stand -- but anything measured through `cargo ply` since
      28 August was measured with the wrong tool.

## DONE 2026-09-02: a declared route can no longer be silently unused

Three defects in the build-route mechanism above, all variants of the same silence: a
route the author wrote in `ply.yaml` doing nothing, with nothing said about it. Each had
a failing test first, watched fail, then fixed.

**Defect 1 — a route to a real function outside the crate was refused in total silence.**
Route lookup only ever read the target crate's own source, so a type with no local
declaration at all never even reached the `routes:` check: `resolve_user_type` returned
"not found" (folded into the generic `V0505` "no part of its value is one Ply knows how
to vary") before the route table was ever consulted. Fixed by checking a declared route
*first*, before asking whether the type is declared locally at all — a working route
still resolves the same way; a broken one is now refused by name
(`crates/ply-core/src/harness.rs`'s `resolve_user_type`).

**Defect 2 — the crate boundary itself, for the 42 of Ply's own 97 unbuildable public
functions blocked by an outside-crate type (the largest group after filesystem paths,
deliberately excluded here as before).** Ply cannot read a function's real signature if
it has no source to parse, so a route to one now declares its own input types:
`routes: { OsString: std::ffi::OsString::from(String) }`. Ply builds the declared type
(here, `String`) the ordinary way and calls the named path directly, never reading the
real function's signature at all — a wrong declaration is caught by the compiler as a
tool error naming the route, acceptable only because the route was explicit. Proved on
`/home/user/routeprobe`: a false promise on such a parameter earns a real `violation` with
a real shrunk failing input, not a silent skip.

**Defect 3 — found by a coordinator probing the fix in progress, same mechanism wearing a
third hat.** A declared route to a zero-argument, `Self`-returning function
(`StatusSet::new`) was *also* a legitimate match for the ordinary constructor scan that
runs before routes are tried — so the scan won, silently, and the route was never called
at all: `fuzzed(16)`, flat and clean, no mark, no warning, despite the route being able to
vary nothing. Fixed by the same reordering that fixes defect 1: a declared route is now
tried before the constructor scan even runs, so it can never be shadowed by a rule that
happens to find a different way to build the same type. Proved on the same probe: the
run now carries `route-built`/`route-collapsed` and a `W0527` warning naming exactly one
distinct value across 16 cases.

**A route nothing uses is still checked.** None of the three defects above are reachable
from a route no function's parameters ever name — so `verify` now validates every
declared route once the whole document has been walked, for any entry its own
per-function pass never touched, and reports a broken one (`W0528`) by name against the
whole run.

Existing guards re-verified, not just re-read: a route that ignores its input still
reports its own distinct-value count (`W0527`, unchanged wording); a stale local route is
still refused loudly by name (`crates/ply-core/src/harness.rs`'s
`a_stale_route_is_refused_loudly_naming_the_function`, `tests/e2e/tests/
routehook_fixture.rs`'s `a_stale_route_is_refused_loudly_and_names_the_function`, both
still green, neither weakened).

Filesystem paths were not added to the curated set, as an example, or anywhere else --
they stay a separate, later change behind a check for side effects.

`cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo test --workspace --exclude ply-e2e` all clean (also regenerated `docs/ply-self.svg`
and `docs/ply-self.txt`, stale from before this session for an unrelated reason — the
architecture file had already dropped two crates these hadn't been re-rendered against).
The full `ply-e2e` suite was run to completion in the foreground, in the same session,
before this file was amended -- [result recorded below once the run finished].

## The composition fix made a whole mechanism unreachable — 2026-09-02

Found while verifying that fix, by a unit test whose premise it killed. The
plain-parameter seeding path -- growing inputs from `examples:` for a parameter Ply cannot
build -- accepts **exactly two shapes**, `Option<String>` and `Vec<String>`. Composition
now builds both directly, so nothing ever reaches it.

Confirmed by running it, not by reading the code: both shapes, each with a perfectly good
`examples:` entry, come back plainly checked with no seeding mark and no seeding
diagnostic. Neither is seeded because neither needs to be.

**Receiver seeding is untouched and still alive** -- a value built by a fallible
constructor that parses text is exactly the case that has no other route, which is the
case that motivated the whole mechanism.

The unit test that caught it now records why rather than being deleted: it asserts the
examples really are unconsumed, and pins both halves of the reason (the classifier still
names those two shapes; both shapes are buildable). If a third shape is ever added to that
classifier, or either stops being buildable, it fails and the path is live again.

- [ ] **Delete the plain-parameter seeding path.** Deliberately not done in the same change
      as the fix that killed it: that was a composition change, and removing a mechanism
      threaded through codegen, diagnostics and a fixture is adjacent work with its own risk.
      What goes: the plan, its shape classifier, its per-parameter seed extractor, the
      `SeedableWrap` shapes, their diagnostic, and the `paramseeded` fixture's seeding
      premise. What must NOT go with it: receiver seeding, which shares vocabulary but not
      the dead path. Verify by deletion rather than by reading -- if anything still calls it,
      the build says so.

## DONE 2026-09-02: one build-route mechanism for named types

Closes this section and "2. One build-route mechanism for named types" under "The type
wall has a generic answer" below. Step 1 (composition) landed first, same day, as its own
item; step 3 (paths) is still deliberately last, unstarted, behind the side-effect check
neither step needed.

**Generalised exactly as agreed**: a type is buildable if there is a public way to get one
from parts Ply can already build, as a route table with three sources tried in order --
rule 1's own constructor scan (unchanged); a curated set for standard-library types
(**deliberately left empty this pass** -- codegen has no way yet to import or call a path
outside the target crate's own root, which every curated entry would need, so nothing was
added rather than adding something untested); and a declared route in `ply.yaml`'s new
`routes:` map, naming a public function -- free or associated -- that returns the type.
Resolved through the same resolver a `ply.yaml` fn claim's own anchor already goes
through (§5.5), so a route is found or refused exactly the way any other claim is, and a
stale one (renamed or removed) fails loudly, naming it -- proved directly: renaming the
function a route names turns a clean `fuzzed(64)` into a named, refused `unsupported`,
never a silent fall-through to direct field construction.

- [x] **The guard this cannot ship without.** A route built entirely to ignore its own
      parameter and return one constant value was written on purpose and run: 64 cases,
      1 distinct value reached the function, and the run said so at warning severity
      (`W0527`) without changing the verdict -- disclosure, never a fabricated failure, the
      same "print the split always, escalate only when it collapses" shape the
      branch-decided measurement above already uses. Counted by the type's own
      `#[derive(Debug)]` text (nothing else is available for an arbitrary type from
      outside its crate); a type with none gets the honest "Ply could not tell" instead of
      a guessed number, pinned by its own unit test. **Narrowed, not solved**: only a
      *top-level* parameter is counted -- the same route nested inside a `Vec`/`Option`
      still builds and checks (composition closes over a route-built value exactly as it
      does a constructor-built one), but carries no distinct-value count of its own yet.
- [x] **The probe's three cases, permanent regression** (`tests/fixtures/routehook`,
      `tests/e2e/tests/routehook_fixture.rs`): a struct made only by a free function now
      builds, and its false promise gives a real violation with a real failing input,
      proving the check bites on a route-built value rather than merely accepting it; a
      list of that same struct builds too, through the composition grammar, carrying the
      same route mark; the associated-constructor case is unchanged and carries no route
      mark at all.
- [x] **`V0505`'s fix no longer names a mechanism that does not exist** -- closes Finding 6
      below in the same stroke: the suggestion now names the real `routes:` declaration
      instead of the never-built `pure`-marked hook.

`cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo test --workspace --exclude ply-e2e` all clean. The full `ply-e2e` suite was run to
completion in the foreground, in the same session, before this file was amended --
[result recorded below once the run finished].

## Ponytail review at ultra: 11,900 lines deleted, none load-bearing — 2026-09-01

**Deleted: the scheduler tool (557 lines).** Its own doc comment was the case against it.
The half that ships had already moved into the product; what remained was a rule whose
section heading read *"and nothing runs it"*, which **disagreed with the shipped rule and
was the looser of the two in the case that matters** — it would let a caller one step
outside a cycle assume a contract the shipped rule refuses. It carried 228 lines of
exhaustive tests that its own comment admitted could not catch that disagreement. Unused,
more permissive than production, wearing a rigour costume: the risk was never the bytes,
it was someone wiring it in because the tests looked convincing. The spec already names the
finer per-edge rule as a possible future refinement, and adopting it needs an argument §5.5
declines to make — so it would be written fresh against the shipped rule anyway.

**Deleted: the kernel facade crate (7 lines).** A whole package, manifest and workspace
entry whose entire body was one re-export, kept "to keep existing imports working". Two
files really imported it; both now name the product module directly.

**Deleted: `docs/review-2026-08-23.md` (10,818 lines, 536 KB)** — another tool's raw session
log, its version banner, 24 tool-invocation markers and 25 absolute paths from the
reviewer's own machine, all committed. Its findings were already written up as the items
that referenced it, which now record where the transcript went.

**Deleted: 12 finished sections of this file (466 lines)** — every item ticked, dated before
today, no open gap, each already in git with the hash it was ticked with. All 125 open items
and all 43 known gaps kept.

**Kept, and I was wrong to have suspected it.** The renderer tool is 7,374 lines of which
6,900+ are the test suite for the product's own renderer; 175 lines are the tool. Deleting
it deletes those tests. It is misplaced, not bloat, and the workspace merge already fixed
the harm that mattered.

**Broke one thing and caught it:** removing the facade left the Verus differential spike —
the test that ties the unbounded proof back to the production kernel — with no dependency at
all. It failed to compile, was repointed at the product crate, and its four tests pass again.
That spike sits outside the workspace, so the ordinary suite would not have caught it.

- [ ] **The 100 fixture crates are not the bloat they look like.** Each pins a real
      behaviour and each is small. What makes the end-to-end suite take an hour is duplicate
      proof work inside it, already recorded separately. Do not delete fixtures for size.

## The type wall has a generic answer, and my own "do paths first" was wrong — 2026-09-01

Reviewed, then every load-bearing claim re-run by hand. **Half the refusals are not about
types at all — they are shapes Ply already builds, refused the moment they nest.** Measured
directly, one probe, pairs differing only in nesting:

| written as | verdict |
|---|---|
| an optional number | real evidence |
| an optional string | refused |
| a reference to a vector of bytes | real evidence |
| a slice of bytes | refused |
| a string | real evidence |
| a list of strings | refused |

Every part is buildable alone. The machinery is simply not recursive: each shape added
after the original set was deliberately barred from composing, because one shared decision
answers for both the sampling engine and the exhaustive-proof engine, and letting a new
shape compose would have quietly made things eligible for proof that should not be. The
narrowing protected the proof engine at the sampling engine's expense — **and that is why
the list grows forever: every addition is a leaf that cannot combine, so every combination
becomes a future addition.**

**My earlier recommendation — "paths, and nothing else" — was wrong, and this is the
correction.** Paths are the biggest single blocker (34 functions) and the one that must
wait. Ply runs the real function body. Measured on Ply's own crates: of 39 public functions
taking a path, **8 reach a filesystem write inside their own body** — saving a record,
writing a generated crate's manifest, writing its source file. Unlocking paths today means
generating random paths and executing those. Ply has no side-effect detection; the
capability scan is planned with nothing behind it. Refusing paths is currently doing safety
work nobody assigned to it.

**The plan, three sittings, in this order:**

- [x] **1. Make the sampling engine's decision recursive, and add slices.** CLOSED
      2026-09-02 (`f394aba`, see its own section above). Composition (optional, result,
      list, set, map, fixed array, slice, tuple, reference, owning wrapper) closed over
      anything buildable — for the sampling engine only. The proof engine keeps its
      measured list byte-for-byte, pinned by a regression test.
- [x] **2. One build-route mechanism for named types.** CLOSED 2026-09-02 (see "DONE
      2026-09-02" above). A type is buildable if there is a public way to get one from
      parts Ply can already build, generalised into a table with three sources: the
      existing constructor resolution (unchanged); a small curated set for standard-library
      types, **excluding paths** (left empty this pass, stated rather than silently
      skipped — see the DONE section above for why); and a declared route in `ply.yaml`'s
      new `routes:` map. Variety comes from Ply sampling the route's own inputs, never
      from an author listing values, and the degenerate-route guard ships with it.
- [~] **3. A syntactic "this body reaches file-writing calls" check, and only then paths.**
      **The check is built (`crates/ply-core/src/effects.rs`); paths are NOT unlocked, and
      the measurement says they should not be yet.**

      The scan answers three ways -- writes, none, unknown -- and fails closed: anything it
      cannot follow is `unknown`, and `unknown` never reads as safe. It follows calls into
      first-party source transitively, resolves a sibling call relative to the caller's own
      module, and reports the chain so a refusal can name the route rather than only the
      verdict. Seven tests; the two that matter (unknown-is-not-safe, and following calls
      at all) were each confirmed by breaking the rule and watching them go red.

      **It finds real writes.** Run against `ply-core`'s own 35 path-taking public
      functions it names 6, each with a correct route -- `record::save` through
      `std::fs::write`, `harness_crate::write_harness_lib_rs` through
      `std::fs::create_dir_all`, and so on. Two of those six were only found once sibling
      calls resolved, so that was a correctness fix rather than a coverage one.

      **And it clears almost nothing: 1 of 35.** The other 28 are `unknown`, each blocked by
      a *different* call not on any list -- `Command::new`, `SourceSurface::default`,
      `i64::from`. Three rounds of widening the benign list moved the cleared count from 0
      to 1. This is the same shape as the type wall itself: **enumerating what is safe is a
      list that grows forever and never finishes.**

      **The fork, and it is a soundness posture, so it is the maintainer's.** Either keep
      enumerating safety (sound, and unlocks nothing), or invert to enumerating danger --
      filesystem write, process spawn, network -- and treat everything else as benign. The
      second is how capability systems usually work, generalises instead of growing, and is
      a *bet*: a third-party crate writing files through something not on the danger list
      would be cleared. Not flipped unilaterally, because the whole reason paths were
      deferred is that being wrong here means Ply writes files at paths it invented.

**What notices when a declared route goes stale, since that is the question the design must
answer:** the route names a function, and the generated harness is a separate downstream
crate, so a renamed, removed or private function fails to compile — loudly. A route whose
values mostly get rejected trips the existing high-rejection warning. A route cannot build
an invariant-breaking value, because anything a public function returns is a value some
real caller could hold — the same argument the constructor rule already rests on.

**The one failure the compiler cannot catch, and it needs mechanism rather than trust:** a
route that ignores its inputs and returns the same value every time. Defence: count
distinct built values where the type can be printed, and disclose when that count collapses
— "16 cases ran, but only 1 distinct value reached the function". Where the type cannot be
printed, the verdict names the route so a reader can judge, and the route joins the audit
listing beside trusted contract helpers. The residual is a known limit, written down.

**Three new ways to print a number that means less than it looks, each needing disclosure:**
values built through a route are sampled through the route's inputs, not the type's natural
domain, so a route-built verdict needs a mark naming the route; the collapsed-diversity case
above; and — the worst — **a valid value aimed at a meaningless domain**: a randomly built
path is a perfectly good value handed to a function whose promise is really about the file
behind it, so nearly every case exits early with "not found" and the count measures one
behaviour many times. The general disclosure for that: where a function returns
success-or-error, count the split and say it — "16 cases; 15 returned an error before the
interesting behaviour engaged". That same disclosure doubles as the detector for a
degenerate declared route, and is the same species as the still-open item about a promise
whose rejection branch decides nearly every case.

- [x] **Spec wording fixed when the hook landed** (2026-09-02, §5.4b): the claim that the
      hook's design was validated in the first spike was thinner than it read. Corrected in
      place rather than inherited: the spike validated the constructor-harness pattern
      (calling a found constructor to build a value), never the declaration surface a user
      writes to name one.

## Heavy ponytail review of the whole repository — requested 2026-09-01, NOT STARTED

The maintainer's read: "we've got a lot of bloat". Run at **ultra** intensity — deletion
over addition, question whether each thing needs to exist at all, and the shortest working
diff wins. This is a review of what is *here*, not of what to build next.

Numbers measured today, as leads rather than findings — each still needs someone to look
before anything is cut:

- **Two parallel implementations of the same ideas.** The product crates are ~49,000 lines
  of Rust; a separate development-tooling tree carries another ~8,700. Its drawing tool
  alone is ~7,400 lines, its checker ~800, its scheduler ~560, and its kernel entry is 7
  lines. Some of this was deliberately promoted into the product and the leftovers were
  meant to become thin consumers — the architecture bundle's own note says so. Whether that
  actually happened is the first thing to check.
- **The docs directory is 2.7 MB across 40 files.** The largest is a 984 KB generated
  walkthrough. The second, at 536 KB, is a review document that has a raw session
  transcript pasted into it — 28 tool-invocation markers, another tool's version banner,
  and someone's local machine paths, all committed. That one is the clearest single candidate.
- **100 fixture directories**, each its own crate that the end-to-end suite builds from
  scratch. A recorded item already suspects duplicate proof work inside that suite.
- **This file is 3,309 lines with 121 open items**, against a 2,372-line spec. A running
  state longer than the thing it describes is itself the smell — and the rule that keeps it
  honest ("a stale list is worse than none") argues for pruning what is recorded and done,
  not just appending.

Two things the review must NOT cut, because they look like bloat and are not:

- **The exhaustive kernel enumeration.** It is nearly a million cases and takes seconds.
  It is the gate, and it is deliberately more than a sample.
- **The honest caveats recorded throughout this file.** A known gap left open on purpose is
  a state worth recording. Deleting the record does not close the gap; it hides it.

## Verified independently, and one blemish found while doing it — 2026-09-01

The `Self`-spelled parameter fix was checked against the real library on a case the
implementing agent did not use: the same method that was refused now runs 64 cases, and a
promise that is false on nearly every input comes back a violation with a concrete failing
input, so the fix makes something reachable without making it toothless.

- [ ] **The failing input is printed with Ply's own generated internal names.** Verified
      output, verbatim: `failing input: other = __ply_leaf_p_other_major_major=0,
      __ply_leaf_p_other_minor_minor=0, __ply_leaf_p_other_patch_patch=1`. A reader is
      supposed to be able to act on a counterexample; these are scaffolding names from the
      generator, doubled and prefixed, not anything in the user's own code. The run is
      otherwise honest here -- it says plainly that it cannot write this shape out as a
      runnable test rather than inventing one -- which makes the printed values the only
      thing the reader gets, and they are unreadable. Fails the newbie bar.

## Fixed: a promise comparing non-numeric values with `==`/`!=` did not compile — 2026-09-01

Closes the item below dated 2026-09-01 ("A promise comparing non-numeric values with `==`
or `!=` does not compile"). Test-first, watched to fail against the real defect (the exact
compiler error, not a shape check), revert-and-confirm-red on both the unit tests and a
real `cargo ply verify` run.

**Cause.** `contract_rt::widen` casts every comparison's leaves to `i128` unconditionally,
so the overflow-safety widening `result == x + 1` needs at `x`'s maximum value also reached
leaves that are not numbers at all: text, an `Option`, a struct, an enum. Reproduced
verbatim against the vendored `semver` copy at `/home/user/semvercheck-c`: putting
`result.is_err() || result.as_ref().unwrap().as_str() == text` on `Prerelease::new` gives
`error[E0606]: casting &str as i128 is invalid`, and because `fuzz`/`test` checks in a
crate share one generated harness, adding one more, completely unrelated, correct function
(`BuildMetadata::new`, checked with the same safe length-comparison property that already
worked) to the same run turned it `tool_error` too — confirmed both ways, verbatim, against
the unfixed binary.

**Fix.** A comparison is now widened only when both sides are provably numeric, decided
from the checked function's own parameter and return types (never guessed): a numeric
literal; a parameter or the result whose declared type is a plain integer scalar, `bool`,
`char`, or a float (every `RustType` shape `as i128` can actually reach through, confirmed
directly against `rustc` rather than assumed — including that a bare fieldless enum *can*
take that cast, right up until it gains a `Drop` impl, at which point it cannot); a
dereference, parenthesised form, or explicit numeric cast of a numeric thing; arithmetic
over numeric operands; or a nested comparison/logical expression (always safe to cast,
since it is always `bool`). Anything else — a method call, a field access, a path to a
constant, an enum variant — leaves the comparison exactly as written, which is always
legal Rust and so can never itself break compilation.

Unit tests (`crates/ply-core/src/contract_rt.rs`): a text comparison, an `Option`
comparison, and a `Drop`-carrying fieldless-enum comparison (chosen over a plain fieldless
enum specifically because a plain one still compiles cast `as i128` — checked directly
against `rustc` first, so the fixture proves the real defect and not a coincidence) each
watched to fail with the cast present, then pass with it gone. The existing overflow test
(`widens_arithmetic_so_overflow_cannot_hide_the_defect`) and the existing nested-comparison
suite (a history of precedence bugs) are unchanged and still pass — a return type of `bool`
needed adding to what counts as numeric to keep the nested-comparison tests green, since a
nested comparison's own cast (always onto a `bool` result) is unconditionally safe
regardless of what it compares.

New permanent fixture and end-to-end test proving the fix on a real `cargo ply verify`
run, not just generated source shape: `tests/fixtures/nonnumericcompare/`,
`tests/e2e/tests/nonnumericcompare_fixture.rs`. Covers, each both true (real passing
evidence) and false (real `violation` with a real failing input): a text comparison, an
`Option` comparison, a `Drop`-carrying enum-variant comparison; plus the overflow case
(`saturating_bump`, checked `bounded(2)` rather than `fuzz` so the one bad `u8` value in
256 cannot simply be missed by random sampling) confirming protection is intact; plus an
entirely unrelated function proving no contagion survives. Reverting the fix reproduces
`tool_error` for every one of these, with the real compiler errors quoted, and confirmed
again directly against `/home/user/semvercheck-c` itself (restored to the state it was
found in afterwards).

`cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo test --workspace --exclude ply-e2e` (335 `ply-core` unit tests, the kernel
enumeration gate at 2.29s under `--release`, every other crate) all clean; the full
`ply-e2e` suite run to completion with no regressions.

## The sampling engine's decision is now a real recursive grammar, and slices exist — 2026-09-02

**Measured defect, fixed.** Ply refused a function the moment it needed a shape it already
builds *nested* inside another one: an optional string, a slice, and a list of a user
struct were all refused, even though a plain string, a plain user struct, and `&Vec<u8>`
were all checked happily alone. Every shape added after the original set was individually
barred from composing, because one shared "is this type supported" decision answered for
both the sampling engine (proptest) and the exhaustive-proof engine (Kani), and letting a
new shape compose would have silently made it eligible for the *proof* tier too, whose
list is measured and deliberate.

**The fix splits the decision, and only ever widens the sampling side.** `Option`, `Result`,
a fixed array, a list, a set, a map, a slice, a tuple, and an owning wrapper (`Box`) now
close recursively over anything the sampling engine can already build alone — a plain
scalar, a string, a float, `NonZero`/`Duration`, a user struct, or another composed shape,
to any depth. The proof (`bounded`) engine's own list is untouched, byte for byte, pinned
by a dedicated regression test (`the_bounded_proof_engines_own_supported_list_never_widens`,
`crates/ply-core/src/harness.rs`) written *before* the composing logic, so a mistake here
would have shown up as Ply claiming exhaustive proof over an unmeasured shape — the test
stayed green throughout because the proof engine's own predicates were never touched, only
read from.

**Slices were not handled as a shape at all before this** (`&[T]`, distinct from `&Vec<T>`)
— added the same way `&Vec<u8>` already works: build the owned list, lend it as a slice at
the call site, no second mechanism.

**A real compile-time trap, found and fixed along the way, not merely a nesting rule.**
Once a user struct/enum can sit *inside* another shape's own sampled value (an
`Option<Doc>`, a `Vec<Doc>`), constructing it via proptest's own `prop_map` fails to
compile the moment the struct does not derive `Debug` — `prop_map`'s own trait bound is
`O: fmt::Debug`, and nothing here can assume a user's own type derives it (a private-field
type could not derive it honestly even if Ply tried to add it from outside). The fix:
composition never constructs a nested user type *inside* a proptest strategy at all — the
strategy only ever draws the raw leaf values (always plain scalars/strings, always
`Debug`), and the real constructor call happens afterwards, in ordinary Rust code in the
harness's own preamble, exactly mirroring how a *top-level* struct parameter was already
built. One honesty condition attaches: a nested constructor carrying its own
`#[ply::requires]` filter or a fallible (`Result<Self, E>`) return has no proptest
case-rejection available at that point (no early return reaches back out through an
already-built container), so **nesting is refused for exactly those two shapes**
(`RustType::is_fuzz_nestable`) even though the identical type is fine as a bare top-level
parameter. Not yet measured whether this narrowing costs anything real — no case in this
session's own probes needed it.

**Superseded, not broken: the corpus-seeding workaround for `Option<String>`/`Vec<String>`**
(`fuzz_gen::plan_param_seeding`/`classify_seedable_wrap`, built before composition existed)
now never engages for either shape, since neither is "otherwise unbuildable" any more — its
own precondition. Left in place rather than removed (out of this task's scope; flagged, not
silently deleted), but every fixture/test that demonstrated it specifically for these two
shapes (`paramseeded`) was rewritten to demonstrate the real capability that replaced it.
Two other fixtures (`skippedctor`, `excludedop`) whose whole premise was "`Option<String>`
is still unbuildable" had their unbuildable argument changed to `&mut u32` (refused for a
structural reason composition does not touch) so they go on testing what they were written
to test.

- [x] Sampling engine composition + slices (f394aba).
- [ ] **Follow-up not attempted this session**: `classify_seedable_wrap`/`plan_param_seeding`
      and the rest of the plain-parameter corpus-seeding apparatus are now dead code for
      their only two shapes (`Option<String>`/`Vec<String>`) — every code path that could
      reach them requires a param that is simultaneously the *one* unsupported one in its
      function *and* textually exactly one of those two strings, which composition makes
      impossible. Still compiles, still unit-tested in isolation, genuinely unreachable from
      `cargo ply verify`. Worth a deliberate removal pass, not a silent one.
- [ ] **Not attempted this session**: `HashMap`/`HashSet` were not added as composition
      shapes (only `BTreeMap`/`BTreeSet`) — deterministic ordering was already the reason
      `BTreeSet` was chosen over `HashMap` in M4, and this task did not re-open that choice.
      A real gap if a measured library's own public surface needs the hasher-backed
      variants specifically.
- [ ] **Not attempted this session**: a reference nested *inside* a composed shape
      (`Vec<&str>`, `Option<&Doc>`) — every reference this task's grammar reaches is the
      existing top-level `&T` mechanism (already correct for any inner shape, since it
      strips the reference before parsing whatever is inside), never a *new* reference
      appearing partway through a container. Genuinely harder (a container's own elements
      would need to lend from a sibling owned collection with matching lifetimes), and no
      case in this session's own probes needed it.

## Review of the three items, and two defects it found that I had missed — 2026-09-01

Independent review of the three open items below, then every claim on both sides re-run
against the real library rather than taken on trust. **The ranking held; one of my own
claims did not, and two worse defects than the ones I named were found.**

**My claim that Ply "already owns the vocabulary and does not reach for it" was wrong.**
The existing "narrower than it looks" mark counts inputs thrown away *before* the call --
its printed legend says so, in terms of constructors and operations the run could not
call. In the case at issue nothing is thrown away: all 64 inputs run. The emptiness lives
*inside* the promise's own "either it was rejected, or ..." structure, after the call,
where no measurement exists at all. Reusing that mark unchanged would make its own legend
false. Either its legend is reworded to honestly cover both, or -- cleaner -- a sibling
mark says what this one actually measures.

- [x] **A factually false `examples:` entry passes in silence -- now disclosed, 2026-09-01,
      9faf5f0.** Verified by hand: `Version::parse("1.2.3").is_err()` -- a
      plainly false sentence, that text parses fine -- under a `fuzz` check earned a
      clean `fuzzed(64)` with not one word about it. Fixed by measurement, not by making
      `fuzz` run examples (that is the separate proposal below, deliberately left
      undone): `cargo ply verify` now warns (`W0525`) whenever a function declares
      `examples:` and nothing declared will actually consume them -- naming how many will
      not run and that `test` is what runs them. The condition asks the seeding machinery
      itself (`fuzz_gen::examples_are_consumed`) rather than re-deriving "does fuzz use
      this," so it does not false-positive on `paramseeded`/`textseeded`, where `fuzz`
      alone genuinely does seed from an example with no `test` declared. This is a
      warning only -- it does not change what any check means or what verdict it earns.
      New fixture `tests/fixtures/examplesnotrun` (the semver reproduction, self-contained)
      pins both the silent "before" and the disclosed "after"; the render's own tooltip
      sentence (`examples_prose`) is reused verbatim rather than said a second way, so the
      picture and the terminal cannot disagree about what ran. §5.4a amended.
- [x] **The escape hatch Ply's own refusal recommends did not work for a method -- fixed,
      2026-09-01, 9faf5f0.** The refusal for an unbuildable parameter says to
      "declare `test` instead, with an `examples:` entry" -- doing exactly that on a
      method gave `error: invalid path separator in function definition`: the generated
      test's own *name* spliced the checked function's `::`-qualified path in verbatim.
      Every fixture exercising this codegen used a free function, so the break was never
      seen even though nearly everything in a real library is a method. Fixed by taking
      the checked function itself rather than a bare name, and deriving the test's name
      from the same safe identifier (`ContractFn::ident`) the fuzz-test generator already
      builds its own test name from, so there is exactly one place that turns a checked
      function into a safe identifier, not two. New permanent fixture
      `tests/fixtures/methodexampletest` (a method taking `&Self`, the exact shape
      `semver`'s `Version::cmp_precedence` has) proves both directions against a real
      `cargo ply verify` run: a true example earns a real `tested` verdict, and a
      rewritten false one earns a real, named `violation` -- the gap that let this
      survive was that no fixture had a method under `test` with examples at all.
      Verified by hand against `semver` itself, both before (the exact quoted compiler
      error) and after (a clean `tested` verdict) the fix.
- [x] **That same refusal contains a false sentence — audited, 2026-09-01 (`f2bfe88`).**
      It says no part of the parameter's value is one Ply knows how to vary. False for a
      `Self` parameter specifically, because the identical type spelled by name was
      varied happily -- a reach defect (see "`Self` as a parameter spelling..." below),
      not a wording defect on its own. Fixing the reach defect closes this too: `Self` no
      longer reaches this sentence's gate at all (it resolves to the same buildable type
      the receiver already does, before the gate runs), and every other parameter shape
      that still reaches it has already failed the same resolution attempt a named type
      gets, so the sentence is true whenever it still fires. Confirmed with a regression
      test, not reworded -- the trailing "declare `test` instead" advice (the escape-hatch
      defect just above) is a separate, still-open defect and was left untouched on
      request.

**Not done, and deliberately left for the maintainer: actually running `examples:` under
`fuzz`/`bounded`.** The stronger fix for the first item above is having `fuzz` itself
compile and run every declared example once, up front, alongside its generated cases --
turning "declared but unconsumed" into "declared and checked" instead of merely
disclosing the gap. Not built this session because it is a semantic change to what a
declared check means, which CLAUDE.md reserves for the maintainer, not an agent mid-fix.
What it would take: `fuzz`'s harness already has a slot for exactly this (the direct
`ply_example_*` tests `test` generates already run inside the same harness module `fuzz`
shares -- see `generate_example_test`/`fuzz_gen::wrap_fn_harness_module`), so the
generation side is close to free -- the real work is deciding what a *failing* example
means for a `fuzz`-only claim's verdict (today `R0502`/`violation` is `test`'s own
vocabulary) and what it means for `bounded`, which builds a completely different kind of
harness and has no equivalent slot at all. It would also change `fingerprint` input 4's
own wording ("the worked examples a `test` check compiles into assertions") the moment
any check besides `test` compiles one, which is a spec-level decision, not a drive-by
edit.

**Two corrections to how the remaining work is priced:**

- **The non-numeric comparison defect is contagious, which I did not say.** Every check in
  a crate shares one generated harness, so one string comparison written the way anyone
  would write it turns the *whole crate's* evidence into tool errors, not just that
  function's. It also gates the trait-method work: the natural phrasing of the ordering
  properties is exactly the shape that trips it. Fix it before that work, not after.
- **Trait methods are two items, not one.** Properties 3, 4, 7 and 12 sit on hand-written
  implementations with real bodies -- moderate work, and plausibly all four fall to it now
  that the return type no longer refuses anything and text parameters work. Properties 1
  and 2 sit on *derived* implementations with no body to anchor to, so they need promises
  written in the config file to reach the engines, which is a separate standing gap.
  Sequence them apart.

**The fix for the confident-verdict problem is measurement first, generation second, and
the order is the point.** Measurement repairs what Ply currently says; generation only
improves what it covers. Record which branch of the promise actually decided each case and
print the split. Three ways that fix would itself be dishonest, to be avoided by name:
evaluating every branch in order to count it (the guard exists to stop a panic -- count
which branch *decided*, never what each would have said); a threshold that prints
unqualified below some skew (print the split always); and wording that promotes a
promise-text count into a claim about which code paths ran. Generation shipped alone is
the trap -- it makes the number look stronger with nothing showing anything moved. With
measurement in place first, the acceptance-branch count rising on the real library is the
acceptance test, and it comes free.

## The acceptance test ran: `semver` reach moved from 1 of 16 to 4 of 16 — 2026-09-01

Measured, not inferred: every property below was written as a real promise on the real
function and run. Full write-up in `docs/reach-measurement-2.md`; the vendored copy it was
run against is at `/home/user/semvercheck` (outside this repository).

Newly reachable, each `fuzzed(64)`: parse rejects whitespace; an accepted identifier is
stored verbatim; at most 32 comparators. The check was proved to bite — a deliberately
false promise came back `violation` with a shrunk failing input.

**The honest caveat, and it is a big one: three of the four are checked almost entirely on
inputs the function rejects**, because random text essentially never parses. The evidence
is real; the author's rules about what is *accepted* are barely exercised. Ply prints
`fuzzed(64)` unqualified and does not say this.

- [x] **Fixed: the branch-decided measurement** (`297dd8f`). Ply now instruments a
      top-level `||` in a postcondition into an `if`/`else if` chain that records which
      side actually decided each case,
      preserving `||`'s own left-to-right short-circuit exactly (proved by a fixture whose
      far side panics if forced to run: it stays green, because the wrong side is never
      evaluated). The split prints unconditionally, on both a balanced and a skewed
      promise alike (`orbalanced`/`orskewed` e2e fixtures) — never gated on the skew
      itself; only the new `promise-lopsided` status is, at the same >50% threshold the
      high-rejection warning already uses. That status is a sibling of `partial-history`
      ("narrower than it looks"), never a reuse of it, exactly as this document's own
      review above concluded it must be: this one is a fact about what happens *inside*
      the promise after the call, not about an input or operation the run could not build
      before it. `The-Ply-Spec.md` §5.4c amended in the same commit. Verdicts pinned
      unchanged (`fuzzed(64)` stays `fuzzed(64)`) by both fixtures' own e2e assertions.
- [x] **A promise comparing non-numeric values with `==` or `!=` does not compile.** The
      generated harness casts both sides of a comparison to `i128` (so it can report a
      broken promise rather than overflow while checking one); against a string, an
      `Option` or a struct that cast is invalid — `error[E0606]: casting &str as i128 is
      invalid`. Reported honestly as a tool error, never as a pass. Not new, but far more
      reachable now the return-type gate no longer refuses these functions first. This is
      what blocks the natural phrasing of property 15. **Fixed**, see the top of this file
      ("Fixed: a promise comparing non-numeric values with `==`/`!=` did not compile").
- [x] **`Self` as a parameter spelling is refused where the same type by name is checked
      — fixed 2026-09-01 (`f2bfe88`).** `cmp_precedence(&self, other: &Self)` was
      `unsupported`; changing nothing but `&Self` to `&Version` was `fuzzed(64)`. Mirror
      image of this document's original headline, which turned on the author having
      typed `-> Self` rather than `-> Version`. `Self` in parameter position now resolves
      to the receiver's own already-resolved type, exactly like the named spelling.
      Proved to still bite: a deliberately broken `cmp_precedence`, reached only through
      this fix, came back `violation` with a real shrunk failing input
      (`other = major=5, minor=1, patch=9`), not a comfortable pass. Property 6 is now
      reachable as written; re-measuring the full 16-property count is separate work.

Still blocked and untouched today: trait methods (properties 1, 2, 3, 4, 7, 12), and a
`VersionReq` or `Comparator` built from text in a parameter position (5, 8, 9, 10, 11).

## Fixed: a declaration-only render no longer calls its own run clean — ffacd9b, 2026-09-01

`cargo ply render --json` (the editor-facing envelope behind semantic focus) built a tree
in which every item is `unclaimed` — nothing has been checked — and then reported
`"outcome": "clean"`. A plugin colouring a badge from that field would have shown green for
a document no run has ever looked at.

The builder now derives the outcome from the tree it constructs rather than trusting the
caller, so it reports `missing_evidence`. Two tests, both watched to fail against the old
code: an end-to-end one over the real CLI that checks its own premise (all six items still
unclaimed) before asserting the outcome, and a unit test that hands the builder `clean` and
requires it back as `missing_evidence`.

## Measured: the return-type gate can come off, and what it hides — 2026-09-01

Fable's ranking put this first and it is not a declaration at all: the gate refusing a
function because Ply cannot *construct* its return type blocks 10 of the 16 properties in
`docs/reach-measurement-2.md`, and the gate's own doc comment already concedes it blocks
nothing technically -- *"nothing in this codegen ever names or constructs a return type.
This gate is therefore a deliberate, requested narrowing... on principle."*

Measured rather than taken on the comment's word. With the gate temporarily removed:

- A function returning `std::cmp::Ordering` -- a type Ply models nowhere -- **earns
  `fuzzed(64)`**, and a false promise about it is caught: `!result.is_lt()` on `a.cmp(&b)`
  gives `violation` with a shrunk failing input. So the comment is right, and the refusal
  really is costing real evidence for no technical reason.
- **But removing it exposes a separate defect the gate was hiding.** A contract that *names*
  the return type -- `|result| *result != Ordering::Greater || a > b` -- fails to compile:
  `error[E0433]: cannot find type Ordering in this scope`. The generated harness brings
  parameter types into scope and not types the contract text names.

- [x] **The import defect, fixed** (`cc2121e`). A contract may now name any type the file
      it lives in can see -- `use_aliases_in_file`'s own scan is carried on `ContractFn` and
      resolved against every identifier the contract text references
      (`fuzz_gen::contract_referenced_use_imports`), not only the parameter/receiver types
      `extra_type_imports` already walked. A glob import of the target crate was considered
      and rejected: it cannot reach `std::cmp::Ordering` at all (an external crate's own
      root export list, not the target crate's), and blindly re-emitting every `use` in the
      file risks importing something private for an unrelated reason and breaking a
      neighbour function's harness that never asked for it. Proved both directions: a unit
      test and a new e2e fixture (`tests/fixtures/ensuresimport`) go red with the exact
      `error[E0433]` before the fix and green after; the fixture also proves the catching
      direction (a broken implementation still reports `violation`).
- [x] **The gate decided: off, on both engines, measured** (`51ef480`). Measured directly
      before removing anything, per the maintainer's brief: a function returning
      `std::cmp::Ordering` earns `fuzzed(64)` on the fuzz engine and a genuine `bounded(2)`
      proof on the bounded (Kani) engine, both completing in seconds -- not a timeout
      mislabeled -- and both independently report `violation` with a real witness on a
      broken promise about the same type. Both engines pay this gate's cost for nothing in
      return, so `is_bounded_return_supported`/`is_fuzz_return_supported`
      (`crates/ply-core/src/harness.rs`) now always answer `true`; the history of why the
      gate was added is kept in their doc comments, retracted rather than deleted. §5.4b
      amended in the same change: "parameters and return type" is no longer true --
      the list binds parameters only, and a function's return type is never a reason
      either engine refuses it. New permanent fixture `tests/fixtures/orderingreturn`
      proves the same clean-and-catches pair against a real `cargo ply verify` run on both
      engines together. **Not yet measured: whether this moves the real 16-property count**
      -- that re-measurement is the maintainer's to run, per `docs/reach-measurement-2.md`'s
      own acceptance rule.

## Design principle, from the maintainer — 2026-09-01

**Requiring the user to write a small declaration is cheap, because an agent writes it.**
Stated plainly by the maintainer when seeded generation landed: "one line is one line from
an LLM".

*The example originally cited here has been withdrawn.* It said one `examples:` line took a
real library's function from no evidence to sixty-four real checks; re-measured by hand
against `semver` 1.0.28, that function gets there with no example at all (see the correction
above). The principle is the maintainer's and stands on its own; the measurement that was
offered in support of it did not.

This changes the economics of a decision already recorded. `docs/rule-registry-design.md`
and the seeding design both weighed "adds something new for the user to write and keep true"
as a significant cost, and rejected options partly on those grounds. That cost is lower than
assumed for anything an agent can write from reading the code. It is **not** lower for
things a user must keep true by hand over time -- a declaration that drifts from reality is
still the failure mode that matters, and an agent writing it once does not keep it true.

So the rule is: **prefer a small declaration over inference where an agent can supply it and
Ply can check it against reality.** Prefer inference where the declaration would have to be
maintained by hand and nothing would notice it going stale.

- [x] **An example now unblocks a parameter Ply cannot build, for the shapes whose parts Ply
      already knows how to vary -- 2026-09-01.** `width(label: Option<String>) -> usize` (the
      measured gap, verbatim) now earns `fuzzed(64) [seeded]` from one `examples:` entry --
      `tests/fixtures/paramseeded`. `Vec<String>` opens the same way (elements and length
      both vary), sharing the exact corpus/mutate/trickle apparatus the constructor path
      already built (`fuzz_gen::plan_param_seeding`, `SeedableWrap`) rather than a second one
      -- the two mechanisms are mutually exclusive by construction (this one only ever fires
      for a non-receiver fn), which is what makes reusing the apparatus's own generated
      variable names safe. **Not opened, disclosed rather than attempted:**
      `Result<String, E>`, and nested `NonZero`/`Duration`/`f32`/`f64` inside any wrapper --
      each needs its own construction or mutation story, which `String`'s existing text
      apparatus does not hand over for free (a number has no character-level mutator to
      reuse; a `Result`'s `Err` arm needs its own construction path).

      The counting condition: `plan_param_seeding` refuses (stays `None`, parameter stays
      refused) whenever the type is not one of the two classified shapes, whenever more than
      one parameter is otherwise unbuildable, or whenever no `examples:` entry supplies a
      seed -- an opaque type never borrows the seeded machinery just because an example
      exists. For an opaque type, `examples:` now still unlocks `test` alone (never `fuzz`,
      which cannot grow a case count for it): `generate_example_test`'s own codegen never
      depended on the parameter being buildable in the first place, so the gate widening
      there is a real bug fix, not new machinery -- `tests/fixtures/paramseedopaque` earns
      `tested`, the vocabulary this project had already written down for "a concrete case
      was run and held", never a fabricated `fuzzed(n)`. A new diagnostic, `W0524`, carries
      the growable case's own provenance (parameter name, example count, real case count) the
      way `W0523` already does for the constructor case, worded for its own honesty
      condition: there is no rejection rate to report here (nothing gates an
      `Option<String>`/`Vec<String>` value the way a fallible constructor gates text), so
      every one of the `n` cases genuinely ran.

      The refusal itself was fixed in the other direction too: `V0505`'s message now names,
      per unbuildable parameter, whether an example would actually help (and what to write)
      or would not (an opaque type, told plainly rather than given false hope).

      Found along the way and fixed as necessary plumbing, not scope creep: the generated
      harness module only ever imported types a fn's own resolved parameters referenced, so
      `test`'s newly-unlocked opaque-type path failed to compile (`error[E0433]: cannot find
      type ... in this scope`) the moment an example's own literal source named a type
      nothing else in the fn's signature resolved -- fixed with a glob import of the target
      crate, which an explicit `use` always outranks on a name clash, so no existing
      generated harness resolves any name differently.

      Every fixture that depends on a shape staying unbuildable (`excludedop`, `skippedctor`,
      `textmutator`) and every one depending on the constructor-seeding behaviour
      (`textseeded`, `textseedempty`) still passes unchanged -- none of them declare an
      `examples:` entry for the parameter this widening reaches, which is exactly the gate
      that keeps their premises intact.

## Seeded generation moved a real library's reach — 2026-09-01

The acceptance test Fable named: not a green fixture, but whether a real library's number
moves. It moved.

`semver`'s `Prerelease::is_empty` -- a method whose receiver must be built by parsing text,
the exact shape that produced the dead end:

- **Before:** 1025 of 1074 generated strings rejected, 49 ever checked, verdict `unclaimed`.
  No evidence at all.
- **After:** 43 of 107 rejected, verdict `fuzzed(64) [narrower than it looks, seeded]`.
  Sixty-four real cases, each one run.

**Correction, 2026-09-01, after re-running this by hand against a fresh vendored copy of
`semver` 1.0.28 rather than trusting the earlier record: the `examples:` entry is not what
moved it, and the earlier version of this section said it was.** The same function reaches
`fuzzed(64)` with no `examples:` line at all, and the run says so itself: *"the 64 cases
were grown from 64 known-valid values: 0 from the `examples:` you wrote, 64 that
`Prerelease::new` accepted from random draws during this run."* What did the work is the
free half of the mechanism -- harvesting every value the constructor accepts during the run,
which used to be thrown away. Adding the example changes the verdict not at all.

Both numbers above were re-measured directly, the "before" by building the product as it
stood at 83949d6 and running it against the same crate, so the delta is real. Only the
attribution was wrong.

This matters beyond bookkeeping: the design principle recorded below was written on the
belief that *one line from a model* bought the evidence. On this function nothing was
bought -- it was free. Examples still matter where the constructor accepts essentially
nothing and there is no case base to grow from (the `textseedempty` fixture: 1025 of 1025
rejected), which is a narrower claim than the one that was recorded.

**This is the first change today that moved a number rather than a failure mode.** Both
reach fixes yesterday and the text fix this morning were real and necessary and left
`semver` at one checkable property. This one produces evidence where there was none.

Verified by hand before being believed: the status appears on a seeded run and propagates to
the root; it survives result reuse (`[seeded, reused]`); an unseeded run carries no such
mark, so the two are never confused; and a seeded run still catches a false promise --
breaking the function under test gives `violation`, not a comfortable pass.

- [ ] **Not measured: whether the whole 1-in-16 count moves.** One property is now reachable
      that was not. The other fifteen are each held by two to four blockers, and this
      addresses one of them. The full re-measurement is owed before any claim about the count.
      A vendored copy set up for it now lives at `/home/user/semvercheck` (outside this
      repository; nothing here modifies `semver` upstream).
- [ ] **KNOWN GAP, disclosed not detected: seeded runs miss the extremes.** Mutations of
      short valid values reach a 280-character identifier or a 20-digit overflow essentially
      never -- and those are exactly the cases `semver`'s author wrote down. The `[seeded]`
      status is honest cover for this, not a fix. The user's defence is to write the extreme
      case as an example, where it becomes a seed; that should be said somewhere a user reads.

## Seeded generation for text-parsed types — 2026-09-01

`docs/reach-measurement-2.md`'s open blocker ("a type built by parsing text cannot be
constructed by random text") is closed for the shape it was measured against: a receiver
whose own constructor takes a `&str`/`String` and rejects most input (a `#[ply::requires]`,
or a fallible `Result<Self, E>` return). Test-first throughout; every fix below was proved
by reverting it and watching the exact same failure come back.

- [x] **Corpus and mix, implemented.** `fuzz_gen::plan_receiver_seeding` decides, per
      receiver constructor, whether its (first) `String`-typed parameter is gated at all;
      if so, `fuzz_gen::extract_examples_seed_strings` pulls every literal string argument
      passed to that constructor anywhere in the crate's `examples:` entries (syntactic
      only, zero new vocabulary), and the generated harness (`fuzz_gen::seed_apparatus`)
      grows that pool at runtime with every value the constructor actually accepts. Future
      draws for that parameter come from a 4:1 mix (`SEED_MUTATE_WEIGHT`:
      `SEED_TRICKLE_WEIGHT`, `fuzz_gen.rs`) of mutating a random corpus entry (character
      edit, splice, truncation, repetition, or a verbatim replay) against a continuing
      uniform trickle -- justified in a doc comment on those two constants and repeated in
      the diagnostic below so the ratio reaches the JSON envelope, not just source. An
      unseeded (ungated, or non-text) constructor takes byte-identical code paths to
      before this feature existed -- pinned by
      `fuzz_gen::tests::an_infallible_unconstrained_text_constructor_is_not_seeded`,
      comparing the seeded and plain entry points on the same fn and asserting equal
      output.
- [x] **The verdict carries its own provenance, honestly.** A `fuzzed(n)` verdict earned
      this way carries a `seeded` status -- structurally the same way `conditional`
      already travels (a plain flag in the same `statuses` list the tree and the record
      already carry, propagated and reused with zero extra plumbing, per
      `crates/ply-cli/src/verify.rs`'s own comment at the push site) -- plus a new,
      `info`-severity diagnostic (`W0523`, never a warning: this describes what the
      evidence *is*, not something incidental about the run) naming the real counts a
      given run produced: how many seeds came from `examples:`, how many the constructor
      accepted from generated draws, and the actual rejected/total split. A verdict
      earning `fuzzed(n)` with nothing ever seeded carries neither the status nor the
      diagnostic -- proved both directions by an e2e fixture pair
      (`tests/fixtures/textseeded`, whose receiver constructor parses text and is fed one
      `examples:` seed, vs. the pre-existing `narrowctor`, whose receiver constructor
      rejects on a plain `u64` and must show neither) and confirmed to survive a reused
      (carried-forward) verdict unchanged.
- [x] **The other honesty condition: no seeds at all names the fix.** When a gated text
      constructor's corpus never grows past zero (no `examples:` entry, and not one
      generated draw was ever accepted), the existing high-rejection abort (`W0503`)
      switches from its generic "widen your `requires`" wording to naming the exact
      action -- add an `examples:` entry for the specific constructor, quoted by name --
      only for this shape; every other cause of that same abort (a plain numeric
      `requires`, unrelated to text) keeps its original wording verbatim, unaffected.
      Fixture: `tests/fixtures/textseedempty` (a constructor accepting essentially none of
      the space uniform sampling could draw, no `examples:` entry at all).
- [x] **Real bugs found by actually compiling the generated harness, not just asserting on
      its text (CLAUDE.md: "assert the observable outcome, not the shape of the output").**
      The corpus's embedded `examples:` literals were spliced as bare `&str` literals into
      a `Vec<String>` initializer (`error[E0308]`, caught only once the `textseeded`
      fixture was actually built and run, not by the unit tests, which merely checked the
      literal text was present) -- fixed by appending `.to_string()` to each; a unit test
      now pins the exact well-typed form. Both new fixtures also needed their receiver's
      field made `pub`, the same requirement every other receiver-postcondition fixture in
      this codebase already has.
- [ ] **KNOWN GAP, disclosed rather than hidden (the design brief's own "known failure
      mode").** Seeded generation is an honest *disclosure*, not a *detection* mechanism:
      64 cases mutated from short, ordinary seeds essentially never reach an extreme an
      author actually cared about (a 280-character identifier, a 20-digit overflow) --
      mutation just does not walk that far from a short starting point in a handful of
      edits. The `seeded` status and its diagnostic say what the evidence *is*; they do
      not detect what it *misses*. The user's defence is unchanged from what `examples:`
      already offers: write the extreme case by hand as its own `examples:` entry, and it
      becomes a seed like any other. Second-order, also disclosed rather than fixed: seeds
      anchor the draw distribution near what is already known-valid, so a pathological
      input that would crash the parser becomes measurably *less* likely to turn up than
      under pure uniform sampling -- which is the whole reason the uniform trickle stays
      in the mix rather than being dropped for a purer (and more self-referential) corpus.
      No code change is proposed for this; it is a property of the technique, named so it
      is never mistaken for closed.
- [ ] **KNOWN GAP: only a receiver's own constructor parameter is seeded.** A plain
      (non-receiver) function whose own text parameter is itself gated by its own
      `#[ply::requires]` -- e.g. a parser checked directly rather than through a
      receiver -- is not seeded at all yet; nor is a `String` parameter nested two levels
      deep (a struct field built via its own constructor, itself an argument to another
      constructor). Scoped out deliberately for this session: the measured probe
      (`docs/reach-measurement-2.md`) and the acceptance shape are both squarely the
      receiver-constructor case, and widening to every gated `String` parameter
      everywhere is a larger, separately-reviewable change. Not yet re-measured against
      `semver` itself -- that measurement is explicitly the maintainer's to run, not
      mine to claim.

## Two more harness-generation compile defects fixed — 2026-08-31

Both were the two `KNOWN GAP`s recorded just below (found the same day, pointing Ply at
`semver`, `docs/reach-measurement-2.md`), confirmed pre-existing and almost certainly a
real share of that measurement's 1-in-16 reach. Test-first, revert-and-confirm-red on
both.

- [x] **A method's own postcondition could not mention the receiver it is called on.**
      `#[ply::ensures(|result| *result >= self.a)]` generated a harness that did not
      compile: `error[E0424]: expected value, found module `self``, because the
      postcondition is spliced into the generated test as a free-standing expression
      outside any `impl` block, where the literal keyword `self` means nothing. Fixed by
      rewriting a bare `self` to the binding the generated harness already builds the
      receiver under (`__ply_receiver`), before `old()` is lifted (so `old(self.x)` still
      reads the receiver's value on entry) and before the postcondition is widened. New
      helper: `contract_rt::rewrite_self_to_receiver`, wired into
      `fuzz_gen::generate_fuzz_test` (the only place a receiver method's postcondition is
      ever spliced into a runnable test -- `contract_rt::render_cex_test`'s replay-test
      path already refuses every receiver method by design, so it needed no change).
      Fixture: `tests/fixtures/selfreceiver/`; test:
      `tests/e2e/tests/selfreceiver_fixture.rs`, covering `self` read alongside the
      result (the reported repro, verbatim), `self` read alongside a parameter, and a
      receiver built through a fallible (`Result<Self, E>`) constructor whose own
      postcondition also reads `self` (the constructor-scan fix and this fix now
      interacting in one run). Reverting the fix reproduces the original `E0424` verbatim
      for all three.
- [x] **A comparison nested inside another comparison as a leaf did not compile.**
      `*result == (a == b)` (a boolean postcondition stated as an equality of two other
      equalities) rendered as `a == b as i128` -- `contract_rt::widen`'s catch-all leaf
      case cast the nested comparison's token stream to `i128` with no parens of its own,
      and because `as` binds tighter than `==`, that parses as `a == (b as i128)`,
      comparing `u64` to `i128` (`error[E0308]`). Fixed by giving `widen_leaf` its own
      case for a nested comparison or logical operator (`==`, `!=`, `<`, `<=`, `>`, `>=`,
      `&&`, `||`): recurse through `widen` itself (which already widens *that*
      expression's own leaves correctly, arithmetic included, so a mixed case like
      `a + 1 == b` nested as a leaf still cannot overflow while being checked), then
      parenthesise the whole result before casting it -- never taking the nested
      expression's tokens verbatim. Fixture: `tests/fixtures/nestedcomparison/`; test:
      `tests/e2e/tests/nestedcomparison_fixture.rs`, covering the reported repro
      (verbatim), a comparison nested under `&&`, one nested under `||`, a comparison of
      two expressions rather than two bare names, and a mixed arithmetic case -- all five
      needed the fix. Two more (`&&`/`||` as the postcondition's own outermost operator,
      no wrapping equality) are in the same fixture and confirmed, by testing against the
      pre-fix binary, to already have worked -- `widen`'s own `&&`/`||` recursion never
      routes through the leaf path those exercise. Reverting the fix reproduces the
      original `E0308` verbatim for all five.

## The text fix closed a recorded false clean — 2026-09-01

CI caught this, and it is the opposite of a regression. `excludedop` exists to record
"the fourteenth false clean" (`docs/review-structs-enums.md` finding 1): `Acc::get`
promises its result is always 0, that promise is false after one call to `Acc::note`,
and Ply could not call `note` because it took borrowed text. So every generated case
ran against a receiver only the constructor had touched, and a broken function reported
a clean pass.

Text arguments now work, so Ply calls `note`, reaches the broken state, and reports a
`violation`. The fixture's test still asserted the old, weaker truth and went red.

- [x] **`excludedop` keeps testing what it was written to test.** Its `note` now takes
      `Option<String>` — a `String` nested inside another type, deliberately never
      built — so the run still genuinely cannot call it and must say so. Verified: the
      verdict is `fuzzed(256)` marked narrower, and the warning names `note` and why.
- [x] **`textmutator` records the win.** The same shape with a `&str`, asserting the
      `violation` Ply now finds. Proved to bite: reverting the one-line text fix brings
      back `fuzzed(256)` on the broken function and the test fails demanding
      `violation`. That is the false clean itself, reproducible on demand.

- [x] **`skippedctor` too, same cause.** Its premise is a constructor Ply finds but cannot
      use, because the constructor took borrowed text. It is usable now, so no constructor
      was skipped and the disclosure it tests never fired. Its argument is now an
      `Option<String>`, and the test passes again.

Swept the rest rather than waiting for CI to find them one at a time: exactly three fixtures
depended on borrowed text being unbuildable, all three are handled, and the only fixture left
with a `&str` parameter is `textmutator`, which uses one deliberately.

Worth stating plainly because it is the first time this has happened today: a capability
improvement made tests fail by making Ply *better*, and the fix was to preserve both truths
rather than weaken any test.

## Text parameters landed, and the next blocker is a design problem — 2026-09-01

`&str` now reaches the sampler (one line: `str` maps to the same type `String` already used;
references were already looked through, so only the borrowed spelling was missing). Measured
as the largest single blocker — 11 of `semver`'s 16 properties.

Re-measured against `semver` immediately. **The count is still one in sixteen, but the
failure mode moved, and that is the finding.**

Probe: `Prerelease::is_empty(&self) -> bool`, whose receiver must be built by
`Prerelease::new(text: &str) -> Result<Self, Error>` — text parameter and fallible
constructor at once, both fixed within the last day. Before today this was refused before
anything ran. Now:

- Ply **builds the receiver and runs the check**. `Prerelease::new` is called with generated
  text, exactly as intended.
- The verdict is `unclaimed`, not `fuzzed(64)`, because **1025 of 1074 generated strings were
  thrown away** by the constructor's precondition. Random text essentially never parses as a
  valid pre-release identifier.
- Ply says so itself, unprompted: *"So this function has no fuzz evidence at all -- its
  verdict is `unclaimed`, not `fuzzed(64)`."* That is the high-rejection machinery working —
  the same machinery whose test was proved to bite this morning by planting a bug that turned
  rejections into passes.

- [ ] **NEXT BLOCKER, and it is a design problem rather than a defect: a type built by
      parsing text cannot be constructed by random text.** Uniform sampling will not produce
      a valid version string, identifier, or any other parsed format, so every property about
      such a type reaches the engine and comes back with no evidence. The honest reporting is
      right and worth keeping. What is missing is a way to generate values that satisfy a
      constructor — seeding from `examples:`, sampling a grammar, or reusing values the
      crate's own tests already contain. None of these is a one-line change, and choosing
      between them is a design decision, not an implementation.

The scoreboard, stated plainly: reach on `semver` has gone from "refused before anything ran"
to "ran, and honestly reported that it learned nothing". No property moved into the checkable
column. That is progress in honesty and none in coverage, and the two should not be confused.

## Re-measured `semver` after the reach fixes: it has not moved — 2026-09-01

The two reach defects fixed today (a promise may now mention its receiver; a comparison may
now nest inside a promise) plus yesterday's receiver-constructor fix were checked against the
library that produced the 1-in-16 result. **Reach is unchanged.** That is exactly what
`docs/reach-measurement-2.md` predicted — every unreached property is held by two to four
independent blockers, and its table records "unblocks alone: 0" for every capability
including the ones just shipped — but it was worth measuring rather than trusting, and the
measurement found something the table missed.

Probe: `Version::cmp_precedence(&self, other: &Self) -> Ordering`, the property about
comparing versions while disregarding build metadata. It converges three defects — a receiver
that must be built, a parameter of the receiver's own type, and a return type Ply can observe
but not construct. Two of those three are now fixed.

- [x] **NEW BLOCKER: a parameter written as `Self` is refused, where the same type spelled
      by name is not — fixed 2026-09-01 (`f2bfe88`), same fix as the entry earlier in this
      file.** `other: &Self` gave "parameter(s) other: Self use a type neither the bounded
      nor the fuzz codegen builds inputs for". Rewriting it as `other: &Version` -- which
      no compiler or reader would call a change -- got past that check entirely. Same
      asymmetry the measurement already found between `-> Self` and `-> Version` in the
      return position, now closed in the parameter position too: `Self` resolves through
      the receiver's own already-resolved type rather than a second lookup that could
      disagree with it.
- [ ] **Confirmed still open: the refusal that names nothing.** With the parameter spelled
      out, the same function is refused with "none of its declared checks apply to this
      function's shape" -- no mention of the return type that is actually stopping it. The
      measurement flagged this ("`unsupported_shape_diag` inspects only parameters, so when
      the blocker is the return type it falls back to a sentence carrying no information a
      user can act on"). It is unchanged.

The honest scoreboard: today's fixes are real and were verified to work in both directions,
but they move `semver` from one checkable property to one checkable property. A blocker only
becomes visible once the ones in front of it are gone, and removing two of four revealed a
fifth rather than a verdict.

## Two harness-generation defects fixed — 2026-08-31

Both were found by pointing Ply at `semver` -- see `docs/reach-measurement-2.md`,
which landed on `main` while this work was in progress and so was not visible
from the branch it was written on. The agent that fixed these noted, correctly
for what it could see, that the cited file did not exist and declined to
either invent the measurement or drop the citation. It exists; that note is
withdrawn rather than left to confuse a later reader.

- [x] **Defect 1 — a receiver's own constructor scan disagreed with the
      parameter path about what counts as a constructor.** A
      `Result<Self, E>`-returning `new` (or one spelling the type's own name
      instead of `Self`, bare or `Result`-wrapped -- four spellings of one
      shape) was recognised when building a *parameter* and reported as not
      existing when the very same scan was asked to build a *receiver* for
      the identical type in the identical run. Fixed by making the receiver
      scan (`scan_file_for_receiver`, `crates/ply-core/src/harness.rs`) call
      `ctor_return_kind` -- the one classifier the parameter path
      (`scan_ctor_candidates`) already used -- instead of carrying its own
      narrower, separate check, and by threading the resulting `CtorReturn`
      through `ReceiverPlan` instead of hardcoding `CtorReturn::Bare`.
      `fuzz_gen::receiver_preamble` now renders the same rejecting `match`
      around a fallible constructor call that `build_user_value_stmt`
      already renders for the parameter path. Fixture:
      `tests/fixtures/receiverresultctor/`; test:
      `tests/e2e/tests/receiverresultctor_fixture.rs` (all four spellings,
      plus the exact `A`/`Bad`/`read_it` reproduction, in one run). Reverting
      the fix reproduces the original false `V0507` refusal verbatim.
- [x] **Defect 2 — a method whose parameter shares its receiver's type
      generated a harness with the same `use` line twice.** The generated
      harness's extra-type-import scan (`extra_type_imports`,
      `crates/ply-core/src/fuzz_gen.rs`) deduplicated against its own output
      only, never against the primary `use` `wrap_fn_harness_module` already
      emits for the checked function's own type -- so a `&self` method
      taking another value of its own type imported that type twice
      (`error[E0252]: the name `Pair` is defined multiple times`). Fixed by
      backing the dedup with a real `HashSet`, seeded with the primary
      import up front, so "the receiver's type" and "a second parameter of
      the same type" are the same case as the existing two-parameters dedup,
      not a second special case beside it. Fixture:
      `tests/fixtures/sharedtypeparam/`; test:
      `tests/e2e/tests/sharedtypeparam_fixture.rs` (receiver+parameter,
      two parameters with no receiver, and receiver+parameter+return all
      naming the same type). Reverting the fix reproduces the original
      `E0252` verbatim.
- [x] **KNOWN GAP, found while writing the defect-2 fixture -- `#[ply::ensures]` on a
      receiver method cannot read `self`.** Nothing rewrites a bare `self` in the
      postcondition closure to the actual receiver binding before splicing it into the
      generated free-standing assertion, so `self.a == other.a` renders as a
      literal `self.a`, which does not exist outside an `impl` block --
      `error[E0424]: expected value, found module `self``. No fixture in the
      crate exercised this before (`grep`-confirmed: no existing
      `#[ply::ensures]` or `#[ply::requires]` on a receiver method reads
      `self`), so it was invisible until this task's own reproduction
      (`same_as(&self, other: &Pair)`, ensures reading `self.a`, matching the
      literal Defect 2 repro handed to this session) tried it. Fixing defect
      2 alone does *not* make that literal reproduction pass -- it trades
      `E0252` for `E0424`. The committed `sharedtypeparam` fixture avoids
      reading `self` in every postcondition so it isolates defect 2 cleanly.
      **Fixed 2026-08-31** — see "Two more harness-generation compile defects fixed" at
      the top of this file.
- [x] **KNOWN GAP, found the same way -- postcondition widening mis-parenthesises a
      nested comparison.**
      `contract_rt::widen`'s catch-all leaf case casts a whole nested
      comparison's token stream to `i128` without wrapping it in its own
      parens first, so `*result == (a.a == b.a)` (a boolean-returning
      postcondition stated as an equality of two other equalities) renders
      as `a.a == (b.a as i128)` -- because `as` binds tighter than `==`,
      that compares `u64` to `i128`, `error[E0308]`. No existing fixture
      wrote a boolean postcondition that way either. Worked around in the
      `sharedtypeparam` fixture by stating the same property as an `iff`
      (`(!*result || lhs == rhs) && (*result || lhs != rhs)`), which
      `widen`'s existing `&&`/`||` recursion handles correctly.
      **Fixed 2026-08-31** — see "Two more harness-generation compile defects fixed" at
      the top of this file.
## Ply pointed at a stranger's code: 1 of 16 — 2026-08-30

`docs/reach-measurement-2.md`. The method of `docs/invariant-reachability.md`, repeated on a
second library chosen before reading what Ply supports: `semver` 1.0.28, 2,117 lines, whose
author documents his guarantees unusually well. Sixteen stated properties. **Ply checks one.**

**Zero of the sixteen are out of the tool's shape.** No threads, no sequences, no hidden
state — sixteen pure single-function properties. This is the most favourable library the
project is likely to meet, and reach is 1 in 16. The single reachable property survives only
because the author wrote `-> Self` instead of `-> Version`; spelling the type out, which no
compiler or reader would call a change, turns the verdict into `unsupported`.

**It contradicts the first measurement almost item for item, which is the point.** Floats
ranked first there; `semver` has no float anywhere, so they unblock zero. Structs and enums
ranked last with "zero effect"; here they gate twelve of sixteen. The two dominant blockers
here — `&str` arguments, and refusal on the *return* type — never appeared on the first list
at all, because in the rate limiter everything was already refused at the parameters, so
nothing ever reached the return check. **A blocker only becomes visible once the ones in
front of it are gone.** One library's ranking does not generalise, and a standing list of
types to build is the wrong instrument; measurement per codebase is the right one.

Also measured: no capability unblocks even one further property on its own. `&str` blocks
eleven of sixteen, but every unreached property is held by two to four independent blockers
at once, so the first fix moves the count by nothing.

### Three defects, each reproduced independently before being written down

- [ ] **Contracts written the documented out-of-source way are accepted, then ignored.**
      `check` reports "6 of 6 fn claims point at a function Ply can find"; `verify` runs none
      of them and explains it with two warnings on the same function that contradict each
      other — one saying the contract exists and was used, one saying there is no contract.
      Neither states the actionable fact: only source attributes reach the engines. `check`
      is the command people run first and gives no hint of it.

Two smaller ones, recorded but not ranked. **When the tool was made to go red on purpose** --
`Version::new` was deliberately broken to return a non-empty pre-release, because a check
that never fails proves nothing -- it caught it, with an input strategy that makes the catch
real rather than lucky. But the terminal said "proptest shrank a failing case to this minimal
example" and then showed no example: the values live in `--json` only, alongside a runnable
failing test Ply wrote into the crate's `src/` without mentioning it. **To be explicit,
because the shorthand invites the opposite reading: no defect was found in `semver`. None was
looked for.** The other smaller one is that
the return-type gate causing much of the loss is documented in its own code comment as
blocking nothing technically, which makes it a deliberate narrowing that is now the
second-largest blocker in the measurement.

Nothing hung and nothing crashed. Cold run 21 seconds, warm run half a second.

## KNOWN GAP: the required-check names can be made impossible to satisfy — 2026-08-30

Branch protection is on, so `main` now requires named checks to pass. That makes the
*names* of CI jobs load-bearing, and four of the six are generated rather than fixed:

    shard: [0, 1, 2, 3]
    name: product-e2e (${{ matrix.shard }}/4)

The shard count is written twice — once as the list, once as the literal `4` in the
display name — and the name is what branch protection matches on.

- [x] **Change the shard count and every pull request blocks forever.** Built 2026-09-03
      as the `ci-gate` job, when the maintainer asked for documentation changes to skip
      the full run -- the same fixed-name gate is what makes *that* safe, since a skipped
      shard is only harmless if the required check is something that always reports.
      The rule on `main` still has to be switched to require `ci-gate`, which only a
      repository admin can do; until then the old per-shard names are what is required.
      Going to six
      shards produces jobs called `product-e2e (0/6)`…`(5/6)`, so a rule requiring
      `product-e2e (0/4)` waits on a check that will never report again. The pull
      request cannot merge and nothing explains why — the failure is a *missing*
      check, not a failing one, which is the harder kind to read. Editing only the
      list and not the string is worse still: the jobs are then named
      `product-e2e (4/4)` and `(5/4)`.

      The standard fix is a gate job that does nothing but depend on every shard and
      succeed, with a fixed name, and require that instead. Then the shard count is
      free to change and the required name never moves. Not built — it is a change to
      CI that cannot be tested without merging it, so it wants a deliberate decision
      rather than being slipped in.

      Cheap partial mitigation available today: require `product` and `kernel-mutants`
      (both fixed names) and leave the shards advisory. That protects the fast checks
      and the mutation gate but not the end-to-end suite, which is the one that has
      actually caught a regression on this branch.

## Review of the scheduler unification — 2026-08-30

An independent adversarial pass over the five commits. It confirmed the soundness argument
link by link, and confirmed the ordering code is byte-identical to what shipped before. It
also found a seventh bug the exhaustive check could not see, after six earlier planted bugs
had all died — which is the point the project keeps having to relearn: a check's adequacy is
measured, never standing.

- [x] **The check could not tell a tie broken on name from a tie broken on position.**
      Every test used names `n0`, `n1`, `n2`, `n3` — sorted in the same order as the
      positions they sat at, so the two rules were indistinguishable. Swapping one for the
      other left all 1,048,576 cases green, all eight smaller tests green, and green the
      test *named after the property it broke*. Names are now `d`, `b`, `a`, `c`, which sort
      neither with the positions nor against them, so neither substitution can imitate the
      real rule. Verified: replanted, and it now dies. This was not academic — real names are
      `component::function`, and a nested component makes name order and position order
      genuinely disagree.
- [x] **The check read its output as a set, so placing something twice was invisible.** One
      line comparing counts closes it.
- [x] **The spec never stated the rule the whole change turns on.** It said a claim *in* a
      cycle falls back; it never said the fallback also covers every claim that reaches one.
      The implementation has always behaved that way and no artifact said so. §5.5 now does,
      including why the coarse rule is the safe one.
- [x] **The unused stub-permission gate reads as though it agrees with the shipped rule.**
      It does not: it refuses only a caller inside the callee's own cycle, so it is looser
      exactly where it matters, and its own exhaustive test cannot notice — that corpus has
      two nodes and the disagreement needs three. Its crate doc now says so, and says that
      adopting it is a deliberate relaxation needing an argument the spec declines to make.
- [x] **Two claims in the spec were not true of the evidence they cited.** The measurement
      was dated 2026-08-30; it was made 2026-08-27. And it was described as "a real outside
      library", which it is not — it is `tests/fixtures/ratelimiter/`, in this repository,
      written from a design brief by someone told not to think about checkability and not
      told this project existed. That provenance is what makes the measurement worth citing,
      so overstating it as third-party was both false and unnecessary. Corrected in the spec
      and here; the commit message that carries it is already pushed and cannot be corrected
      in place, which is why it is written down here instead.

### KNOWN GAP: a function that calls itself is never denied credit by the ordering

- [ ] **Self-recursion is filtered out before the ordering ever sees it**, so a
      self-recursive claim is placed normally rather than denied. Credit for the self-call
      is still refused, but only because the claim's own result is not yet available when
      the decision is taken — an accident of sequence that no test pins. Meanwhile the
      exhaustive check *does* include self-loops and requires them denied, so the tested
      rule and the real input space quietly disagree about this one case. Pre-existing, not
      introduced by this change; found by review 2026-08-30. Wants a test pinning that a
      self-recursive claim earns nothing from itself.

### KNOWN GAP: the spec still claims a restriction nothing enforces

- [ ] **§5.4a says contract strings are restricted to a closed subset. Nothing checks
      that**, and this repository's own rate-limiter fixture violates it. Flagged inside
      `docs/invariant-reachability.md`, which the spec now cites as evidence — so the spec
      leans on a document that names one of the spec's own claims as needing retraction, and
      the retraction is still undone. Predates this branch; recorded 2026-08-30 rather than
      left to be found again.

## What widening the types is actually worth — decided 2026-08-30

Agreed with the maintainer: **stop treating "support more types" as the roadmap.** The
evidence against it is already in this repository and it is unusually direct.

On the one library anyone measured against -- `tests/fixtures/ratelimiter/`, designed by
someone told not to think about checkability and not told this project existed, who wrote
down eleven properties they cared about -- the share of supported types went from 21% to about
80%, and the number of those properties that became checkable went from zero to zero.
Sixty points of work, no movement in the thing the work was for. The number was counting
how often a type appears on a public surface, which turned out to be nearly unrelated to
whether anything could be proved. It was also dominated by getters and configuration,
while the type that library's whole correctness argument rested on had a public-surface
count of zero, because it was internal state.

What actually blocked those eleven: finding the function at all, building the object a
method needs before it can be called, mutation, and floating point. Floats have since
landed (`2443b85`), which is the strongest form of the argument -- the single
highest-ranked blocker is discharged, so "more types" is not what stands between Ply and
the next real property.

**The replacement question, to be answered before any further type work is scheduled:**
take a library whose author enumerated their own properties, and for each one record
whether it is a single-function property at all, what specifically stops it, and whether
the author flagged it as risky. Then rank by *which single missing capability unblocks
the most properties*. That ranking put structs and enums last on the one library it was
run against -- the opposite of what type coverage implied.

**Two reasons a function cannot be checked, and they are not the same thing.** Conflating
them is what makes "Ply's checkable subset is too narrow" sound damning when it mostly is
not:

- **Out of shape, permanently.** A sixteen-thread stress test is better evidence than
  anything a single-function checker could produce. Refusing by name is the product
  working. This category should be counted and reported, never quietly widened toward.
- **In shape, unplumbed.** A genuine single-function property over a value Ply could
  sample, blocked only by not being able to build the argument. This is a gap.

Only the second is measurable, so effort drifts toward it whether or not it is where the
value is. That drift is exactly what the 21%-to-80% episode was.

**Consequence for the plan:** the honesty machinery (the rule registry, staleness
reporting, the "what was NOT checked" output) is scheduled ahead of further type work.

**Corrected the same day, before this was acted on.** The first version of this paragraph
said the ledger "works across a whole codebase regardless of types" and called the proof
engine "a bonus on the slice where it happens to be cheap". The second half is wrong and
is withdrawn. A ledger with no engine behind it is a spreadsheet of assertions: the
evidence ladder only means anything because its top rungs are sometimes reached, and if
nothing ever reaches them the whole design collapses into "tested / not tested". Proof is
what makes an entry in the ledger worth more than a claim; the ledger is what makes proof
safe to trust and safe to lack. They are complementary, not ranked.

What is true is narrower: **proof pays where it is concentrated, not where it is spread.**
A small pure core carrying consequence out of proportion to its size is exactly where it
earns its cost -- which is why this repository's own kernel gets exhaustive enumeration
over every tree to a bound plus an unbounded inductive proof, and why the mutation run
that guards it has already found real dead code and a blanked failure message that would
have left every future counterexample unreadable. The mistake was never valuing proof; it
was expecting proof to spread evenly across a codebase.

That gives a targeting rule rather than a coverage programme: **widen toward the shapes
where proof is cheap AND the consequence is concentrated**, and let the rest be recorded
honestly. It also makes the gap below worse rather than acceptable -- Ply's kernel is
precisely the shape where proof pays most, and Ply cannot reach it.

### KNOWN GAP: Ply's own file does not record what Ply's own evidence is

- [ ] **Ply's self-declaration is silent about both halves of its own honesty.** `ply.yaml`
      declares which crates exist and which may depend on which, and stops. It does not
      record that the verdict kernel and the check scheduler carry the strongest evidence
      in the repository -- exhaustive enumeration over every tree to a bound, an unbounded
      inductive proof for the kernel, a mutation run in CI that checks the check can still
      see. Nor does it record that the parsing, rendering and process-driving shell is out
      of reach and always will be.

      Both omissions matter, and the second more. Ply's own argument is that saying "I
      cannot see this" is the whole premise, and that the count of out-of-reach things
      should be reported proudly rather than hidden. Ply does not do that for itself: a
      reader of its self-portrait sees neither the proof nor the gap.

      Sharpest form of it: **Ply cannot check its own most-proved code.** The kernel's
      entry point takes a reference to a recursive tree; the scheduler takes a set of
      numbers, a slice of strings and a map of sets. Ply can build a value for none of
      those. Pointed at its own core, it would report that it cannot see it.

      This is a gap in the one file whose entire purpose is that its claims are checked,
      and it was not written down anywhere before today.

## Verification results now change what the drawing looks like — 2026-08-30

Left in the working tree, not committed (explicit constraint for this session) —
`crates/ply-core/src/visual/svg.rs` and `tools/render/tests/visual.rs` only.

- [x] Fn chips now colour by the five display states (declared/earned/violated/
      unanswered/stale), computed purely from the stored evidence a run already
      reported — never fabricated. Each state pairs its own fill/border with its own
      drawn character (a reader with no colour vision still tells them apart), and
      "earned on assumptions" reuses the earned colour with an attached mark rather
      than inventing a sixth state, per the settled state model. `violated` is the
      only new red; `unanswered`/`stale` are neither red nor the existing findings-red.
- [x] The opening verdict strip now states result counts ("2 earned, 1 broken, ...")
      alongside its existing promise counts, only once a run's evidence actually
      settles something — a document with no evidence, or evidence that resolves to
      nothing beyond "declared", renders its strip exactly as before (checked, not
      assumed: `the_strip_states_no_results_when_evidence_settles_nothing`).
- [x] A collapsed box now states its earned-over-promised split as a plain count
      (`"1 of 3 earned"`), never a percentage — the rejected two-part-meter design
      that would let nine-earned-one-untouched read as "90% healthy" stays rejected.
      `a_collapsed_boxs_earned_split_equals_the_counts_folded_beneath_it` renders the
      same evidence both expanded and collapsed and checks the two counts agree.
- [x] New public API: `render_svg_with_evidence_and_options`, so evidence and
      `--depth`/`--focus`/`--collapse` can be exercised together (previously only
      `render_svg_with_evidence`, always fully expanded, existed). Not wired into
      `cargo ply verify`'s own publish path — out of scope for this change, which is
      the renderer only.
- [x] 12 new tests in `tools/render/tests/visual.rs`, including the two invariants
      named above and one confirming red still means only `violated`/`deny`/`finding`
      for evidence-drawn output specifically (the pre-existing red test only ever
      renders fixtures with no evidence, so it could not have caught a regression
      here). `cargo test --workspace --exclude ply-e2e`: 616 passed, 0 failed (604
      baseline + 12). `cargo fmt --all` and `cargo clippy --all-targets -- -D
      warnings` both clean. `git diff --stat -- vetting/ docs/` is empty — no
      committed artifact changed.

## Verus pin moved forward — 2026-08-30

Done. The spike pinned **0.2026.08.15.7d4628a**; it now pins **0.2026.08.23.fbbbbcf**,
the current stable. (A 0.2026.08.30.b432e82 rolling build also exists and was not used:
a rolling build is the wrong thing to rest a recorded proof on.)

- [x] **Kernel proof moved to Verus 0.2026.08.23.fbbbbcf.** Re-obtained rather than
      bumped, because the claim rests on what the verifier said and not on a string:
      **22 verified, 0 errors**, identical to the old pin, with the proof file needing no
      edits at all -- no syntax migration, no deprecation, same required toolchain.
      1.43s against the old ~2s, which is one measurement on one machine and is not
      claimed as a result.
- [x] **The vacuity checks were re-run too, and they are the load-bearing half.** A proof
      that verifies against a broken kernel proves nothing, so both recorded mutations
      were replanted on the new release: each still produces 20 verified / 2 errors, in
      the same two obligations as before. Reverted, 22/22 clean afterwards. The newer
      Verus is not passing this proof more easily -- it fails in the same places.
- [x] Incidental: `diff/Cargo.lock` was stale (the spike had not run since `ply-core` grew
      its dependencies) and is refreshed by 248 lines. Not an effect of the upgrade.
- [ ] **The honesty condition is unchanged and still applies**: the proof runs over a
      shadow of the kernel, not its production source, and the differential test is what
      licenses the shadow to speak for `aggregate()`. Re-check it whenever either side is
      edited. Not a task so much as a standing condition, kept here so it travels.

## One workspace, and evidence that reaches the drawing — 2026-08-30

- [x] **`tools/` merged into the product workspace.** The split existed because the
      tooling "predates the product", while every crate in it already depended on
      `ply-core` — one dependency graph pretending to be two. It cost three real things:
      `cargo mutants` could see neither side (pointed at tools it found no members;
      pointed at the product it ran `ply-core`'s thin suite while the renderer's 91-test
      suite sat across the boundary), the tests for `visual/svg.rs` lived in the other
      workspace, which is how a green test came to pin a false sentence, and the two
      clippy invocations differed so a lint firing in one was invisible in the other.
      `tests/spike` and `tests/fixtures` stay excluded, and that exclusion is principled:
      each carries its own workspace root and several exist to be built in isolation.
- [x] **Ply's own document now describes all eight of its crates**, not four. The four
      tooling crates and their six real dependency edges were invisible to Ply's
      self-check while they lived in the second workspace — a file whose entire purpose
      is that its claims are checked, silently omitting half its own codebase.
- [x] **Evidence attaches while the picture is drawn.** It used to render the SVG, then
      search its own output as a string to find the shapes it had just drawn. Its doc
      comment conceded the consequence — elements "left unattached rather than guessed" —
      and it was happening: a nested component never attached at all, because the matcher
      compared a bare function name against a dotted path it could never equal. Top-level
      components worked by coincidence, which is why every existing test looked right.
      ~230 lines of re-parsing deleted; output byte-identical with no evidence passed.

### The two schedulers order cycles differently — analysis, 2026-08-30

Before anyone unifies these, the mapping is not vocabulary. The two implementations
disagree about **where a call cycle goes**, and only one of them is tested.

**Shipped** (`crates/ply-cli/src/verify.rs`, `topological_order`, ~50 lines): returns
`(order, cyclic)` — a linear order of everything it could place, plus the set it could
not. The caller concatenates them, so **every cycle member is processed last, in a
lump**, no matter where the cycle sits in the dependency graph.

**Pure** (`tools/schedule`, `plan`): collapses strongly-connected components and returns
layered batches, so **a cycle is processed at its own layer** — early if things depend on
it, late if it depends on things.

The pure version is the more defensible ordering: a cycle that half the codebase depends
on should not be verified after its dependents. But the shipped version is the one that
has absorbed real review fixes — the `domain` restriction (adversarial review,
2026-08-26) exists because an earlier version sized everything off `node_ids.len()` and
silently admitted reused and fuzz-only claims into the ordered pass. The pure copy never
saw that class of bug because nothing calls it.

So unification is a decision, not a move:

- [x] **Decided which ordering ships: the shipped one** (`4dd4d30`). Its leftover
      set is not merely the cycle's own members — a function only becomes orderable
      once every function it calls has been placed, so a function that calls into a
      cycle, however many steps removed, never becomes orderable either. Adopting
      `plan`'s layering instead would have handed assumed-contract credit to exactly
      those steps-removed dependents, which the shipped ordering deliberately
      withholds. The ordering moved into `crates/ply-core/src/schedule.rs` unchanged
      (same Kahn's-algorithm body, same id tie-break), and the returned leftover set
      got a name that says what is in it (`tainted`, not `cyclic`).
- [x] Carried across unchanged, so every review fix travelled with the code rather
      than needing to be re-found: the `domain` restriction (2026-08-26, the one with
      its own comment) and the reuse-decided-after-ordering fix are both still in the
      moved function's doc comment and behaviour, verbatim.
- [x] **The exhaustive check moved with it and grew a second dimension**
      (`crates/ply-core/tests/schedule_enumeration.rs`): it now varies which
      functions are in scope, not just the call graph's shape — 1,048,576
      combinations, because scope (a claim that should never have entered the
      ordered pass at all) is exactly where the 2026-08-26 bug lived. Checked
      against an oracle computed a different way (SCC + reachability) than the
      implementation (indegree counting).

### KNOWN GAP that outranks the rest, found 2026-08-30 — CLOSED (`4dd4d30`)

- [x] **The check scheduler no longer exists twice.** `tools/schedule`'s `plan`/
      `Batch` (the untested, more permissive copy) are deleted; `may_stub` and its
      own exhaustive test are untouched and still live there. The one real ordering
      is `ply_core::schedule::order`, called directly by `crates/ply-cli/src/
      verify.rs`, and it is the copy the exhaustive test now covers. Verified after
      the fact, independently: three deliberate breakages of the new module (a node
      placed before one of its callees, a cycle's dependent left out of the tainted
      set, the id tie-break replaced with a hash-based one) each made the
      enumeration fail, and each failure named the actual defect rather than a bare
      assertion. The full engine-backed suite (`cargo test -p ply-e2e`, 89 fixture
      binaries, Kani proofs included) was run to completion afterward: 163 tests,
      0 failures — the landing commit's own note ("the engine-backed suite has not
      reported yet") is now resolved.

### A fourth review, and the half-fixes it found — 2026-08-30

The pattern in all three: I had fixed the instance in front of me and reported the class
as done.

- [x] **The drawing still told both lies the text form had been taught out of.** The
      morning's fix landed on the transcript only, and a green test *required* the
      drawing to say an example was "compiled into a test" against a function declaring
      `[bounded(3), fuzz(1024)]` — a passing test pinning a false sentence in exactly the
      configuration where it is false. Both sentences now come from two shared helpers,
      and both are future-conditional: neither view runs a compiler, so neither may say
      one ran. The TODO tick claiming otherwise was an overclaim about my own work.
- [x] **`SCHEMA.md` §8 still opened "None of the rules in this section is enforced"** while
      §2 and §14, corrected earlier the same day, said crate-level `edges:`/`deny:` are
      checked. Replaced with a tier matrix stating row by row what is enforced (`A0401`,
      `A0405`) and what is only recorded, verified against the code rather than the prose.
- [x] **The malformed-example refusal wore the wrong identity**: `V0507` (a code in no
      documentation anywhere), `severity: "warning"` for something that refuses a claim
      and exits non-zero, and `open_item: "unsupported_signature"` — false, since the
      signature is fine and the document is malformed. A dedicated constructor emits a
      real `E0501` at error severity now. The regression asserted on a substring of the
      human title, so it passed with the code wrong; it asserts the `code` field.

### KNOWN GAPS, left open deliberately

- [ ] **The doc test is a substring ratchet, and its weakness is measured, not assumed.**
      A reworded blanket lie inserted under §8 alongside an intact matrix passes all three
      assertions. The real fix is the rule registry below; until it exists this catches
      the historical sentence and the matrix vanishing, which is worth having and is not
      a proof.
- [ ] **The malformed-example diagnostic carries no pointer at the offending YAML line.**
      `diag.rs` documents pointers as present only on E0201/E0204, so this is consistent —
      but that rationale ("a diagnostic about a function points at source, not at YAML")
      argues a diagnostic that *is* about a YAML line deserves one. Follow-up, not a
      defect; the title quotes the entry.
- [ ] **The tier matrix omits a `profile:` row** that §8 documents in its own subsection.
      Nothing is claimed enforced that is not, but the honest summary skips one construct.


- [ ] **The rule registry.** The ratchet above catches a phantom *code*; it cannot catch a
      phantom claim with no code in it ("compiled into a test" had none). The real fix is
      a table of rules as data — code, tier, implemented, severity, gloss — that the
      checker, its NOT-CHECKED paragraph, and both views all derive from, so an unenforced
      rule cannot be described as enforced by construction. That is a design change and
      wants review, not an agent's initiative.
- [ ] **Multi-line author text can still impersonate transcript structure.** A note
      containing a newline renders at column 0 and can look like a heading the renderer
      wrote. Control bytes are handled; layout is not. Re-flowing multi-line prose legibly
      has real design questions (indent, wrap, quoting) and the threat model today is a
      trusted author, so half-solving it would be churn.
- [ ] **`block()` finds the first heading with a given name**, so two functions sharing a
      name across components remain a blind spot in the scoped needle checks.

## Review of the transcript, and what it found — 2026-08-30

A second model reviewed the feature below. It was right about almost everything, and the
headline is bad: **the safety net was largely an illusion.** Thirteen deliberate
breakages of the text renderer were run against the whole suite and only one died. All
are now fixed and all die.

- [x] **The worst one: the feature lied in the exact place it was sold on.** A function
      that wrote no checks line at all, and inherited an empty list from an ancestor, was
      told it had *written* an empty list. Same sentence, byte for byte, as a function
      that really did write one — the two opposite statements this view exists to keep
      apart. It now says which ancestor switched checking off, and says the function did
      not ask for it.
- [x] **The completeness test skipped four fields it never mentioned:** the seal, the
      build-fails-here flag, machine-written functions, and worked examples. Deleting the
      entire worked-examples block left every test green. Both structures are now bound
      field by field with no catch-all, so a field added later stops the test compiling
      rather than being quietly unchecked. Two fixtures that exercise those fields were
      added to the set it reads; there was no `mode: synth` in any of them before.
- [x] **The derived sentences had no test at all.** They restate no field, so a walk over
      fields cannot see them — deleting how strongly a component is checked, or why, or
      that it declares nothing yet, all passed. Every component block must now answer both
      questions, and each of the sentences that answers them is pinned word for word.
- [x] **Wrong rule, wrong severity, in the sentence a reader would quote.** Both views
      said a sealed component touching a capability "is an error (A0408)". It is A0403,
      and a warning unless the component is also marked to fail the build. A0408 is a
      different rule about helpers used inside contracts. Pre-existing in the drawing; the
      text form copied it onto a second surface instead of catching it.
- [x] **A component marked sealed *and* declaring capabilities silently lost the
      capability list** — the view telling a reader the document said less than it did,
      and dropping the half that would explain a surprising finding.
- [x] **Two header sentences were false.** "Nothing here has been run" is not something a
      renderer handed a parsed document can know, and is flatly wrong for anyone who just
      ran a verification; it now says no result reaches this page. And the summary's gloss
      called the counted functions "code this document says nothing about" — in the
      trading-system example both counted functions wrote `checks: []`, so the document
      says something very deliberate about 2 of the 2 it described.
- [x] **Both views claimed an enforcement that does not exist.** An open question was said
      to cap a function's checks; §5.6 says in as many words that the cap is not enforced,
      and a verification runs the full claim anyway. `worklist` has always said so on
      every marker line. The two views now do too, and share one copy of the sentence
      instead of two.
- [x] Newbie bar: "contract at the watermark" (jargon, glossed by more jargon) and "the
      level above" (reads as the parent component, not the previous line) rewritten.
- [x] **`plural()` leaked a fresh allocation on every call**, under a comment claiming no
      allocation could enter. In this repo of all repos.
- [x] Spec §7.1a said the walk "visits every field" and that there is "only one wording"
      of a shared fact. Neither was true when written; both are retracted and replaced
      with what actually holds, including the day the seal sentence was worded two ways.

Goldens moved and were read, not accepted: the three vetting drawings and the
architecture diagram changed by exactly the three corrected sentences, no geometry.

**KNOWN GAP, left open on purpose.** A component's stated level ignores open questions,
so a component can say it declares checks up to the strongest level while a function
inside it says an open question holds it down. Both sentences are individually true and
they sit four lines apart. The fix belongs in the shared ceiling computation and changes
the drawing too, so it is its own change rather than a rider on this one.

## Component notes, and the envelope's reasoning — 2026-08-28

Both from a second person's smoke test of the grammar, decided by review rather than on
the spot: the first instinct -- add a free-text `description` -- would have been wrong.

- [x] **A component may carry a `note:`.** Every prose slot in this grammar already sits
      where checking is impossible, and `externals`' note is *required* on exactly the
      reporter's own argument -- "a bare name tells a newbie nothing". A component's
      rationale is the fourth of those. Ply's own three load-bearing rules moved out of
      comments and into notes, so the file keeps its reasoning instead of discarding it.
- [x] **Not on functions.** The reporter had written a real invariant as an `examples`
      string because no better slot was visible; the answer there is `ensures`. A note
      beside it makes that mistake comfortable -- the next invariant lands in the note
      and never becomes a promise.
- [x] **The envelope carries the contract and the trusted claims.** The tree said only
      what came out, so an agent could read `fuzzed(64)` and not know what it was fuzzed
      for -- and §7.1 already assumed otherwise. Set from the claim rather than the run,
      so a reused verdict carries it too; wiring it to the run first and watching the
      second run come back bare is how that gap was found.
- [x] **`cargo ply render` says when a selection folded nothing away.** The subcommand
      that landed on main does not carry that notice; the standalone binary did. It was
      written for a first-time reader -- one of them recorded the silence as a bug before
      deciding it was correct behaviour -- and delivering it from one entry point and not
      the other would be a poor joke.
- [x] Half the report was wrong and is recorded as such rather than quietly fixed:
      `audit --json` has always carried the trust surface. What was missing was the tree.
- [ ] Comments still evaporate, deliberately: making them survive needs a position-aware
      second parse this codebase does not have, and would let prose reach output without
      passing validation. The component note is the declared home instead.
- [x] `cargo ply render --help` advertises the global `--json` flag. It now emits the
      declaration-only visual envelope, so clients can navigate the YAML hierarchy before
      code or evidence exists; every item is explicitly `unclaimed`.

## Agreed 2026-08-28, not yet done

**Order the maintainer set: land the release, then the bug backlog, then suite speed.**
Nothing below jumps that queue.

- [x] **Put the pre-code renderer on the installed command (`15adc47`).** The documented loop says
      Ply can render a `ply.yaml` before code exists, but the working renderer is shipped
      only as a separate development tool. `cargo ply render` must use that same renderer,
      accept its existing depth/focus/collapse controls, and write either stdout or an
      explicit SVG path. A second renderer is refused.

- [x] **The end-to-end CI job is sharded across a matrix of four.** The suite is 84
      independent test binaries that ran as one job for over an hour, which was most of
      the wait on every pull request. No test, no product code and nothing about what is
      checked changed. The split is computed at run time from the files on disk rather
      than written into the workflow, so a test added later lands in a shard by its
      position and cannot be silently left out of all of them -- a hand-maintained list
      would rot the first time someone forgot it, and a test nobody runs is the kind of
      absence this project treats as a defect. Round-robin rather than contiguous
      blocks, so the slow Kani-backed tests, which sit together alphabetically, spread
      3/2/2/3 instead of landing in one shard. Verified before pushing: the four shards
      are disjoint and cover all 84 files, and the exact command the workflow runs
      compiles a real shard. The honest cost: every shard pays the engine install, only
      partly cached, so the real figure sits above total/4.

- [ ] **Cut the duplicate proofs out of the end-to-end suite.** Measured today: 137
      fixture copies across 71 distinct fixtures, so the same code is proved many times
      over -- `resultreuse` and `implmethod` nine times each, `structenumparam` and
      `clamp` eight, `reusehelper` seven. Kani-heavy binaries alone are 1,020s of the
      3,053s of test time. The fix is test design, not infrastructure: several tests
      that each prove one fixture to assert on different parts of the same run become
      one test making several assertions about one run. Caching was already tried and
      measured (docs/suite-speed-finding.md): 2,533s before, 2,569s after, no speed-up,
      because those tests run concurrently and all miss together. Deferred by the
      maintainer until after the bug backlog -- a large refactor of the tests that
      vouch for a release is the wrong thing to do while landing one.

- [x] **The Rust toolchain is pinned (`rust-toolchain.toml`, 1.98.0).** Done 2026-08-30:
      CI and a contributor's machine now agree by construction rather than by anyone
      remembering to type `cargo +stable`. Raising it is deliberate — bump the line, run
      the suite, and fix the lints the new release brings in the same change.
- [ ] ~~Consider pinning the Rust toolchain (`rust-toolchain.toml`).~~ CI installs
      `stable`, which was 1.98.0; this container had 1.94.1, four releases behind, and
      clippy gained lints in between. Two `-D warnings` failures therefore could not be
      reproduced locally and were pushed red. Installing `stable` here and running
      `cargo +stable clippy` reproduces CI exactly and is the working practice from now
      on, but a pinned toolchain would make the two agree by construction rather than
      by remembering.

- [ ] **Make CI a required check on `main`, so a red pull request cannot be merged.**
      Asked for by the maintainer after PR #4 offered a merge button while all three
      jobs were failing -- twice, on two different causes. GitHub will not block a merge
      on a failing check unless the branch is protected and the check is named as
      required. Needs a branch protection rule on `main` requiring `product`,
      `product-e2e` and `tools` to pass. This is a repository setting, not a code
      change, so it needs doing in GitHub's settings by the repository owner (or via
      the API with admin rights) -- Ply cannot set it from here.

## Smoke test on a real project — 2026-08-28

The maintainer ran Ply against a project of their own and reported what broke. Findings
in their words, and what happened to each.

- [x] **`cargo ply --version` did not exist.** It reports two numbers now, because they
      answer different questions: the release, and the build identity -- the content
      hash that decides whether a stored result may be carried forward, so the number a
      run means when it says "the build of Ply that checked it changed".
- [x] **The help text contradicted itself**, describing the CLI as a slice that
      "implements only `verify`" while listing four working commands under it.
- [x] **`--depth`/`--focus` on a flat document emitted identical output silently**, which
      the maintainer initially recorded as a bug before deciding it was correct. It now
      says so: the note is earned by rendering the default drawing and comparing, so it
      cannot disagree with what was drawn.
- [x] **Flow labels stranded at `--depth 1`** -- one in the title band, one at y=162 on a
      canvas 152 tall, invisible. Reproduced exactly. The label placement escalates away
      from its line until it clears every box, and between two boxes side by side there
      is no such spot, so the search ran off the page. A position outside the drawn
      content is no longer a candidate. Pinned by an invariant test that walks the real
      output of several documents at several depths and fails on the first label outside
      the canvas.
- [ ] **KNOWN GAP, reproduced and not yet fixed: edge lines strike through box text.**
      An edge between two boxes with a third between them is drawn as a straight line
      through the middle box. Repro: three top-level components in a row, an edge from
      the first to the third. Fixing it means routing a line around obstacles -- a
      waypoint on the path and an obstacle test -- rather than the single straight
      segment `render_edge` draws today.
- [x] **`check` cannot check architecture without a Cargo workspace**, so the pre-code
      half of the loop is render-only. True, and the README implied otherwise. The
      development-loop section now says which half of step 2 works before the code
      exists: the drawing always, the document's grammar always, the architecture check
      not until there is a Cargo project to read a dependency graph from.

## Bug fixes after 0.1.0 — branch `claude/bugfix-post-0-1-0`

- [x] **The README says how to install it.** There was no install path written down at
      all, which is a strange gap in a tool someone is about to try on a real project.
      Every command in the new section was run before it was written: installing the
      CLI straight from the repo, adding the attribute crate to a project that had
      never seen Ply, and getting a real `fuzzed(64)` out of `cargo ply verify` -- with
      the project's `Cargo.toml` byte-identical afterwards. The engine prerequisites
      (`kani-verifier` for `bounded`, `cargo-mutants` for `mutate`) are named, along
      with what Ply leaves on disk and what it does not.

- [x] **A constructor in a qualified `impl` block was invisible on the parameter path,
      and Ply said the type had none.** The receiver path learned in 2026-08-27 that
      `impl super::T`, `impl crate::T` and `impl Alias` name the same type as
      `impl T`; the parameter path kept the older bare-name-only rule. So a type
      declared in `lib.rs` with its `impl` block in a submodule -- which has no other
      spelling available -- was reported as having "no constructor Ply can call", about
      a type with a public `new`. A false sentence rather than a missing feature. Both
      paths now use the same resolution. An `impl` that ends in the same bare name but
      cannot be resolved to the type's own canonical module is still refused: building
      the wrong type is worse than refusing to build one.
- [x] **The same false sentence had a second cause, on both paths: a constructor that
      returns the type by name rather than `Self`.** `pub fn new(..) -> super::Quota`
      is ordinary Rust; only `-> Self` was recognised, so the constructor was invisible
      for an ordinary parameter *and* for a receiver, the receiver message reading
      "none was found". Found by probing the same family rather than by the suite. Both
      paths now accept the type's own name, resolved to its canonical declaration
      (`Confirmed` only -- another module's same-named type is a different type).
- [x] **Two more causes of the same sentence, found by writing ordinary Rust and
      watching Ply be wrong about it.** An `impl` block inside an inline `mod` in the
      same file was never looked at -- the scan walked only the file's top-level items;
      it now flattens inline modules, carrying the module path down so `super::` still
      resolves to the right place. And a parameter spelled `crate::Beta` rather than
      `Beta` was carried as the rendering of a token stream, `crate :: Beta`, which the
      by-bare-name type lookup could never match and no sentence should quote at a
      reader; a plain path now keeps its bare last segment. `ordinaryspellings` fixture
      and test, both watched red.
- [x] **REGRESSION I INTRODUCED, found by review and fixed.** The first version of the
      qualified-parameter fix trimmed every path to its last segment before looking the
      type up. A parameter naming another crate's type (`v: depx::Thing`) then resolved
      to a same-named local type, built the wrong thing, and reported a compile failure
      in Ply's own generated code -- a calm, correct refusal turned into an internal
      error blaming Ply. Reproduced outside the repo before fixing. A plain path now
      keeps its qualifiers, and a qualified spelling is accepted only when those
      qualifiers match the module the type is really declared in; `super::` with no
      module context to resolve against is refused rather than guessed at. Pinned in
      `ordinaryspellings` and watched red against the trimming version.

- [ ] **OPEN, out for review: a type whose only constructor is `impl Default`.** Ply
      says "it has no constructor Ply can call", which is false -- `T::default()` is a
      constructor anyone can call. Building via Default yields exactly one value, so a
      `fuzz(256)` claim would report 256 cases having tried one distinct input, which
      is the silent-narrowing failure this project exists to prevent. Three options
      (correct the sentence only; build via Default; build via Default plus the bounded
      operation sequence the receiver path already uses) went to a reviewer, whose
      answer is: take the third, and correct the sentence now regardless, because it is
      false on both the parameter and the receiver path today. The second option cannot
      be made honest by disclosure, because the case count in the verdict is itself the
      claim: 256 runs of one value is one test run 256 times. Two cautions recorded
      with it -- `#[derive(Default)]` declares no `fn default` in the source, so a scan
      reading only `impl` blocks would recognise the hand-written one and miss the
      derived one; and when a type has no operations at all the sequence degenerates
      back to a single value, which needs the count clamped or the disclosure escalated
      rather than the general sentence quietly covering it.
- [x] **A constructor found and then found unusable is no longer reported as absent.**
      Every refusal opened "it has no constructor Ply can call", which is true only
      while nothing was found. A constructor Ply found and could not use -- private, or
      with an argument it cannot build, `fn new(n: impl Into<u32>)` being the ordinary
      case -- was recorded and then dropped by every refusal arm. The note now replaces
      that clause wherever it exists, so the sentence names what was found and why.
- [x] Fixed alongside: types were quoted with token-stream spacing (`impl Into < u32 >`,
      `Vec < u8 >`) in every message that fell back to that rendering; and
      `NotFound`'s wording told someone whose parameter was `impl Into<u32>` that Ply
      found no such struct declared, sending them to look for a declaration they never
      wrote.
- [x] **The Default-only sentence is corrected**, as the reviewer said to do regardless
      of the construction work. A type whose only constructor is `Default` -- written by
      hand or derived, both now recognised -- is told exactly that, and told why Ply
      does not build through it yet: one value is not many sampled cases, however many
      times it is run. Construction itself (option (iii)) stays open below.
- [x] **A type declared inside an inline `pub mod` was invisible to the type index.**
      The blindness fixed in the constructor scan was still in
      `scan_crate_type_locations`, which walked only each file's top-level items and so
      recorded an inline-module type as living at the crate root -- where it matched
      neither the `holder::Gauge` a caller writes nor the path the generated harness
      has to import. A fully public type with a public `new` was refused as if Ply had
      never heard of it. The index now records the inline `mod` chain alongside the
      file, and the harness imports the type by its real path. A type inside a
      *private* inline module is real and unreachable, which is a different answer
      again, and now gets its own sentence rather than the generic refusal.
- [x] KNOWN GAP, unchanged and now written where it is: a `Result<Self, E>`
      constructor is recognised for a parameter and still not for a receiver.
- [x] `qualifiedctor` fixture and end-to-end test, watched red. The test also weakens
      what the constructor guarantees and requires the verdict to go red, so a green
      run cannot be Ply quietly not calling it. It asserts each path's own leaf verdict
      rather than the worst-of root, which either path alone could satisfy.
- [x] Fixed while there: a diagnostic quoted a type as `super :: Quota` -- token-stream
      spacing leaking into a sentence held to the newbie bar. It now reads
      `super::Quota`, as written.

## Ply borrows the user's Cargo.toml and gives it back — 2026-08-28

- [x] **A run on a crate with its own workspace no longer leaves an edit behind.**
      Found while cleaning up a stray modified file in this checkout, which turned out
      to be a Ply run artifact, then reproduced on a scratch crate outside the repo.
      Registering the generated harness as a workspace member is still how the mutate
      engine finds it, but the registration is now held by a guard that puts the
      original manifest back byte-for-byte when the run ends, error paths included.
      Confirmed end to end: a real crate with `members = ["."]`, checked with both
      `fuzz` and `mutate`, comes back `fuzzed(64)·spec-strong` with its `Cargo.toml`
      byte-identical to what was there before.
- [x] **The generated failing test survives the cleanup.** Removing the membership
      alone would orphan the harness -- neither a workspace root nor a member of one,
      so unbuildable -- and the counterexample Ply just rendered would be unrunnable.
      The same guard release rewrites the harness manifest into the standalone shape;
      `cargo test` in `target/ply/fuzz/<name>/` fails on the seeded bug afterwards,
      checked directly rather than assumed.
- [x] Four unit tests, each watched red against the specific defect it names (restore,
      whole-line removal, don't-touch-what-changed, clear a stale entry), plus the
      `existingworkspace` end-to-end test rewritten to assert the new contract.
      The-Ply-Spec.md §5.4c amended.
- KNOWN GAP (in the spec, deliberately): a run killed outright runs no guard, so the
  `members` entry survives a `SIGKILL` or a crashed container. The next run clears it,
  since the restore target is always the original minus the harness entry.

## D5's first branch lands: `stub_verified` — 2026-08-26

Full write-up of both red-first passes (the feature, then the reuse gap an adversarial
review found in it) belongs beside this entry's own literal failures, recorded here
since no separate doc was asked for this session.

- [x] **`stub_verified` works, mechanically, against the generated harnesses.** Confirmed
      by direct reproduction against real `cargo kani` runs (not just unit tests): a
      caller stubbing a callee proved clean this run verifies in a fraction of a second
      via `#[kani::stub_verified]` plus a never-run "existence" harness satisfying
      Kani's purely-syntactic existence check (`tests/spike/FINDINGS.md` item 4 --
      confirmed again here, not just cited). §5.5's opening claim ("verification runs
      callees-before-callers") is real now: a topological order over the call graph,
      ties broken by node id, cycles falling back to the second branch.
- [x] **Ordering** (`callgraph`/`verify.rs`): claimed functions with a `bounded` check
      are ordered callees-before-callers; a cycle (mutual recursion, direct or
      transitive) cannot be ordered and every claim in it falls back to D5's second
      branch, `conditional`, exactly as before this feature -- not an error, not a hang
      (`stubverifiedcycle` fixture).
- [x] **The bound composes to the weaker of the two, never the caller's own declared
      one** (`stubverifiedminbound` fixture) -- the anti-overclaim test, and the one that
      matters most.
- [x] **A real Kani limitation found and worked around, not papered over**: plain
      `#[kani::stub]` cannot target a function that itself carries a contract (issue
      #4591, "Failed to find contract closure" -- a compile error killing the whole
      crate, reproduced here against this feature after `tests/spike/kani-pin` found it
      for a different case). Both of D5's branches, when reached through a same-crate
      contracted callee, therefore use `#[kani::stub_verified]` mechanically; what marks
      one `conditional` and not the other is entirely Ply's own bookkeeping (did the
      ordering above establish the callee clean this run), never anything Kani checks.
- [x] **A second defect, found by adversarial review of this feature and not by any
      test already in the suite**: the composed bound depends on a callee's *earned*
      verdict, and nothing hashed it. Editing only a callee's declared `checks:` (its
      bound going from `bounded(5)` down to `bounded(2)`, no source touched anywhere)
      correctly re-earned the callee's own record while the caller's record -- and its
      now-stale deeper bound -- went untouched. Closed by adding `verified_bounds` to
      `FingerprintInputs` (`record.rs`, new `INPUT_GROUPS` entry "the callees it stands
      on") and by *deferring* a bounded-eligible claim's reuse lookup until its
      fingerprint is finalised in dependency order, rather than deciding it from the
      Pass-1 fingerprint before that composition is known. Pinned permanently:
      `stubverifiedstalebound` fixture, red-first against the isolated defect, green
      with the fix restored.
- [x] **The same restructuring incidentally fixed a second, independent bug the review
      also caught**: an earlier version of the ordered pass decided "reused" before
      ordering and then unconditionally re-ran every bounded-eligible claim's engine
      regardless, wasting the exact cost reuse exists to avoid and writing a proof
      module for a claim the envelope reported `reused: true` -- caught by
      `resultreuse_fixture` going from 5/7 to 7/7 once the ordered pass was made to
      consult the reuse decision instead of ignoring it.
- [x] The missing "§5.5's limits" subsection §5.5 already pointed at ("see this
      section's limits below") now exists, gathering: cross-crate `stub_verified` (out
      of scope for v1, unchanged), a call outside the workspace, a call Ply's reader
      cannot see, branch one requiring a callee *clean* (never merely `bounded`-shaped
      -- a conservative, stated restriction on composing across more than one hop of
      assumption), the cycle fallback being decided per claim rather than per edge, and
      the whole mechanism's soundness resting entirely on Ply's own scheduler, never on
      Kani (`tests/spike/FINDINGS.md` item 4, restated where a reader of this rule would
      need it).
- [x] **A third defect, found independently by re-verifying this commit against a
      fresh fixture rather than trusting the 313 green tests that had just landed**:
      standing on a proved callee made a caller *permanently* unreusable, not merely
      briefly stale. The `verified_bounds` fix above closed the stale-number gap, but
      `record.rs`'s own "is this verdict one the declared checks could earn" integrity
      check (`W0516`) predates D5's first branch and still assumed a `bounded(k)` check
      could only ever produce `bounded(k)` verbatim -- so a claim declaring `bounded(5)`
      that genuinely composed down to `bounded(2)` looked identical to a hand-edited
      record on every run after the one that earned it, and was silently re-verified
      from scratch, forever, paying full engine cost each time. Confirmed by direct
      instrumentation before touching anything (the lookup and stored fingerprints for
      the caller were byte-identical, including the new "callees it stands on" group --
      ruling out the first, more obvious suspicion before fixing anything): the actual
      divergence was `W0516` itself, refusing an exact-fingerprint match because the
      composed verdict's number differed from the check's own. Fixed by making
      `bounded(k)` earn any `bounded(j)` with `j <= k`, never only `k` itself, and never
      a `j` deeper than declared -- the one place a stored verdict is allowed to differ
      from its own check's number, stated as exactly that in `verdict_is_earnable`.
      Pinned permanently: `stubverifiedwarmreuse` test (reusing the plain `stubverified`
      fixture, two runs, nothing edited between them) -- red-first against the isolated
      defect, green with the fix restored, run three consecutive warm runs by hand
      first to confirm the fix holds indefinitely and not just once.
- [x] Fixtures: `stubverified`, `stubverifiedminbound`, `stubverifiedcycle`,
      `stubverifiedfuzzedcallee`, `stubverifiedstalebound` (`tests/fixtures/`), each
      with its own e2e test under `tests/e2e/tests/` (`stubverifiedwarmreuse`'s test
      reuses the `stubverified` fixture rather than needing its own). `cargo test
      --workspace`: 315 passed, 0 failed, fmt and clippy clean.
- [ ] KNOWN GAP, deliberate: a claim declaring **both** `bounded` and `fuzz`/`test` in
      the same `checks:` list is bounded-eligible (needs the ordered pass) but its
      fuzz/test portion needs the harness crate built in the *unchanged*, earlier pass
      that the ordered pass's own reuse decision now runs after. No current fixture
      declares such a mixed list, so this has not bitten anything real, but it is not
      solved either -- recorded here rather than discovered later.
- [ ] KNOWN GAP, stated in §5.5's new limits paragraph: branch one requires a callee's
      own verdict to be clean (never `conditional`) before this claim can stand on it.
      Composing branch one across more than one hop of assumption -- does a claim
      resting on a clean callee that itself rested on a clean callee inherit anything
      the second hop assumed, transitively -- is a real question this design declines
      to answer rather than guesses at.

## Phase 1a — landed 2026-08-25

Full write-up with verbatim red-first failures: `docs/phase-1a.md`.

- [x] **The `ply.yaml` model lives once, in the product** (`ceb52aa`). `tools/model` and
      `tools/check`'s library became `ply_core::{model,check}`; the hand-rolled subset in
      `config.rs` is deleted, closing that file's own `TODO(M1)` ("promote one, delete the
      other"). `tools/render` and `tools/check`'s binary consume them by path dependency.
      Behaviour-preserving: 169 passed / 0 failed on the full suite, committed SVGs
      byte-identical, fmt and clippy clean in both workspaces.
- [x] **`schema/ply.schema.json` exists and is normative** (`c8528ce`) — §5/D3 have called
      it that since the spec was written while the file did not exist. Load-bearing, not
      decorative: the `E0204` key vocabulary and required-field list are read out of it at
      runtime and six Rust constants that duplicated them are gone. It now rejects things
      the product silently accepted: non-snake_case names (§5.1a rule 2 was enforced by
      nothing), unknown capabilities and bans, `unresolved` id 0 — and, found by the
      schema-vs-parser invariant test rather than by design, `fuzz(0256)`/`fuzz(+5)`, which
      the parser had inherited from `u32::from_str`. All 49 existing documents still pass.
- [x] **`cargo ply check`** (`5212cfa`) — schema + anchor tiers, `--json`, exit 0/1/2,
      **0.074s with no engine installed**. It reports what it did NOT check
      (`coverage.not_checked`) and says plainly that every node reading `unclaimed` means
      this command gathered no evidence, not that the code is unverified.

- [x] `check`'s architecture tier — crate level BUILT (`6fac707`), and it reads the real
      dependency graph rather than guessing. Carries a known defect found by review: it is
      blind to binary-only crates, so it reports a clean pass on this repo's own violation.
      FIXED (`a4c8675`), verified against this repo in both directions, and the repo now
      declares and checks its own architecture (`ply.yaml`, committed). A second review
      then found a blocker that outranks it: a run in which the architecture check could
      not happen at all — a broken manifest, no cargo, or a dependency cycle — prints "No
      problems found" and exits 0, so CI goes green on a run that checked nothing. That is
      the eighth instance of absence-of-evidence reading as success. Reproduced
      independently; fix dispatched. The item level is CANCELLED as specified — see the resolvability
      measurement (`fed5bf3`): one call site in five is resolvable from source, so that tier
      would report on a minority of the program and its silence would read as approval.
- [ ] JSON-pointer → (line, col) index for `E0201`/`E0204` (§5). The pointer ships; the line
      does not, and §5 now says a guessed line is worse than none.
- [ ] Multi-file `ply.yaml` discovery and merge (§5) — and with it, `E0202` across files,
      currently unreachable.
- [ ] Wire `--fail-on` / `--only-changed` to `check` once a tier exists they can mean
      something for.
- [ ] `check` should accept a loose `*.ply.yaml` path, so `tools/check`'s binary can retire.

## Result reuse, and the gap a review found in it — landed 2026-08-25

Full write-up with the literal red-first failures and the timings: `docs/reuse-hash-gap-closed.md`.
The review that forced it: `docs/review-result-reuse.md`.

- [x] **Ply remembers a checked result and skips re-checking while nothing it depended on
      moved** (`107a491`, superseded below). Cold 11.8s / warm 0.028s on the small fixture;
      97.3s / 0.067s on the older one.
- [x] **The review found the feature's load-bearing claim false, and it was fixed before
      merge** (`c650e55`, write-up `eca129f`). The hash covered the checked function's own
      lines and the promises written for old code it calls — not the ordinary helpers the
      check actually runs, not the bodies a proof walks into, not the worked examples, not
      the resolved dependency versions. Break a helper so the checked function genuinely
      violates its own guarantee, and the tool answered "carried forward, still fine" in
      0.03s while printing a line claiming the code hashed the same. That is this
      project's own worst failure mode — a green result over code nobody checked — and it
      is the seventh instance of it found and closed.

      The hash now covers every first-party body a check can reach: through calls, through
      a function named as a value, and through the claim's own contract expression. Where
      the walk cannot be trusted — a method call, a hand-written operator, a macro, an
      unrecognised attribute — it is abandoned and the whole crate is hashed instead:
      coarser, never wrong, and which mode ran is itself hashed. Allowlist on purpose, so
      an unanticipated construct costs engine time rather than a false pass.

      Verified independently of the agent that built it, on a fixture written for the
      purpose: helper broken → the claim re-runs and reports the violation with a
      counterexample, while an unrelated claim in the same crate keeps its result; only the
      unrelated claim edited → the reaching claim still reuses; nothing touched → both
      reuse in 0.039s; and the method-behind-a-type case, which no syntactic walk could
      follow, caught by the coarse mode with the unrelated claim honestly re-run too.

- [x] **Two smaller review findings closed in the same commit.** A stored verdict none of
      its own checks could ever have produced is now refused, said out loud, and re-run,
      instead of being believed forever. And a run that cannot use a stored result now
      names the input that moved — distinguishing "the function's own source changed" from
      "the code it runs changed" — rather than silently re-paying full cost.
- [x] **Every place claiming the hash covered "everything" now states what it covers and
      what it does not** — the spec, the schema page, the module comment, the fixture
      comment, the exact-string test, and the line printed on every reused run. §5.2a
      carries a "what it does not cover, stated rather than implied" paragraph.

- [ ] KNOWN GAP, recorded not hidden: build environment that never appears in a file
      (`RUSTFLAGS`, `[profile]` settings such as `overflow-checks`, a `#[path]` attribute)
      is not hashed; nor is what an outside proc macro expands to beyond its crate
      identity. Reuse across machines needs a committed lockfile — without one Ply records
      that it does not know the versions rather than guessing.
- [ ] KNOWN GAP: a hand-edited record is caught only where the stored verdict is one the
      stored checks could never earn. A hash cannot defend the file against a text editor.
- [ ] The fuzz engine's recorded version is the requirement written in the manifest, not
      the version actually resolved.
- [x] **The coarse mode now explains itself** (`bf6048f`). It already worked out why
      it abandoned the call walk and kept it to itself, so a person who edited one function
      and watched an unrelated claim re-run was told only "the code it runs changed" — true,
      and useless. The run now names the construct that cost the walk and says the crate is
      the unit: *"For `x` and `y`, \"the code it runs\" means every line of the crate, not
      only the functions they call, because src/lib.rs declares an `impl` block for
      `Scaler`, and Ply cannot tell by reading the source which of its bodies a method call
      or an operator would run."* Said once per crate however many claims it displaced —
      the first build repeated it per claim, which reading the real output caught — and
      never printed for a bounded walk, where it would be false. Both of those are pinned
      by tests, the second one negatively. New fixture `reusewiden` carries the shape.
- [ ] Open question, deliberately left: whether every Ply release should invalidate every
      record. Fable's answer was yes — a hand-maintained "only when it matters" flag
      recreates the judgment call the design exists to eliminate. Not yet revisited.

## Reach — types, methods, and four false cleans found closing them (2026-08-27)

Driven by a yardstick rather than by opinion: `tests/fixtures/ratelimiter/` is a working
rate limiter designed by someone told NOT to think about checkability and not told this
project existed. Every number below is measured against it.

- [x] **`usize`, `isize`, the `NonZero` family and `Duration`** (`4ce1c1c`). Type coverage
      on the yardstick went from 3 of 70 uses to 25. My own estimate had been 82 percent;
      the real figure is 36, because a duration nested inside an `Option` is not a bare
      one and covering those would reintroduce the false-counterexample risk the work
      exists to prevent. Second over-estimate of the day, same mistake both times:
      counting something adjacent to the question and reporting it as the answer.
- [x] **Methods resolve, and receiverless functions check** (`c1ea364`). Ply could not find
      methods at all — its own schema documented `Type::method` and the anchor did not
      resolve — and **no config in this repository that had ever been run through a real
      check claimed a method**, which is why nobody noticed. Two defects were found by hand
      before it landed: the generated harness imported a method as if it were an importable
      item, and separately **any** zero-parameter function failed the same way, latent since
      the sampling tier was built.
- [x] **The ninth and tenth false cleans** (`62f4c74`, review `7f6bfe8`). Ply decided which
      function a promise was ABOUT separately from which function the test would CALL, so
      two same-named types in different modules made them disagree: a promise saying the
      answer is 999, on a body returning 5, reported a clean pass on a different function
      entirely. Fixed structurally — the called path is now derived from the resolution, so
      the two cannot drift. Building the multi-module fixture that proved it then exposed
      the tenth and worse: **the filter selecting which generated test to run matched
      nothing for any method, so zero tests ran and the result was reported as held.** Every
      method check had been passing without executing anything.
- [x] **Floats, and one type list per engine** (`2443b85`). A type the sampler can build is
      now checkable even where the prover cannot reason about it; a proof requested on such
      a type is refused by name. Floats reach the property the yardstick's author named as
      least trusted. Three honesty features arrived unasked: a run whose precondition
      rejected 92 of 156 draws says its 64 accepted cases are weaker evidence than the
      number suggests; NaN and infinity are excluded by default and the run says it
      therefore says nothing about them; an unrenderable failing input says so and adds
      that Ply never invents one.

- [ ] **Strings and collections on the sampling tier.** Did not land; the mechanism now
      exists. **Demoted 2026-08-30, and the old justification here was withdrawn** -- it
      read "this is where parsing and validation bugs live, so it is the highest-value
      remaining type work", which is a guess dressed as a ranking. Build it when a
      property somebody wrote down is blocked on it and nothing else, not as a programme.
      See "What widening the types is actually worth" below.
- [ ] KNOWN GAP, **narrowed 2026-09-03**: `NonZero` and `Duration` nest fine on the
      sampling tier — the 2026-09-02 composition work made `is_fuzz_nestable` fall through
      to true for both. What is still refused is the *proving* tier: neither is part of
      `is_leaf`, deliberately, because nesting one inside `Option`/`Result`/`[T; N]` there
      would hand construction back to a generic `kani::any::<T>()` this module does not
      control, and the whole point of the `NonZero` wrapper is that its constraint reaches
      the solver rather than being assumed by convention.
- [ ] **The suite re-proves the same fixture up to eight times per run**, which is most of
      the wall clock on every verification cycle. Making a proof earned once in a run serve
      the other tests that need it would turn forty-minute waits into minutes. Deferred all
      day behind more interesting work; it is now the biggest drag on the loop.

**The method that actually worked, recorded because it is the transferable part:** verify
with a promise that is FALSE. A passing check proves nothing — my own verification of the
methods work used a true promise, saw a pass, and called it verified while nothing was
running at all. Ten false cleans on this branch, every one found by real code or
adversarial review, none by the suite.

## Agreed with the maintainer, not yet started

- [x] **The promise ramp widened, and a contrast invariant added** (2026-08-29). The
      maintainer said the render had gone monotone. Measured, and he was right for a
      reason worth recording: adjacent rungs of the ramp sat 1.13-1.33 contrast apart,
      close to indistinguishable, so the evidence ladder -- this tool's main lever -- was
      encoded in the least readable way available. Now 1.27-1.54 per step, full range
      2.18 -> 3.15. Writing the check also found a real legibility defect that predated
      it: the anchor and ownership lines sat at 2.2 against the strongest fill, i.e.
      unreadable on exactly the boxes a reader most wants to read. Both fixed, and held by
      a contrast floor over every ink/fill pair.


- [x] **A dark palette, following the reader's system setting** (2026-08-29). The render
      paints its own near-white background, so a dark-mode reader got a bright panel. One
      alternative palette, explicitly **not** a theming hook: the colour meanings are
      enforced by CI, and a user-redefinable palette would make every one of those
      guarantees unenforceable. The colour-blindness floor now runs against both palettes
      and immediately earned its keep -- it rejected the first dark red proposed, which sat
      0.200 from ordinary structure, inside the measured confusable band. Replaced with one
      that clears the floor against both structure and the attention amber.

- [x] **Diagram-layout and cartography evidence applied** (2026-08-29). The research doc
      cited perception and notation theory but no experimental work on diagram *layout*.
      Two fields have it. Landed: (a) **a shallow-crossing invariant** — crossings are the
      one layout property with a large replicated effect on reading speed, and the useful
      form is the refinement (a near-90° crossing is ignored by the eye; a shallow one
      costs accuracy), so shallow ones are now forbidden in CI while ordinary ones are
      not; (b) **"route edges orthogonally" struck from the research doc** as folklore --
      controlled studies found no measurable benefit, and it manufactures exactly the
      shallow near-parallel runs the eye-tracking work condemns; (c) **position and size
      named in the spec's channel table**, including the ones that mean nothing. Evidence
      for (c): on the Underground map the drawn geometry moved route choice about twice as
      much as travellers' own journey times, so readers trust geometry whether or not it
      was meant to carry meaning.

- [x] **Four forbidden-call lines drawn along each other, fixed** (2026-09-03,
      `7f5ae36`). Found by writing the crossing invariant: three shared one
      vertical corridor in vetting 003 and two shared a horizontal run, rendering as a
      single line and hiding a declared rule from the reader. Worse than a crossing: a
      crossing slows a reader, an overlap hides a rule. Fixed by giving every routed
      forbidden-call line and every routed external/`entry:` line a rank (the same
      monotone-by-target-y order that already keeps the wildcard-node fan crossing-free)
      and nesting each further-ranked route's corridor and rail one step further from the
      obstruction it dodges than the last, so two routes that would otherwise compute an
      identical detour land on visibly separate lines. The ratchet (`KNOWN_OVERLAPPING_
      LINES` in `tools/render/tests/render.rs`) is now 0, and the canvas grows to hold a
      nested rail rather than letting one run past the edge. Verified by eye, not just by
      test: rasterized `vetting/003-trading-system.svg` before and after — three lines
      that used to read as one now run on their own separate tracks.
      **Correction worth recording:** an earlier measurement in-session reported "zero
      crossings in every diagram". That was true of X-shaped crossings and completely
      missed these overlaps, because the detector used treated collinear segments as
      non-crossing. The repo's own `segments_cross` does not, which is how they surfaced.

- [ ] **When results can reach the drawing: the chips are the promise/earned encoding,
      not a meter.** Supersedes the research doc's two-part meter *and* the split-fill idea
      proposed in-session. Both were wrong for the same reason, from opposite directions:
      a small meter is small first and an encoding second (size is the most detectable
      visual variable, so a tiny track is the least noticeable thing available); and a
      split box fill creates a second enclosed region, and enclosure is the strongest
      grouping cue there is -- it would perceptually sort the chips inside into "earned"
      and "not", which is a lie unless made true. The evidence favours discrete countable
      units over continuous fills for part-whole reading, with the advantage largest for
      untrained readers -- which is the newbie bar. So: colour earned *chips*, sort them
      together so the grouping the eye infers is true, and use a split fill only on
      collapsed boxes where no chips can contradict it. Invariant when it lands: a
      collapsed box's split must equal the earned-over-promised count folded beneath it,
      and no chip may be coloured earned without a result behind it.

- [ ] **If a diff view is ever wanted: side-by-side, with changes drawn as marks.** A
      reader comparing two renders by eye *will* miss changes -- change blindness is among
      the most robust findings in vision science, and two separately-opened files are its
      worst case. Small multiples measured faster than animation on every comprehension
      task; animation won only on "what was added just now". Never ship "spot the
      difference".

- [ ] **Decided against, with reasons, so they are not re-proposed:** crossing-*count*
      minimisation in the layout (effect sizes come from dense abstract graphs; Ply's are
      single-digit box counts, and reordering rows trades away the version-to-version
      stability a diff view would want); symmetry as a layout goal (a weak effect, weaker
      still in the study closest to Ply); a fold-prominence test (marks already draw at
      fixed size at every depth -- verified -- so the test would compare two constants);
      and Lynch's *Image of the City* as evidence (thirty sketch-map interviews per city,
      no tasks, no measurements -- useful vocabulary, not a source you can test against).

- [x] **Four more of the visual-language items applied** (2026-08-29), after the maintainer
      correctly pointed out that the first three changed almost nothing visible: they were
      all *subtractive* (green removed, red removed, two labels moved), and every additive
      idea was still outstanding. (a) **Absence is now hatched** rather than left blank --
      the single biggest visible change, and the document's sharpest perceptual point.
      (b) **A verdict strip** opens every render with what is declared and how much of it
      promises nothing. (c) **Checks read as words when zoomed in** -- `B3 F4096 M` becomes
      "proves for all inputs, loops up to 3 / tries 4096 random inputs / plants bugs; the
      checks must catch them", following the same zoom rule the contract clauses use.
      (d) **A colour-blindness gate in CI.** Its first version was nearly vacuous -- the
      distance metric passed pure red against pure green, the textbook confusion -- so it
      was rewritten around the guarantee that actually holds: every meaning also carries a
      non-colour mark. The weak floor is kept, documented as weak.


- [ ] **Judgement call to revisit: at focus, a fn chip now shows both `B3 F4096 M` and the
      spelled-out lines.** The document wanted the letters confined to hover. Kept both so
      the chip looks the same at either zoom, with the words as the expansion -- but it is
      redundant, and if the band gets busier the letters should go.

- [ ] **Decided against: making the strict notch and pure seal more visible.** The document
      asks for one or the other (amplify or demote); demote is the honest answer. "Strict"
      means findings here are errors rather than warnings, which only matters when there IS
      a finding -- at which point red already shouts. Spending a glance-level channel on a
      modifier only legible alongside another signal is a poor trade. Hover-tier.

- [x] **The diagram no longer paints promises green** (2026-08-29, from
      `docs/visual-language-research.md` via Fable's review). Two of the review's three
      "land now" items done. (a) Capability tags were red — the same red as a real
      failure — though a declared capability is neither forbidden nor wrong; they are now
      neutral, and the deny lines visibly alarm for the first time. (b) The promise ramp
      moved off green onto ordered neutral greys, so a project where nothing has run no
      longer renders as a field of healthy green. That was §1's absence-of-evidence
      failure drawn in pixels, by the tool that exists to prevent it. Both are guarded by
      invariants over the emitted stylesheet, not spot-checks: red must belong to
      something forbidden or wrong, and un-run work must contain no green. The-Ply-Spec.md's
      channel-discipline rule is amended, retracting "pastel = promised, saturated = earned"
      with the argument for why it neither held nor could hold.

- [x] **Edge lines no longer strike through labels** (2026-08-29). The third and last of
      the review's "land now" items, and the longest-standing recorded render defect. The
      ratchet that pinned this debt at 13 collisions is now at **0**. Two causes, both
      real: the placement search checked candidate positions against the canvas edge and
      the boxes but never against the *lines*, and it ran before the forbidden-call routes
      existed, so a label could not avoid a line not yet routed. Placement now runs as a
      second pass once every line exists, and candidates vary where along the edge the
      label sits as well as how far out it is pushed -- without that second axis a steep
      edge's perpendicular is nearly horizontal, so every candidate slid the label *along*
      the horizontal line it was stuck under. Exactly 2 labels moved across the whole
      corpus; 001 and 002 are byte-identical.

- [ ] **Deferred with a bug to fix first: the two-part promise/earned meter.** Good idea,
      wrong arithmetic as written — it folds by *summing*, so a collapsed box with nine
      earned functions and one untouched reads 90% healthy, which is exactly what the
      kernel's first standing obligation forbids. Fold by weakest-descendant plus a count
      instead. Also needs earned-result data to reach pixels, a path that today exists only
      as tooltip text — that, not the drawing, is the real lift.

- [ ] **Cut from the proposal, recorded so it is not re-proposed:** rank-band layout
      (position already means containment; a second meaning breaks the one-meaning rule and
      would draw layers nobody declared), demoting deny bars to a lock glyph (a red barred
      line is the most instinctive form in the grammar, and it draws a *rule*, not an
      alarm), and a third amber corner flag (two markers for that already exist, one in the
      same corner as the strict notch). Also: the document claims the check badges carry no
      tooltip — false, verified in the renderer; the defensible version is that hover is
      not glanceable.

- [x] **A focused function now draws its promise, instead of hiding it in hover** (2026-08-29).
      Prompted by the maintainer's question of whether the visual language is sufficient --
      "at a glance it is not always obvious what a function does". It already was declared:
      `requires`/`ensures` are contract clauses, and unlike free pseudocode they cannot drift
      into fiction, because a check stands behind them. They were simply hover-only. Now
      `--focus` draws them as `needs`/`gives` lines under the fn name. Rejected in the same
      breath: adding a separate unchecked pseudocode block, which would put the only ink on
      the canvas with no evidence behind it -- the exact failure Ply exists to prevent.

- [ ] **KNOWN GAP -- only one function in the whole vetting corpus declares a contract.**
      Found while looking for an example to render: 001 and 002 declare none at all, 003
      declares exactly one (`check_order`). So the clause band is real but barely exercised
      by our own scenarios, and the vetting corpus is not currently evidence that contracts
      are pleasant to write at scale. Worth writing clauses into 001/002 as a grammar
      exercise in their own right -- that is what `vetting/` is for.

- [x] **Automatic bug-planting now runs against the kernel on every build** (2026-08-28).
      Fable's call, taken: the exhaustive tree check is the gate, but whether it can SEE
      is a measured property that can regress, and nothing was keeping the 2026-08-25
      repair true. First run: 35 planted bugs, 24 caught, **6 survived** -- none in the
      aggregation logic, all six in the status-set helper, and every one genuine rather
      than an artefact. Three were an unused `union` the merge site was hand-rolling
      around (now called, so the duplication is gone and those die by construction); one
      an unpinned `FromIterator`; one `is_empty`, whose only behaviour-changing consumer
      is the renderer in another crate; and one the `Debug` impl, which could be blanked
      leaving every test green while every future counterexample printed unreadable.
      After the fixes: **30 caught, 5 unviable, 0 survivors.** The CI job requires zero
      and deliberately carries no excused-failures list.

- [ ] **Mutation coverage stops at the kernel.** Fable's second recommendation, not yet
      done: an occasional (not gating) run over the model/check parsing and validation
      code -- the surface neither the exhaustive check nor the unbounded proof reaches,
      and where the third hand-planted fault of 2026-08-25 lived ("check 0 examples"
      accepted). Expect a noisy first survivor list; triage into this file rather than
      gating on it, because a gate with a hastily-blessed baseline is theatre.

- [ ] **The "your filter hid nothing" notice is written out twice.** `cargo ply render`
      and the standalone renderer each build the same read-parse-draw-and-warn sequence,
      so the notice a first-time user relies on lives in two places and can drift out of
      one of them. Closed PR #8 already factors it into one shared helper: fetch it with
      `git fetch origin refs/pull/8/head` (commit `968a274`; that reference outlives the
      branch being deleted) and lift the helper onto current `main` rather than rewriting
      it. Recorded 2026-08-28 when #8 was closed as superseded, so the one good idea in it
      is not lost with the branch.

- [x] **DECIDED 2026-08-26: the architecture verdict is code, never a model.** The
      question was raised and settled: since the architecture tier is *approximate* by
      nature — warnings by default, an escape hatch that takes a written reason — would a
      model be the better enforcer? No. Approximate and nondeterministic are different
      properties: a source-reading checker is wrong in the same places every run and can
      enumerate what it could not see, while a model is wrong in different places each run
      and cannot. Full argument, including two repairs to the reasoning that reached the
      right answer for partly wrong reasons: `docs/review-architecture-enforcement.md`
      (`b32deba`).

      What that review changed about the plan, and what to actually do when this starts:

      - **Crate tier first.** Cargo already knows the dependency graph exactly. Cheap,
        sound, errors rather than warnings. Then turn it on this repository immediately —
        self-hosting is the point, not a follow-up.
      - **Measure before committing to depth.** A rough count says ~60% of this
        workspace's own call sites are method calls, the shape a source reader handles
        worst. Turn that estimate into a real number first; it decides how much
        function-level checking is worth building at all.
      - **The hand-run spike is rejected as designed** — circular (a model drafts the
        description it then checks itself against), and unscoreable. Replaced with seeded
        violations, including one behind dynamic dispatch, measured against ground truth.
      - **The unexhausted middle rung**: a real type-resolving backend sits between
        reading source and asking a model, and the extractor was made swappable for
        exactly that. "Source reader or model" was a false choice.
      - **A model's place is upstream only** — drafting the architecture description,
        triaging call sites the reader could not place, proposing where an exception is
        warranted. Propose, never decide. Anything it proposes gets confirmed
        mechanically (the span exists, the item exists, the name matches) before it is
        ever reported.
      - **KNOWN RISK, recorded because it fails quietly**: "propose, never decide"
        collapses the moment proposals are rubber-stamped. A model-drafted architecture
        description that someone skims and approves is architecture-as-vibes laundered
        through a deterministic checker. The rule that goldens are reviewed rather than
        blind-accepted has to reach it.
      - **Not an exception to the thesis.** A model rung has no oracle, and `strict: true`
        means this tier is designed to gate merges — so "it's only warnings" is not a
        defence available here.


- [ ] **D7 for stubbed crossings — `W0541 stub_substituted`** (planned
      2026-08-25, `docs/plans/d7-stub-failures.md`). The Kani-pin spike proved a
      stub-caused failure has no faithful plain-Rust reproduction, on any engine version:
      the rendered test calls the real callee, which never returns the stub's invented
      value, so it is emitted **green** (that test is in `tests/fixtures/boundarycontract`
      right now and passes). D7's unqualified red-test promise is corrected in the spec
      today. Build: a third `W0541` reason, the fabricated value + admitting clause in the
      diagnostic, a `fixes` entry proposing the tightening, and *stop emitting the passing
      `ply_cex_*` test* — a green reproduction that reproduces nothing is worse than none.
      Refused by name: rendering the test against a rewritten body, which would go red for
      a program the user does not run.

- [x] **Trusted boundary declared in `ply.yaml`** — CLOSED 2026-08-25, will not be built.
      The gate below answered no on evidence, and the maintainer closed the idea in
      conversation the same day: a per-function promise is the whole of what was wanted,
      and there is to be no region-wide or module-wide variant. Kept here for the
      evidence, not as pending work. Original framing follows. — the
      coarse-grained sibling of §5.5's per-callee rule: declare a region taken as given,
      rather than writing a contract per legacy function a new feature happens to touch.
      Fills a real hole in §7.2's taxonomy — *our code, checkable in principle,
      deliberately not checked* — which is distinct from an `external` (someone else's
      system, permanently). Three conditions agreed up front, all learned the hard way
      here: it must never read as evidence (crossing one marks the caller `conditional`,
      never clean, or trusting the whole tree goes green); it must be counted on the
      audit surface so the trusted region is under pressure to shrink; and it must draw,
      per §7.1's gate. Carry `trusted`'s own lesson: it shipped with no staleness and an
      attestation would have outlived the code it vouched for. Proposal first, gate,
      then adopt — the sequence that worked for external elements.

      **GATE RUN 2026-08-25 — the answer is no; take the fallback.**
      `docs/plans/trusted-boundary.md` bound itself to one empirical claim: that real
      callers are defensive enough to verify with the callee replaced by an
      unconstrained symbolic return, and that "if most crossings fail under havoc, this
      is a hint generator wearing a grammar construct." **Most crossings fail: 2 of 8
      pass (25%), and both passes are 004's own functions.** Zero of six callers written
      without the experiment in mind passed; six of six failed, five with a
      counterexample and one by timing out at the 300s floor with no witness at all.
      The falsifiable prediction on record **held** — `tier_fee_cents` passes under
      havoc (133.74s, inside the floor) because of its own `.min(10_000)`, and so does
      `approve_withdrawal` above it (212.63s) — but it held only on the two functions
      the plan already knew about. Cost is not the objection: havoc costs the same as a
      declared contract stub (133.74s vs 148.62s on the same function) and lands inside
      §6's floor. Three findings the plan has no row for: a havoc'd loop bound turns a
      22s proof into a 300s timeout with no diagnostic; the breaking value names the
      callee and the direction but never the threshold (`2_813_465` where the contract
      needed is `<= 100_000`); and Ply would print the *least* useful witness where
      several exist. **Recommendation: do not build `given:` as a grammar construct;
      adopt the plan's own open question 6 fallback** — let a clause-free boundary entry
      mean havoc, per callee, no new grammar (the codegen already emits it; only
      `verify.rs`'s `if claim.requires.is_empty() && claim.ensures.is_empty() { continue; }`
      stands in the way). Evidence, fixtures and a reproducing `run.sh`:
      `tests/spike/havoc/FINDINGS.md`. Commit `ff15b23`.

## Kani pin — spiked 2026-08-25; recommendation: stay put, two gaps left open

- [x] **Bump the Kani pin — a D13-shaped spike, not a fork.** Ran, against Kani `main`
      built from source (`245709373965fcb78209135822cbafb59c08d036`, 2026-08-25, CBMC
      6.10.0, `nightly-2026-04-01`) beside the untouched 0.67.0. **Recommendation: do
      not move the pin, and do not fork.** Four measured reasons.
      (1) There is nothing to bump *to* — crates.io's newest `kani-verifier` is still
      0.67.0, so a bump means pinning a commit of an unreleased branch that still
      reports itself as `0.67.0`, which would stamp two different engines with one D14
      fingerprint.
      (2) Blocker 2 is **not fixed**: `#[kani::stub]` over a contracted target still
      fails with `Failed to find contract closure __kani_recursion_check_<fn>` on
      today's `main` (Kani #4591, open).
      (3) Blocker 1 as recorded here was **never true** — at 0.67.0 a stubbed harness
      that fails *does* print a concrete-playback witness, and Ply's own
      `extract_witness_bytes` would accept it. The real limit, identical on both
      toolchains and stated in Kani's own generated doc comment, is that the playback
      test **does not apply the stub**: replaying a stub-caused failure panics on
      leftover concrete values instead of reproducing anything. That is worse than the
      documented blocker because a naive "the test is red" check passes.
      (4) Ply's real §5.5 shape already works at the pin — `boundarycontract`'s stubbed
      proof verifies (94.6s, 85 checks) and a violation in the same configuration yields
      a witness — at ~12-14% *lower* cost than the candidate (107.7s, 110 checks).
      Evidence, fixtures and a reproducing `run.sh`: `tests/spike/kani-pin/FINDINGS.md`.
      Commit `82555a9`.
- [ ] **KNOWN GAP, raised by the spike, no Kani version fixes it.** §5.5 can produce a
      violation that no test of the real code can reproduce: the counterexample's third
      value is the *stub's* invented return, and the real callee never returns it. Written
      out D7-style at the two real inputs, the test is green
      (`tests/spike/kani-pin/boundary/src/lib.rs::witness_replay`, observed passing).
      §8 forbids a witness-free `violation`; here the witness exists but is not
      replayable. A spec conversation about §5.5/§8, not an engine upgrade.
- [ ] **`boundarycontract`'s clean proof does not exercise its own assumption.** Delete
      the generated stub's `kani::assume` so the callee is unconstrained and
      `ply_proof_tiered_fee` still verifies (86.4s at the pin, 107.1s on the candidate):
      `legacy_rate(tier).min(10_000)` clamps whatever comes back, so the proof holds for
      *any* callee. The `conditional` verdict is still honest, but the fixture does not
      show the assumption doing work. Consider adding a harness that does (the spike's
      `tiered_fee_halfclaim` is one).

## Post-004 review closure — landed 2026-08-25

Disposition of every finding in `docs/review-post-004-fixes.md`, with the red-first
failure message and literal before/after for each: `docs/post-004-review-closure.md`.
Six commits, one per finding.

- [x] **Review D1 (MAJOR) — an ordinary `use` import bypassed the boundary rule.** The
      resolver never read `use` declarations, so a bare-name call classified `Unresolved`
      and `Unresolved` meant descend: `bounded(2)`, zero diagnostics, **exit 0** in
      40.562s over an unclaimed body, against `unclaimed`/`W0512`/**exit 1** in 0.007s for
      the identical claim spelled with a qualified path. Resolution now follows `use`
      (renames, groups, globs), inline and file modules, re-exports, and a path
      dependency's `src/lib.rs`; first-party source Ply cannot read is refused (`W0513`,
      new) rather than descended into. New fixture `useimport` + e2e, nine `ply-core` unit
      tests. §5.5's "just never a first-party one" retracted here, in
      `docs/post-004-fixes.md` and in this file. Commit `e83ccb9`.
- [x] **Review D2 — the fail-by-default rule missed absences encoded as statuses.**
      `mutate` with cargo-mutants masked: `fuzzed(64)` + status `inconclusive` + exit 0,
      against §6's own exit-3 row. The rule is now over **names, not slots** — one
      absence vocabulary read against a node's verdict *and* its statuses — and `mutate`'s
      non-results name which absence they are, so exit 3/2/1 follow the fact. Masked-engine
      e2e (§9's matrix, first entry built) + unit tests, including that `conditional`,
      `owed-evidence`, `weak-spec` and `stale` still exit 0. §3's "It never fails the run"
      reconciled with §6. Commit `a92e61f`.
- [x] **Review D4 — `W0541`'s wording was false for the shapes it now fires on.** It named
      `BTreeSet`/`Vec` to users whose parameter is a `[u32; 4]`. It now names the
      parameters and types that blocked the rendering; `RustType::display_name` fixes the
      same omission in `X0901` ("`xs: `", type missing). Commit `681fc75`.
- [x] **Review D5 — `evidence` described runs that never happened.** `cases: n` was
      attached whenever `fuzz(n)` was *declared*: a harness that never compiled reported
      `cases: 64`. Now built where the run happens, with `cases` only when the count is
      real. Commit `283cd83`.
- [x] **Review D6 — `owed-evidence` was emitted but defined nowhere.** Defined in §0's
      glossary and D6's status list as the debt half of `conditional`; §5.5 calls it a
      status; the verdict kernel gains the variant and a round-trip test over the whole
      vocabulary. Enumeration unchanged and green. Commit `9c730dd`.
- [x] **Review G1 — the `conditional` path was dead at the tool's own defaults.** 004's
      `tier_fee_cents` is scalar-signature, so plain `cargo ply verify` gave it 60s and
      reported `timeout` in 1m6.776s, saying nothing about the assumption. A **stubbed**
      `bounded` harness now gets a 300s floor — derived split (a stub is knowable before
      the run and always trades concrete values for a symbolic one), fitted constant
      (201.77s measured, plus the ~107s CBMC variance the M3 findings recorded), with
      9.72s and ~110s as the second and third measurements showing the cost is the body's
      as much as the stub's. `K0601` explains the premium when there was one. The
      `boundarycontract` fixture now carries 004's body shape and its e2e passes **no**
      `--engine-timeout`: the only test that observes §6's default end to end.
      Commit `182e9e1`.
- [x] **Review O2, O3, O5 — overstatements corrected in place** ("Nothing was lost" on
      seeding; §5.5's present-tense `audit`/`worklist`; "s1/s2 behaviour unchanged", which
      item 2 falsified by flipping their exit codes).
- [x] **Review O4 — DISPUTED, with evidence.** The tree holds **21** e2e `#[test]`
      functions at `3adca0e`, counted file by file, so `70 + 11 + 21 = 102` was right and
      the review's 22 was wrong. Nothing changed.
- [ ] **KNOWN GAP — the boundary rule inspects the claimed function's own body only.**
      Until D5's first branch lands, a contracted callee `g` is inlined rather than
      stubbed, so an unclaimed callee one level below `g` still travels into the caller's
      proof unnamed. Same pattern as review D1, a different bypass; stated in §5.5's
      limits. Not started deliberately (out of that task's scope).
- [ ] **KNOWN GAP — calls Ply's reader cannot see are not call sites for the rule**:
      macro-generated calls, `#[path = "..."]` module attributes, function pointers and
      trait methods.
- [ ] **KNOWN GAP (review G2) — the assumed-contract enforcement loop, as ONE item**,
      because the three parts are one loop and their conjunction is the risk: (1) no
      vacuity check — a declared `ensures: ["|result| false"]` makes the stub's
      `kani::assume` unsatisfiable and the caller's proof vacuously green, and a
      `kani::cover!` after the stubbed call would catch it cheaply; (2) no staleness — D14
      fingerprints trusted claims, nothing fingerprints a declared boundary contract
      against the callee's body, so legacy code can change under a standing assumption
      (the hazard §5.4d closed for `trusted`, reopened one mechanism over); (3) no
      accumulating surface — `audit`/`worklist` are not built, so the debt lives only in
      per-run output that scrolls away, and the run is CI-green by default.

      **Narrowed 2026-09-03**: two of the three parts are now built and only part (2)
      survives. The vacuity check exists — `crates/ply-core/src/promise.rs` generates
      satisfiability and tautology harnesses, wired in from `verify.rs` — and both
      `audit` and `worklist` are real commands with their own modules. Nothing
      fingerprints a declared boundary contract against the callee's body, so legacy code
      can still change under a standing assumption; that is the whole of what is left, and
      the conjunction argument above no longer applies to it.
- [ ] **KNOWN GAP (review G3) — declared-contract keying assumes the anchor equals the
      Cargo.toml dependency key.** `ledger = { package = "real-name", path = ... }` with
      `anchor: real-name` would not match the path a caller writes, and the callee would
      classify `Unclaimed`. It fails **closed** (a loud `W0512` naming a callee whose
      contract the user just wrote), so this is a usability gap, not an honesty one. Fix:
      resolve the anchor through the same rename logic the resolver already has.
- [ ] **Recorded-entropy fuzz mode** (the review's complement to the seeding decision):
      vary the seed by default in some contexts and *always* record it, so cross-run
      detection accumulation comes back without reopening the re-roll-until-green channel
      that determinism closed.

## Post-004 fixes — landed 2026-08-25

Closes the five items `docs/review-post-004-strategy.md` sequenced after vetting 004.
Full write-up with literal before/after output, red-first failure messages and measured
costs: `docs/post-004-fixes.md`. Four commits, one per item plus item 1's spec-and-code.

- [x] **Finding 2 / D5's third branch — the boundary rule.** §5.5 rewritten (three-way
      split, three honesty conditions, and its own stated limits); §2's D5 row amended.
      Built: a `bounded` check whose fn calls a callee no contract describes refuses to
      descend and names it (`W0512`), 004's `run.sh s3` going from `timeout` after
      **11m23.094s** to a named refusal in **0m0.005s**. With a contract declared in
      `ply.yaml`, the callee is stubbed (`#[kani::stub]`, cross-crate, real) and the
      caller earns `bounded(2)` + statuses `["conditional", "owed-evidence"]` + `W0511`
      listing the assumption — 004's `run.sh s5`, **3m15.9s** wall at a 600s budget.
      Commit `2cf09c2`.
- [x] **Finding 7, `anchor:` half.** A component anchored at another crate is a boundary
      component: contracts read, `checks` not run here (`W0303`), no node. A fn entry
      declaring only `requires`/`ensures` is a boundary contract declaration, not a claim.
      Commit `2cf09c2`.
- [x] **Finding 1 — a run that checked nothing exits 0.** §1 gains the
      absence-of-evidence principle; §6's exit table gains the missing row and
      `--fail-on=warn|evidence|error` (default `evidence`, `error` the documented
      opt-out). Exit codes 2 and 3 are returned for the first time. Commit `d73558f`.
- [x] **Finding 4 — `fuzzed(n)` is not reproducible**, and the escalation the review
      added to it. Seed derived per fn, recorded in the §8 envelope as
      `evidence: { engine, seed, cases }`, `--seed <hex>` replays; proptest's own
      persisted-failure replay switched off. `run.sh s8`: six identical `fuzzed(256)`,
      where it used to split 3/3. And a panicking body now earns a `violation` carrying
      proptest's own shrunk input instead of `X0901` — the class of real bug that could
      not be reported at any seed. Commit `c8e231b`.
- [x] **Finding 7, `ensures:` half.** `config::validate_keys` enforces §5.1a rule 1 on the
      verify path (`E0204`, location, nearest key) against the **whole** §5 key
      vocabulary, so the keys `verify` ignores are still accepted. §5.1a amended to say
      the rule binds every reader of the file, and so does its converse. Commit `23e8f67`.
- [x] **Finding 5 — the implemented fragment is narrower than §5.4b.** `char`,
      `Option<T>`, `Result<T,E>`, `[T; N]` and top-level type aliases are in. Measured
      first (Kani `Verification Time`, trivial bodies): 0.028s `u32`, 0.064s `char`,
      0.036s `Option<u32>`, 0.040s `Result<u32,u8>`, 0.036s `[u32; 4]`, 0.041s
      `[u32; 16]`, 0.028s alias. No unwind annotation for an array — its bound is a
      compile-time constant. Commit `593cf9a`.
- [x] **D5's first branch IS implemented** (`5671ab5`, then `dc1e7ed`/`4ca1c9e` closing six
      defects an adversarial review found — one of them a false clean verdict). Superseded
      text follows, kept because its concrete example is still the right one. **KNOWN GAP
      (was) — D5's *first* branch is still not implemented.** A callee that passed
      its own Kani proof this run is inlined, not `stub_verified`, because callees-first
      scheduling (ADR-0003's "entire soundness guarantee", living unlinked in
      `tools/schedule`) is not promoted into the product. Concretely: 004's
      `total_debit_cents` still times out at 120s with `fee_cents` inlined. The review
      sequences this as the next tranche.
- [ ] **KNOWN GAP — §5.5's rule does not reach `std`/`core`/registry callees.** A call
      into a crate whose source Ply cannot read is left alone, so a `bounded` verdict can
      still include a body Ply never examined. Stated in §5.5, not left to be discovered.
      (The clause "just never a first-party one" was **retracted 2026-08-25**: an
      ordinary `use` import bypassed the rule entirely — see the closure of the review's
      D1 below.)
- [ ] **KNOWN GAP — a boundary assumption is reported as owed, and nothing exercises it.**
      §5.5 says an unexercised assumption is owed evidence and that `audit`/`worklist`
      list it. The `owed-evidence` status and `W0511` are built; `cargo ply audit` and
      `cargo ply worklist` are **not built**, and fuzz-checking a declared contract
      against the real legacy body is not built either.
- [ ] **KNOWN GAP — `ply.yaml` `requires`/`ensures` are still not ANDed into the fn's own
      check** (§5.4 says they are). They are read, and used for §5.5's boundary
      assumption; a warning said out loud which of the two a user was getting. Retired
      2026-09-03, when the contract began being merged into the fn's own checks and the
      "declared here, not folded in" condition stopped existing.
- [ ] **KNOWN GAP — no witness decoder for the newly admitted shapes.** `char`,
      `Option`, `Result` and `[T; N]` reach the engines, but `WitnessValue` cannot spell
      them, so a Kani violation on one is reported `X0901`/`tool_error` naming the
      parameter (never a witness-free `violation`) and a fuzz violation lands on the
      existing `W0541` witness-only path.
- [ ] **NOT DONE, deliberately deferred**: cross-crate type-alias resolution (004's
      `withdraw` takes `ledger::AccountId`; resolving it changes nothing, because the
      `&mut ledger::Ledger` beside it keeps the fn `unsupported` either way), structs of
      scalars, `--only-changed`, `cargo ply check`, `schema/ply.schema.json`, and the
      renderer's earned-vs-declared split (finding 8).
- [ ] **Rendered cex test for a panicking body fails with the function's own panic**, not
      the contract message, because the call sits outside the test's `catch_unwind`.
      §9's cex-oracle clause "failure output states the contract" therefore does not hold
      for that shape; the contract is named in the generated test's comment. The oracle
      test itself (`clamp_oracle.rs`) is unaffected and green.
- [x] **`run.sh` budgets raised, annotated in place**: s5 120s → 600s (the stubbed proof
      needs ~202s of Kani time), s7 120s → 600s (arrays are cheap, this fn's body is not).
      Both original runs are quoted in `docs/post-004-fixes.md`. (A record of a done thing;
      the unchecked box was a bookkeeping slip the 2026-08-25 review caught.)

## Vetting 004 — legacy boundary, fragment-first — landed 2026-08-24

The first vetting scenario designed inside §5.4b's fragment from line one, and the first
run against the real `cargo ply verify` (Kani 0.67.0) rather than reasoned about on
paper. Write-up: `vetting/004-legacy-extension.md`; two crates + `ply.yaml` + `run.sh`
under `vetting/004-legacy-extension/`; SVG committed. Nothing in `crates/`, `tools/` or
`The-Ply-Spec.md` was touched — this scenario finds, a later session decides.

- [x] `legacy/` (ordinary `BTreeMap`/`Vec`/generic-helper module, no `ply::` anywhere) +
      `feature/` (five fns, all claimed, contracts inline) + one `ply.yaml` read by
      `verify`, `ply-check` (clean, exit 0) and `ply-render`.
- [x] Twelve `verify` invocations across `run.sh s1..s8`, all reproducible; every verdict
      quoted in the write-up is literal tool output (two long envelopes are cut to their
      verdict spine, and say so).
- [x] **The boundary's answer is `timeout`.** `tier_fee_cents` (fragment-clean signature,
      body calling one unclaimed `BTreeMap`-backed legacy fn) never finished: `timeout` at
      120s and again at **600s** (11m23s wall). Control: the identical fn with the legacy
      call replaced by a `match` earns `bounded(2)` in 1m20s total. `conditional`/D5 never
      fired — none of D5 (`stub_verified`, `W0511`, `ply-schedule`) is linked into
      `crates/` at all.
- [x] `--only-changed` and `cargo ply check` confirmed **absent** (§6 specifies both);
      recorded as findings, not built.
- [x] **Finding 1 — a run that checked nothing exits 0.** CLOSED 2026-08-25 (`d73558f`). `K0601 timeout` is warning
      severity, `--fail-on` is unimplemented, so a run whose root verdict is `timeout` is
      CI-green. Proposal in the write-up: absence of evidence fails by default.
- [x] **Finding 2 — D5 has no branch for an *unclaimed* callee.** CLOSED 2026-08-25 (`2cf09c2`). Both its branches assume
      the callee has a contract. Needs an explicit third rule, and the diagnostic must name
      the callee that was descended into (K0601 today names only the caller).
- [ ] **Finding 3 — checkability is about bodies, and §5.4b gates on types.**
      `total_debit_cents` (no legacy contact at all) also timed out at 120s in the same run
      where `fee_cents` passed.
- [x] **Finding 4 — `fuzzed(n)` is not reproducible.** CLOSED 2026-08-25 (`c8e231b`), with the panic-witness escalation. Six fresh runs of the *same*
      unfixed source: 3 × `fuzzed(256)`, 3 × `tool_error` (X0901, the real panic). Seed is
      entropy-derived (`Config { cases, ..default() }`) and recorded nowhere; exit code
      flips with it. The §8 envelope needs the seed, and a `--seed`/lockfile replay.
- [x] **Finding 5 — the implemented fragment is narrower than §5.4b.** CLOSED 2026-08-25 for arrays, aliases, `char`, `Option`, `Result`; structs of scalars still open. `[u32; 4]` (the
      spec's own *preferred* bounded shape) is `Unsupported`; so is a `type X = u64` alias.
      No `Type::Array` arm and no alias resolution in `rust_type_from_syn`.
- [x] **Finding 6 — V0505's fix names a mechanism that does not exist.** CLOSED 2026-09-02
      (see "DONE 2026-09-02: one build-route mechanism for named types" above): the fix
      now names the real `routes:` declaration in `ply.yaml`, which exists.
- [x] **Finding 7 — `verify` is single-crate** — CLOSED 2026-08-25 for both halves (`anchor:` consumed in `2cf09c2`, `E0204` parity in `23e8f67`); multi-crate *verification* is still out of scope.: `anchor:` is parsed and never used, every
      component's fns are looked for in one `src/lib.rs`, and ply.yaml `requires`/`ensures`
      are silently dropped (unknown serde fields) while `ply-check` on the same file
      enforces `additionalProperties: false`.
- [ ] **Finding 9 — `--only-changed` is the delta thesis's mechanism**, not a convenience.
- [ ] **Finding 10 — `verify` writes into the crate under test** (generated modules in
      `src/`, harness member appended to `[workspace]`), which is why `run.sh` copies to a
      scratch tree. Second vote for the "where the harness crate should live" item below.
- [ ] **NOT COVERED: 004's document is outside the renderer's invariant sweep.**
      `tools/render/tests/render.rs` walks a hardcoded list of 001/002/003 plus its own
      fixtures. Adding 004 means editing `tools/`, which this session was not permitted to
      do. The committed SVG was rasterised (CairoSVG) and checked by eye instead.
- [ ] NOT RUN, recorded: `mutate`/`prove` in this scenario; a boundary callee with a
      non-scalar signature; any bound other than `bounded(2)`; any budget above 600s.

## External systems and actors — landed 2026-08-24 (31a669d)

Full detail in `docs/external-elements-adoption.md`; the gate this was conditioned
on (vetting re-run before any spec amendment) is recorded as a numbered finding
plus an "external-elements gate" section in `vetting/003-trading-system.md`.

- [x] `tools/model`: `externals:` (`External { note }`, required field) and
      per-fn `entry: Vec<String>` on `FnClaim`.
- [x] `tools/check`: five new document-local rules — `E0202` (name collides with a
      component), `E0207` (external in a `->`/`deny`), `E0208`
      (`external ~> external`), `E0209` (`entry:` names an undeclared external),
      `W0410` (external declared, never referenced) — all fixture-tested,
      `tools/check/tests/externals.rs`.
- [x] `tools/render`: external box outside the frame, `~>`/derived `entry:` edges
      routed around intervening components, frame border weight bumped to read as
      a boundary. New invariant `frame_boundary::no_external_box_intersects_
      the_frame_deny_wildcards_stay_inside_and_external_edges_cross_once`
      (`tools/render/tests/render.rs`) — written red first (confirmed: it failed
      on its own vacuous-pass guard before the renderer had any external support,
      not a compile error), green after, mutation-tested (two real mutations,
      each reverted). Fixed two pre-existing routing-algorithm limitations the
      real 003 picture exposed (wrong rail-side heuristic, obstruction filter too
      narrow) in a new dedicated function, without touching the existing
      (already-tested) deny-line routing at all.
- [x] **Correction, same session, coordinator review**: the committed
      `003-trading-system.svg` (`--collapse ingest`) drew `venue ~> ingest.feed`
      straight through `strategy`/`signals` — a real crossing
      `no_drawn_element_intersects_a_box_it_is_not_inside` did not catch, because
      that test never rendered `--collapse <name>` for any single component, only
      "default" and "--depth 1" (collapse-everything). Root cause and full fix in
      `docs/external-elements-adoption.md`; short version: the test now sweeps
      one `--collapse` per top-level component per fixture (watched go red on
      the exact defect, plus a second, previously-unknown one on
      `--collapse gateway` crossing `pnl`, before the routing fix landed), and
      the router's first-leg sweep — sound for deny's always-off-to-the-side
      `from`, unsound for an external edge's `from` (an ordinary component
      border, which can sit inside another component's column) — now tries a
      straight vertical run first and only detours sideways when that specific
      run is blocked. Both 003 SVGs regenerated again and rasterised with
      headless Chromium; confirmed by eye, no line crosses a box it shouldn't.
- [x] **Second correction, same session, coordinator review**: the committed
      `003-trading-system.svg`'s `RawFrame` edge label was struck by a drawn
      line (the derived `entry` edge's, not even its own path) — same shape of
      gap as the correction above, this time in text, not boxes. Extended
      `no_drawn_element_intersects_a_box_it_is_not_inside` to check every
      `edge-label` against every drawn line (tried and rejected two narrower
      forms first — all-text-vs-all-lines produced false positives on `any`
      `*`/deny `except` labels, own-path-only produced a false negative on this
      exact bug, since the striking line belonged to a *different* edge);
      watched red first, naming the exact label and line, before any placement
      code changed. Fixed by splitting external-edge rendering into a
      route-then-draw two-pass structure so each label can be checked against
      every sibling line (regular edges, deny lines, and other external
      routes), not just its own, plus widening the label-placement escalation
      to vary the anchor point along the segment as well as the perpendicular
      offset. Mutation-tested (line-avoidance clause disabled, confirmed red,
      reverted, confirmed green). 13 pre-existing, out-of-scope violations on
      edges that predate this feature (`BookUpdate`/`OrderIntent`/`Order`/
      `Fill`) are now surfaced by the general check but not failed on —
      recorded, not fixed, per `docs/external-elements-adoption.md`. Both 003
      SVGs regenerated a third time and rasterised again; confirmed by eye, no
      label is struck by a line in either image.
- [x] `vetting/003-trading-system.ply.yaml`: `venue` external, three flow edges,
      `entry: [venue]` on `Oms::submit`; `ply-check` clean. Both committed SVGs
      regenerated and diffed line-by-line before accepting; `vetting/001-*.svg`,
      `vetting/002-*.svg`, and the disruptor insta snapshot regenerated too (the
      only diff in each: the frame stroke-width bump, confirmed by diffing).
- [x] `The-Ply-Spec.md` amended: §5.1 (structure + example), §5.1a rule 6, §5.3
      (external edges), §7.1 (two table rows + the dash-channel restatement),
      §7.2 (the fourth kind of unspecified — "out of scope by ownership").
- [x] Gate passed — no fallback to the flag-only form was needed.
- [x] `cd tools && cargo test`, `cargo fmt --check`, and
      `cargo clippy --release --all-targets -- -D warnings` all green/clean.
- [ ] **Left for the maintainer, not attempted**: the *holistic* squint test —
      does this read well, beyond "nothing overlaps" — is explicitly the
      maintainer's own call, per the task brief. The specific correctness
      property (no line crosses a box it shouldn't) is now confirmed two ways,
      not just judged: the extended invariant, and a direct-eye check of both
      committed SVGs rasterised with headless Chromium.
- [ ] NOT RUN: a document with more than one external, or with two external
      edges to the same external from different components sharing no lane —
      the layout code has a defensive width-overflow guard but no fixture
      exercises multi-external layout or that specific lane-fan gap.
- [ ] Out of scope by the task brief, not attempted: `crates/` (the
      `entry:`/audit surface lands there at M5); `tools/kernel` untouched (and
      correctly so — externals never enter the verdict tree).

## M4 — fuzz + test + mutate tier — landed 2026-08-24 (2520f8b)

Note on provenance: 2520f8b's own message flags that the full-suite result was "NOT
yet independently confirmed" at commit time (a session-salvage commit, written before
verification finished). It now is: `cargo test --workspace` (single-threaded) is green,
5m31.8s wall clock, zero warnings on a fresh `cargo check --workspace --tests` — recorded
in docs/m4-findings.md along with two deliberate self-mutations, each caught and reverted.

- [x] Task 0: engine-timeout default made shape-aware
      (`verify::default_engine_timeout_secs`) — a `Vec`-typed `bounded(k)` harness now
      gets `30 + 15·k` seconds (reproduces the M3-measured 150s for `bounded(8)` exactly);
      scalar-only stays at 60s. §6 amended with the reasoning.
- [x] `fuzz(n)` check: proptest harness generation (ints biased small, `Vec`/`BTreeSet`
      length 0-8, `requires` as a reject filter with a >50%-rejection `W0503` warning),
      shrink-on-failure rendered through the *same* `contract_rt` renderer the Kani path
      uses. Struct-parameter fuzzing NOT implemented -- deliberate scope cut, recorded in
      docs/m4-findings.md, not silently skipped.
- [x] `test` check: `examples` entries (parsed as arbitrary `==` Rust exprs, §5.4a) +
      generated direct-contract boundary cases.
- [x] `mutate` check: cargo-mutants wired via the spike's verified mechanism, with a real
      correction the spike didn't know about -- `--copy-target true`, not `--gitignore
      false` (see below). `E0504` (mutate with no test/fuzz kill signal) implemented and
      fixture-tested.
- [x] Shape-aware default-check routing (§5.4c's own MUST, unimplemented in M3):
      `default_checks_for` -- `[bounded(2)]` only when Kani-supported, `[fuzz(256)]` when
      the shape is fuzz-supported but Kani-excluded, `[]` otherwise.
- [x] 5 new fixtures + e2e tests (`fuzzbug`, `weakspec`, `strongspec`, `mutatetier`,
      `btreeset`) -- all 6 of the M4 brief's acceptance criteria pass, including the
      milestone's own headline case: a `BTreeSet<u8>` fixture (Kani-excluded per §5.4b)
      earning an honest `fuzzed(256)` verdict via the default route, no `checks:` declared
      by hand.
- [x] **Falsified spec claim, real cost**: §5.4c's mutate mechanism said `--gitignore
      false` was the fix for the harness crate's git-ignored `target/ply/fuzz/`
      placement. Real runs found this wrong on two counts -- `--gitignore`'s own default
      is already off, and there is a *separate*, gitignore-independent skip
      (cargo-mutants prunes any directory literally named `target` at the copy root,
      unconditionally) that hit every real `mutate` run. Fixed with `--copy-target true`
      (which cannot even be passed alongside `--gitignore` -- confirmed, they share a
      clap argument group). Honest cost: this copies the crate's entire `target/` build
      cache into every scratch tree cargo-mutants builds (~13s against a 189MB `target/`
      in this session's fixtures) -- a real, size-dependent tax flagged for M5, not a free
      fix. §5.4c amended.
- [x] Falsified: `engines::fuzz`'s failed-test-name parser looked for libtest's per-test
      `---- name stdout ----` header, which never appears under `--nocapture` (which this
      adapter always passes, for the high-rejection marker) -- caught because it silently
      reported a real seeded bug as a clean pass on the first real run against a fixture,
      not by a unit test in isolation. Fixed; a regression test pins the real output shape.
- [x] Two deliberate self-mutations, each caught and reverted (docs/m4-findings.md):
      suppressing the `·spec-strong` suffix append (caught by `strongspec_fixture`);
      removing `--copy-target true` again (caught by `weakspec_fixture`, reproducing the
      exact tool-error this session hit for real before the fix).
- [ ] **KNOWN GAP, recorded not hidden**: fuzz-found witnesses are not persisted across
      `verify` runs the way Kani's are (M3 finding 6's `target/ply/witness/<fn>.json`
      convention has no fuzz-path equivalent yet) -- a fix that narrows a bug to a
      *different* input than the one already rendered would leave a stale red test behind.
      Needs its own `<fn>_fuzz.json` path (never the same file Kani writes to, since one
      fn could in principle declare both `bounded` and `fuzz`).
- [x] `W0541` (unrenderable fuzz witness) was implemented but NOT exercised against a real
      failing case. **Now run** (2026-08-24 review closure): `tests/fixtures/btreesetbug` --
      a `BTreeSet<u8>` violation reported witness-only, shrunk to `[3]`, no `cargo_test`
      artifact, exit 1. The item's original wording ("`Vec`/`BTreeSet` of non-`u8`") was
      itself wrong: the path fires for every `BTreeSet`. Still not run: a `Vec<i32>`-shaped
      witness.
- [ ] `mutate`'s `--re <fn>` is an unanchored substring match on cargo-mutants' own
      descriptive mutant names (anchoring with `^fn$` matched *zero* mutants in a real
      run -- confirmed and fixed to the unanchored form). This means a fn whose name is a
      substring of another's in the same crate could see cross-fn mutate scope leak; no
      fixture here exercises more than one fn per crate, so this was not reproduced, only
      named.

## M4 adversarial review — closed 2026-08-24 (see docs/review-m4-2026-08-24.md and docs/m4-review-closure.md)

Every item below was fixed red-first: the test that fails *because of that defect* was
written and watched fail before the fix, and its failure message read to check it named
the defect. `cargo test --workspace -- --test-threads=1` green afterwards: 405s (6m45s)
wall clock, 72 tests (was 53), zero warnings on `cargo check --workspace --tests`.

- [x] **D1 (SEVERE) — the fuzz/test adapter failed open on a harness that would not
      compile**: an ill-typed `examples` entry earned `fuzzed(64)`/`tested` with zero
      diagnostics and exit 0. A run that did not succeed, did not time out and named no
      failing test ran *zero* cases: now `X0901` + verdict `tool_error` for every check in
      that harness, carrying the compiler's own first error and two concrete fixes. Pinned
      by `tests/fixtures/badexample` + `tests/e2e/tests/badexample_fixture.rs`. §5.4c
      amended with the rule.
- [x] D2 — counterexample `inputs` mislabeled for non-alphabetical parameter order: fixed
      by the reviewer in `94e0a2d`, not redone here.
- [x] D3 — the `>50%` rejection `W0503` was arithmetically unreachable (rejected draws
      counted on both sides of the ratio, i.e. `accepted < 0`). Now `rejected/total`;
      `tests/fixtures/highreject` (~62% rejection) pins it, and the wording no longer
      claims fewer cases ran than the verdict says.
- [x] D4 — a fuzz run proptest *abandoned* (global-reject abort) still earned
      `fuzzed(256)`. Now `unclaimed` + a `W0503` naming the real accepted/rejected counts,
      via a distinct `PLY_FUZZ_ABORT` marker. `tests/fixtures/rejectabort`. §5.4c amended.
- [x] D5 — `M0601` was dead code and cargo-mutants ran with no wall-clock cap (`-t` caps
      only each mutant's test phase, so a hung copy or baseline build hung `verify`
      silently, which §5.4c forbids). The invocation is now wrapped in `timeout` like the
      fuzz and Kani adapters, exit 124 classifies as `Timeout`, and the cap is 10x the
      per-mutant budget (min 120s; measured runs use ~4% of it). M0601's wording no longer
      says "per mutant".
- [x] D6 — a `violation` could be emitted with no witness (marker-parse-failure path),
      breaching §5.4c's MUST. The label now comes from what the renderer could establish;
      `tests/fixtures/panicbug` (a panicking body — an ordinary case, not a contrived one)
      pins `tool_error` with rewritten text and fixes.
- [x] D7 — `W0541`'s wording was false for the exact shape that triggers it (it fires for
      every `BTreeSet`, `BTreeSet<u8>` included). Reworded and exact-string tested; the
      same false claim corrected in three doc comments and docs/m4-findings.md.
      `harness::tidy_contract_text` also widened for method calls, so the quoted contract
      reads `xs.len() as u32` instead of `xs . len () as u32`.
- [x] D8 — five in-tree doc comments asserting claims the M4 commit itself falsified
      (`--gitignore false` "must always pass it explicitly", the mutants mechanism block,
      `failed_tests`' `---- name stdout ----` claim, "Ply always anchors this",
      `write_harness_cargo_toml`'s phantom parameter) all corrected.
- [x] O1 — "derived, not guessed" overstated two fitted constants: `verify.rs`'s doc
      comment and §6 now separate the derived shape split from the fitted coefficients,
      and both record that no e2e exercises the default.
- [x] O2/O4 — the shrinking claim and the `btreeset` acceptance were weaker than their
      names. `tests/fixtures/btreesetbug` (the Kani-excluded shape with a real bug) closes
      both *and* docs/m4-findings.md's own NOT RUN item: witness-only `W0541`, shrunk to
      `[3]`, no `cargo_test` artifact, exit 1.
- [x] O3 — "every M4 non-result diagnostic carries a concrete `Fix`" was not true; the
      claim is corrected in docs/m4-findings.md and all five named paths now carry fixes.
- [x] O5 — partly closed (no-violation-without-witness, witness-only, and
      never-claim-evidence-you-lack are now all tested end to end); the remainder is
      recorded below.
- [x] Found while fixing the above: an `examples` entry containing a `"` generated invalid
      Rust (the entry is echoed into the assert message unescaped), and a `mutate` run that
      produced no result at all was reported as `weak-spec` — a finding no engine made. Both
      fixed red-first; inconclusive mutate runs now carry D6's own `inconclusive` status.
- [ ] **NOT RUN, recorded not hidden**: `M0601` against a genuinely hung cargo-mutants, and
      `P0601`/`R0601` against a genuinely slow harness. The caps and classifications are
      unit-tested; no fixture is slow enough to trip them without making the suite
      timing-fragile.
- [ ] **NOT RUN**: the `W0110` engine-missing paths (`prove`, cargo-mutants absent) — no
      fixture masks an engine, so their newly populated `fixes` are unobserved. §9's own
      engine-absence matrix is the right home for this.
- [ ] §5.4c's "MUST carry the distinguishing engine output into the diagnostic" is now met
      by the new `X0901` (carries the compiler's error line) and `W0503` (real counts), but
      every other adapter still drops `raw_output` (`let _ = raw_output;`) — M3-inherited,
      unchanged.
- [ ] The shape-aware engine-timeout default is exercised by a unit test only: every e2e
      passes `--engine-timeout` explicitly, so no test observes the default in real use.
- [ ] `ensure_workspace_member` bails on any crate whose `Cargo.toml` lacks a `[workspace]`
      table — i.e. every ordinary crate inside a larger workspace, and every standalone
      crate without the marker. `fuzz`/`test`/`mutate` therefore work only on
      fixture-shaped crates today. Needs the same decision as the `--copy-target true`
      cost: where the harness crate should live.
- [ ] `mutants.out/` is left in the user's crate root after every mutate run (removed at the
      *start* of the next one) — outside the `target/ply/` housekeeping convention.
- [ ] A missing-engine label beats a passing check in `combine_fn_check_verdicts`
      (`checks: [prove, fuzz(256)]` with fuzz passing yields `engine-missing`), contradicting
      D9 and D6's status-vs-order split. Unreachable until M7 declares `prove` fixtures —
      fold into the M5 verdict-kernel work.
- [ ] `checks: [fuzz(n), test]` on a fn with no `ensures` silently drops the `test` check
      too (the no-`ensures` `V0505` branch returns before the harness runs, examples
      included). Examples need no postcondition, so this is a routing fix with its own
      fixture, not a wording change.
- [ ] Mutants whose tests time out land in cargo-mutants' `timeout.txt` and do not block
      `all_caught()`, so a fn can earn `·spec-strong` with timed-out (uncaught) mutants.
      Defensible as cargo-mutants' own convention; undocumented until now.


- [x] `ply-render --depth N` / `--focus` / `--collapse <component>` (8d8910f) —
      collapsed box shows contents line, rolled-up capability badges, pin and finding
      counts; edges reattach; default output byte-identical without flags.
- [x] Collapsed boxes draw as a stack (dc1ad4b, repaired in 26cdeb6); 003's canonical
      artifact is now the collapsed system view, full depth moved to -full.svg.
- [ ] **Color SVG config** — make the renderer's palette configurable (the style
      constants: ceiling scale, finding red, ink, amber) instead of hardcoded; must
      keep the §7.1 channel discipline (a config can retune a hue, not repurpose a
      channel) and the style-rule invariant test.
- [ ] `ply-render --legend` — opt-in legend strip below the frame, generated from the
      live style constants (§7.1, specced 2026-08-23).
- [x] `W0409` redundant parent-to-descendant edge lint (7d4c6fc) — both directions,
      both edge kinds; brought a W-warns/E-fails severity model with it.
- [x] Edge and deny routing + collision-freedom invariant (b3da43c, 2b07bd0) — 003
      render findings 1, 3, 4 closed. KNOWN GAP left open deliberately: deny lines in
      *different* margin columns can still cross (repro:
      tools/render/tests/fixtures/deny_stress.ply.yaml). Needs a routing policy
      decision (§7.1), not a guess.
- [x] Gate debt closed for real — `strict` notch, `mode: synth` violet chip, `examples`
      e×N token all drawn and test-pinned (worktree merge).

## Engine strategy — settled 2026-08-24 (fable review)

- [x] **No pivot to VeriFast, and no additional engine now.** It is a category error:
      Ply is multi-engine by design (D9, §5.4c), so there is no "primary engine" to
      swap — only check kinds and adapters. Three independent reasons VeriFast is the
      wrong first tenant: (1) it emits a symbolic-execution trace, never a concrete
      counterexample, and a failure means *unproved*, not *false* — so a VeriFast-primary
      Ply could never emit a `violation` from its main engine, deleting §1's core
      mechanism rather than weakening it; (2) LLM proof-closure measures 31.4%
      (arXiv:2606.26490, C) against Verus's 44% / AutoVerus ~90%, and our users are
      agents; (3) measured cost on the real verify-rust-std proofs: linked_list
      2,254 → 4,390 lines (+95%; 39 lemmas, 166 `open`, 229 `close`), raw_vec
      854 → 3,246 (+280%).
- [x] **Today's answer for external proofs**: they enter as `trusted` claims — no new
      grammar, and safe now that attestations go stale with the code.
- [ ] **When we reach M7**, the `prove` slot takes a deductive engine, with **Verus as
      first tenant** (Rust-shaped, better agent proof-closure), not VeriFast. Adding any
      engine is milestone-sized, and we are one milestone of seven in with no working
      `cargo ply` — so the next step stays the thin end-to-end slice, not a second engine.
- [x] **Verus feasibility spike done** (`tests/spike/verus/`, FINDINGS.md) — the
      deductive-vs-bounded question the scale spike left open. Result: a `Seq`/`Set`-based
      Verus shadow of the kernel proves all four standing obligations, unbounded, by
      structural induction, in ~2s (mutation-tested, not vacuous) — exactly where Kani's
      bounded model checking cannot terminate at all on the same recursive shape. A
      differential test (4,000 generated trees, plain `cargo test`) binds that shadow's
      executable transcription to the real `ply-kernel` crate. **Open before M7 commits**:
      this proved a faithful shadow, not `tools/kernel/src/lib.rs`'s literal
      `Vec`/`String`-based source — whether Verus's own executable-collection support
      pays the same symbolic cost that stalled Kani on `Vec<String>` is untested and is
      the next spike, not a foregone conclusion of this one.
- [ ] **Revisit triggers** (a decision with a trigger, not a dismissal): an
      AutoVerus-equivalent for VeriFast reaching Verus-level proof-closure; VeriFast
      emitting machine-readable failure output an adapter can parse; a vetting scenario
      showing `fuzzed·spec-strong` is genuinely insufficient on recursive structures; or
      the arena-flattening experiment failing.
- [x] **Trusted claims had no staleness** — the evidence-lying hazard the fallback would
      have shipped: an entry outlived the code it attested and rendered identically fresh
      forever. §5.4d now fingerprints the attested item, marks it stale on change, and
      requires human re-attestation (`accept` does not clear it).
- [ ] **Separation-logic constructs: split the question.** Lemma functions and ghost
      open/close are proof steps, not specification — below the watermark, never
      spec-resident. Heap *predicates* are admissible in principle (the §7.1 gate admits
      them on the same mark-plus-tooltip precedent as contract clauses; separation logic
      is highly diagrammable), but largely unnecessary: §5.4a already admits calls to
      `pure` helpers, so a recursive `pure fn len(&self)` is legal in a contract TODAY.
      What is missing is an engine that can earn `proved` on it, not vocabulary.
- [ ] Smallest useful version for M7: a per-fn `proof:` field naming a proof artifact,
      drawn as a badge, fingerprinted under D14 so it goes stale with the body. No
      predicate sub-language until a vetting scenario forces one.

## From the external review (codex, 2026-08-23)

*The raw transcript that was committed as `docs/review-2026-08-23.md` was deleted on
2026-09-01: it was another tool's session log, complete with its version banner, 24 raw
tool-invocation markers and 25 absolute paths from the reviewer's own machine. Its findings
are the items below; the transcript itself is in git history if anyone ever wants it.*

- [x] **M0 spike done, ADR-0003 accepted** (0974f57). 8/9 mechanisms work; cross-crate
      stubbing works via caller-local re-proof. Fixtures + run.sh under tests/spike/.
- [x] **M0 fully discharged** — the cargo-mutants item is now exercised
      (tests/spike/mutants/). It found §5.4c asserting a mechanism that does not
      exist: there is no "custom test command" flag, and the claim "confirmed in the
      M0 spike" was false. Real mechanism verified and specced; `--gitignore false`
      must be pinned or the build fails; `W0502` now caveats equivalent mutants
      (strong spec killed 13/14, the survivor was provably equivalent, not a gap).
- [x] **Scale spike done; §5.4b rewritten around evidence** (tests/spike/scale/).
      Headline: recursive/self-referential types are NOT supported in v1 — a 3-node
      tree makes 64,147 verification conditions and doesn't finish in 180s, even with
      the unwind fix that makes flat `Vec` cheap. That is the shape of our own verdict
      tree. Also: `Vec` works ONLY if codegen emits `#[kani::unwind(N+1)]`; fixed
      arrays are cheap and become the preferred shape; BTreeSet/BTreeMap are out past
      one element; HashMap needs a codegen hasher swap or it won't compile.
- [x] **Self-hosting resolved**: the enumeration IS bounded-kind evidence (exhaustive
      within a stated bound, independent oracle, covering more than the Kani harness
      would have). CLAUDE.md reframed rather than apologised. Reshaping the kernel was
      rejected on evidence — the stall just moves to the next unbounded field.
- [x] **Enumeration REDUCTION ARGUMENT written 2026-08-25** (`docs/kernel-honesty-cleanups.md`
      part 2). One leg held (per-bit uniformity of StatusSet). The other — content-independence
      of the assumption merge — **did not**, and was measured not holding: six one-line
      breakages of the real kernel, four survived the corpus as it then stood. The corpus was
      repaired in the same change (period-2 payload cycles in `tools/kernel/tests/enumeration.rs`)
      and those four now die. See the KNOWN GAP below for the fifth.
- [x] **Kani harnesses DELETED 2026-08-25**, not gated (`docs/kernel-honesty-cleanups.md`
      part 1). They contradicted our own §5.4b rule — a recursive shape is one Ply refuses
      by name rather than routing to an engine that times out — and the role they filled is
      now filled better by `tests/spike/verus/`, which proves all four obligations unbounded
      by induction in ~2s. The investigation survives as a historical note in
      `crates/ply-core/src/kernel.rs`'s doc comment.

- [ ] **KNOWN GAP — one kernel mutant still survives the enumeration.** A node carrying
      BOTH a status flag AND a conditional at once is not in the enumerated corpus, so a
      breakage that treats the two as mutually exclusive (miscounting what is still owed)
      goes unnoticed. Left open deliberately: closing it costs 3,117,996 trees, roughly
      tripling the gate's runtime. Recorded 2026-08-28 — it was measured on 2026-08-25 and
      never written down, which is exactly the failure this list exists to prevent.

- [ ] **KNOWN GAP — is an empty assumption list representable, and should it be?** Raised
      by the same 2026-08-25 reduction work and never carried into this list. A spec
      question, not a bug: decide and write it into The-Ply-Spec.md either way.
- [ ] **Generalise D13 beyond M0**: each milestone opens by spiking its riskiest
      external-tool claim, and no spec sentence may say "confirmed"/"verified" without
      naming the artifact that shows it. §5.4c carried a fabricated confirmation until
      an adversarial re-check caught it.
- [ ] Measure whether the unwind annotation rescues ITERATOR-CHAIN bodies (marked NOT RUN
      in the scale sweep). Until measured, §5.4b's gate still admits functions that hit
      the exact failure it was rewritten to prevent.
- [x] **Callee-before-caller ordering got kernel-grade treatment** — new `ply-schedule`
      crate: SCC-condensation planning (cycles land in one batch, never deadlock) and a
      `may_stub` decision that returns Allowed ONLY when the callee's proof actually
      passed this run. Invariants enumerated exhaustively: all 65,536 four-node digraphs
      for planning, 4,096 graph+config combinations for the stub decision, both against
      oracles written from D5's text rather than from the production code. Mutation-
      checked by letting `NotRun` license a stub — the exact unsound shortcut Kani
      itself takes — and confirming it goes red.
      **RETRACTED as of 2026-08-30 (`4dd4d30`): the SCC-condensation planning
      described above never shipped** — `crates/ply-cli/src/verify.rs` carried its
      own, stricter, untested ordering the whole time, and the two disagreed about
      where a cycle's dependents go (see "The two schedulers order cycles
      differently" and its resolution, above). The planning half (`plan`/`Batch`) is
      now deleted; `may_stub` and its own enumeration, described accurately above,
      are untouched. Read this bullet as "the stub-decision half only" from here on.
- [ ] **D5 ambiguity surfaced by the scheduler**: cross-crate proof results are really
      scoped per (calling-crate, callee), since each consumer re-proves locally, but
      `ProofResults` models one global status per fn. Exact for same-crate; a
      simplification cross-crate. Decide before M3 whether the distinction matters.
- [x] **Real defect fixed: component-level `checks` inheritance** (merged from
      worktree): a fn's own list wins entirely; otherwise it inherits the nearest
      ancestor component's default. Resolution lives once in `ply-model` so the
      validator and renderer cannot drift. Tooltips now name the source —
      "inherited from component `pricing`: bounded(2) — …". E0504 evaluates the
      effective list. All five committed SVGs byte-identical (no vetting document
      uses component defaults — grep-confirmed, not assumed).
- [x] Engine-limit diagnostics specced (52222ab) — §8 now requires timeout/unsupported
      to name the cause and populate `fixes`, with the boundary written in: Ply
      proposes, never rewrites. IMPLEMENTATION still owed when the engines are wired.
- [x] `schema/ply.schema.json` was called normative in §5/D3 while not existing. BUILT
      (`c8528ce`) and load-bearing: the key vocabulary and required-field list are read from
      it at runtime. Also recorded at the top of this file under Phase 1a.
- [ ] Separate declared ceilings from earned verdicts in the type system (both are
      `Evidence` today; only convention keeps them apart).
- [ ] `trusted` claims are unrestricted prose — no identity, date, commit, scope, or
      expiry. The shield can read as approval.
- [ ] `conditional` assumptions are free-form strings, untied to the call graph.
- [x] Renderer CLI now covered — 11 tests over flags, exit codes, and error wording;
      two messages rewritten to the newbie bar (`--depth 0` and a non-numeric depth
      used to fail silently or with clap's raw error).

## M3 thin vertical slice — landed 2026-08-24 (7e6fc79)

- [x] First production code of `cargo ply` itself: `crates/ply-attrs` (the
      `#[ply::requires]`/`#[ply::ensures]` proc macros, D2), `crates/ply-core`
      (`config`, `harness`, `engines::kani`, `contract_rt`, `diag` — exactly the five
      modules authorized, nothing more), `crates/ply-cli` (`cargo-ply verify` +
      `--json`). Root `Cargo.toml` is now the product workspace
      (`members = ["crates/*", "tests/e2e"]`, `exclude = ["tools", "tests/spike",
      "tests/fixtures"]`), separate from `tools/Cargo.toml`.
- [x] Four fixtures under `tests/fixtures/` (`clamp`, `passing`, `vecbound`,
      `timeout`) plus 5 black-box e2e tests under `tests/e2e/` that build the real
      binary and run it — the §9 cex validity oracle, for real, on the `clamp`
      fixture: FAIL (stating the contract + "postcondition", never the overflow trap)
      before the fix, PASS (the same `ply_cex_clamp_01` test) after. `cargo test
      --workspace` green (17 unit tests + 5 e2e tests).
- [x] Measured (not copied from §5.4b's own number, which is for a different harness
      shape) the Vec unwind bound for this slice's own harness: `k+1` for a manual
      indexed-loop consumer of `any_vec::<u8,k>` — 9 at k=8, confirmed 8 fails
      ("unwinding assertion loop 0") and 9 succeeds, with an adversarial e2e test
      proving the emission is load-bearing (the identical harness minus the
      annotation does not verify within a bounded cap).
- [x] Timeout correctly distinguished from violation end to end (`K0601` vs `K0502`,
      `timeout` status carries no counterexample) — see docs/m3-slice-findings.md
      finding 3 for a real, load-bearing caveat: this environment shows CBMC/CaDiCaL
      SAT-solve wall-clock variance (~1s to ~107s on an *identical* harness), and one
      run's raw CBMC log showed a SATISFIABLE result reached before Kani's own
      "CBMC timed out" text was printed — meaning the timeout/violation textual
      distinction can, rarely, itself be racing the engine's own reporting. Routed
      around with generous timeouts here; not fixed. Flagged for the next session.
- [x] Spec amended: D2 (the `unexpected_cfgs` lint requirement, confirmed again);
      D7 + §0 + §1 + §8 + §9 + the M3 milestone bullet, applying
      `docs/plans/d7-replayable-tests.md`'s own pre-drafted deltas now that the D7
      renderer is actually built (the `kani_playback`→`kani_witness` rename is live in
      code, pinned by a unit test).
- [x] Two deliberate self-mutations (§ CLAUDE.md), each caught and reverted: disabling
      the "CBMC timed out" check in `parse_output` (caught by a unit test); making the
      rendered cex test's `Ok(false)` arm a no-op, i.e. "renderer skips the assertion"
      (caught by the real `clamp_oracle` e2e test going red, not a unit test).
- [ ] **KNOWN GAP, recorded not hidden**: `docs/m3-slice-findings.md` finding 6 — the
      witness-persistence mechanism that makes the D7 oracle's "same test transitions
      FAIL→PASS" promise hold across two `verify` runs (`target/ply/witness/<fn>.json`)
      is a real design decision this slice made ad hoc; it was not settled in the D7
      plan and duplicates a sliver of what full D14 staleness tracking will eventually
      own. Needs an explicit call, not silent acceptance, once `ply.lock` lands (M1).
- [ ] Witness-replay half of the §9 oracle (`cargo kani playback` reproducing a stored
      `kani_witness`) is implemented (`engines::kani::run_playback`) but NOT wired into
      `verify` or any e2e test — recorded as NOT RUN in §9, not silently skipped.
- [ ] Not attempted this session, all explicitly out of scope per the M3 brief:
      `impl`-method contracts (`&self`, `old()`), generic fns/`check_with`, cross-crate
      callees, `stub_verified`/`conditional` (D5), the `ply.yaml`
      `requires`/`ensures`-merge path (only inline attributes are read),
      `BTreeSet`/`HashMap` handling, the engine-timeout reliability fix above.
- [ ] TODO(M1), recorded in `crates/ply-core/src/config.rs`'s own doc comment:
      reconcile the hand-rolled ~4-struct `ply.yaml` model here with `tools/model`'s
      full model (promote one, delete the other).
