#!/usr/bin/env bash
# M0 feasibility spike: re-run every item in tests/spike/FINDINGS.md end to end.
#
# Pinned toolchain: cargo-kani 0.67.0 / CBMC 6.8.0 (as observed via `cargo kani
# --version` on the spike machine; this script does not pin or install Kani,
# it assumes it's already on PATH exactly as the M0 task found it).
#
# Two throwaway crates, neither a member of tools/Cargo.toml's workspace:
#   tests/spike/fixture         -- everything except item 5's callee
#   tests/spike/fixture_callee  -- the cross-crate callee for item 5
#
# Every `cargo kani` call below uses --harness-timeout 300s per the spike
# brief (a harness ran over an hour with no verdict the night before this
# spike; nothing here should ever be allowed to do that again).

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/fixture"

KANI_FLAGS=(-Z function-contracts -Z stubbing -Z concrete-playback -Z unstable-options --harness-timeout 300s)

echo "== cargo kani --version =="
cargo kani --version

echo
echo "== Item 1: private free function, proof_for_contract (+ item 9: in-crate proof module) =="
cargo kani "${KANI_FLAGS[@]}" --exact --harness item1_proofs::check_private_increment

echo
echo "== Item 2: contracted method (&self, uses old()) =="
cargo kani "${KANI_FLAGS[@]}" --exact --harness item2_proofs::check_counter_bump

echo
echo "== Item 3a: user-defined struct, public invariant-free fields =="
cargo kani "${KANI_FLAGS[@]}" --exact --harness item3a_proofs::check_shift_point

echo
echo "== Item 3b: user-defined struct, private field + invariant (hand-written Arbitrary) =="
cargo kani "${KANI_FLAGS[@]}" --exact --harness item3b_proofs::check_bump_nonzero

echo
echo "== Item 4a: g proved by its own contract =="
cargo kani "${KANI_FLAGS[@]}" --exact --harness item4_proofs::check_g

echo
echo "== Item 4b: f verified with g stubbed via stub_verified (same crate) =="
cargo kani "${KANI_FLAGS[@]}" --exact --harness item4_proofs::check_f_stubs_g

echo
echo "== Item 5: cross-crate stub_verified, WITH the local-re-proof workaround =="
echo "   (the workaround-free version fails to compile; see FINDINGS.md item 5"
echo "   for the exact captured error -- reproducing that failure here would"
echo "   require reverting item5_proofs::check_g_remote_locally, which would"
echo "   break every later item in this script, so it's not automated)"
cargo kani "${KANI_FLAGS[@]}" --exact --harness item5_proofs::check_f_remote_stubs_g_remote

echo
echo "== Item 6: real contract violation + concrete playback (cex, x = 255) =="
echo "   (expected to FAIL -- that's the point of this item)"
cargo kani "${KANI_FLAGS[@]}" --exact --harness item6_proofs::check_saturating_bump --concrete-playback print || true

echo
echo "== Item 6 (playback replay): cargo kani playback reproduces the same cex =="
cargo kani "${KANI_FLAGS[@]}" --exact --harness item6_proofs::check_saturating_bump --concrete-playback inplace || true
cargo kani playback -Z concrete-playback -Z function-contracts -Z unstable-options --lib \
  -- --exact item6_proofs::kani_concrete_playback_check_saturating_bump_5881385579587027251

echo
echo "== Item 7: the same cex, hand-written as a plain #[test] (no Kani needed) =="
cargo test --lib item7_handwritten_test::saturating_bump_breaks_its_own_contract_at_255

echo
echo "== Item 8: cfg_attr emission is inert under plain cargo build/test =="
cargo build
cargo test --lib

echo
echo "== Full batch: all 9 contract harnesses in one invocation (item 6 is a known FAIL) =="
cargo kani "${KANI_FLAGS[@]}" || true

echo
echo "== cargo kani list: contracts/harnesses table =="
cargo kani list -Z function-contracts -Z stubbing -Z unstable-options

echo
echo "Done. See tests/spike/FINDINGS.md for the recorded verdicts, exact"
echo "outputs, and the spec amendments this run forces."
