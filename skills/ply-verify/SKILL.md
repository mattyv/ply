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
