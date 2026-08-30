#!/usr/bin/env bash
# Verus feasibility spike: re-run both halves end to end.
#
#   1. Verus deductive verification of tests/spike/verus/proof/shadow.rs --
#      the four standing obligations (CLAUDE.md), proved for ALL trees by
#      structural induction, not bounded enumeration.
#   2. The differential test (plain `cargo test`, no Verus needed) that
#      checks the shadow's executable transcription against the real
#      ply-kernel crate's aggregate() on a generated corpus -- this is what
#      licenses claim 1 to say anything about the production kernel.
#
# Pinned toolchain (see FINDINGS.md for the exact install steps): Verus
# 0.2026.08.23.fbbbbcf, which requires rustup toolchain
# 1.97.1-x86_64-unknown-linux-gnu to be installed (Verus ships its own
# compiler build; the toolchain install is a one-time `rustup toolchain
# install 1.97.1-x86_64-unknown-linux-gnu`, independent of whatever
# toolchain `tools/` itself uses).
#
# This script does not install Verus -- point VERUS at the `verus` binary
# from an unpacked release (not committed to this repo; see FINDINGS.md for
# why). If VERUS is unset, it falls back to `verus` on PATH.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

VERUS="${VERUS:-verus}"

echo "== Verus version =="
if ! "$VERUS" --version; then
    echo
    echo "Verus not found at '$VERUS'. Set VERUS=/path/to/verus (see FINDINGS.md" >&2
    echo "'Installing Verus' for the exact release/toolchain this spike used)." >&2
    exit 1
fi

echo
echo "== Step 1: Verus proof (tests/spike/verus/proof/shadow.rs) =="
echo "   Proves, by structural induction over ALL trees (not bounded enumeration):"
echo "     1. aggregation never reports evidence stronger than the weakest child"
echo "     2. conditional never disappears without its assumptions being discharged"
echo "     3. a violation anywhere always reaches the root"
echo "     4. no rule sequence assigns one node two different verdicts"
time "$VERUS" proof/shadow.rs

echo
echo "== Step 2: differential test (shadow executable vs. real ply-kernel) =="
echo "   Plain cargo test -- no Verus needed for this half."
( cd diff && cargo test --release )

echo
echo "Done. Both halves green: see FINDINGS.md for what this does and does not establish."
