#!/usr/bin/env bash
#
# Reproduces every run recorded in vetting/004-legacy-extension.md.
#
#   ./run.sh              # every stage, in order (Kani-heavy: ~35 min)
#   ./run.sh s2 s3        # named stages only
#
# Each stage works on a *fresh copy* of the scenario under $PLY_004_OUT
# (default /tmp/ply-004), never on the checked-in source: `cargo ply verify`
# writes generated modules into the crate's `src/` and appends its harness
# crate to `Cargo.toml`, so a run in place would leave the scenario dirty.
# The copy also rewrites the `ply-attrs` path dependency to an absolute path,
# the same way tests/e2e/src/lib.rs does for the fixtures.
#
# Every stage tees its own output to $PLY_004_OUT/<stage>.txt; the write-up
# quotes those files verbatim.

set -uo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
SCEN="$REPO/vetting/004-legacy-extension"
OUT=${PLY_004_OUT:-/tmp/ply-004}
mkdir -p "$OUT"

CARGO_PLY="$REPO/target/release/cargo-ply"
PLY_CHECK="$REPO/tools/target/release/ply-check"
PLY_RENDER="$REPO/tools/target/release/ply-render"

build_tools() {
  ( cd "$REPO" && cargo build --release -p ply-cli ) || exit 2
  ( cd "$REPO/tools" && cargo build --release -p ply-check -p ply-render ) || exit 2
}

# fresh <name> — a pristine copy of both crates in $OUT/<name>, printed on stdout
fresh() {
  local w="$OUT/$1"
  rm -rf "$w"; mkdir -p "$w"
  cp -r "$SCEN/legacy" "$SCEN/feature" "$w"/
  rm -rf "$w"/legacy/target "$w"/feature/target "$w"/legacy/Cargo.lock "$w"/feature/Cargo.lock
  sed -i "s|path = \"../../../crates/ply-attrs\"|path = \"$REPO/crates/ply-attrs\"|" \
     "$w/feature/Cargo.toml"
  echo "$w"
}

# The one-line fix for the u32 overflow stage s1 finds: widen the product.
apply_overflow_fix() {
  local lib="$1/feature/src/lib.rs"
  perl -0pi -e 's{\n    amount_cents \* bps / 10_000\n}{\n    \(\(amount_cents as u64 \* bps as u64\) / 10_000\) as u32\n}' "$lib"
  grep -q 'as u64' "$lib" || { echo "FIX DID NOT APPLY"; exit 2; }
}

# Keep only the named fn claims in the copy's ply.yaml (crude but exact: the
# claims are one two-line block each, in the order written).
keep_only_fn() {
  local yaml="$1/feature/ply.yaml" keep="$2"
  python3 - "$yaml" "$keep" <<'PY'
import re, sys
path, keep = sys.argv[1], sys.argv[2].split(",")
text = open(path).read()
head, fns = text.split("    fns:\n", 1)
fns, tail = fns.split("\nedges:", 1)
blocks, cur = [], []
for line in fns.splitlines(True):
    if re.match(r"      \w+:\s*$", line) and cur:
        blocks.append(cur); cur = [line]
    else:
        cur.append(line)
blocks.append(cur)
kept = [b for b in blocks if re.match(r"      (\w+):", b[0]).group(1) in keep]
open(path, "w").write(head + "    fns:\n" + "".join("".join(b) for b in kept) + "\nedges:" + tail)
PY
}

s0() { # document-local gate: ply-check + the committed SVG
  echo "=== s0: ply-check + ply-render on the scenario's ply.yaml ==="
  "$PLY_CHECK" "$SCEN/feature/ply.yaml"; echo "ply-check exit: $?"
  "$PLY_RENDER" "$SCEN/feature/ply.yaml" -o "$SCEN/../004-legacy-extension.svg"
  echo "ply-render exit: $? -> vetting/004-legacy-extension.svg"
}

s1() { # the scenario as written: natural bodies, all four claims
  echo "=== s1: verify, as written (natural u32 fee arithmetic) ==="
  local w; w=$(fresh s1)
  ( cd "$w/feature" && time "$CARGO_PLY" verify . --engine-timeout 120 --json )
  echo "verify exit: $?"
}

s2() { # after the one-line overflow fix s1 earns
  echo "=== s2: verify, after widening the fee product to u64 ==="
  local w; w=$(fresh s2); apply_overflow_fix "$w"
  ( cd "$w/feature" && time "$CARGO_PLY" verify . --engine-timeout 120 --json )
  echo "verify exit: $?"
}

s3() { # the boundary, isolated: the same fn with the legacy call in place
  echo "=== s3: the boundary fn alone, 600s budget ==="
  local w; w=$(fresh s3); apply_overflow_fix "$w"; keep_only_fn "$w" tier_fee_cents
  ( cd "$w/feature" && time "$CARGO_PLY" verify . --engine-timeout 600 --json )
  echo "verify exit: $?"
}

