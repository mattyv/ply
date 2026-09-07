---
name: ply-verify
description: Verify early and after meaningful implementation changes, interpret Ply results, repair defects, and optionally publish a completed visual run without weakening declared intent.
---

# Ply Verify

Use Ply's public CLI as the authority. Do not reproduce its verifier, verdict rules, record format, or artifact writer.

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
tracks. Start with `fuzz` for new suitable claims; retain existing evidence requirements,
including bounded checks, when they are part of the task.

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

3. If the check passes, run verification with its default evidence threshold:

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
| internal_tool_error | must-not-complete | report the tool failure and preserve its output | unavailable |

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
