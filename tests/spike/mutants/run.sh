#!/usr/bin/env bash
# cargo-mutants feasibility spike: re-run every item in MUTANTS-FINDINGS.md
# end to end.
#
# Closes the gap the M0 spike (tests/spike/FINDINGS.md) left open: §10's M0
# list named "cargo-mutants with a custom test command running a generated
# harness" as a feasibility item, but the M0 spike never exercised it, and
# §5.4c cites "confirmed in the M0 spike" for a claim that was never
# actually checked. This spike checks it.
#
# Pinned toolchain: rustc/cargo 1.90.0, cargo-mutants 27.1.0 (installed via
# `cargo install cargo-mutants --locked` -- the unlocked resolve pulls a
# cargo-platform release requiring rustc 1.91, newer than this machine's
# 1.90.0, and fails; --locked pins to the versions in cargo-mutants' own
# checked-in Cargo.lock, which build fine). This script does not install
# cargo-mutants; it assumes it is already on PATH.
#
# Two throwaway workspaces, neither a member of tools/Cargo.toml's workspace
# nor of each other:
#   tests/spike/mutants/colocated  -- items 1, 2, 4, 5 (checks live in the
#                                      mutated crate's own #[cfg(test)])
#   tests/spike/mutants/scoped     -- item 3 (the load-bearing question) and
#                                      item 6 (non-standard harness location),
#                                      checks live in a separate crate, one
#                                      copy of which sits at lib/target/ply/fuzz/
#                                      to reproduce §5.4c's own placement
#                                      ("generated harness crate under
#                                      target/ply/fuzz/") literally.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

echo "== cargo mutants --version =="
cargo mutants --version

echo
echo "== Item 1: does cargo mutants run at all, what does it report by default? =="
echo "   (colocated/: strong_target has a real property test + boundary"
echo "   examples, weak_target has a vacuous smoke test; both bodies are"
echo "   identical, so any caught/missed split comes from the spec, not"
echo "   the code under test)"
(cd colocated && rm -rf mutants.out && cargo mutants --no-times) || true

echo
echo "== Item 2: does --re <fn> scope mutation to exactly one function? =="
echo "   (proof is the --list output naming only that fn, for both fns)"
(cd colocated && cargo mutants --re strong_target --list)
(cd colocated && cargo mutants --re weak_target --list)

echo
echo "== Item 4: strong spec catches its mutants, weak spec lets them survive =="
echo "   (scoped runs, so the per-function verdict is unambiguous)"
(cd colocated && rm -rf mutants.out && cargo mutants --re strong_target --no-times) || true
(cd colocated && rm -rf mutants.out && cargo mutants --re weak_target --no-times) || true
echo "   NOTE: strong_target's one MISSED mutant (replace > with >= on the"
echo "   y > 0 comparison) is an equivalent mutant, not a spec gap: when"
echo "   y == 0, x + 0 == x - 0, so no test oracle can distinguish the"
echo "   mutated branch condition from the original by output alone. See"
echo "   MUTANTS-FINDINGS.md for the algebraic argument."

echo
echo "== Item 5: timing for a scoped run on a trivial function =="
(cd colocated && rm -rf mutants.out && /usr/bin/time -p cargo mutants --re strong_target --no-times) || true

echo
echo "== Item 3 (the load-bearing question): a custom test command =="
echo "   (scoped/: lib/ has NO local tests at all -- the mutated package's"
echo "   own test suite is empty, standing in for a real Ply target whose"
echo "   checks live entirely in a generated harness crate. First, the"
echo "   naive default -- mutating lib with no extra flags -- to show the"
echo "   exact failure mode the spec worries about)"
(cd scoped && rm -rf mutants.out && cargo mutants -p ply-spike-mutants-lib --re strong_target --no-times) || true
echo "   ^ every mutant MISSED: lib's own (empty) test suite can never catch"
echo "   anything, because cargo-mutants defaults to testing only the"
echo "   mutated package's own tests. This is what \"custom test command\""
echo "   in §5.4c has to fix."
echo
echo "   The actual mechanism in cargo-mutants 27.1.0: there is no"
echo "   --test-tool custom / arbitrary-shell-command flag (--test-tool"
echo "   only accepts cargo or nextest -- see --help and the config"
echo "   schema's TestTool enum). The real substitute is package-based"
echo "   test *selection*: -p <mutated-package> chooses what to MUTATE,"
echo "   --test-package <harness-package> chooses what tests to RUN, and a"
echo "   name filter after -- narrows that further to one function's"
echo "   harness. Proving all three:"
(cd scoped && rm -rf mutants.out && cargo mutants -p ply-spike-mutants-lib --test-package ply-spike-mutants-harness --re strong_target --no-times) || true
(cd scoped && rm -rf mutants.out && cargo mutants -p ply-spike-mutants-lib --test-package ply-spike-mutants-harness --re weak_target --no-times) || true
echo "   ^ strong: 13/14 caught (the same equivalent mutant survives)."
echo "   weak: 14/14 missed. Now the same strong run, with a cargo-test"
echo "   name filter narrowing execution to just strong_target's own"
echo "   harness module (proof it's a real filter, not just --re picking"
echo "   the right mutants while the whole harness crate's suite runs as"
echo "   noise): see MUTANTS-FINDINGS.md for the \"1 filtered out\" log"
echo "   line that proves this."
(cd scoped && rm -rf mutants.out && cargo mutants -p ply-spike-mutants-lib --test-package ply-spike-mutants-harness --re strong_target --no-times -- strong_target_harness) || true

echo
echo "== Item 6: generated harness in a non-standard (target/ply/fuzz/-style) location =="
echo "   (scoped/lib/target/ply/fuzz/ is a second copy of the harness crate,"
echo "   physically placed exactly where §5.4c says Ply's real fuzz"
echo "   harnesses live. The repo's root .gitignore's bare 'target/' pattern"
echo "   matches it at any depth -- confirmed with git check-ignore.)"
(cd scoped && rm -rf mutants.out && cargo mutants -p ply-spike-mutants-lib --test-package ply-spike-mutants-harness-genloc --re strong_target --no-times) || true
echo "   ^ works identically to the non-gitignored harness above --"
echo "   cargo-mutants' DEFAULT does not respect .gitignore at all (see"
echo "   MUTANTS-FINDINGS.md: --gitignore's schema default is described as"
echo "   \"exclude patterns in .gitignore\" but the observed runtime default"
echo "   copies everything, gitignored or not)."
echo
echo "   The landmine: turning gitignore-respecting copying ON explicitly"
echo "   is exactly what a real, large target crate would want (to avoid"
echo "   copying gigabytes of unrelated build cache on every mutant), and"
echo "   it breaks this placement outright:"
(cd scoped && rm -rf mutants.out && cargo mutants --gitignore true -p ply-spike-mutants-lib --test-package ply-spike-mutants-harness-genloc --re strong_target --no-times || true)
echo "   ^ expected: FAILED Unmutated baseline / cargo build failed in an"
echo "   unmutated tree, so no mutants were tested -- a loud, immediate"
echo "   failure (not a silent false-pass), but a total one: the harness"
echo "   crate is simply missing from the copied workspace."

echo
echo "Done. See tests/spike/mutants/MUTANTS-FINDINGS.md for the recorded"
echo "verdicts, exact outputs, and the spec amendments this run forces."
