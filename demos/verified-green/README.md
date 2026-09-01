# A render with real evidence in it

Every other drawing committed to this repository is a *before* picture: it shows
what a `ply.yaml` promises, drawn without running anything. Until this demo there
was no committed artifact showing the other half — what a drawing looks like once
checks have actually run and earned something. The style rule for an earned chip
existed in every rendered file and was used by none of them.

`verified.svg` is that picture. Three functions, all green, and a header line
reading **3 earned**.

## What earned what, and why it is worth looking at

| function | verdict | what that means |
|---|---|---|
| `clamp` | `bounded(2)` | a real proof, from Kani — checked over every value, not sampled |
| `digit_count` | `fuzzed(64)` | 64 generated strings, each one run |
| `total` | `fuzzed(64)` | 64 generated integer pairs |

`digit_count` takes a `&str`. Until 2026-09-01 Ply refused any function with a text
parameter outright, which was measured as the single largest reason it could not
check a real library. It is in this demo deliberately.

## The run says two honest things about itself

Neither is a defect, and both should stay visible:

- **`clamp` threw away most of its inputs.** Its precondition requires `lo <= hi`,
  and 69 of 133 random draws did not satisfy that. proptest kept drawing until it
  had 64 accepted cases, so the count is honest — but those cases all come from
  the corner of the input space the precondition allows, which is weaker evidence
  than 64 sounds.
- **`digit_count` was never given control characters.** Ply excludes raw bytes like
  a null or an escape code by default, on the grounds that they are more likely to
  trip something unrelated than to find a real bug. Accented and CJK text *is*
  generated. If the function needs to handle control characters, this run says
  nothing about that.

## Regenerating it

```
cargo run -p ply-cli -- ply verify demos/verified-green --publish-view
```

The drawing is written under `demos/verified-green/target/ply/views/<run>/visual.json`,
inside the envelope's `svg` field. `verified.svg` here is that field, extracted.
