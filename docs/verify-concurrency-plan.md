# Bounded concurrency for `cargo ply verify`

Status: implemented first milestone; measured results are in
[`verify-concurrency-benchmark.md`](verify-concurrency-benchmark.md).

## Scope

`cargo ply verify -j N` means at most `N` Ply verification tasks are active at
once. Kani, Cargo, rustc, and other engines may create additional processes or
threads. The default remains `-j 1`.

The first release overlaps only independent, dependency-ready `bounded(k)`
claims whose complete checks list can use the bounded worker path. Fuzzing,
worked examples, mutation testing, state-history checks, linked-document runs,
mixed-engine claims, and relocation-sensitive source closures remain serial. This is deliberately narrower than
parallelising every function or every check.

## Execution and shared-state map

Planning reads the document, source, Cargo metadata, dependency identity,
toolchain identity, and the existing `ply.lock`. A bounded caller can depend on
a same-crate callee's newly earned clean bound. Its final fingerprint and cache
lookup therefore wait until every prerequisite has resolved. Cycles and
dependencies on cycles retain the existing conservative assumed-contract path.

Serial bounded checks temporarily use `src/ply_generated.rs`, guarded so the original
crate root is restored byte-for-byte and the scratch file is removed after each proof;
recognized leftovers from older runs are pruned before planning. Violations share
`target/ply/witness`; all claims publish counterexample tests through
`src/ply_generated_cex.rs`; and every claim ultimately contributes to one
`ply.lock`. Fuzz, test, mutate, and state-history checks additionally share a
generated harness crate and may temporarily register it in the Cargo workspace
manifest. Those shared-harness checks are not concurrency-safe and remain
serial.

Kani and Cargo write compilation and engine output below Cargo's target
directory. The first implementation gives every active bounded task a private
source shadow, target directory, and witness directory. The shadow copies the
Cargo workspace and the owning workspaces of packages in its relative
path-dependency closure, preserving inherited workspace configuration and their
filesystem layout while excluding `.git`, Cargo's resolved target directories,
and cargo-mutants output. A legitimate source directory merely named `target`
is retained. The worker overwrites only its shadow's canonical
`ply_generated.rs`, so a broken or stale proof cannot enter another worker's
compilation. The user's Cargo feature and compiler-flag settings are unchanged.
Each `cargo kani` process starts in the original crate (so Cargo and rustup see
the same ancestor configuration and toolchain) but receives the shadow's
`--manifest-path`. The original source and manifests are never edited by a
concurrent worker.

Planning validates the original dependency resolution with `cargo metadata
--locked`. A missing or stale lock keeps the run serial, where ordinary Cargo
may create or update it. Each shadow must accept its copied lock with the same
locked metadata check, and the coordinator rejects a worker outcome if its
engine changed that copy. The fingerprint can therefore name the versions that
actually governed the proof.

Source relocation is observable to `env!`, `option_env!`, `file!`, the
`include*!` family, and external `#[path]` modules. A conservative token-tree
scan, including aliases, forwarded identifiers, nested and namespaced macro
invocations, keeps any first-party closure containing those constructs serial.
The same scan runs over the complete generated proof after YAML contracts and
callee stubs have been merged, so relocation-sensitive code introduced by the
document cannot bypass the source gate.

The current first-party walker resolves a deliberately small set of literal
`path = ...` spellings. Planning compares its package roots and scanned source
files with Cargo's resolved local dependency closure and actual library entry
points. Any mismatch—including `workspace = true`, a custom library path, or a
valid TOML quoting/table form the text walker does not recognise—keeps the
closure serial and prevents reuse or recording. Otherwise a relocated helper
or a helper build script could sit outside both safety gates. A serial run that
creates a missing lock revalidates this closure after execution and before
record publication, so valid first-run evidence can still be cached.

First-party closures containing a build script remain serial. Build scripts can
write outside Cargo's target directory or consume inputs Ply cannot enumerate;
isolating that arbitrary surface is wider than this milestone.
When a serial fuzz/test/state harness temporarily registers itself as a Cargo
workspace member, bounded claims in that mixed run also remain serial; a worker
shadow excludes the harness below `target/` and must never copy a manifest that
still names it.

The repository's non-`target`, non-`.git` tree is about 14 MB on 2026-09-08.
The source shadow copies that tree once per active proof, but never copies a
build directory. Private target directories already require separate Cargo
outputs; the measured source-copy setup cost is reported separately from proof
time in the benchmark.

## Planning, execution, publication

1. Planning resolves claims and contracts, builds the callee-to-caller graph,
   and identifies dependency-ready waves. It finalises a bounded claim's
   prerequisite bounds and only then decides whether its record can be reused.
2. Execution starts up to `N` fresh bounded tasks from one ready wave. Each
   task runs its promise probes and proof in its own output environment and
   returns only the structured engine and promise-probe outcomes. Workers do
   not construct report nodes or edit `ply.lock`, the report, or central
   counterexample files.
3. Publication consumes worker results in deterministic claim order, updates
   the known clean bounds before releasing dependent claims, interprets each
   outcome against its original claim, publishes all witnesses and
   counterexample tests centrally, and writes `ply.lock` once.

An ordinary violation does not cancel unrelated work. An engine setup or parse
failure remains a tool error attached to its own claim. Every controlled
subprocess, including planning probes, owns a process group. Interruption stops
new work, terminates those process groups, removes worker shadows and outputs,
and leaves the previous verification record untouched. A direct engine crash
also triggers process-group cleanup so it cannot orphan a compiler or solver.

## Test and measurement plan

Controlled workers prove the concurrency bound and actual overlap without
depending only on elapsed time. They also cover serial execution, dependency
release, deterministic collection, worker failure, cancellation, timeout
cleanup, and process-tree cleanup. Real Kani fixtures cover independent proofs,
pass/fail attribution, two counterexamples, caller/callee evidence,
cached-plus-fresh runs, and workspace restoration.

After correctness coverage passes, benchmark a representative bounded project
at `-j 1`, `-j 2`, and `-j 4`. Record total time, environment setup overhead,
peak memory where available, compilation duplication or contention, and any
serial/concurrent output difference other than timing and private paths. Keep
the default at one until those measurements justify another recommendation.

Completed on 2026-09-08. Four independent real proofs took 104.91 s at `-j 1`,
83.68 s at `-j 2`, and 56.69 s at `-j 4`; coordinator-only overhead measured
1.85 s. Private targets duplicated compilation output, and aggregate peak
memory could not be measured reliably, so the default remains one.
