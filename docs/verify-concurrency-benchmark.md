# `cargo ply verify -j` first-milestone benchmark

Measured on 2026-09-08 with the release build, Kani 0.67.0, Cargo 1.98.0,
and a 12-core/24-thread AMD Ryzen AI 9 HX PRO 370 with 54 GiB RAM. Each run
used a fresh copy of `tests/fixtures/parallelbounded`: four independent
`bounded(2)` claims, two passing and two producing counterexamples.
`CARGO_NET_OFFLINE=true` and `--engine-timeout 120` were held constant.

| Jobs | Wall time | CPU | Maximum reported RSS | Result |
|---:|---:|---:|---:|---|
| 1 | 104.91 s | 112% | 2,684,060 KiB | two bounded, two violations |
| 2 | 83.68 s | 215% | 2,684,188 KiB | equivalent |
| 4 | 56.69 s | 379% | 2,684,696 KiB | equivalent |

GNU `time -v` reports the largest resident set observed for the command or
one waited-for child, not the aggregate memory of all simultaneous process
trees. The near-identical RSS numbers therefore must not be read as proof
that four concurrent Kani runs consume no additional total memory. Aggregate
peak memory was not reliably measurable in this pass.

The JSON reports had identical trees, verdicts, statuses, diagnostic order,
cache decisions, and counterexample ownership. Kani chose `4294967295` as one
valid failing input in the serial run and `4294967292` in both concurrent runs;
after normalising that solver-selected value, the reports were byte-for-byte
identical. The regression suite permits only that value to differ. The
generated counterexample module retained both failing reproductions in
deterministic claim order.

To separate coordinator setup from solver and compiler time, the same fresh
four-claim fixture was run through a controlled Kani-compatible worker that
returned success immediately. Planning, private source-shadow preparation,
result collection, and publication completed in 1.85 s with 35,104 KiB maximum
reported RSS. Each active proof receives a source shadow, but Cargo/Ply target
trees, `.git`, and cargo-mutants output are excluded. Shadows are prepared in
batches of at most `N`, so source-copy storage scales with the concurrency bound
rather than the number of ready claims.

Private Cargo target directories deliberately trade duplicated compilation
for write isolation. The higher job counts used more CPU, and GNU `time`
reported roughly 724,000 filesystem-output events at `-j 2`/`-j 4`, compared
with roughly 166,000 at `-j 1`. On this
four-proof fixture, `-j 2` reduced wall time by 20% and `-j 4` by 46%, but the
extra builds and unmeasured aggregate memory make a universal higher default
premature. The default remains `1`; users with enough CPU and memory can opt
in to `-j 2` or `-j 4` for independent bounded proofs.

Checks still serial in this release: fuzzing, worked examples, mutation
testing, state-history checks, linked-document verification, claims with a
mixed checks list, cyclic/dependency-tainted bounded claims, and any bounded
claim whose call boundary cannot use the isolated worker path. First-party
dependency closures containing a build script also remain serial because a
build script can write outside Cargo's target directory or consume undeclared
inputs. Runs also remain serial when Cargo.lock is absent or stale, a shared
harness is registered, or compile-time path/environment/inclusion constructs
make source relocation observable. A mismatch between the text-scanned
first-party package/source set and Cargo's resolved local dependency closure or
library entry points is also serial and uncached.
