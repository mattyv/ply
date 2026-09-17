---
name: ply-verify
description: Verify early and after meaningful implementation changes, interpret Ply results, repair defects, and optionally publish a completed visual run without weakening declared intent.
---

# Ply Verify

Use Ply's public CLI as the authority. Do not reproduce its verifier, verdict rules, record format, or artifact writer.

## Establish scope and run ownership

Before starting, establish the user's product verification root and visual client.
Remember an existing preference rather than asking again. A narrow crate run may prove
the first compiling slice early; finish verification and requested publication on the
selected product root. If composition is unsupported, report the limitation instead of
publishing another root or copying child evidence into a root artifact.

Serialize verification jobs sharing a checkout, linked roots, or generated artifact
directories. Check whether a run is already owned and active before starting another.
A rendering or review worker must not probe `verify` while that run is active, or
interrupt it to refresh a drawing. Recover a finished process's output before assuming
its completion notification is still pending. This concerns separate CLI invocations;
the verifier's own `--jobs` schedules work within one owned run.

## Verify while building

Use verification to guide implementation, not only to approve the finished feature.

- **Establish a baseline before changing existing claimed behavior.** Run the affected
  root's check and verification, or use a result already obtained in this task for the
  same unchanged code. Record existing failures separately from failures your change
  introduces. For new code, write a meaningful contract and a compiling first slice,
  then verify it before building callers or further behavior on it.
- **Repeat after each coherent change to claimed behavior.** Examples include a new
  decision function, a changed state transition, or an edit to a helper a claim relies
  on. Run the public workflow below for the affected roots, including callers in other
  roots that depend on the changed behavior. Do not wait until the whole feature is
  implemented. A workspace-level structural check does not replace crate-level
  verification of function claims.
- **Read the first result before expanding the implementation.** A violation, unsupported
  shape, tool error, or narrowed input domain should inform the next step. Repair the
  problem or explain the unresolved gap before building on that evidence. Independent
  work can continue; do not treat the affected behavior as verified.
- **Keep repairs cheap.** Once a counterexample has a regression test, use that test for
  individual repair edits. When it passes, rerun verification against the unchanged
  obligation. Ordinary tests still cover integration behavior and properties outside
  Ply's reach. Do not run every engine after every keystroke or weaken the checks to
  make iteration faster.
- **Finish on the final code.** After the last meaningful edit, run relevant ordinary
  tests and the declared verification for all affected roots. A passing run already
  obtained for that final code need not be repeated. Report what passed and what remains
  unresolved; a saved visual is evidence for its recorded run, not for later edits.

Keep `ply.lock` and let the public verifier decide which recorded results it can reuse.
Do not delete it routinely to force work, edit it to claim success, or decide that its
presence makes a check unnecessary. Reuse is limited to the inputs this Ply version
tracks. Choose checks for the requested evidence. Use `fuzz` for suitable sampling
claims; run requested bounded checks from the first meaningful slice and retain
existing evidence requirements.

## Interpret incomplete and mixed results

Diagnose unsupported input generation, solver timeout and a counterexample separately.
Unsupported generation means the engine did not construct the requested domain; a timeout
means the proof did not finish within its recorded budget; a counterexample is evidence
that the real body violates the claim for a named input. Preserve the exact outcome and
reason instead of flattening them into one failed status. A generated harness that
fails to compile is a tool error: no cases ran and no behavioral evidence was earned.
Check suggested fallbacks on the real signature before calling them working routes.

A red aggregate status does not erase evidence earned by individual functions. Report
each passing function with its check kind and bounds, then name every unresolved or failed
function separately. Partial evidence never implies full component or whole-document
coverage. When direct Kani checks supplement Ply, label them separately and do not convert
them into Ply results.

## Before running real code

Fuzz checks and regression tests execute the implementation, including its callees.
Inspect the affected path for writes, network calls, and other side effects before a
baseline or repair run. Use the project's isolated test setup or check the decision logic
separately; a supported signature does not make live side effects safe to exercise.

## Workflow

1. Find the verification root requested by the user. A workspace/root `ply.yaml` may contain hollow top-level components that derive one-hop links to crate-local `ply.yaml` files. Verifying that root verifies each eligible linked component in the same invocation; do not run or publish the linked crates separately as a substitute. For an implementation change with no requested workspace root, walk to the nearest ancestor containing `ply.yaml`. If scope remains ambiguous, state the root selected before running anything.
2. Run the fast public check for each root:

```bash
cargo ply check path/to/crate --json
```

3. Inspect the check diagnostics. Fix schema or document-semantic errors that prevent
loading the intended root. Missing proposed anchors or an unsupported claim do not
prevent verifying other resolved claims: run verification to earn partial evidence,
keep those gaps visible, and retain the default evidence threshold:

```bash
cargo ply verify path/to/crate --json
```

Do not add `--fail-on error` to turn missing evidence into success. Use `--engine-timeout` only to give the same declared checks more time. Use `--seed` only with the 64-character seed emitted by a prior public JSON result.

A composed root run freezes each linked document before engines start, uses the child crate's own configuration and result record, and grafts the selected child result before root aggregation and publication. Read linked evidence as evidence from this one run, not as cached evidence copied from an independent child run. `E0211` means Ply could not map the child result without guessing (for example, another top-level owner also has claims or ids collide); report and fix the ambiguity rather than falling back to separate snapshots.

