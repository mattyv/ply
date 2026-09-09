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

echo
echo "== 6. the adapter's own output, end to end (expect: 4 obligations, 0 errors) =="
echo "   Emitted by crates/ply-core/src/compose/verus.rs; re-bless with"
echo "   PLY_BLESS=1 cargo test -p ply-core --lib compose::verus::golden"
"$V" generated_bucket.rs

echo
echo "== 7. the satisfiability probes (expect: an error for EVERY obligation) =="
echo "   These read backwards. One VERIFYING means that contract contradicts"
echo "   itself, and every other answer about it is worthless."
"$V" generated_bucket_probe.rs || true

echo
echo "== 8. does the range obligation actually bite? (expect: 1 error) =="
echo "   Drops the guard that makes try_take's subtraction safe -- the exact"
echo "   contract the wrapping counterexample in step 5 satisfies."
sed 's|&& (ok == (pre.available >= tokens))|\&\& (true)|' generated_bucket.rs > /tmp/ply_no_guard.rs
"$V" /tmp/ply_no_guard.rs || true

echo
echo "== 9. adding back the premise that used to defeat it (expect: 1 error) =="
echo "   Same broken contract as step 8, plus the range premise about the"
echo "   state after. Until 2026-09-09 this PASSED -- a wrapped value is"
echo "   perfectly in range, so that premise made the set contradictory and"
echo "   every question answered yes, and the whole defence was leaving one"
echo "   line out. Review then found a contract clause that hands the same"
echo "   fact back (step 10), so the obligation is now given only the clauses"
echo "   that say nothing about the state after, and both routes are shut."
python3 - <<'PY'
s = open('/tmp/ply_no_guard.rs').read()
at = s.index('proof fn ob_arith_TokenBucket_try_take')
head, tail = s[:at], s[at:]
tail = tail.replace('        typed(pre),\n', '        typed(pre),\n        typed(post),\n', 1)
open('/tmp/ply_typed_post.rs', 'w').write(head + tail)
PY
"$V" /tmp/ply_typed_post.rs || true

echo
echo "== 10. the route round step 9 (expect: 1 error) =="
echo "   The premise in step 9 is not the only way to learn that the state"
echo "   after is in range. A contract clause restating what the declared"
echo "   type already guarantees does it too -- available >= 0 on a u32 --"
echo "   and a function proof discharges such a clause from the field's own"
echo "   type. On the same broken contract this used to pass."
python3 - <<'PY'
s = open('generated_bucket.rs').read()
s = s.replace('    && (ok == (pre.available >= tokens))\n', '    && (true)\n')
s = s.replace('    && (post.capacity == pre.capacity)\n}',
              '    && (post.capacity == pre.capacity)\n    && (post.available >= 0)\n}', 1)
open('/tmp/ply_restated.rs', 'w').write(s)
PY
"$V" /tmp/ply_restated.rs || true
