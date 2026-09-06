---
name: ply-review
description: Review a completed Ply visual run, explain its declared structure and earned evidence, and navigate its diagnostics to exact source without reimplementing Ply semantics.
---

# Ply Review

Review Ply's immutable public artifact. Treat its outcome and evidence fields as authoritative; do not derive a new verdict from SVG marks, diagnostic severity, or private records.

## When there is no run to review

This skill reviews a **completed run**. If no run has been published — or the question is
about what the document declares rather than what a run found — do not reach for the SVG.
Read the text form instead:

```bash
cargo ply render path/to/crate --text
```

A screenshot omits tooltip text. Use the text form to read YAML declarations and their
meaning; it does not include Rust contract attributes. For a completed run, use the
public envelope for those contracts and evidence. Say plainly that a declaration render
is declared intent and not evidence. When the user requests a visual inspection, inspect
the drawing too; text alone cannot establish that the layout is readable.

## Select the run

1. Identify the relevant directory containing `ply.yaml`.
2. Read `target/ply/view.json`. Require `protocolVersion` 1, then select the entry named by `currentRun` unless the user chose another indexed run.
3. Require the selected entry's path to be exactly `views/<run-id>/visual.json`, relative to `target/ply`, with the same ID as the entry. Reject absolute paths, traversal, unknown protocol versions, and mismatched IDs.
4. Read that `visual.json` without modifying it. Confirm its `run.id` matches the selected entry and report `run.completedAt`, `run.root.path`, `run.tool`, and `run.outcome` so the developer knows exactly what was reviewed.

If the index or envelope is incomplete or invalid, keep the last valid result if the host provides one and report the new artifact error. Label that fallback with its original run and root; it is not the requested new result. Do not guess at missing fields or silently switch runs.

A completed snapshot does not establish that the current source still matches it. When asked whether current code passes, hand off to `$ply-verify` if verification is within the task; otherwise state that only the saved run was reviewed. A source location is the range recorded for that run, and edits may have moved it.

## Review the evidence

Use `svg` for the picture and `elements` for its semantics. For each relevant element, explain its label, kind, declared relationship, `evidence.verdict`, statuses, reuse state, engine, seed, and cases when present. Join `diagnosticIds` to the top-level `diagnostics` array by stable ID. Explain each diagnostic's message and source; do not invent assumptions or repairs that are absent from the artifact.

Distinguish a clean outcome from an incomplete one in the first sentence. A failure can still be a valid completed snapshot.

## Outcome review

| outcome | handling | review emphasis |
| --- | --- | --- |
| clean | report-honestly | earned evidence, reuse, and any attached diagnostics |
| violation | report-honestly | failing element, diagnostic message, and exact source |
| missing_evidence | report-honestly | elements without required evidence and the stated reason |
| narrowed_evidence | report-honestly | which exploration was narrowed and why the result is not complete |
| timeout | report-honestly | timed-out element, engine, and the unresolved evidence |

## Source navigation

| item | envelope field |
| --- | --- |
| path | source.file |
| start | source.startLine:source.startColumn |
| end | source.endLine:source.endColumn |
| coordinate_base | zero-based |

Resolve `source.file` beneath the selected run's Ply root, rejecting absolute paths, traversal, and links that escape that root. Convert the zero-based coordinates to one-based lines when presenting editor links. Use the recorded range, but do not claim it still identifies the same code if the file has changed. If `source` is absent, say that the artifact supplies no source link; do not search for a likely match and present it as exact.

## Data boundary

| resource | access |
| --- | --- |
| view_index | read |
| visual_envelope | read |
| ply_lock | forbidden |
| internal_serializer | forbidden |
| client_side_verdict_classifier | forbidden |

Do not read `ply.lock`, import internal artifact serializers, rewrite retention state, mutate a published run, or add IDE-specific behavior. To obtain a new snapshot, hand off to `$ply-verify` and use the public `--publish-view` workflow.

## Change authority

The table states defaults when the task has not already authorized the change. Honor
existing user authorization; do not ask again for work already approved. A request to
review alone does not authorize changing requirements.

| target | authority |
| --- | --- |
| contract | ask-first |
| declared_check | ask-first |
| evidence_requirement | ask-first |
| architecture_contract | ask-first |

Recommendations need no approval: present the evidence and a concrete proposal. Applying a change to these protected targets needs developer authorization. Review explains what Ply recorded; it does not redefine what Ply should require.
