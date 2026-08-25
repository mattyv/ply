#!/usr/bin/env bash
#
# The gate experiment for docs/plans/trusted-boundary.md: does a real caller
# still verify when the callee it crosses to is replaced by an unconstrained
# symbolic return?
#
#   ./run.sh            # every row in FINDINGS.md, in order (~25-40 min)
#   ./run.sh n1 g1      # named rows only
#
# Nothing here implements `given:`. Every harness is hand-written in the shape
# crates/ply-core/src/harness.rs::generate_proof_module already emits, with one
# difference: the stub body is a bare `kani::any()` with no `kani::assume`.
# That is the empty contract -- what a `given:` region can honestly mean at a
# `bounded` crossing (proposal §2, option 3).
#
# Toolchain: the pin, `cargo-kani 0.67.0` / CBMC 6.8.0, whatever `cargo kani`
# resolves to on PATH. Nothing here installs or disturbs it.
#
# Mutation rows (`m1`, `m2`) edit source, so they work on a scratch copy under
# $WORK and leave the committed fixtures pristine.

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${WORK:-$(mktemp -d)}"
OUT="${OUT:-$WORK/logs}"
mkdir -p "$OUT"

FLAGS=(-Z function-contracts -Z unstable-options -Z concrete-playback
       -Z stubbing --harness-timeout 300s)

# kani <crate-dir> <harness> [extra cargo-kani args...]
kani() {
  local d="$1" h="$2"; shift 2
  echo "--- $h  ($d)"
  local t0=$SECONDS
  ( cd "$d" && cargo kani "${FLAGS[@]}" --exact --harness "ply_generated::$h" \
      --concrete-playback print "$@" ; echo "kani exit=$?" ) 2>&1 | tee "$OUT/$h.txt" \
    | grep -E "^VERIFICATION|Verification Time|kani exit=|Failed Checks|failed \(|error(\[|:)|Status: FAILURE|concrete_vals|^ *[a-z_]+ = " \
    | head -40
  echo "wall $((SECONDS - t0))s"
  echo
}

# fresh <name> -> a clean copy of both given004 crates, path printed
fresh() {
  local w="$WORK/$1"
  rm -rf "$w"; mkdir -p "$w"
  cp -r "$HERE/given004/legacy" "$HERE/given004/feature" "$w"/
  rm -rf "$w"/legacy/target "$w"/feature/target
  echo "$w"
}

banner() { echo; echo "======================================================"; echo "$*"; echo "======================================================"; }

version() { banner "Toolchain"; cargo kani --version; }

# ---------------------------------------------------------------- 004's crossings
g1() { banner "g1 -- 004's flagship crossing under havoc (the prediction on record)"
       kani "$HERE/given004/feature" ply_proof_tier_fee_cents_havoc; }

g2() { banner "g2 -- the same claim with NO stub: Kani descends into the real
       BTreeMap-behind-OnceLock lookup. 004 s3 measured timeout at 120s and 600s."
       kani "$HERE/given004/feature" ply_proof_tier_fee_cents_baseline; }

