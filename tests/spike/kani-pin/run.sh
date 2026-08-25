#!/usr/bin/env bash
# D13 spike: does a newer Kani deliver the two stubbing capabilities the pin
# blocks? Reproduces every run recorded in FINDINGS.md, on both toolchains.
#
# TOOLCHAIN A (the pin): cargo-kani 0.67.0 / CBMC 6.8.0, whatever `cargo kani`
#   resolves to on PATH. Nothing here installs or disturbs it.
# TOOLCHAIN B (the candidate): Kani `main` built from source. There is no
#   newer *release* -- crates.io's latest kani-verifier is still 0.67.0 -- so
#   the candidate is a git checkout, built with `cargo build-dev`, and driven
#   through `<kani-repo>/scripts/cargo-kani`. It needs CBMC 6.10.0 on PATH
#   (kani-dependencies on main pins 6.10.0; the bundled 6.8.0 is too old).
#
# Point KANI_MAIN_REPO and CBMC610_BIN at your own build:
#
#   git clone https://github.com/model-checking/kani && cd kani
#   git submodule update --init --depth 1 charon
#   rustup toolchain install nightly-2026-04-01 \
#       --component llvm-tools,rustc-dev,rust-src,rustfmt --profile minimal
#   cargo build-dev
#   curl -sSL -o cbmc.deb https://github.com/diffblue/cbmc/releases/download/\
# cbmc-6.10.0/ubuntu-24.04-cbmc-6.10.0-Linux.deb && dpkg-deb -x cbmc.deb cbmc610
#
# Every fixture is copied into a scratch directory before it runs:
# `--concrete-playback inplace` edits source, and the two toolchains generate
# different test names, so running in place would make the second run depend on
# the first. The committed fixtures stay pristine.

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

KANI_MAIN_REPO="${KANI_MAIN_REPO:-/home/user/model-checking/kani}"
CBMC610_BIN="${CBMC610_BIN:-}"
WORK="${WORK:-$(mktemp -d)}"

FLAGS=(-Z function-contracts -Z unstable-options -Z concrete-playback -Z stubbing
       --harness-timeout 300s)

pinned() { ( cd "$1" && shift && cargo kani "$@" ); }
candidate() {
  local d="$1"; shift
  ( cd "$d" && PATH="${CBMC610_BIN:+$CBMC610_BIN:}$PATH" \
      "$KANI_MAIN_REPO/scripts/cargo-kani" "$@" )
}
pinned_playback() { ( cd "$1" && shift && cargo kani playback "$@" ); }
candidate_playback() {
  local d="$1"; shift
  ( cd "$d" && PATH="${CBMC610_BIN:+$CBMC610_BIN:}$PATH" \
      "$KANI_MAIN_REPO/scripts/cargo-kani" playback "$@" )
}

fresh() { # fresh <fixture> <tag> -> echoes a clean copy's path
  local d="$WORK/$2-$1"
  rm -rf "$d"; mkdir -p "$d"
  cp -r "$HERE/$1/Cargo.toml" "$HERE/$1/src" "$d/"
  echo "$d"
}

banner() { echo; echo "======================================================"; echo "$*"; echo "======================================================"; }

banner "Toolchain A (the pin)"
cargo kani --version
banner "Toolchain B (the candidate)"
( PATH="${CBMC610_BIN:+$CBMC610_BIN:}$PATH"; "$KANI_MAIN_REPO/scripts/cargo-kani" --version
  echo "git: $(git -C "$KANI_MAIN_REPO" rev-parse HEAD) $(git -C "$KANI_MAIN_REPO" log -1 --format=%ci)"
  cbmc --version )

