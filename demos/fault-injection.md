# Fault injection — watch the suite catch seeded bugs

Three deliberate bugs live on the branch [`checkpoint/fault-injection`](https://github.com/mattyv/ply/pull/new/checkpoint/fault-injection),
one commit ahead of the fixed state. Check it out, run the suite, watch each get
caught, then diff against this branch to see the fixes:

```
git checkout checkpoint/fault-injection
cd tools && cargo test --release --no-fail-fast
```

## Fault 1 — worst-of flipped to best-of (kernel)

`combine_claimable` takes `.max()` instead of `.min()`: aggregation reports the
*strongest* child instead of the weakest — the exact lie D6 exists to prevent.

Caught twice, in under two seconds:

```
a_violated_child_drags_a_proved_root_down_to_violation
  assertion `left == right` failed: a violation anywhere must reach the root
    left: Tested
   right: Violation
```

and by the 991,389-tree enumeration proof, which prints the smallest tree that
disagrees with the independent oracle:

```
evidence mismatch: expected worst-of-claimable Violation, got Unclaimed, for subtree
VerdictNode { kind: Claimable(Unclaimed), children: [
    VerdictNode { kind: Claimable(Violation), children: [] } ] }
```

## Fault 2 — conditional assumptions silently dropped (kernel)

`aggregate_raw` stops merging a child's `conditional` upward: evidence that rests
on an unproved assumption sheds the assumption on its way to the root.

```
conditional_on_a_child_propagates_with_its_assumptions
  assertion `left == right` failed
    left: None
   right: Some(["parser::parse fuzzed(256)"])
```

This is standing obligation 2 (CLAUDE.md, SPEC.md §7): `conditional` never
disappears without its assumptions being discharged.

## Fault 3 — the validator accepts `bounded(0)` (model)

`parse_check`'s range loosened from `1..=64` to `0..=64`. A claim of "proved to
loop depth zero" — a check that verifies nothing — now parses as legitimate.

```
bad_check_syntax_is_e0203
  expected E0203 naming the out-of-range check string, got: []
```

### What the diagram does with fault 3 — and why that's a finding

[`fault3.ply.yaml`](fault3.ply.yaml) is vetting 002 with `decode`'s check set to
`bounded(0)`.
[`fault3-as-drawn-by-faulted-toolchain.svg`](fault3-as-drawn-by-faulted-toolchain.svg)
is what the *faulted* toolchain made of it: **`B0`** in the same confident green
as every honest claim. The picture cannot distinguish a vacuous claim from a
proved one.

That is the standing gap this demo makes visible: diagrams draw *declarations*,
and no visual layer exists yet for *findings or verdicts*. Two consequences,
both queued:

- the renderer should run the document-local rules and flag offending items in
  the drawing (red-marked chip, diagnostic code in the tooltip), so an invalid
  document cannot render respectable;
- when `cargo ply` exists, verdict state (violation / unclaimed / conditional)
  needs a §7.1 visual form — the kernel already computes the truthful number,
  and the diagram is where a viewer should meet it.