4. Read the command's public JSON and exit status together. Report what passed, what failed, what evidence is absent or narrowed, and any concrete counterexample or repair offered by the diagnostics. Never infer success from a partial tree or from the absence of an error message.
5. Repair implementation code when the declared intent is clear, then rerun the same root. Stop at the approval boundary below instead of editing the goal to fit the code.
6. Publish a visual run only when the user asks for one or the task explicitly requires a visual client artifact. Publication must occur through the same verification command:

```bash
cargo ply verify path/to/crate --json --publish-view
```

`--publish-view` records the completed outcome; it does not turn that outcome into success. Do not construct or edit `target/ply/view.json`, a `visual.json`, or `ply.lock` yourself.

When a verified drawing is requested, read installed CLI help and add `--svg` and
`--svg-overview` with explicit output paths to this same verification command.
For an editor such as ply-vis, include `--publish-view`. `cargo ply render`
shows declarations; it cannot refresh earned colours.

Confirm each requested file exists. With `--publish-view`, read the selected root's
`target/ply/view.json` and its indexed snapshot, checking run identity,
`run.root.path` and linked results. For SVG-only exports, inspect the drawings
against this invocation's JSON; an older or absent view index says nothing about them.
Inspect JSON exit status, node verdicts, statuses, structured domains, and rendered
nodes together. A bounded pass can coexist with another check's tool error.
Count function claims by node kind; total document nodes are not engine-running functions.

Inspect the requested SVG or viewer visually before reporting publication complete.
Put the verified view first in a viewer you are authorized to update, label declaration
views and saved-run freshness, and expose the raw result, bounds, and failed checks.
If HTML is part of the requested client, check that its views open without script errors.
Never manually recolour proof artifacts. If the viewer cannot be inspected or updated,
report that limitation separately from the successfully published files.

A completed snapshot may include tool errors and remain useful to review. Publish it
only when requested, and label verification unresolved. An aborted command with no
completed snapshot supplies nothing to publish; do not invent one.


## Repair a broken promise

When a function violation produces a generated `#[test]` at `src/ply_generated_cex.rs`,
it holds the input that broke the promise. Confirm the diagnostic names that artifact
before using it as the repair loop:

```bash
cargo test        # from the crate root -- it fails the same way the run just did
```

It is ordinary Rust and needs no engine, so iterate against it directly and only re-run
`cargo ply verify` once it passes.

Read the diagnostic and test failure before changing anything. A failed comparison may
show its two values; a panic in the body may happen before the postcondition is evaluated.
Do not assume every violation has the same failure shape.

| What you conclude | What to do |
| --- | --- |
| The body is wrong | Fix the body. This is the default and needs no approval |
| The promise is wrong | **Stop and ask.** Weakening a promise until a test passes converts a real finding into a green result |
| The promise is right but far too broad for this callee | Ask, with the proposed narrowing and the failing input |

Two failure shapes are not repairable this way and must not be treated as one:

- **No generated test, only a recorded input** (`W0541`). Ply found the failing case and
  could not write it as Rust source — usually a value built by a constructor plus a
  sequence of calls, which has no literal form. The violation is real; reproduce it by
  hand from the recorded input rather than assuming it is spurious.
- **A tool error.** The generated check did not compile or did not run. Nothing is known
  about the promise. Never report this as a failing promise, and never as a passing one.

Leave the generated test in place after the fix. It stays as a regression test, and it is
the one artifact that proves the repair addressed the actual case.

## Decode any code Ply prints

```bash
cargo ply explain <CODE>
```

Every diagnostic ends in a short code. This says what it means, who reported it — the
prover, the sampler, or Ply itself — and whether a run carrying it passed. It also says
when a code is described but not emitted by this build, which must never be reported as a
check that ran.

## Result policy

| scenario | completion | next action | visual publication |
| --- | --- | --- | --- |
| clean | may-complete | report the earned evidence | only-by-explicit-flag |
| violation | must-not-complete | repair implementation or ask if intent must change | only-by-explicit-flag |
| missing_evidence | must-not-complete | restore evidence or explain the unresolved gap | only-by-explicit-flag |
| narrowed_evidence | must-not-complete | remove the narrowing or explain the unresolved gap | only-by-explicit-flag |
| timeout | must-not-complete | diagnose the check or rerun it with an explicit time budget | only-by-explicit-flag |
| internal_tool_error | must-not-complete | report the tool failure and preserve its output | only-by-explicit-flag |

For every `must-not-complete` result, say that verification remains unresolved. A published failure remains a useful review artifact, but it is not approval to finish.

## Change authority

The table states defaults when the task has not already authorized the change. Honor
existing user authorization; do not ask again for work already approved. A request to
review alone does not authorize changing requirements.

| target | authority |
| --- | --- |
| implementation | may-edit |
| contract | ask-first |
| declared_check | ask-first |
| evidence_requirement | ask-first |
| architecture_contract | ask-first |

Ask the developer before any change that makes the specification, a declared check, required evidence, or an architecture rule weaker or different. Include the failing evidence and the smallest proposed intent change. Do not make the change until approved.