# ---------------------------------------------------------------- blocker 1
for TC in pinned candidate; do
  banner "Blocker 1 / $TC -- #[kani::stub] + --concrete-playback"
  d=$(fresh stub_playback "$TC")

  echo "-- (a) failure caused BY THE STUB: witness?"
  time $TC "$d" "${FLAGS[@]}" --exact --harness proofs::check_stub_only_failure \
      --concrete-playback inplace
  t=$(grep -o "kani_concrete_playback_check_stub_only_failure_[0-9]*" "$d/src/lib.rs" | head -1)
  echo "-- (a) replay of that witness -- is the red for the right reason?"
  ${TC}_playback "$d" -Z concrete-playback -Z stubbing -Z function-contracts \
      -Z unstable-options --lib -- --exact "proofs::$t"

  echo "-- (b) CONTROL: failure caused by the harness's own input, same stub"
  $TC "$d" "${FLAGS[@]}" --exact --harness proofs::check_input_failure_with_stub \
      --concrete-playback inplace
  t=$(grep -o "kani_concrete_playback_check_input_failure_with_stub_[0-9]*" "$d/src/lib.rs" | head -1)
  ${TC}_playback "$d" -Z concrete-playback -Z stubbing -Z function-contracts \
      -Z unstable-options --lib -- --exact "proofs::$t"
done

# ---------------------------------------------------------------- blocker 2
for TC in pinned candidate; do
  banner "Blocker 2 / $TC -- #[kani::stub] over a CONTRACTED target (Kani #4591)"
  d=$(fresh stub_on_contracted "$TC")
  time $TC "$d" "${FLAGS[@]}" --exact \
      --harness proofs::check_fee_over_contracted_stub --concrete-playback print
  echo "exit=$?"
done

# -------------------------------------------------- what Ply actually needs
for TC in pinned candidate; do
  banner "Boundary / $TC -- §5.5's real shape (tests/fixtures/boundarycontract)"
  d=$(fresh boundary "$TC")

  echo "-- clean: contracted caller, callee stubbed by its declared contract"
  time $TC "$d" "${FLAGS[@]}" --exact \
      --harness ply_generated::ply_proof_tiered_fee --concrete-playback print

  echo "-- violation in the same configuration: is there a usable witness?"
  time $TC "$d" "${FLAGS[@]}" --exact \
      --harness ply_generated::ply_proof_tiered_fee_halfclaim --concrete-playback inplace
  t=$(grep -o "kani_concrete_playback_ply_proof_tiered_fee_halfclaim_[0-9]*" \
      "$d/src/ply_generated.rs" | head -1)
  ${TC}_playback "$d" -Z concrete-playback -Z stubbing -Z function-contracts \
      -Z unstable-options --lib -- --exact "ply_generated::$t"

  echo "-- the witness, replayed by hand against the real callee (no Kani):"
  echo "   this #[test] PASSES, and that is the finding."
  ( cd "$d" && cargo test --lib )

  echo "-- MUTATION: strengthen the stub to the real body's range (<= 150) and"
  echo "   the same violation must disappear. This is the test that the stub is"
  echo "   really applied and really load-bearing: the only thing that changed"
  echo "   is the assumed contract, and the verdict has to follow it."
  sed -i 's/\*result <= 10_000/\*result <= 150/' "$d/src/ply_generated.rs"
  $TC "$d" "${FLAGS[@]}" --exact \
      --harness ply_generated::ply_proof_tiered_fee_halfclaim --concrete-playback print

  echo "-- VACUITY CHECK: delete the stub's assume entirely, so the callee is"
  echo "   unconstrained, and re-prove the CLEAN harness. It still verifies --"
  echo "   tiered_fee's own .min(10_000) clamp defends it whatever the"
  echo "   callee returns, so that proof never leans on the assumption. See"
  echo "   FINDINGS.md; this is a fact about the fixture, not about Kani."
  d2=$(fresh boundary "$TC-vac")
  sed -i '/kani::assume/d' "$d2/src/ply_generated.rs"
  $TC "$d2" "${FLAGS[@]}" --exact \
      --harness ply_generated::ply_proof_tiered_fee --concrete-playback print
done

echo
echo "Done. Scratch copies left in $WORK. See FINDINGS.md."