s4() { # the control: identical fn, legacy call replaced by an in-fragment match
  echo "=== s4: control — same fn, same contract, no call across the boundary ==="
  local w; w=$(fresh s4); apply_overflow_fix "$w"; keep_only_fn "$w" tier_fee_cents
  perl -0pi -e 's{    let bps = ledger::fees::bps_for_tier\(tier\)\.min\(10_000\);}{    let bps = match tier \{ 0 => 150, 1 => 90, 2 => 45, 3 => 0, _ => 150 \};}' \
     "$w/feature/src/lib.rs"
  grep -q 'match tier' "$w/feature/src/lib.rs" || { echo "CONTROL EDIT DID NOT APPLY"; exit 2; }
  ( cd "$w/feature" && time "$CARGO_PLY" verify . --engine-timeout 600 --json )
  echo "verify exit: $?"
}

s5() { # §5.4's ply.yaml-declared contracts, on the unclaimed legacy callee
  # Budget raised from 120s to 600s on 2026-08-25, when D5's second branch
  # started assuming this contract instead of dropping it: the stubbed proof
  # needs ~202s of Kani verification time (measured, docs/post-004-fixes.md),
  # so at 120s the stage reported `timeout` and said nothing about the
  # assumption. The original 120s run is quoted in that document.
  echo "=== s5: a requires/ensures declared for the legacy callee in ply.yaml ==="
  local w; w=$(fresh s5); apply_overflow_fix "$w"; keep_only_fn "$w" tier_fee_cents
  python3 - "$w/feature/ply.yaml" <<'PY'
import sys
p = sys.argv[1]; t = open(p).read()
t = t.replace("""  ledger:
    anchor: ledger
""", """  ledger:
    anchor: ledger
    fns:
      fees::bps_for_tier:
        checks: []
        ensures:
          - "|result| *result <= 10_000"
""")
open(p, "w").write(t)
PY
  sed -n '/ledger:/,/^$/p' "$w/feature/ply.yaml"
  ( cd "$w/feature" && time "$CARGO_PLY" verify . --engine-timeout 600 --json )
  echo "verify exit: $?"
}

s6() { # §6 surface: does `check` exist, does `--only-changed` exist
  echo "=== s6: CLI surface — cargo ply check, --only-changed ==="
  local w; w=$(fresh s6)
  echo "--- cargo-ply --help"
  "$CARGO_PLY" --help 2>&1
  echo "--- cargo-ply check ."
  ( cd "$w/feature" && "$CARGO_PLY" check . 2>&1 ); echo "exit: $?"
  echo "--- cargo-ply verify . --only-changed"
  ( cd "$w/feature" && "$CARGO_PLY" verify . --only-changed 2>&1 ); echo "exit: $?"
}

s7() { # §5.4b's *preferred* bounded shape: a fixed-size array parameter
  echo "=== s7: a fixed-size array parameter — §5.4b's preferred bounded shape ==="
  local w; w=$(fresh s7); apply_overflow_fix "$w"
  cat >> "$w/feature/src/lib.rs" <<'RS'

/// The fragment-first way to keep the legacy lookup out of the proof entirely:
/// take the whole rate card as data instead of looking it up. §5.4b calls a
/// fixed-size array "v1's **preferred** bounded shape".
#[ply::requires(amount_cents <= 100_000_000 && tier < 4)]
#[ply::ensures(|result| *result <= amount_cents)]
pub fn carded_fee_cents(amount_cents: u32, tier: u8, card_bps: [u32; 4]) -> u32 {
    let bps = card_bps[tier as usize];
    fee_cents(amount_cents, if bps > 10_000 { 10_000 } else { bps })
}
RS
  python3 - "$w/feature/ply.yaml" <<'PY'
import sys
p = sys.argv[1]; t = open(p).read()
head, rest = t.split("    fns:\n", 1)
_, tail = rest.split("\nedges:", 1)
open(p, "w").write(head + "    fns:\n      carded_fee_cents:\n        checks: [bounded(2)]\n\nedges:" + tail)
PY
  ( cd "$w/feature" && time "$CARGO_PLY" verify . --engine-timeout 120 --json )
  echo "verify exit: $?"
}

s8() { # is a fuzz verdict reproducible? same code, three fresh runs
  echo "=== s8: the same fuzz check on the same (unfixed) code, six fresh runs ==="
  for i in 1 2 3 4 5 6; do
    local w; w=$(fresh "s8_$i"); keep_only_fn "$w" approve_withdrawal
    ( cd "$w/feature" && "$CARGO_PLY" verify . --engine-timeout 120 --json ) | python3 -c "
import json, sys
j = json.load(sys.stdin)
fns = [(c['id'], c['verdict']) for c in j['root']['children'][0]['children']]
print('run $i:', j['root']['verdict'], fns, [d['code'] for d in j['diagnostics']])
"
  done
}

build_tools
stages=("$@")
[ ${#stages[@]} -eq 0 ] && stages=(s0 s1 s2 s3 s4 s5 s6 s7 s8)
for s in "${stages[@]}"; do
  "$s" 2>&1 | tee "$OUT/$s.txt"
done