g3() { banner "g3 -- the cost baseline: the SAME stub with the declared contract's
       one kani::assume put back (004 s5's shape, measured 201.77s end-to-end)"
       kani "$HERE/given004/feature" ply_proof_tier_fee_cents_contract; }

g4() { banner "g4 -- the transitive crossing: approve_withdrawal never names ledger"
       kani "$HERE/given004/feature" ply_proof_approve_withdrawal_havoc; }

g5() { banner "g5 -- withdraw: the refusal row, as an observed compile error"
       kani "$HERE/given004/feature" ply_havoc_withdraw --features withdraw_row; }

# --------------------------------------------------- the naturally-written callers
# Each is run twice: ply_base_* (no stub, real callee) and ply_havoc_* (empty
# contract). The baseline is what makes the havoc row attributable.
for i in gross_cents line_total_cents top_band_price_cents \
         batches_needed manifest_weight_grams remaining_limit_cents; do
  eval "n_$i() { banner \"n -- $i\"
      kani \"\$HERE/natural/feature\" ply_base_$i
      kani \"\$HERE/natural/feature\" ply_havoc_$i; }"
done
n1() { n_gross_cents; }
n2() { n_line_total_cents; }
n3() { n_top_band_price_cents; }
n4() { n_batches_needed; }
n5() { n_manifest_weight_grams; }
n6() { n_remaining_limit_cents; }

n1f() {
  banner "n1f -- N1 again, with the u32 overflow its OWN BASELINE reported fixed.
       The n1 baseline fails on 'attempt to multiply with overflow' against the
       real callee, so the n1 havoc row measures nothing about havoc. The fix is
       004's own s1->s2 one-liner, widen the product; it is applied to a scratch
       copy so the committed sample stays byte-for-byte as pre-registered."
  local w="$WORK/n1f"
  rm -rf "$w"; mkdir -p "$w"
  cp -r "$HERE/natural/legacy" "$HERE/natural/feature" "$w"/
  rm -rf "$w"/legacy/target "$w"/feature/target
  python3 "$HERE/widen_n1.py" "$w/feature/src/lib.rs" || return 2
  kani "$w/feature" ply_base_gross_cents
  kani "$w/feature" ply_havoc_gross_cents
}

# ------------------------------------------------------------------- mutations
m1() {
  banner "m1 -- MUTATION on a PASS: delete tier_fee_cents's .min(10_000) clamp.
       If the havoc row (g1) passed because the stub was never applied, this
       still passes. It must not."
  local w; w=$(fresh m1)
  sed -i 's/ledger::fees::bps_for_tier(tier)\.min(10_000)/ledger::fees::bps_for_tier(tier)/' \
      "$w/feature/src/lib.rs"
  grep -q 'bps_for_tier(tier);' "$w/feature/src/lib.rs" \
    || { echo "MUTATION DID NOT APPLY"; return 2; }
  kani "$w/feature" ply_proof_tier_fee_cents_havoc
}

m2() {
  banner "m2 -- a sensitivity check, NOT a mutation on a pass. It was written as
       one, on the guess that n6 would pass under havoc; n6 does not, so the
       premise is void and the row is reported as what it is: tightening
       remaining_limit_cents's ensures ceiling from 100_000_000 to the real
       table's 500_000 moves the witness and leaves the verdict FAILED."
  local w="$WORK/m2"
  rm -rf "$w"; mkdir -p "$w"
  cp -r "$HERE/natural/legacy" "$HERE/natural/feature" "$w"/
  rm -rf "$w"/legacy/target "$w"/feature/target
  python3 - "$w/feature/src/lib.rs" <<'PY'
import sys
p = sys.argv[1]; t = open(p).read()
old = "#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000))]\n#[cfg_attr(kani, kani::ensures(|result| *result <= 100_000_000))]\npub fn remaining_limit_cents"
new = "#[cfg_attr(kani, kani::requires(amount_cents <= 100_000_000))]\n#[cfg_attr(kani, kani::ensures(|result| *result <= 500_000))]\npub fn remaining_limit_cents"
assert old in t, "MUTATION DID NOT APPLY"
open(p, "w").write(t.replace(old, new))
PY
  [ $? -eq 0 ] || return 2
  kani "$w/feature" ply_havoc_remaining_limit_cents
}

m3() {
  banner "m3 -- MUTATION on the OTHER pass: the same deleted .min(10_000), this
       time under the transitive crossing (g4). approve_withdrawal calls
       tier_fee_cents, which is what actually crosses; if g4's pass were an
       artefact of the stub not being applied, this would still pass."
  local w; w=$(fresh m3)
  sed -i 's/ledger::fees::bps_for_tier(tier)\.min(10_000)/ledger::fees::bps_for_tier(tier)/' \
      "$w/feature/src/lib.rs"
  grep -q 'bps_for_tier(tier);' "$w/feature/src/lib.rs" \
    || { echo "MUTATION DID NOT APPLY"; return 2; }
  kani "$w/feature" ply_proof_approve_withdrawal_havoc
}

version
rows=("$@")
[ ${#rows[@]} -eq 0 ] && rows=(g1 g2 g3 g4 g5 n1 n1f n2 n3 n4 n5 n6 m1 m2 m3)
for r in "${rows[@]}"; do "$r"; done
echo
echo "Done. Logs in $OUT. See FINDINGS.md."
