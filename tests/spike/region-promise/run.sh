#!/usr/bin/env bash
#
# The follow-up to tests/spike/havoc: does ONE promise, written once for a whole
# legacy region and applied to every function in it, rescue the six crossings
# havoc failed?
#
#   ./run.sh              # every row in FINDINGS.md, in order (~30-45 min)
#   ./run.sh r5 p5        # named rows only
#
# Nothing here implements `given:` or a region construct. Every harness is
# hand-written in the shape crates/ply-core/src/harness.rs::generate_proof_module
# already emits. The only difference from the havoc spike is the stub body: there
# a bare `kani::any()`, here a `kani::any()` constrained by the one clause
# PROMISES.md pre-registered for that region.
#
# The callers and callees are byte-for-byte the havoc spike's -- `hashes` checks
# it -- so the comparison is like-for-like.
#
# Toolchain: the pin, `cargo-kani 0.67.0` / CBMC 6.8.0. Nothing here installs or
# disturbs it.

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${WORK:-$(mktemp -d)}"
OUT="${OUT:-$WORK/logs}"
mkdir -p "$OUT"

FLAGS=(-Z function-contracts -Z unstable-options -Z concrete-playback
       -Z stubbing --harness-timeout 300s)

# kani <crate-dir> <harness> [label] [extra cargo-kani args...]
kani() {
  local d="$1" h="$2" label="${3:-$2}"; shift 3 2>/dev/null || shift 2
  echo "--- $label  ($h)"
  local t0=$SECONDS
  ( cd "$d" && cargo kani "${FLAGS[@]}" --exact --harness "ply_generated::$h" \
      --concrete-playback print "$@" ; echo "kani exit=$?" ) 2>&1 | tee "$OUT/$label.txt" \
    | grep -E "^VERIFICATION|Verification Time|kani exit=|Failed Checks|failed \(|error(\[|:)|Status: FAILURE|concrete_vals|^ *[a-z_]+ = " \
    | head -40
  echo "wall $((SECONDS - t0))s"
  echo
}

banner() { echo; echo "======================================================"; echo "$*"; echo "======================================================"; }
version() { banner "Toolchain"; cargo kani --version; }

hashes() {
  banner "Hash pins -- the sample is byte-for-byte tests/spike/havoc's"
  sha256sum "$HERE/natural/feature/src/lib.rs" "$HERE/natural/legacy/src/lib.rs"
  echo "expected (pre-registered in havoc/FINDINGS.md and PROMISES.md):"
  echo "c0cf3136d6dcc095de2eb53d3417e31afad6c42764d104e83bf14fb2d25bbf4a  natural/feature/src/lib.rs"
  echo "b109337d248e9c6c9c79cb6ba733fb2d6594f6c23826065970e3e26d99b95bac  natural/legacy/src/lib.rs"
  diff <(cd "$HERE" && sha256sum natural/feature/src/lib.rs natural/legacy/src/lib.rs) \
       <(printf '%s  %s\n%s  %s\n' \
           c0cf3136d6dcc095de2eb53d3417e31afad6c42764d104e83bf14fb2d25bbf4a natural/feature/src/lib.rs \
           b109337d248e9c6c9c79cb6ba733fb2d6594f6c23826065970e3e26d99b95bac natural/legacy/src/lib.rs) \
    && echo "HASHES MATCH" || echo "HASHES DIFFER -- the sample was edited, every row below is void"
  echo
  echo "and against the havoc spike's own copies, including given004 and the manifests:"
  diff -r --exclude=ply_generated.rs --exclude=target --exclude=Cargo.lock \
       "$HERE/../havoc/given004" "$HERE/given004" \
    && echo "given004 IDENTICAL" || echo "given004 DIFFERS"
  diff -r --exclude=ply_generated.rs --exclude=target --exclude=Cargo.lock \
       "$HERE/../havoc/natural" "$HERE/natural" \
    && echo "natural IDENTICAL" \
    || echo "^ expected: natural/feature/Cargo.toml gains the pre-registered [features] tight stanza; nothing else may differ"
}

# fresh004 <name> -> clean copy of both given004 crates, path printed
fresh004() {
  local w="$WORK/$1"; rm -rf "$w"; mkdir -p "$w"
  cp -r "$HERE/given004/legacy" "$HERE/given004/feature" "$w"/
  rm -rf "$w"/legacy/target "$w"/feature/target
  echo "$w"
}
freshnat() {
  local w="$WORK/$1"; rm -rf "$w"; mkdir -p "$w"
  cp -r "$HERE/natural/legacy" "$HERE/natural/feature" "$w"/
  rm -rf "$w"/legacy/target "$w"/feature/target
  echo "$w"
}

# ================================================ Region A -- ledger::fees (A1)
r1() { banner "r1 -- tier_fee_cents under region promise A1 (havoc: SUCCESSFUL)"
       kani "$HERE/given004/feature" ply_region_tier_fee_cents r1; }
r2() { banner "r2 -- approve_withdrawal, the transitive crossing, under A1 (havoc: SUCCESSFUL)"
       kani "$HERE/given004/feature" ply_region_approve_withdrawal r2; }

