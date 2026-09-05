---
name: ply-verify
description: Run and interpret Ply verification for an implementation change, repair implementation defects, and optionally publish a completed visual run without weakening declared intent.
---

# Ply Verify

Use Ply's public CLI as the authority. Do not reproduce its verifier, verdict rules, record format, or artifact writer.

## Workflow

1. Find every affected crate by walking from changed implementation files to the nearest ancestor that contains `ply.yaml`. Treat each such directory as a separate Ply root. If scope is ambiguous, state the roots you selected before running anything.
2. Run the fast public check for each root:

```bash
cargo ply check path/to/crate --json
```

3. If the check passes, run verification with its default evidence threshold:

```bash
cargo ply verify path/to/crate --json
```

Do not add `--fail-on error` to turn missing evidence into success. Use `--engine-timeout` only to give the same declared checks more time. Use `--seed` only with the 64-character seed emitted by a prior public JSON result.

4. Read the command's public JSON and exit status together. Report what passed, what failed, what evidence is absent or narrowed, and any concrete counterexample or repair offered by the diagnostics. Never infer success from a partial tree or from the absence of an error message.
5. Repair implementation code when the declared intent is clear, then rerun the same root. Stop at the approval boundary below instead of editing the goal to fit the code.
6. Publish a visual run only when the user asks for one or the task explicitly requires a visual client artifact. Publication must occur through the same verification command:

```bash
cargo ply verify path/to/crate --json --publish-view
```

`--publish-view` records the completed outcome; it does not turn that outcome into success. Do not construct or edit `target/ply/view.json`, a `visual.json`, or `ply.lock` yourself.

## Repair a broken promise

A violation comes with a generated `#[test]` at `src/ply_generated_cex.rs`, holding the
exact input that broke the promise. That file is the repair loop:

```bash
cargo test        # from the crate root -- it fails the same way the run just did
```

It is ordinary Rust and needs no engine, so iterate against it directly and only re-run
`cargo ply verify` once it passes.

Read the panic before changing anything. It prints what the promise's left and right sides
each evaluated to for that input, which usually says immediately whether the body is wrong
or the promise is.

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

| target | authority |
| --- | --- |
| implementation | may-edit |
| contract | ask-first |
| declared_check | ask-first |
| evidence_requirement | ask-first |
| architecture_contract | ask-first |

Ask the developer before any change that makes the specification, a declared check, required evidence, or an architecture rule weaker or different. Include the failing evidence and the smallest proposed intent change. Do not make the change until approved.
