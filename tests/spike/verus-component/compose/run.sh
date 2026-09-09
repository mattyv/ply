#!/usr/bin/env bash
# Every number the composition addendum in docs/component-proof-design.md
# reports, re-runnable. Until 2026-09-09 these probes existed only in a
# session scratchpad and the doc said "measured" about files nobody else
# could run -- which is the same defect as a green test nothing executes.
#
# Verus is not installed by this script (same stance as the sibling spikes):
# point VERUS at the binary from an unpacked release, or have `verus` on
# PATH. Install steps are recorded in ../../verus/FINDINGS.md.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
V="${VERUS:-verus}"

echo "== 1. contracts entail the invariant (expect: verified, 0 errors) =="
"$V" entail.rs

echo
echo "== 2. weakened refill contract (expect: 1 error, postcondition) =="
"$V" weak_refill.rs || true

echo
echo "== 3. capacity frame fact omitted (expect: 1 error, postcondition) =="
"$V" no_frame.rs || true

echo
echo "== 4. a contradictory premise set (expect: verified -- and worthless) =="
"$V" vacuous.rs

echo
echo "== 5. the arithmetic unsoundness, as a running program =="
echo "   The contract holds and the invariant is false. No Verus needed."
rustc -O -C overflow-checks=off -o /tmp/ply_wrap_cex wrap_counterexample.rs
/tmp/ply_wrap_cex