# ==================================================== Region B -- catalog (B1)
r3() { banner "r3 -- gross_cents under B1. Run on a scratch copy carrying the u32
       overflow its OWN BASELINE reported (havoc n1 -> n1f); the committed sample
       stays byte-for-byte as pre-registered."
       local w; w=$(freshnat r3)
       python3 "$HERE/widen_n1.py" "$w/feature/src/lib.rs" || return 2
       kani "$w/feature" ply_base_gross_cents r3-base
       kani "$w/feature" ply_region_gross_cents r3
       kani "$w/feature" ply_percallee_gross_cents p3; }
r4() { banner "r4 -- line_total_cents under B1 (havoc: FAILED, unit_price_cents = 2_813_465)"
       kani "$HERE/natural/feature" ply_region_line_total_cents r4; }
r5() { banner "r5 -- top_band_price_cents under B1 (havoc: FAILED, band_count = 0)"
       kani "$HERE/natural/feature" ply_region_top_band_price_cents r5; }
r6() { banner "r6 -- batches_needed under B1 (havoc: FAILED, batch_size = 4_294_574_072)"
       kani "$HERE/natural/feature" ply_region_batches_needed r6; }
r7() { banner "r7 -- manifest_weight_grams under B1 (havoc: TIMEOUT@300s, no witness)"
       kani "$HERE/natural/feature" ply_region_manifest_weight_grams r7; }
r8() { banner "r8 -- remaining_limit_cents under B1 (havoc: FAILED, spend_limit_cents = 183_558_144)"
       kani "$HERE/natural/feature" ply_region_remaining_limit_cents r8; }

# ============================== Region B at the pre-registered tightest ceiling
tight() {
  banner "t4..t8 -- the SAME region promise at B1-tight (500_000), the tightest
       ceiling still true of catalog. Pre-registered in PROMISES.md before any
       run, so this answers 'you picked a loose bound' rather than reacting to it."
  for h in line_total_cents top_band_price_cents batches_needed \
           manifest_weight_grams remaining_limit_cents; do
    kani "$HERE/natural/feature" "ply_region_$h" "t-$h" --features tight
  done
}

# ======================================= the per-callee column (spec 5.5 br. 2)
percallee() {
  banner "p4..p8 -- the route that already works: one clause per callee, from
       PROMISES.md's pre-registered table. Four of them carry a lower bound the
       region promise cannot."
  for h in line_total_cents top_band_price_cents batches_needed \
           manifest_weight_grams remaining_limit_cents; do
    kani "$HERE/natural/feature" "ply_percallee_$h" "p-$h"
  done
}

# ==================================================================== mutations
# x1 -- mutation on a Region B pass: delete the region promise's kani::assume,
# leaving a bare kani::any(). That is exactly havoc. If the row still passes, the
# stub was never applied and its green means nothing. MUT_FN names the row; it is
# set to whichever Region B crossing passed under B1.
x1() {
  local fn="${MUT_FN:-remaining_limit_cents}"
  banner "x1 -- MUTATION ON A PASS: strip the region promise out of $fn's stub,
       leaving the empty contract. Must flip to the havoc verdict."
  local w; w=$(freshnat x1)
  python3 - "$w/feature/src/ply_generated.rs" <<'PY'
import sys, re
p = sys.argv[1]; t = open(p).read()
old = "    kani::assume(catalog_promise!(result));\n"
assert t.count(old) == 6, f"MUTATION DID NOT APPLY: found {t.count(old)}"
open(p, "w").write(t.replace(old, ""))
PY
  [ $? -eq 0 ] || return 2
  kani "$w/feature" "ply_region_$fn" "x1-$fn"
}

# x2/x3 -- the pair that proves Region A's promise is applied AND load-bearing.
# Havoc's m1 deleted tier_fee_cents's own .min(10_000) clamp and FAILED. Under
# A1 the promise supplies exactly what the clamp did, so x2 must PASS where m1
# failed -- a flip in the opposite direction is just as probative -- and x3, the
# same mutated caller with the promise removed again, must fail like m1.
x2() {
  banner "x2 -- delete tier_fee_cents's .min(10_000), KEEP region promise A1.
       Havoc's m1 on the same mutation FAILED. If A1 is applied, this passes."
  local w; w=$(fresh004 x2)
  sed -i 's/ledger::fees::bps_for_tier(tier)\.min(10_000)/ledger::fees::bps_for_tier(tier)/' \
      "$w/feature/src/lib.rs"
  grep -q 'bps_for_tier(tier);' "$w/feature/src/lib.rs" || { echo "MUTATION DID NOT APPLY"; return 2; }
  kani "$w/feature" ply_region_tier_fee_cents x2
}
x3() {
  banner "x3 -- the same mutated caller with the promise removed (the empty
       contract). This is havoc's m1, re-run here as the control for x2."
  local w; w=$(fresh004 x3)
  sed -i 's/ledger::fees::bps_for_tier(tier)\.min(10_000)/ledger::fees::bps_for_tier(tier)/' \
      "$w/feature/src/lib.rs"
  grep -q 'bps_for_tier(tier);' "$w/feature/src/lib.rs" || { echo "MUTATION DID NOT APPLY"; return 2; }
  kani "$w/feature" ply_havoc_tier_fee_cents x3
}

version
hashes
rows=("$@")
[ ${#rows[@]} -eq 0 ] && rows=(r1 r2 r3 r4 r5 r6 r7 r8 tight percallee x1 x2 x3)
for r in "${rows[@]}"; do "$r"; done
echo
echo "Done. Logs in $OUT. See FINDINGS.md."
