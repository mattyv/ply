# Ply — Implementation Specification

Ply is a **specification and verification layer over plain Rust**. It is a cargo
subcommand (`cargo ply`) that reads declared claims about a codebase, routes each claim to
an existing checking engine, and returns unified, machine-readable results. Every
falsified executable claim carries a concrete counterexample.

Audience: the implementing agent (Claude Code). Read §1–§6 before writing any code.

## 0. Terminology

Terms this spec uses without further explanation. Ply-specific terms are marked (Ply).

| Term | Meaning |
|---|---|
| cargo | Rust's build tool and package manager: it compiles code, fetches dependencies, and runs tests. Any binary named `cargo-x` becomes runnable as `cargo x`, which is how Ply plugs in. |
| rustc | The Rust compiler itself; cargo drives it. |
| crate | Rust's unit of compilation and distribution — a library or binary package. |
| workspace | A group of crates built together from one root `Cargo.toml`. |
| claim | (Ply) Any statement Ply checks: a function contract, an architecture rule, or a profile ban. |
| contract | A function's `requires` (precondition: what must hold on entry) and `ensures` (postcondition: what the function guarantees about its result). |
| counterexample (cex) | A concrete input that violates a claim — the failing witness. |
| engine | An external checking tool Ply drives as a subprocess: rustc, Kani, proptest, cargo-mutants, Verus, Miri. |
| Kani | AWS's model checker for Rust. It explores every execution up to a loop bound and, when a claim can fail, reports a concrete failing input. Its function-contract support is unstable and needs the `-Zfunction-contracts` flag. |
| bounded model checking | Exhaustive exploration of program behavior up to a fixed bound *k*. It finds every bug within the bound and says nothing beyond it. |
| concrete playback | Kani's mechanism for replaying a counterexample: it emits the raw input bytes and re-runs them through the harness via `cargo kani playback`. Tied to the Kani version and flags that produced it. Replays the harness and the real body only; contract closures are not re-evaluated, so a replay of an `ensures` violation passes. Input storage, not a reproduction. |
| Verus | A deductive verifier for Rust: it proves claims for *all* inputs, with no bound, in exchange for a restricted language subset and more annotation work. |
| proptest / property testing | Running a function on hundreds of generated random inputs and checking its contract on each. *Shrinking* reduces a failing input to a minimal one. |
| Arbitrary | The trait (in both Kani and proptest, separately) that lets an engine construct values of a type. A function is only checkable if its inputs are constructible. |
| mutation testing | Planting small deliberate bugs (*mutants*) in the code and checking whether the specs catch (*kill*) them. A surviving mutant means the spec is weak. `cargo-mutants` is the engine. |
| vacuous spec | A specification that verifies while constraining nothing. |
| Miri | An interpreter that detects undefined behavior in unsafe Rust. |
| harness | A small generated program that exercises one function under one engine. Ply generates these; users never write them. |
| check | (Ply) One verification method applied to a function: `test`, `fuzz(n)`, `bounded(k)`, `prove`, or `mutate`. A function declares a *checks list*. |
| verdict | (Ply) The evidence level a function's passing checks earned: `tested`, `fuzzed(n)`, `bounded(k)`, `proved`. |
| status | (Ply) A qualifier on a verdict: `conditional` (rests on assumed contracts), `owed-evidence` (an assumed contract that nothing has yet checked against the real code — it travels with `conditional` and is what turns "we assumed this" into "and here is the debt"), `weak-spec`, `unsupported`, `engine-missing`, `timeout`, `inconclusive`. |
| component | (Ply) A named architectural unit, declared in `ply.yaml` and anchored to a crate or module. |
| capability (cap) | (Ply) A coarse effect a component is allowed: `net fs db time rand proc unsafe`. |
| anchor | (Ply) The real code item (crate, module, or function) a claim attaches to. |
| fingerprint | (Ply) The recorded hash of what a result depended on: item body, contract text, **the first-party bodies the check runs or descends into** (or all of the crate's source, when Ply cannot bound that set — §5.2a), the worked examples it asserts, the contracts assumed for the callees it crosses into, the same-crate callees it stands on rather than assumes and the bound each earned (D5's first branch, §5.5), the checks that ran, engine name + version + flags, active features, target triple, compiler version, the resolved versions of packages outside the workspace, **and Ply's own version**. It is recomputed from today's inputs every time a recorded result is used or shown: it matches, the result is reused; it does not, the check runs again (D14, §5.2a). |
| verdict tree | (Ply) The aggregated per-node verdicts for the whole workspace, rendered by `cargo ply tree`. |
| visual grammar | (Ply) The fixed one-to-one mapping between grammar constructs and visual forms (§7.1). A grammar feature that cannot be drawn is not admitted. |
| watermark | (Ply) The per-function line where declaration stops and code begins: signature plus contract (§7.2). Below it, Ply verifies but never specifies; the un-specifiable imperative interior is the *floor*. |
| Dafny | A Microsoft Research language with verification built in: code and spec are written together, and the compiler proves the spec (via the Z3 solver) as part of building. Cited here only as evidence (§1); Ply deliberately avoids the new-language approach. |
| stubbing | Replacing a callee's body with its contract during verification (Kani: `stub_verified`). |
| golden test | A test that compares output byte-for-byte against a reviewed reference file. We use the `insta` crate. |
| fixture | A small sample cargo project that tests use as a check target. |
| ADR | Architecture decision record: a one-paragraph note in `docs/adr/` stating a decision and its reason. |
| syn | The Rust library that parses Rust source into a syntax tree. It performs no name resolution, type inference, or macro expansion — a limit this design respects. |
| serde | Rust's standard serialization framework: `#[derive(Serialize, Deserialize)]` on a struct generates conversion to and from any data format with a serde backend. A serde-YAML implementation is the backend that plugs YAML in — it is how Ply reads `ply.yaml` into typed structs with no hand-written parser (D3). The canonical `serde_yaml` crate is archived; ADR-0002 picks a maintained fork. |

## 1. Mission & evidence base

Coding agents write plausible code; checkers make it correct. Four empirical findings from
the 2025–26 verification literature drive the design:

1. **The generate→check→repair loop works and is improving fast.** Dafny auto-verification
   went from 68% to 96% in one year ([DafnyBench](https://arxiv.org/abs/2406.08467) →
   [vericoding benchmark](https://arxiv.org/abs/2509.22908)); Verus/Rust sits at 44% and
   climbing (same source; [AutoVerus](https://arxiv.org/abs/2409.13082) reports ~90% on
   its own bench vs 45% for direct prompting).
2. **Feedback quality is the dominant variable.** With the model held fixed,
   counterexample-guided repair lifts success from 12% to 97% in controlled ablations
   ([CEGR](https://arxiv.org/pdf/2605.13817)); structured repair values roughly triple
   agent-loop success on identical models ([Ray & Goyal](https://arxiv.org/html/2607.14167v1)).
   Feedback without a witness performs little better than none. Every falsified
   executable claim Ply reports therefore MUST carry a concrete failing input and, where
   possible, a replayable test (the replayable test is Ply-rendered with the postcondition
   asserted explicitly; engine playback data alone does not reproduce contract violations —
   ADR-0003, D7). (Architecture violations, timeouts, tool errors, and
   surviving mutants have no input witness; they carry spans and evidence of their own
   kind.)
3. **Local success does not compose.** Models whose single-function verification
   approaches 96% collapse below 4% on multi-function programs requiring cross-boundary
   reasoning ([DafnyCOMP](https://arxiv.org/abs/2509.23061): 3.69% at Pass@8 with >99%
   syntax correctness — the failure is semantic, not syntactic). Verification is
   therefore modular by construction: Ply checks every function against its own
   contract, and checks callers against their callees' *contracts*, never their bodies.
4. **Machine-written specs go vacuous.** A spec can verify while claiming nothing.
   Mutation testing is therefore a first-class check: a contract that fails to kill
   mutants is flagged as weak.

**What v1 optimises for.** `fuzzed(n)·spec-strong` is the workhorse tier: fuzzing reaches
every signature shape — including the recursive and collection-heavy ones §5.4b excludes
from `bounded` — and mutation testing is what turns a passing check into measured
evidence that the spec constrains anything. `bounded(k)` is exhaustive-within-bound
reinforcement for the shapes it can reach, and it is genuinely better at boundaries
(measured: proptest at 256 uniform samples missed `>`→`>=` mutants that `bounded` kills by
construction). But note the asymmetry it cannot escape in v1: mutation's kill signal is
scoped to `test`/`fuzz` (D12), so **`bounded` can never earn `·spec-strong` — a bounded
proof of a vacuous contract is a green nothing.** The evidence story is therefore two
axes, rung and spec-strength, not one ladder.

**A run succeeds only if every claim earned its declared evidence.** `timeout`,
`unsupported`, `tool_error`, `unclaimed`, `engine-missing` and `inconclusive` are
absences of evidence, and absence of evidence fails the run by default — `--fail-on`
exists to relax that, never to enable it.

**An absence is a name, not a slot.** Those names appear in two places in a §8 node — as
its `verdict`, and as a `status` beside it (D6) — and they mean the same thing in both.
A `mutate` check whose engine is missing leaves the fn's `fuzzed(64)` verdict alone,
because the fuzz check really did run, and records the missing engine as a status; a rule
that reads only verdicts calls that run clean. It did: until 2026-08-25 this principle was
implemented as an enumeration of verdict *strings*, complete over every verdict the tool
can emit and blind to every absence encoded as a status, so `checks: [fuzz(64), mutate]`
with cargo-mutants absent exited 0 against §6's own exit-3 row (adversarial review of the
post-004 fixes, D2). The rule is stated over names precisely so that the next absence
recorded in a new field is caught by the vocabulary rather than by another special case. And every verdict, passing or failing, must name the evidence that
produced it concretely enough to reproduce it: a fuzz verdict carries its seed and case
count the way a violation carries its witness.

This is stated here, beside the evidence rules, because it is a *principle* and its
absence was not an implementation slip. Until 2026-08-25 the exit-code table in §6 had
rows for clean, violations, tool error and missing engine, and no row for "checked
nothing" — so `exit_code_for` faithfully returned 0 for vetting 004's run in which two of
five claims produced no evidence at all and one of those was the function the scenario
existed to test (`vetting/004-legacy-extension.md`, finding 1: root verdict `timeout`,
7m13s, exit 0). A tool whose green CI run can contain zero evidence cannot be trusted
about anything, including its own future fixes; a green nothing is the one failure this
project cannot ship.

Ply's job: one schema for claims, one router to engines, one JSON result schema, one
worklist. The engines already exist and others maintain them. **We build glue and UX,
never solvers.** A session that finds itself implementing SMT encoding, model checking, or
proof search has gone off-spec — stop.

## 2. Fixed design decisions (do not relitigate)

| # | Decision | Rationale |
|---|---|---|
| D1 | Plain stable Rust is the only executable artifact. Specs never compile to code, except Ply-generated harnesses, proof modules, and tests. | Zero migration cost; everything works with bare cargo. |
| D2 | Contracts are written as **`#[ply::requires(...)]` / `#[ply::ensures(...)]`** attributes on the function. The `ply-attrs` macro re-emits the original function unchanged, adding `#[cfg_attr(kani, kani::requires(...))]` (and the ensures equivalent). Under plain cargo the attributes vanish; under `cargo kani` they instrument **the real function** — never a copy. Proof harnesses are generated into a `cfg(kani)`-gated module inside the target crate so they see private items. Pre-existing `#[cfg_attr(kani, kani::requires(...))]` attributes are harvested and merged by conjunction. | Kani's `proof_for_contract` verifies the function the attributes annotate; contracts on a generated copy would verify a different symbol. `cfg_attr` keeps bare `cargo build` working (D1) but not *warning-clean* on its own: every `#[cfg_attr(kani, ...)]`/`#[cfg(kani)]` triggers `unexpected_cfgs` under bare cargo unless the instrumented crate also carries `[lints.rust] unexpected_cfgs = { level = "warn", check-cfg = ['cfg(kani)'] }` (M0 finding, confirmed again by the M3 slice's fixtures, each of which carries this line by hand). Automating that one-line insertion into a consuming crate's `Cargo.toml` the first time `cargo ply verify` instruments it is a natural near-term enhancement, not yet built: this M3 slice's fixtures all set it manually and `verify` does not currently check for or offer it. The in-crate module mechanism (generated file + one module declaration, or an equivalent include) is settled by the M0 spike; the fallback is verifying `pub` items only from a sibling harness crate — with reduced coverage for private-field invariant types (what smart constructors produce): a sibling crate can never supply `kani::Arbitrary` for them (field visibility, and the orphan rule), so witnesses must come from `pub` constructors plus `kani::assume` — verified to work (ADR-0003, item 3b), but capped at pub-reachable states and hand-written per type; only a type with no `pub` construction path is walled off entirely. |
| D3 | Architecture claims, checks, capabilities, ownership, profiles, and the unresolved registry live in **`ply.yaml`**, validated against a normative JSON Schema (§5). | These claims are cross-cutting and have no natural attribute location. YAML plus a schema needs no parser of our own, and agents emit YAML reliably. The schema, not prose, is the formal definition. |
| D4 | Architecture enforcement has two tiers. **Crate-level dependency rules** (from `cargo metadata`, which is exact) are errors and default-deny between declared components. **Item-level rules** (calls, capabilities, ownership — from syn, which is approximate) are warnings by default; a component opts into item-level errors with `strict: true`. | Default-deny is only honest over facts that are sound. Crate dependency data is sound today; syn-derived call data is not (no name resolution, no macro expansion). Advisory-until-strict gives teeth without theater. |
| D5 | Verification is modular and evidence-honest. Kani's `stub_verified(g)` is used only when `g` itself passed a Kani contract proof this run — in the same crate, or via the caller-local re-proof below. **Kani does not enforce this and cannot: it checks only that a `#[proof_for_contract]` harness *exists* for the stub target, never that it ran or passed (ADR-0003, item 4 — a caller reported clean SUCCESS while assuming a deliberately falsified callee contract; Kani's RFC-0009 promises pass-gating, but 0.67.0 observably runs harnesses in arbitrary order and never retracts a caller's verdict when its callee's harness fails in the same invocation). Ply's scheduler — callees proved first, the caller credited only if those proofs passed — is therefore the entire soundness guarantee; an implementation that relaxes it is unsound and nothing downstream will notice.** Cross-crate callees are supported after all, by declaring a caller-local `proof_for_contract` for the remote `pub` item (ADR-0003, item 5; verified against the real linked body — a mutated callee body fails the caller-local proof); there is no cross-crate proof caching, so each consumer re-proves. Any weaker case — callee merely fuzzed or tested, a cross-crate callee that cannot be re-proved caller-locally (not `pub`, or its witnesses unconstructible per D2), cycle in the call graph — verifies the caller against an *assumed* contract and marks the verdict **`conditional`**, listing the assumptions. A conditional verdict never reads as plain `bounded`. **A callee with no declared contract at all — the case every unannotated legacy module falls into — is a third branch, added 2026-08-25 after vetting 004: Ply refuses to descend into it, the caller's `bounded` check earns no evidence, and the diagnostic names the callee (§5.5).** | Stubbing is a soundness claim; fuzzing does not license it. Kani proof harnesses are crate-local, so cross-crate means caller-local re-verification of the real linked body, never reuse of the callee crate's proof. Never inline a contracted callee's body. |
| D6 | Verdicts aggregate upward as **worst-of** over the evidence order `violation < unclaimed < tested < fuzzed < bounded < proved`. Statuses (`conditional`, `owed-evidence`, `weak-spec`, `unsupported`, `engine-missing`, `timeout`, `inconclusive`) do not sit in that order; they propagate upward as flags and open-item counts alongside it. `owed-evidence` is the debt half of `conditional` (§5.5, added to this list 2026-08-25 — it was being emitted before it was defined): `conditional` says the verdict rests on an assumed contract, `owed-evidence` says nothing has yet checked that contract against the real body. They are two facts, and a run that discharges the second keeps the first. | A proof in one corner must not hide a merely-tested boundary in another; and a timeout is not a weaker proof, it is a different kind of fact. |
| D7 | Every counterexample is stored as **Kani witness data** (the exact input bytes, engine-version-bound — input storage, not a reproduction: Kani's playback replays the function body only and never re-evaluates contract closures, so an `ensures` violation replays green) and, whenever the inputs can be rendered as stable Rust source, additionally as an ordinary `#[test]` that **asserts the postcondition explicitly** and therefore fails under plain `cargo test` — the only red artifact, and D7's repair target. The assertion is rendered overflow-safe (widened/checked arithmetic), so it fails by stating the contract, never by re-triggering an incidental panic inside the check. When rendering fails, the diagnostic says so (`W0541`) — inputs are never fabricated. **The red-test promise is qualified to failures arising in the function's own body.** A failure that depends on a *stubbed* callee's invented return (§5.5's assumed-contract branch) has no faithful plain-Rust reproduction: the rendered test calls the real callee, which never produces the stub's value, so it is emitted green — verified in `tests/spike/kani-pin/FINDINGS.md`, and unfixable by engine version. Such a failure reports `W0541` with reason `stub_substituted`, carrying the fabricated value and a proposed contract tightening, because its repair target is the declared contract rather than the code (docs/plans/d7-stub-failures.md). Rendering a red test against a rewritten body is refused: it would fail for a program the user does not run. | Playback is exact but body-only (ADR-0003 caveat 3); the portable test is the agent-friendly repair target and carries the red-test promise alone. Implemented and verified end-to-end in M3 (docs/m3-slice-findings.md): the `clamp` fixture's rendered `#[test]` fails before a contract fix and passes after, on the pinned Kani 0.67.0. |
| D8 | `synth` mode (the model writes the function body) is orchestration over the check pipeline: prompt assembly, a retry loop, and marking the output as derived. It ships last and adds no checking machinery. | Thin by design. |
| D9 | Implementation language Rust; three crates (§4); engines run as subprocesses with version detection at startup, never linked as libraries. The Kani version is pinned in `ply.toml` and recorded in every fingerprint. | Engine version churn must not break our build or silently invalidate old evidence. |
| D10 | `node_id` is the component path + item path only. The blake3 `content_hash` is a sibling field, never part of the ID. All JSON output shares one envelope (§8) containing a tree of nodes plus a flat diagnostics list. | IDs must survive edits so external consumers (a future canvas UI) can track nodes across runs. Design for the canvas now; build it never (out of scope). |
| D11 | Extraction may be incomplete but never silently so: every call site the extractor cannot resolve is counted and reported (W0412 plus a per-component coverage metric in `check` output). | Visibility is what makes the advisory tier (D4) and any future `strict` opt-in meaningful. |
| D12 | A function declares a **checks list**, e.g. `checks: [bounded(3), fuzz(256), mutate]`. `mutate` requires a `test` or `fuzz` entry in the same list (else `E0504`) and uses only those as its mutant-kill signal, scoped per function with cargo-mutants' `--re`. | One base check could not express "bounded plus a fuzz-backed mutation tier". Running Kani once per mutant costs minutes per mutant per function; proof-backed mutation needs an opt-in budget, which is out of scope. |
| D13 | **Spike before build** (milestone M0): every engine-facing detail in this spec is provisional until a hands-on spike, with the pinned Kani version, records in ADR-0003 what actually works — attribute emission, in-crate harness modules, `stub_verified`, playback, input construction. The spec is then amended to match reality. | The engine surface is the highest-risk part of the design; paper decisions there are guesses. |
| D14 | `ply.lock` (committed) records, per claim, a **fingerprint** beside the result that fingerprint earned: item token-stream hash, merged contract text, **the first-party bodies the check runs or descends into** — or, where a syntactic walk cannot bound that set, the whole of the crate's source (§5.2a) — the worked examples a `test` check asserts, the contracts assumed for the callees it crosses into, the same-crate callees it stands on rather than assumes and the bound each earned (D5's first branch, §5.5), the checks that ran, engine name + version + flags, active features, target triple, compiler version, the resolved versions of packages outside the workspace, **and Ply's own version**. `verify` recomputes the fingerprint from today's inputs *before* it uses or shows a recorded result — matches, the result is reused and the run says so on the node; differs, the check runs again and the record is rewritten. **There is no `stale` state and nothing for a human to re-bless**: the hash is the confirmation, and it is checked at every single use. Only a result that earned evidence is recorded, so no failure, timeout or absence is ever carried forward. | Committing the record is what stops CI and the next colleague re-proving what is already proven, and lets a reviewer see in a diff that a claim was checked. Re-hashing at every use is what makes storing verdicts safe at all: a stored verdict a human re-blesses can drift from the code between blessings, and every warning that accumulates faster than it is cleared ends up meaning nothing. **Ply's own version is in the hash because a fix to Ply changes what a result means.** The four defects fixed on 2026-08-25 — a harness that failed to compile earning a confident pass, an ordinary `use` import letting an unvouched-for body into a proof, an unsatisfiable declared promise passing vacuously, a claim inside a nested component skipped in silence — would every one of them have hash-matched perfectly against a record written the day before, because the source had not changed: Ply had. |

## 3. Toolchain

Rust stable, edition 2024. Cargo workspace of three crates (§4).

Internal dependencies: `syn` (features `full`, `visit`) + `proc-macro2`;
`serde`/`serde_json`; a maintained serde-YAML implementation (`serde_yaml_ng` at time of
writing — confirm at M1, record in ADR-0002); `jsonschema`; `clap`; `insta`; `walkdir`;
`toml`; `similar`; `blake3`.

External engines run as subprocesses and are detected at startup. Each is optional: a
missing engine downgrades the checks that need it, with warning `W0110` and status
`engine-missing`. **It is never reported as a failure of the check itself** — nothing
about a missing cargo-mutants says a spec is weak, and nothing about a missing Verus says
a contract is false. **It does not make the run pass, either**: the check earned no
evidence, so §1's absence-of-evidence rule applies and §6 returns exit 3. (Reconciled
2026-08-25: this paragraph used to end "It never fails the run", which contradicted §6's
own exit-3 row outright, and the implementation had split the difference — `prove` with no
engine exited 3, `mutate` with no engine exited 0. `--fail-on error` is the documented
opt-out for a codebase mid-adoption.)

| Engine | Used for | Invocation |
|---|---|---|
| rustc/cargo | ground truth: build, type/borrow check; `cargo metadata` for crate-dependency facts | `cargo check --message-format=json` |
| Kani (pinned version) | `bounded` check; contract verification; stubbing | `cargo kani -Zfunction-contracts` on in-crate harnesses |
| proptest | `fuzz` check | generated harness crate under `target/ply/fuzz/` |
| cargo-mutants | `mutate` check (weak-spec detection) | `cargo mutants --re <fn>` |
| Verus | `prove` check | translated file set under `target/ply/verus/` (M7, optional) |
| Miri | optional undefined-behavior check | `cargo +nightly miri test` (M7, optional) |

The proc-macro crate `ply-attrs` provides `#[ply::requires(expr)]`,
`#[ply::ensures(|result| expr)]`, `#[ply::pure]`, `#[ply::allow(ban, reason = "...")]`,
`#[ply::derived(spec_hash = "...")]`, and the function-like macro
`ply::unresolved!(id, "note")` (stable Rust forbids attribute macros at expression
position). `requires`/`ensures` re-emit the item with `cfg_attr(kani, ...)` attributes
(D2); the rest expand to nothing except `unresolved!`, which expands to
`unimplemented!("unresolved #id: note")` (§5.6). Claims sit next to the code and survive
refactors. `ply.yaml` can extend anything the attributes say; on conflict, `ply.yaml`
wins and Ply emits `W0111`.

## 4. Repository layout

```
ply/
  Cargo.toml                # workspace
  schema/
    ply.schema.json         # NORMATIVE definition of ply.yaml (JSON Schema 2020-12)
  crates/
    ply-attrs/              # proc macros (§3)
    ply-core/               # everything between input and output, as modules:
                            #   config/   ply.yaml loading, schema validation,
                            #             micro-syntax parsing, JSON-pointer→line map
                            #   model/    components, claims, anchors, IDs, verdicts
                            #   extract/  trait Extractor + syn backend: item index,
                            #             call graph, caps, contract harvest, anchors
                            #   arch/     crate-tier and item-tier rules
                            #   engines/  trait Engine { plan, run, parse } + adapters
                            #   harness/  codegen: kani proof modules, proptest suites,
                            #             cex tests
                            #   diag/     Diagnostic types, JSON envelope, human renderer
    ply-cli/                # cargo-ply binary: check|verify|tree|worklist|audit|
                            # synth|skill
  templates/                # harness code templates (plain format strings)
  tests/
    ui/                     # ply.yaml + Rust fixtures + golden JSON (insta)
    fixtures/               # small cargo projects used as check targets
    e2e/                    # end-to-end: fixture → engines → expected verdicts
  docs/
    SCHEMA.md               # user-facing reference for ply.yaml — hand-written, exists
                            # as of 2026-08-25; every YAML example in it is validated by
                            # running `cargo ply check` against it. Not generated: it
                            # states what this build does and does not do, which no
                            # generator can read out of the schema.
    PLY.skill.md            # agent-facing reference, generated by `cargo ply skill`
    adr/                    # one-paragraph ADRs
  .archi/                   # archi-techture diagram bundle: the design's diagrams and
                            # guided tour; the working example of the visual grammar (§7.1)
  tools/
    render/                 # static ply.yaml → SVG renderer proving §7.1 total (not the canvas)
  vetting/                  # design-vetting scenarios: real designs written in ply.yaml
                            # before the tool exists, with the findings each produced
```

Split a module out of ply-core into its own crate only when compile times or a real reuse
need demands it — never preemptively.

ADRs to write as their milestones start:
- **ADR-0001** (M2): item-level extraction stays syn-based and advisory; upgrade trigger
  for a rust-analyzer (`ra_ap`) backend: unresolved-call rate above ~20% of
  cross-component-candidate sites on real projects. rustdoc JSON rejected — it contains
  no function bodies, so no call sites.
- **ADR-0002** (M1): YAML crate selection (`serde_yaml` itself is archived).
- **ADR-0003** (M0): spike results — what the pinned Kani version actually supports
  (D13), and every spec amendment that follows from it.

---
## 5. The `ply.yaml` format

Files named `ply.yaml` (or `*.ply.yaml`) are discovered from the workspace root downward
(lexicographic path order; `target/` and gitignored paths excluded) and merge into one
model. Merge order cannot matter: duplicate component names are errors. **The JSON Schema
at `schema/ply.schema.json` is the normative definition of the format**; this section is
its prose rendering, and any divergence is a bug in this section. The schema is embedded
in the binary, shipped in the repo, versioned by the top-level `ply: 1` field, and
golden-tested. `cargo ply skill` embeds it in the generated skill file. The embedded
copy is authoritative: Ply never reads a schema from the target workspace, so editing an
on-disk copy changes nothing at runtime — the shipped file exists as read-only reference
and for IDE tooling. The grammar evolves only through Ply releases and the `ply:`
version field.

**The file exists as of 2026-08-25 (Phase 1a), and this paragraph describes what it
does rather than what it should.** For most of this project's life the sentence above
was false: there was no `schema/ply.schema.json`, and the key vocabulary lived as Rust
constants in two places. It is now load-bearing in the way "normative" has to mean
something: `ply-core` embeds it with `include_str!`, and the accepted-key set every
reader enforces (`E0204`) is read out of the schema's `properties` at runtime, so
deleting a key there changes what Ply accepts. Two constraints are declared in the
schema as regexes and enforced at runtime by hand-written matchers instead — the
check-string form (`parse_check`) and the code-path form (`is_valid_path_form`), so the
shipping binary carries no regex engine — and each is held to the schema by an invariant
test that walks a corpus and fails on the first string the two disagree about
(`crates/ply-core/tests/schema.rs`). That test is what caught the first real divergence:
`fuzz(0256)` and `fuzz(+5)` were accepted by the parser (Rust's `u32::from_str` takes
both) and refused by the schema. The parser narrowed to the schema, not the other way
round. `cargo ply skill` does not exist yet, so the clause about it remains a plan.

Everything is declarative data. The only embedded syntaxes are:
1. **check strings** — `test | fuzz(N)? | bounded(K)? | prove | mutate`.
   Schema-validated by regex, parsed in ply-core.
2. **edge strings** — `A -> B` declares that component A may call component B (checked).
   `A ~> B : path::Type` declares a data flow (parsed and rendered, NOT checked in v1).
3. **deny strings** — `PAT -> PAT [except C1, C2]` where `PAT := IDENT | *`.
4. **Rust expressions** — contract strings restricted to the closed subset (§5.4a);
   example strings are arbitrary Rust expressions (they compile as plain tests).

### 5.1 Document structure

```yaml
ply: 1                           # schema version, required

externals:                       # optional; named outside parties (§5.1a rule 6
  venue:                         #   namespace; §5.3 edge rules; §7.1 visual form)
    note: "the exchange: accepts orders, returns fills"   # required — no bare names

components:
  pricing:                       # component name (unique across ALL merged files → E0202)
    anchor: pricing              # crate name, or crate::module::path — required
    pure: true                   # optional, default false: no capabilities at all
    strict: false                # optional: item-level violations become errors (D4)
    uses: [time]                 # optional; caps: net fs db time rand proc unsafe
    owns: [pricing::Book]        # optional; only this component may mutate these types
    state:                       # optional; the structure this component's state lives in
      of: Book                   #   required inside `state:` — resolved under the anchor
      show: [quotes, curve]      #   optional; the fields worth drawing. Omitted = none
    profile: hot_path            # optional; must name a declared profile
    checks: [bounded(2)]         # optional default checks for all fns in scope
    components:                  # optional nested components, same shape
      curves:
        anchor: pricing::curves
    fns:                         # optional fn claims, keyed by path relative to anchor
      quote:                     # impl methods use Type::method
        checks: [bounded(3), fuzz(256), mutate]
        mode: check              # check (default) | synth
        requires:                # optional; Rust expression strings, ANDed with
          - "inst.tick > 0"      #   any inline attributes
        ensures:
          - "|result| result.bid <= result.ask"
        examples:                # optional; each a Rust `==` expression string
          - "quote(Instrument { id: 1, tick: 2 }).bid == 4"
        check_with: { T: u64 }   # optional; concrete types for generic params (§5.4b)
        trusted:                 # optional; externally-attested claims (§5.4d)
          - { claim: "cross-thread safety", evidence: "loom test tests/loom_quote.rs" }
        unresolved:              # optional registry links for markers in this fn
          - { id: 147, note: "employee discount undecided" }
        entry: [venue]           # optional; names of externals that can reach this
                                  #   fn — its requires become environmental
                                  #   assumptions on the audit surface (§5.3)

edges:
  - pricing -> parser
  - "pricing ~> risk : pricing::Quote"
  - "pricing ~> venue : pricing::Quote"   # ~> only — a `->` or `deny` naming an
                                          #   external is an error (§5.3)

deny:
  - "* -> db_raw except migrations"

profiles:
  hot_path: [no_panics, no_trait_objects]
  # available bans: no_unsafe no_trait_objects no_interior_mut
  #                 no_panics no_async exhaustive_match

unresolved:                      # registry entries with no code anchor
  - { id: 151, note: "settlement rounding rule TBD" }

routes:                          # optional; §5.4b's generator hook — bare type name to
  Handle: open_handle            #   a public function (free or associated) that returns it
  OsString: std::ffi::OsString::from(String)  # outside the crate: declare its input type(s)
```

Schema violations produce `E0201` diagnostics carrying the JSON-pointer path and source
line. Line mapping: serde derive builds the model; a second lightweight pass over a
position-marked YAML parse builds a JSON-pointer → (line, col) index used only for
diagnostics. **The pointer is implemented; the source line is not** (2026-08-25): every
`E0201`/`E0204` carries an exact JSON pointer, and the human sentence says where in the
dotted form a reader scans a YAML file in (`components.pricing.fns.quote.ensure`), but
the position-marked pass does not exist, so no diagnostic carries a line number. A
guessed line is worse than none — it sends a reader to the wrong place with full
confidence — so none is emitted. Duplicate component names across merged files → `E0202` (nested names are
qualified by their parent, so only siblings can collide). A string that passes the schema
regex but fails the real micro-syntax parser → `E0203`, stating the expected form.
`mutate` without a `test` or `fuzz` entry in the same checks list → `E0504`.

**Externals.** A top-level `externals:` map declares named outside parties — systems
or people this codebase talks to but Ply can never verify: an exchange, a payment
processor, a human operator. Each entry is a name plus a required `note:`; there is
nothing else to declare, because an external carries no verdict, no ceiling, no
checks, and is not a node of the §7 verdict tree — the kernel never sees it.
Externals are top-level only (no interior, cannot nest) and share the component
reference namespace (§5.1a rule 6): a name collision with a component or another
external is `E0202`; a bare reference ambiguous between an external and a component
leaf is `E0206`. An external may appear only as an endpoint of a `~>` flow edge, or
be named in a fn's `entry:` list — never in a `->` call edge or a `deny` pattern
(§5.3). `entry: [name, ...]` on a fn claim names the externals that can reach it:
each name must resolve to a declared external (`E0209` otherwise), and the fn's own `requires` clauses
become environmental assumptions Ply cannot discharge (nothing inside the workspace
calls the fn, so no in-workspace caller ever checks them) — listed on the future
`cargo ply audit`'s trust surface, never counted as an open item, and never changing
the fn's own verdict. An external declared but named by no `~>` edge and no fn's
`entry:` list is `W0410` — nothing in the document says how it connects. An external is never
fingerprinted and never recorded (§5.2a): there is no body, no contract and no evidence
string to hash, so the tooltip and the (future) audit line always say plainly
"declared, never checked by Ply" rather than pretending otherwise.

### 5.1a Strictness & lexical rules

The schema must encode all of the following; the goldens (§9) pin them.

1. **Unknown fields are errors.** Every object in the schema sets
   `additionalProperties: false`; an unrecognized key → `E0204` with a
   nearest-known-key suggestion. A typo must be caught, never ignored. **This binds
   every tool that reads a `ply.yaml`, not only `check`** (2026-08-25): until then
   `ply-check` enforced it while `cargo ply verify` read the same document with plain
   serde and dropped whatever its own structs did not name, which is how a team's
   external `ensures:` for a legacy callee reached no engine and raised no warning
   (vetting 004 finding 7). Two tools disagreeing about one document is the defect; the
   rule is the file's, not a command's. The converse binds equally: a tool must accept
   **every** key §5 defines even where it acts on none of them — one document is read by
   `verify`, `check` and `render`, and a reader that refuses the keys it ignores breaks
   that outright.
2. **Identifiers.** Component and profile names match `[a-z][a-z0-9_]*` (snake_case,
   ASCII) — and external names too, since they share the namespace (rule 6). Enforced at
   load time since 2026-08-25; a name outside the pattern is `E0201`. Before that the
   rule was stated here and checked by nothing, so `components: { Pricing: ... }` loaded
   silently. In edge and deny strings, tokens are separated by one or more spaces; the
   parser accepts any run of whitespace and the canonical form uses single spaces.
3. **Code paths.** Anchors and fn keys are plain segment paths: `IDENT(::IDENT)*`, where
   a segment may also be a type name in `Type::method` position. No generics, no
   trait-qualified paths (`<T as Trait>::f`), no lifetimes. An anchor or fn key outside
   this form → `E0304 unsupported path form`, naming the construct.
4. **Numeric bounds.** `fuzz(N)`: 1 ≤ N ≤ 1_000_000. `bounded(K)`: 1 ≤ K ≤ 64. Out of
   range → `E0203`.
5. **Unresolved ids** are positive integers, unique across the whole merged workspace
   (registry and fn entries together); a duplicate → `E0205`.
6. **Component references** in edge and deny strings use the component namespace, not
   Rust paths: a bare name resolves only if it is unique across the whole merged tree;
   otherwise the dotted qualified form `parent.child` is required. An ambiguous bare
   reference → `E0206`, listing the candidates. (Discovered in vetting: two parents may
   legally have same-named children, so `A -> B` alone can be ambiguous.) Externals
   share this same namespace and this same rule (§5.1) — they are top-level only, so
   a dotted form never applies to one, but a bare external name is exactly as
   resolvable, and exactly as ambiguity-checked against every component leaf, as a
   bare component name is.

### 5.2 Anchoring

Every component anchors to a real crate or module; every fn claim anchors to a real
function. `ply check` resolves anchors via the extractor. An unresolvable anchor →
A crate with no `src/lib.rs` is a third case, and the one that used to read backwards.
`check` counted every claim in such a crate as *anchored to another crate* — the shape of
a deliberate boundary — and exited 0, while `verify`, asked about the same crate, refused
the same claims with `E0301` and exited 1. Nobody had declared a boundary; there was
simply no library to look in. A binary-only crate (`src/main.rs` alone) therefore got a
clean bill from the fast command and a refusal from the slow one. Under §1 the command
that looked at nothing is the one that must not come back happy, so `check` now reports
every such claim as unresolved, names the missing `src/lib.rs` as the obstacle, and its
summary line says a search did not happen rather than reporting a zero that could be
mistaken for zero problems.

**A component may carry a `note:`; a function may not.** Every prose slot in this
grammar sits where checking is impossible -- an outside party (`externals`, whose note is
required because "a bare name tells a newbie nothing"), an unmade decision
(`unresolved`), a human's word (`trusted`). A component's rationale is the fourth: why a
crate must stand alone, why one layer may never reach into another, has no contract form
and no engine will ever consume it, and it is exactly what a reader or an agent needs.
Ply's own document was forty lines of comment to twelve of configuration, every one of
them discarded by the parser -- the format inviting prose and then throwing it away.

Not offered on a function, and the reason is a failure observed in use rather than a
principle: someone wrote a real invariant as an `examples` string -- a test case wearing
a specification's clothes -- because no better slot was visible. The answer there is
`ensures`, which both a reader and an engine can act on. A prose slot beside it would
make that mistake comfortable rather than correcting it. What a function does belongs in
its contract, and where there is no contract the drawing already says so.

**The envelope carries what was promised, not only what came of it.** §7.1's contract
mark says inline attributes "join when `cargo ply` emits the §8 envelope", which only
means something if the envelope carries clauses; it did not. A node now carries its
effective `requires`/`ensures` -- declared and inline, merged -- and any `trusted` claims
with the evidence named for each. Both are set from the claim rather than from the run,
so a carried-forward verdict says the same thing a freshly earned one does. Additive, so
a reader of the old shape is unaffected.

`E0301` with nearest-name suggestions (edit distance over the item index). **A renamed
function must break CI, not silently orphan its claims.**

**A claim's anchor is resolved by the same walk that classifies a call (§5.5), and that
sentence is load-bearing.** It was false as written until 2026-08-25: call classification
followed `use` imports, inline `mod`s, file modules and re-exports, while anchor
resolution read `src/lib.rs` and walked its top-level items only. So `verify` would name
`rates::legacy_rate` as a callee nobody had vouched for and, in the same envelope, reject
with `E0301` the claim written to vouch for it. That closed off the per-function-promise
route for essentially all real legacy code, which lives in `rates.rs` or `pricing.rs` or
inside a `mod` and almost never at the top level of a crate root. One resolver now answers
both questions, so a fn claim may be keyed on any path a reader of the crate would write
(`rates::legacy_rate`, `pricing::caps::cap_bps`), and the same function reached by two
spellings — a claim written `rates::legacy_rate`, a call written `legacy_rate` after a
`use` — is recognised as one function. The item index behind `E0301`'s suggestions is the
same set, for the same reason it always was: a suggestion naming something resolution
would then refuse is worse than no suggestion.

**Every claim in the document is read, at every depth (2026-08-25).** Components nest
(§5.1), and `verify` iterated the top level of that tree only: a claim written inside a
nested component earned no node, no diagnostic and no mention of any kind, while `check`
walked the whole tree and reported the same claim as pointing at real code. The document
looked correct and the claim never ran. Both commands now walk the whole tree, and both
name a claim the same way — the component's qualified name and the fn key,
`ingest.book::OrderBook::apply`.

What both commands can *resolve* stays narrower than what the grammar can express: a fn
key is read as a path from the crate root, so a claim under a component anchored at a
module of this crate (`anchor: ingest::book` while verifying `ingest`) cannot be resolved
from the key as written. It is reported as not run (`W0303`) with the crate-root spelling
that would run — `book::OrderBook::apply`, under a component anchored at `ingest` — and
never as a missing function. Anchor-relative key resolution is not built.

One case stays closed, and it is not a limit of the walk. Ply's generated harness is a
module at the crate root, so a **private** item below the root is a name that harness
cannot write. Such a claim is refused with `E0301` naming the actual obstacle — the
private `fn`, or the private `mod` between it and the root — never "no such function",
which would send a reader hunting for a typo that is not there. Items private *at* the
crate root are unaffected: the generated module is a child of the root and sees them.

### 5.2a Recorded results and reuse (D14)

`verify` records what it earned, and re-earns it whenever anything it depended on
changed. The record is `ply.lock`, it sits beside `ply.yaml`, and **it is committed** —
not a throwaway cache. Committing it is what stops CI and the next colleague from
re-proving what is already proven, and it puts "this claim was checked, and here is the
fingerprint of what it was checked against" into a diff a reviewer reads.

**What the fingerprint covers**, for each claim, in this order:

1. the checked function's own source (its token stream, so formatting and comments do
   not count as change but every token does);
2. its contract — the inline `#[ply::requires]`/`#[ply::ensures]` and anything `ply.yaml`
   declares for it;
3. **the code the check actually runs or descends into** — see the paragraph below, which
   states this input and its one limit precisely;
4. the worked `examples:` a `test` check compiles into assertions: each one *is* part of
   what that check asserts, so editing one is editing the check;
5. the promises assumed for everything it crosses into: each stubbed callee's canonical
   path and the contract text assumed for it (§5.5's second branch). A caller's proof is
   *about* those promises, so a promise edited in `ply.yaml` re-runs every caller resting
   on it;
6. **the same-crate callees a claim stands on rather than assumes, and the `bounded(k)`
   each earned this run** (D5's first branch, §5.5, added 2026-08-26). Composing a
   caller's bound against a callee's own depth is depending on that number: before this
   input existed, editing only a callee's declared `checks:` — its bound moving with no
   source touched anywhere — re-earned the callee's own record correctly while the
   caller's record, and its now-stale composed bound, went untouched;
7. the checks that were run, as written, plus the `fuzz` seed that drew the cases
   (§5.4c) — a `--seed` that replays a different run must not match a record written by
   the derived one;
8. the engine behind each of those checks: name, version, and the flags that shape the
   obligation it discharged;
9. the build target triple, the compiler that builds for it (D9's "an old success must
   not bless ... a different toolchain"), and the crate's declared feature table — Ply
   passes no `--features`, so the set that is active is the default set that table
   defines;
10. the resolved versions of every package outside this workspace that the crate depends
    on, as the lockfile pins them. A `bounded` proof descends into registry code and every
    `fuzz`/`test` run executes it, so `cargo update` changes what was checked. Where there
    is no lockfile, Ply records that fact instead of a version list, and a result recorded
    with one never matches a run without one;
11. **Ply's own version.**

**Input 3 is the one that has to be stated carefully, because it cannot always be
narrow.** A check does not run the claimed function alone: it runs whatever that function
calls, and a `bounded` proof reads those bodies. So the fingerprint takes the token
stream of every first-party function the claim can reach — following calls *and* plain
mentions of a function by name (`map(helper)` never writes `helper(..)` and still runs the
body) *and* the functions named inside the claim's own contract expression, which run on
every generated case — transitively, stopping at a callee replaced by a declared promise
(whose promise is hashed instead, input 5) and at anything outside the workspace (covered
by inputs 9 and 10). That walk is syntactic, and a syntactic walk cannot follow a method
call, an operator that some `impl` defines, a macro expansion, or a trait method reached
under another name. So the walk is trusted **only** under conditions that make all of
those impossible: every item in first-party source is a function, a module, an import, a
type alias, or a plain data type with only `std` derives; no reached body invokes a macro;
and no reached function carries an attribute Ply does not recognise, since an attribute
macro can replace a body with anything at all. When any of that fails, Ply hashes **every
line of first-party source in the crate and its path dependencies** instead, and says so
inside the hash. That is coarser — any
edit anywhere in the crate re-earns every claim in it — and it is never wrong. The
condition is an allowlist on purpose: an item kind nobody anticipated costs engine time,
where a denylist would have cost a user a green verdict over code nobody checked.

**The coarse mode explains itself, once.** Under it, "the code it runs changed" is true
of an edit in a function the claim never calls, so a run that only announced the re-run
would leave the reader with no way to tell a real dependency from the widening. When a
claim displaced this way is reported, Ply names the construct that cost it the walk —
the `impl` block, the macro, the unrecognised attribute — and says that the crate is the
unit now. The reason belongs to the crate rather than to any one claim, so it is stated
once for all the claims sharing it, naming them; the same paragraph repeated per claim
would bury the list it exists to explain. It is never printed for a bounded walk, where
it would be false.

**What it does not cover, stated rather than implied.** Environment that shapes a build
without appearing in any file Ply reads — `RUSTFLAGS`, `[profile]` settings such as
`overflow-checks`, a `#[path]` module attribute — is not an input. Neither is anything a
proc macro from outside the workspace expands to beyond the identity of the crate it came
from. And a hash cannot defend the record against a text editor; see the closing
paragraph for what is done about the honest version of that.

Ply's own version is in there for the reason D14 gives, and it is the input that makes
this scheme sound rather than merely fast: a defect fixed in Ply changes what a recorded
result *means*, and every result recorded by the previous build would otherwise
hash-match perfectly, because the user's source did not change. Putting the version in
the hash invalidates exactly the results a tool fix should invalidate, automatically,
with nobody having to remember which release carried which bug.

Two inputs are deliberately *not* in it. The per-check wall-clock budget
(`--engine-timeout`) is not: a proof that finished inside 300s is not made false by a
later run that would only have allowed 60s, and folding the budget in would re-pay every
proof in a CI job that sets a different one. And the *result* of an engine run is not an
input to its own fingerprint, which is why only results that earned evidence are stored:
a timeout, an absence of any kind, and a violation are never recorded, so nothing that
failed can be carried forward, and a timing-out function re-pays its cap on every run
(§5.4c).

**The honesty rule: a recorded result never reaches a user's eyes without being
re-hashed first.** That is the whole difference between this and a file of remembered
verdicts. The record cannot drift out of agreement with reality, because agreement is
what is checked at the moment of use — every use, every run, with no command in between
and nothing for a human to confirm. It follows that there is no `stale` status, no
`W0302`, no `cargo ply accept` and no `E0303`: a claim is never in a state of "recorded
but possibly no longer true", because the recorded result is either used with its
fingerprint verified or thrown away and re-earned.

**A reused result says so.** The §8 node carries `reused: true`, and the printed tree
marks that node `[reused]` beside its verdict, glossed in plain words beneath the tree
the way `[assumed]` and `[evidence owed]` are (§6). A person reading `bounded(2)` can
therefore tell whether it happened just now or was carried forward from an earlier run
whose inputs still hash the same. Reuse is a fact about *when the run happened*, not a
qualifier on the evidence, so it is not a D6 status: it never enters the evidence order,
never propagates upward, and never changes an exit code. What is reused is the whole
result — verdict, statuses, evidence block and the diagnostics that came with it — so a
reused conditional verdict still prints the assumption it rests on, word for word.

**The file holds only what the last run stood behind.** Every entry the run neither
reused nor earned is dropped: a claim deleted from the document, a claim whose function no
longer resolves, a claim this run checked and got no evidence for. None of them could ever
be reused — their fingerprints cannot match — and leaving one there would show a reviewer
a verdict the run did not produce, which is the remembered opinion this design refuses.
A consequence worth stating: a machine that cannot reproduce a result (no engine
installed, a different toolchain) drops it rather than keeping somebody else's, so two
machines with different toolchains take turns rewriting the record. That is inherent in
one entry per claim.

Two gaps, stated rather than implied. The `fuzz` tier's engine version is the version
*requirement* Ply writes into the harness crate it generates, not the version cargo
resolved: a patch release of the sampling library that changed how a strategy draws would
keep that string, so a record written before it can be reused after it. And the
fingerprint guards a stored result against its inputs *moving*, not against somebody
editing `ply.lock`. Nothing short of signing could, and signing is out of scope — but the
honest version of that mistake is caught: a stored verdict must be one the checks
recorded beside it could actually have earned (`fuzz` can never yield `proved`), and one
that could not is refused with `W0516`, said out loud, and the claim checked again. One
exception, added with D5's first branch (§5.5, 2026-08-26): a `bounded(k)` check earns
not only `bounded(k)` but any `bounded(j)` genuinely composed against a shallower
same-crate proof — and the exact value that composition must equal is pinned, not
merely bounded above. **The rule is equality against a computed number, never `j <= k`
on its own**: the expected value is `min(k, the shallowest bound among every callee
this claim stands on)`, read from the fingerprint's twelfth input (`verified_bounds`),
and a stored verdict must match that number exactly. A same-day widening of this rule
to "any `j <= k`" was itself found unsound by a second adversarial review (2026-08-26):
it accepted a hand-edited `bounded(4)` for a claim whose real composed value was
`bounded(2)`, because `4 <= 5` (the claim's own declared bound) was the only thing the
looser rule checked — a tampered record that widened, rather than merely stayed within
the declaration, slipped through. `W0516` learning the first version of this the hard
way (a caller standing on a proved callee was refused as "impossible" and silently
re-verified from scratch on every single subsequent run, until that was found and
fixed) is why this sentence names the exception rather than leaving it implicit; the
second finding is why the exception is equality, not a ceiling.

**A result that could not be carried forward says which input moved.** When a claim has a
recorded result whose fingerprint no longer matches, `verify` names it and names the
inputs that changed — "the code it runs", "the compiler and the build target" — in the
`not_carried_forward` array of §8's envelope and in one block beneath the printed tree.
A silent full-price re-run is the experience the record exists to end, and it must not
reappear unexplained the first time a compiler updates.

Deleting `ply.lock` re-runs everything; there is no `--force` flag.

### 5.3 Architecture semantics (always-on, M2)

**Crate tier (sound, errors).** From `cargo metadata`: the exact crate dependency graph.
A dependency between two crates belonging to different declared components with no `->`
edge → `A0401`. `deny` patterns over components resolve against this graph → `A0405`.

**Item tier (approximate, warnings; errors under `strict: true`).** From the syn-backed
extractor, behind `trait Extractor` (ADR-0001):

- `calls(from_item, to_item)` — direct calls, method calls, and fully-qualified calls,
  resolved best-effort through `use` maps and local type declarations.
- `calls_dyn(from, trait)` — dynamic dispatch; reported `W0411` if the trait has
  implementations in a component the caller may not reach.
- `calls_unresolved(from, span)` — call sites the extractor cannot place (D11).
- `touches_cap(item, cap)` — capability approximation: uses of
  `std::net`/`std::fs`/`std::process`/`std::time`, rand crates, and `unsafe` blocks,
  plus a user-extensible `capmap.toml` mapping crate paths to caps.
- `mutates(item, TYPEPATH)` — mutation of a named type.

Containment implies permission: a component may always call into its own descendants
(and they into it), the same way it calls between its own functions — no edge is
declared for calls within one nesting line. An explicit edge whose endpoints are a
component and its own descendant is redundant → `W0409` (document-local; `ply check`
reports it, and the renderer draws nothing for it — vetting 003's parent-to-child edge
drew as a diagonal slash through the parent's own content, which is what a drawing of
"I may call myself" deserves). Edges are for crossings between nesting lines, including
into another component's descendant (`strategy -> ingest.book`).

Item-tier rules (each `W`-severity by default, `A`-severity error under `strict`):
1. A call crosses two declared components with no `->` edge → `A0402`.
2. A `pure` component touches any capability → `A0403` (names the cap, spans the item).
3. A component reaches a capability outside its `uses` set through its own code, rather
   than through a declared `->` edge into a component that has the cap → `A0404`.
4. `owns T`: an item outside the owning component mutates `T` → `A0406`.
5. Profile bans (syntactic checks over the component's items — these are reliable and
   always errors) → `A0407`.

**`state:` — the structure a component's state lives in** (2026-09-03). `owns` answers
"who may change this type"; `state` answers the question a reader asks first, which is
"what does this component *hold*". It names one type and, optionally, the fields of it
worth drawing:

```yaml
components:
  book:
    anchor: ingest::book
    state:
      of: OrderBook          # resolved under this component's anchor
      show: [bids, ticks]    # the fields a reader should see; omitted draws none
```

**The document names, the code says what.** `show:` lists field *names* only — never
their types, never their shapes. Ply reads `OrderBook` from source and draws each named
field as whatever it actually is. Writing the shapes in the document would be a second,
hand-maintained copy of a fact the compiler already owns, and it would drift the first
time somebody changed a field: the exact rot this project has spent its documentation
budget removing. A field's *name* is a stable thing an author chooses; its type is not
theirs to restate.

**Not every field.** A real state struct has twenty fields and two that matter. `show:`
is the author saying which two. This is the one place in the grammar where Ply draws
less than it knows, on purpose.

**Where the fields appear.** The box draws `state T — N of M shown` and then one row per
field: its shape glyph, its name, and its type as the source spells it. `N` is what was
drawn and `M` is what the type really has, both counted from code — so a deliberate
selection of two fields from twenty never reads as a small type, and neither number
restates the document. The count is drawn *only* when it was measured: a document
rendered with no code under it draws `state T` alone rather than a number Ply invented.

The type column is one column per box, set by its longest field name. A ragged column is
read a row at a time; an aligned one is read as a column, which is the reason to draw
rows rather than a comma list at all.

**What rows cost, measured.** A row is height, and height moves arrows. Before edge
routing had lanes, adding the header line alone to `vetting/003` took the crossing
ratchet from 4 overlapping lines to 6. With lanes (§7.1's routing paragraph), four of
that document's five candidate components take a full `state:` at zero overlaps, and the
fifth still costs two — bisected one component at a time, and left out with the reason
recorded in the document itself. Height is a real budget, and the honest way to spend it
is per component with a measurement beside each, not by a rule declared once.

Three things are checked. The first two are cheap resolution facts rather than
behaviour; the third is the admission that the first two could not run:

1. `of:` names a type Ply can find under the component's anchor → else `A0414`.
2. every name in `show:` is a field of that type → else `A0415`, naming the field and
   the fields that do exist.
3. Ply could not find a crate to resolve against at all → `W0413`, a warning that says
   the claim went unchecked. A document Ply cannot check its `state` lines against must
   say so out loud; the alternative is a silent exit 0 that reads as verification.

**Where a state type is resolved.** Under the component's anchor, and nowhere else. The
anchor's first segment names the crate when the document spans a workspace
(`ply_core::visual`); the rest is a module path inside it, and the type must be declared
at or under that module. A type in a module *below* the anchor counts — a component
anchored at `visual` may name a type declared in `visual::svg`, which is still its own
code.

That restriction is the point rather than a detail, and it was added after being
measured missing (2026-09-03). A crate-wide scan finds a type of the right name wherever
it sits, so a component that misfiles one passes: the type really exists, nothing looks
wrong, and only the *attribution* is false. That is the failure one level subtler than
the one `state:` was built for — and this section already promised it could not happen,
which made the promise the untrue part. `ply_core::kernel` claiming a type declared in
`ply_core::diag` exited 0 before the rule, and fails by name after it.

Crates are found by walking three directories below the document for a `Cargo.toml` with
source beside it, keyed by that manifest's package name with dashes underscored. **A
binary-only crate counts**: its modules are real code a component can anchor at, and
refusing to look at them would mean a command-line crate could never say what it holds,
for no better reason than that its root is `main.rs`. Two approximations remain, both
failing the safe way — toward `W0413` rather than a false clean. A crate that renames its
library with an explicit `[lib] name` different from its package name is keyed by the
package name (`vetting/004`'s own two crates do exactly this). And a crate reached as a
*dependency* rather than as a sibling under the document is not walked at all, which is
why `vetting/004`'s legacy component carries no `state:`.

Three things are **not** checked, and the tier table in SCHEMA.md says so: that the
component actually holds a value of that type, that no other component holds one, and
that the fields named are the important ones. `state` is a declaration in the sense
`owns` is — its value is that it is written down, drawn, and kept honest about
existence.

**What it earns beyond the picture.** A component whose declared state cannot be built
is the reason its functions come back `unsupported`, and today that connection is only
visible by reading a diagnostic. Drawn, it is the first thing a reader sees. This is
also the grammar's natural home for a future type invariant (§5.4c's "type invariants
are assumed, never asserted"): the fields are already named, and the receiver machinery
already builds constructor-plus-mutator sequences that such an invariant would be
checked across. That is recorded as the next step, not claimed here.

The per-item escape `#[ply::allow(name, reason = "...")]` accepts a ban name or an
item-tier diagnostic code (`A0402`–`A0404`, `A0406`, `A0408`) and suppresses that finding
on that item. Without it, the first false positive from the approximate extractor under
`strict: true` would force the whole component back to advisory. Every escape is recorded
in the audit list.

**Resolution visibility (D11)**: `check` output reports, per component, the share of call
sites the extractor resolved. An unresolved call site whose textual candidates include an
item in a component that would need an undeclared edge → `W0412 possible undeclared edge
(call unresolved)`. Unresolved sites with no such candidate are counted but not itemized.
SCHEMA.md must state plainly that item-tier facts are approximate and how `strict`
changes severity.

Edges constrain **direct** calls only: `A -> B` and `B -> C` neither grants nor requires
`A -> C`.

**External edges.** Every solid arrow in a Ply diagram is a checked claim (a `->` edge,
crate-tier or item-tier); every dashed arrow is declared-not-checked (`~>`, §5 item 2:
"parsed and rendered, NOT checked in v1"). An external endpoint can never be checked —
there is no crate, no item, nothing Ply's extractor can see — so routing it through
anything but the dashed form would be a new "looks checked but isn't" surface, which
this project refuses on principle (§1). Concretely: a `->` call edge naming an external,
or a `deny` pattern naming one, is an error (`E0207`) whose message says why and points
at `~>`/`entry:` instead — Ply cannot verify a call into code it cannot see, and cannot
enforce a ban on a system it cannot observe. A `~>` flow needs at least one workspace
endpoint; `external ~> external` describes the outside world talking to itself, which is
none of this codebase's business to declare, and is `E0208`.

### 5.4 Contract semantics

The canonical contract source is the inline `#[ply::requires]`/`#[ply::ensures]`
attributes on the function (D2). `requires`/`ensures` entries in `ply.yaml` are ANDed in,
for teams that prefer external specs. Because the attributes annotate the real function,
rustc type-checks contract expressions whenever the crate builds under `cfg(kani)`; under
plain cargo they are inert.

**"ANDed in" became true on 2026-09-03**, and the gap before that is worth recording
because of what kind of gap it was. The document's clauses were read, drawn, written into
the transcript and offered to callers as a boundary assumption — and never checked against
the function they were written for. A warning said so on every run, which made it
disclosed rather than hidden, but disclosure is not checking: a passing example beside a
false `ensures:` came back clean. That is this specification's own central failure mode, a
promise that reads as checked and is not, sitting in the file whose entire purpose is that
its claims are checked.

Three properties of the merge, each chosen against a way it could have gone wrong:

- **Both sources hold.** A document clause is ANDed with an inline attribute, never
  substituted for it; several clauses in one list are a conjunction. A clause that
  quietly replaced an inline one would be a different silent drop from the one being
  fixed.
- **Nothing half-merges.** Every clause is parsed before any is applied, so a function
  whose second clause is malformed keeps the contract it had rather than a partial one.
  A partially-applied contract is checked against something nobody wrote.
- **A clause Ply cannot read is refused by name** (`E0505`), and the function's checks do
  not run. Dropping it silently is the behaviour this change exists to end, and running
  the *rest* of the contract while dropping one clause would be the same failure wearing
  a smaller hat.

Clauses are conjoined with parentheses at every step, because they are the author's own
text: `a || b` and `c` joined without them would silently become `a || (b && c)`, a
different promise from the one written. Where two `ensures:` clauses name the returned
value differently (`|result|` against `|r|`), the later one is renamed to match the first
rather than refused — refusing would be a new way for a valid document to stop working.

#### 5.4a Spec expression subset (contracts only)

This subset applies to `requires`/`ensures` — the expressions sent to proof engines.
`examples` entries are exempt: they are arbitrary Rust `==` expressions, compiled as
plain `#[test]`s and never translated for an engine.

**Declaring `examples:` is not the same as running them (added 2026-09-01).** Only
`test` compiles an entry into a real assertion; `fuzz` reads one only when it seeds a
shape it could not otherwise build at all (an unbuildable receiver constructor or plain
parameter — §5.4c), and `bounded`/`prove`/`mutate` never read `examples` at all. A
function whose declared checks include none of these earns whatever those checks find
while its examples silently do nothing — worse than plain neglect, because §5.2a still
reads and fingerprints them as part of what the claim's result depends on, so editing a
never-run example still re-earns the claim, exactly as though it mattered to the
verdict. `verify` now warns (`W0525`) whenever this is so, naming how many examples will
not run and that `test` is what makes them run — a warning, not a verdict change: the
declared checks that did run are reported exactly as they would be without this
disclosure. The rendered drawing already carried this exact disclosure on a function's
own tooltip; `W0525` is the same fact, said once, reaching the terminal too.

Boolean Rust expressions over the function's parameters and `result`; literals (integer,
bool, char, string); calls to `pure`-marked helper fns; `==,!=,<,<=,>,>=`; `&&,||,!`;
arithmetic; field access; `.len()`; `.is_ok()/.is_err()/.is_some()`; `matches!()`. The
list is closed: any construct outside it — indexing, other method calls, paths to
constants, closures other than the `|result| expr` ensures form — is rejected with
`E0501` naming the construct. No side effects (checked syntactically). Identifiers must
resolve to parameters or `result`, sanity-checked against the anchored signature.

`old(expr)` — the value `expr` had on entry — is admitted in `ensures` only, with `expr`
itself drawn from this subset and `old()` non-nested. It exists because single-state
postconditions cannot correctly specify anything that mutates (vetting 001: the
push/pop contracts flagged correct code without it). Engines: Kani maps it to
`kani::old(expr)`; generated test/fuzz harnesses evaluate `expr` before the call and
substitute the snapshot (implemented 2026-08-25 — before that the clause reached the
generated harness verbatim and the harness crate did not build). `old()` in `requires`
is meaningless and rejected (`E0501`).

**Honest limit as of 2026-08-25.** The mutating shape `old()` was introduced for cannot
be checked at all, because §5.4b's supported signatures stop at a shared `&T`: a
parameter the function writes back through is not a value either engine can construct
and observe, so such a function is `unsupported`/`V0505` naming the parameter and its
type. `old()` is therefore usable today only over values the function *reads* — the
distinction v1 supports is "the entry value of an argument", not "the state before and
after a mutation". Lifting that needs `&mut` in the supported set, which for Kani also
needs `modifies` clauses; it is not in this build.
Full two-state/model-based specs (sequence histories, FIFO ordering) remain out of
scope — `old()` is the single two-state primitive.

A `#[ply::pure]` helper called from any contract is a trust surface, not a free pass. It
is always checked for capability use — a pure fn that touches any capability is an error
(`A0408`), regardless of `strict` — and every contract-used helper appears in
`cargo ply audit` as a trusted assumption. If the helper does not itself carry a passing
check, every verdict whose contract calls it is `conditional`, listing the helper: an
unverified helper that panics or lies poisons every contract built on it.

#### 5.4b Supported signatures

An engine can only check a function whose inputs it can construct — and, as the scale
spike established (`tests/spike/scale/SCALE-FINDINGS.md`, 2026-08-23), constructibility is
not sufficient: a type Kani can *build* may still be one it cannot *finish*. The list below
is evidence-backed, measured, not inferred.

**One list per engine, not one list.** Until 2026-08-27 this section named a single set
of types and both engines were held to it. That was an accident of build order: the list
was justified by a study of the *prover*, and when the sampling tier arrived months later
it inherited the prover's constraints wholesale. A tool that draws random values does not
need the constraints of one that reasons about every value at once. A type may therefore
be **sampled but not proved**, and where that is so, a `bounded`/`proved` check on it is
refused by name rather than hanging or silently returning weaker evidence under a
stronger word. The refusal says what would work instead: such a function is not
unchecked, it needs a different check.

**How the list grows is measured too, and not by counting types.** The same discipline
that justifies each entry above governs which entry comes next, because the obvious
instrument for that turned out to be the wrong one. Measured 2026-08-27 and adopted 2026-08-30
(`docs/invariant-reachability.md`), against a library designed with no knowledge of Ply --
`tests/fixtures/ratelimiter/`, written from `docs/greenfield-ratelimiter-design.md` by
someone told not to think about checkability and not told this project existed, who then
enumerated eleven properties they cared about: the share of that library's public-surface types Ply
could construct rose from 21% to roughly 80%, and the number of those eleven properties
that became checkable went from **zero to zero**. The metric moved sixty points while the
outcome did not move at all, because it counted how often a type appears rather than
whether any property could be reached; it was dominated by getters and configuration,
while the type that library's whole correctness argument rested on had a public-surface
count of zero, being internal state. Its own denominator was soft enough that two
paragraphs of one commit quoted different totals for the same measurement without anyone
noticing -- a percentage invites comparison against its own history rather than scrutiny
of what is underneath it.

Additions to this list are therefore ranked by **which single missing capability unblocks
the most properties somebody independently wrote down**, recorded per property as: whether
it is a single-function property at all, what specifically stops it, and whether its
author flagged it as risky. On the library measured, that ranking put floating point first
(built since on the sampling tier, `2443b85` -- proving over floats remains out of scope,
as the type list below states) and put `struct`s and `enum`s last -- the reverse of what type
coverage implied. It also separates two things a coverage share cannot: a property that is
**out of this tool's shape entirely** (a sixteen-thread stress test is better evidence than
any single-function check could produce) from one that is in shape but unreachable for want
of plumbing. Only the second is a gap. The first is what `coverage.not_checked` (§5.6, §5.7)
exists to report, and its count is a result to state plainly rather than a deficit to close.

None of this demotes proof itself. The evidence ladder means something only because its
strongest rungs are sometimes reached; a record of claims with no engine behind it is a
list of assertions. What the measurement demotes is the expectation that reach should
spread *evenly* -- proof earns its cost where consequence is concentrated in a small pure
surface, which is the shape §5.4b's cheapest entries already describe.

**The return type is not on this list, on either engine — retracted 2026-09-01, measured
(`docs/reach-measurement-2.md`).** From 2026-08-27 this section named a return-type gate
alongside the parameter list below, refusing a function whose return type this list does
not cover, by analogy with a parameter: `matches!(self, SelfType | Unit) ||
self.is_bounded_supported()`/`is_fuzz_supported()`, the same list, asked of the return
type too. The analogy did not hold even when it was written, and its own reasoning said
so: neither engine ever *constructs* a return value — the real call produces it — so
nothing this list exists for (constructibility, then the scale bound on top of it) has
anything to say about a type that is only ever *read back*. It stood anyway, as "a
deliberate, requested narrowing... on principle," until measured against a real library
(`semver`) rather than argued about: it alone blocked 10 of that library's 16
independently-written properties — the single largest blocker after `&str` parameters.
Measured directly before removal, on *both* engines, over a return type this list does
not model at all (`std::cmp::Ordering`): the fuzz engine earned `fuzzed(64)` on a correct
implementation and a real `violation` with a shrunk failing input on a broken one; the
bounded (Kani) engine earned a genuine `bounded(2)` proof on the correct implementation
(completed in seconds — not a timeout mislabeled) and a real `violation` with a concrete
counterexample on the same broken one. Both engines pay this gate's cost for nothing in
return, so it is gone on both, not narrowed to one: **a function's return type is never a
reason either engine refuses it.** This is the one respect in which "one list per engine,
not one list" (above) still understates it — the list below binds *parameters* per
engine; a return type is never checked against either engine's list at all.

v1 supports functions whose parameters are, recursively:

- **integers, `bool`, `char`, `Option<T>`, `Result<T,E>`** of supported types — cheap
  unconditionally (~0.1s);
- **`usize` and `isize`** — ordinary integers, with one caveat that is recorded rather
  than implied: they are pointer-width, so evidence earned on a 64-bit target is not
  evidence for a 32-bit one. The build target is already a fingerprint input, so a
  recorded result never crosses that line silently;
- **the `NonZero` family** — top-level only, never nested. The non-zero constraint
  reaches the solver rather than resting on convention: a generated zero would explore a
  state the type forbids and produce a counterexample that cannot happen, which is a
  false alarm, and this project treats those as nearly as costly as a false pass;
- **`Duration`** — two integers, with the sub-billion nanosecond bound enforced for the
  same reason. Whether its seconds field needed a bound was measured, not assumed: about
  seven seconds per check against a sixty-second budget, so none was added;
- **a `struct` or `enum` of the user's own** — sampling tier only, and built by the same
  three rules, in order, that govern a receiver. **First**, the type's own constructor,
  honouring the constructor's own precondition and recursing through nested user-type
  arguments to a bounded depth; every value so built is one the real program could
  produce. **Second**, and only when the first cannot apply, direct construction of the
  fields or variant data — permitted solely when every one of them is already public and
  the type is not `#[non_exhaustive]`, on the ground that a caller could then already
  build any combination, so there is no invariant left to violate. **Otherwise refused by
  name**, saying which type and why.

  The second route carries an assumption and the run says so rather than leaving it
  implied: public fields mean nothing *restricts* what a caller may build, but a type's
  own methods can maintain a relationship between public fields that nothing enforces, so
  a value built this way could in principle be one the program never produces. Evidence
  earned on that route rests on that assumption and is not proved. The first route
  carries no such assumption, which is why it is tried first rather than second.

  An enum is admitted or refused whole. Rust gives a variant no visibility of its own, so
  "every field public" reduces to "every variant's data is buildable"; admitting only the
  variants that happen to qualify would quietly drop the rest, and a harness that skips a
  case without saying so is the failure this document exists to refuse;

  **A parameter written `Self` names the enclosing `impl` block's own type, and is
  resolved exactly as that type would be if spelled by name — fixed 2026-09-01, measured
  against `semver`** (`docs/reach-measurement-2.md`): `cmp_precedence(&self, other:
  &Self)` was `unsupported`, and only rewriting `&Self` to `&Version`, with nothing else
  changed, made it `fuzzed(64)`. The receiver's own type was already resolved this way;
  `Self` in parameter position now reuses that same resolved name rather than a second,
  narrower answer that disagreed with it;
- **`f32` and `f64` — sampling tier only.** Proving over floating point is real work not
  attempted in v1, so a `bounded` check on a float is refused by name. Generated floats
  **exclude NaN and infinity by default**: a generated NaN makes almost any promise look
  broken on a value the function may never receive. The run says so plainly rather than
  leaving the exclusion silent — it states that it says nothing about those cases,
  because it was never asked to;
- **fixed-size arrays `[T; N]`** — cheap with no annotation, because the bound is a
  compile-time constant. Our own sweep only measured to 16; the Rust standard library's
  verification project routinely uses `MAX_SIZE = 32`, `ARRAY_LEN = 40`, and `MAX_LEN =
  512`, and `kani::vec::exact_vec::<T, 17>()` runs in Kani's CI with no unwind annotation
  at all. This is v1's **preferred** bounded shape, and generated harnesses should reach
  for it first — building a fixed array and taking a symbolically-bounded subslice is how
  the professionals express variable length, rather than a symbolic `Vec`;
- **`Vec<T>`** — supported **only because Ply's harness codegen emits an explicit
  `#[kani::unwind(...)]`**. Without it a `Vec` times out at *every* length measured,
  including length 1. The bound is **not** `N+1`: it must cover the whole construction
  loop, and `kani::vec::any_vec::<u8, 8>` needs 22. Codegen must derive the bound from
  the actual construction, and an under-sized unwind is a silent unsoundness risk, not a
  slow run;
- **structs and enums** of the above with Ply-derivable `Arbitrary` (public,
  invariant-free fields);
- **`&T`**, of the above (built from an owned value in the harness) — `&[T]` never actually
  was, on either engine, until the composition amendment below; this bullet's own spelling
  was aspirational until then.

**Composition is a real recursive grammar, sampling engine only — 2026-09-02, measured
(TODO.md, "make the sampling engine's decision recursive, and add slices").** Until this
date, every shape added to this list after the original set was individually barred from
*nesting* inside another: `Option<String>`, a list of a user struct, and `&[T]` were all
refused outright, even though a plain `String`, a plain user struct, and `&Vec<u8>` were
all checked happily alone — one shared "is this type supported" decision answered for
both engines, and letting a newly-added shape compose would have silently made it eligible
for the *proof* tier too, whose list above is measured and deliberate. The fix splits the
decision and only ever widens the sampling side: `Option`, `Result`, a fixed array, a list
(`Vec`), a set (`BTreeSet`), a map (`BTreeSet`'s own sibling, `BTreeMap`), a slice (`&[T]`,
finally a real shape rather than an aspiration — built the same way `&Vec<u8>` already is:
an owned `Vec<T>`, lent), a tuple, and an owning wrapper (`Box<T>`) now close recursively
over *any* sampling-buildable element — a scalar, a `String`, a float, `NonZero`/
`Duration`, a user struct, or another composed shape, to any depth. **The bounded (Kani)
engine's own list above does not move, byte for byte**, pinned by a dedicated regression
test written before the composing logic (`crates/ply-core/src/harness.rs`,
`the_bounded_proof_engines_own_supported_list_never_widens`): none of the four new shapes,
and no newly-nestable inner, is ever bounded-supported.

One honesty condition attaches, found while building this, not merely asserted: a struct
or enum *nested* inside another shape's own sampled value cannot be built by calling its
constructor from inside a proptest strategy — proptest's own `prop_map` requires its
output type to implement `Debug` (`Map<S, F>` only implements `Strategy` when `F::Output:
Debug`), which cannot be assumed of an arbitrary user type. So a nested struct/enum's
strategy only ever draws the raw leaf values its constructor or fields need (always plain,
always `Debug`), and the real construction happens afterwards in ordinary Rust code in the
harness's own preamble — exactly how a *top-level* struct parameter was already built, now
reached one level of nesting later. That mechanism has no proptest case-rejection
available partway through an already-built container, so nesting is refused, by name, for
a constructor carrying its own `requires` filter or a fallible (`Result<Self, E>`) return —
even though the identical type is fine as a bare top-level parameter.

Two documented limits narrow what *every* `bounded` verdict means, however clean it looks:
generated arguments **never alias each other**, so a bug that needs two parameters to
point at the same thing is invisible; and type invariants are **assumed, never asserted**,
so a proof may rest on an invariant the code itself breaks. Both belong in the verdict's
own explanation, not only here.

Measured exclusions, each named rather than left for a user to discover by timing out:

- **`BTreeSet`/`BTreeMap` beyond a single element** — `insert`'s own generic algorithm is
  intractable at two elements even with the unwind fix applied.
- **`HashMap`/`HashSet` with the default hasher** — a compile error, not a timeout
  (`RandomState` has no `Arbitrary`). Ply's codegen must substitute a deterministic hasher
  itself; a user cannot be expected to know this.
- **Recursive or self-referential types** (`Vec<Self>`, `Box<Self>` — any tree or linked
  structure) — not supported in v1, even at one level of real recursion, even with the
  unwind fix. A 3-node tree produced 64,147 verification conditions and did not finish in
  180s. This is the exact shape of Ply's own verdict tree, which is why it is the spike's
  headline finding rather than a footnote. Everything else — trait objects, generics, smart pointers,
non-exhaustive or private-field types with no route (below) — yields status
`unsupported` with diagnostic `V0505` naming the offending type. Unsupported is a
reported fact, never a harness build failure.

**The generator hook, built 2026-09-02** (TODO.md, "one build-route mechanism for named
types"). This section's own earlier text asserted a `pure`-marked constructor function
and a `ply.yaml` key for it, and that its design was "validated in the M0 spike" — thinner
than it read even when the hook was still unbuilt: the spike validated the
constructor-**harness** pattern (calling a found constructor to build a value), never the
**declaration surface** a user writes to name one, which did not exist to be validated.
Corrected here rather than inherited: **a type is buildable if there is a public way to
get one from parts Ply can already build**, generalised into a route table with three
sources, tried in this order for a struct/enum parameter the rules above cannot reach:

1. the type's own constructor (already described above — unchanged);
2. a small curated set for standard-library types, **excluding anything filesystem-path-
   shaped**. Not populated in this pass — deliberately, not by oversight: codegen has no
   way yet to import or call a path outside the target crate's own root, which every
   curated entry would need, and paths themselves stay refused everywhere until a later
   change adds a check for side effects (Ply runs the real function body, and several of
   its own path-taking functions write files);
3. **a route declared in `ply.yaml`**: a top-level `routes:` map, the type's bare name to
   a public function's path — free or associated, Ply's resolver does not distinguish —
   that returns the type. Resolved by the same walk that classifies a call (§5.5), so a
   route is found, or refused, exactly the way any other claim's anchor already is.
   Variety comes from Ply sampling **the route's own parameters**, never from an author
   listing values: `routes: { Handle: open_handle }` naming `pub fn open_handle(id: u32)
   -> Handle` samples `id` exactly the way any other `u32` parameter already is.

A route found but not usable — a renamed or private function, a return type that does not
name the declared type, or a parameter Ply cannot itself build — is refused loudly,
naming the broken declaration, never silently falling back to rule 1 or 2: a stale route
is a fact about that declaration, and reporting it as though nothing were declared would
bury the real defect under a generic one. A type with no route declared and no other way
in is still refused as before, with one more sentence naming the declaration
(`routes: { TypeName: <a public function that returns TypeName> }`) that would unlock it.

**A declared route is tried first, unconditionally (corrected 2026-09-02, three defects
found while proving this section, none by reading).** The order above ("tried in this
order") originally meant a route was only ever *reached* once rule 1 and rule 2 had
already failed to find the type at all — and that reading hid two silences. First: a type
this crate does not declare locally has no source for rule 1 or rule 2 to fail *against*,
so the door to rule 3 never opened either — a route naming a real, correct function
outside the crate refused with the same sentence as no route at all (`V0505`, "no part of
its value is one Ply knows how to vary"), never naming the declaration. Second: a type
this crate *does* declare, whose only public constructor happens to also satisfy rule 1
(`StatusSet::new()`, a zero-argument fn returning `Self`, is both), had its declared route
silently skipped in favour of whichever constructor rule 1 found first — an explicit
`routes:` entry, quietly never called. Both are the same defect wearing different hats: a
declared route must be tried, and reported on, before either rule ever runs, not only when
they fail. Fixed by moving the check: `routes:` is now consulted before rule 1's
constructor scan even starts, for every type whether or not it is declared in this crate.
A working route always wins; a broken one is always named. The type's own constructor
(rule 1) and direct field construction (rule 2) are unreachable for a type with a
declared route — which is exactly right, since the author already said which function to
call.

**A route to a function outside the crate declares its own input types (built
2026-09-02).** Source two's own constraint — "codegen has no way yet to import or call a
path outside the target crate's own root" — is why entries there stayed empty; it is not
why a *declared* route to such a function must stay unbuildable. Ply cannot read that
function's source to infer its parameters, so the author states them instead, in
parentheses after the path: `routes: { OsString: std::ffi::OsString::from(String) }`
builds a `String` the ordinary way (recursively, so a composed or user-defined declared
type works too) and calls `std::ffi::OsString::from` on it directly — no crate-local
lookup is attempted for this form at all. The original form (no parentheses) is unchanged:
its parameters are still inferred from this crate's own source. Ply never reads the named
function's real signature in the parenthesized form — not its return type, not its real
parameter count or order — so a mismatched declaration is not refused here: it is caught
by the compiler when the generated harness fails to build, reported as a tool error naming
the route. That trade (a wrong signature surfacing as a compiler error rather than a named
Ply refusal) is honest only because the route was explicitly declared; an ordinary
unsupported type is never reported this way. Filesystem paths remain excluded from this
extension exactly as source two's own text already excludes them — a later change behind
a check for side effects, not this one.

**Every declared route is used or refused by name (added 2026-09-02).** A route whose
type nothing in the document's own parameters or fields ever names would never be
resolved at all under the rules above — nothing asks for it, so nothing notices a broken
one. Once every function in the crate has been checked, `verify` validates any `routes:`
entry its own per-function walk never touched, on its own terms, and reports a broken one
by name (`W0528`) attached to the whole run rather than to any one function. A route that
resolves fine but happens to be unused earns no diagnostic — only a broken declaration is
worth a reader's attention.

**The one failure a stale-route compile error cannot catch.** A route is a function an
author wrote, and nothing stops it from ignoring its own inputs and returning the same
value every time — the harness still compiles and every case still runs. Ply counts how
many genuinely distinct values a route-built *top-level* parameter actually produced
across a run's cases (by the type's own `#[derive(Debug)]` text — the one thing every
value comparison here can lean on, since there is no blanket `PartialEq`/`Hash` Ply can
rely on from outside the crate) and discloses the split unconditionally, the same
"print the split always, mark only when it is skewed" shape the branch-decided
measurement (§5.4c) already follows: "64 cases ran, but only 1 distinct value reached the
function" is the plain sentence this exists to make possible. Severity rises from an
info-level disclosure to a warning (`W0527`) exactly when a debug-derivable parameter
built exactly one distinct value across more than one case; a type with no
`#[derive(Debug)]` gets the honest disclosure that Ply could not count at all, never an
invented number. Narrowed, stated rather than hidden: only a *top-level* parameter is
counted this way today — the identical route nested inside a composed shape
(`Vec<Handle>`) still builds and checks (composition closes over a route-built value
exactly as it does a constructor-built one), but does not yet carry its own distinct-value
count (open item, TODO.md). Every value built through a route, top-level or nested, still
carries a `route-built` status mark — the `seeded` mark's own precedent — naming that the
evidence came through a declared door rather than the type's whole range.

**A constructor taking no arguments has nothing to vary, and this is said rather than
counted** (`W0529`, 2026-09-02). The route guard above exists because an author's function
*might* ignore its inputs; a constructor with no inputs has none to ignore, so one value
follows from the signature and needs no run to discover. That makes it both stronger and
cheaper than the route guard: it holds for a type that derives neither `Debug` nor
anything else, and it costs nothing at runtime. It applies to a top-level parameter and to
the receiver a method is called on alike, and it is suppressed for a receiver exactly when
the type has an operation taking `&mut self` that this run could call — which really does
move the value off what the constructor made. The verdict carries `one-value`, drawn as
`one value over and over`, the same mark a collapsed route earns, because it is the same
fact about the evidence. Ply already refused to build values through `T::default()` on
precisely this reasoning ("it produces a single value, and reporting that as many sampled
cases would overstate what was checked"); the rule had been written against the trait and
never generalised, so an inherent `new()` of the identical shape was accepted in silence —
a deliberately broken method on such a type reported `fuzzed(256)` with no qualifier at
all. The completeness sentence in the receiver disclosure (`W0520`) degrades with it: "every
value reachable within N steps" is a broad-sounding phrase for a set with one member, so
where the set is one value the disclosure says its size instead.

Generic functions are checkable only through a concrete instantiation: `check_with:
{ T: u64 }` names one concrete type per type parameter, and every harness for that fn
instantiates with it. A generic fn without `check_with` is `unsupported` (V0505). One
instantiation per fn in v1; the verdict names it (e.g. `bounded(3) as T=u64`) so nobody
mistakes evidence about one instantiation for evidence about all.

#### 5.4c Check → engine mapping

| check | engine | verdict on success |
|---|---|---|
| test | generated `#[test]`s from `examples`, plus generated direct contract cases (concrete inputs run through the real function, contract asserted) | `tested` |
| fuzz(n) | proptest harness, n cases (default 256), shrinking on; `requires` as a rejection filter, with a warning when the rejection rate is high | `fuzzed(n)` |
| bounded(k) | Kani contract proof (`proof_for_contract`), loop bound k (default 2) | `bounded(k)` |
| prove | Verus translation (M7, optional) | `proved` |
| mutate | cargo-mutants scoped `--re <fn>`; kill signal = the `test`/`fuzz` checks in the same list (D12) | appends `·spec-strong`, or flags `W0502 weak spec (N surviving mutants)` |

cargo-mutants runs the workspace test suite by default, which would never execute the
generated fuzz harnesses under `target/ply/fuzz/`. Earlier drafts of this section said the
adapter passes a "custom test command" and called the mechanism confirmed; **both were
wrong** — no such flag exists (cargo-mutants 27.1.0: `--test-tool` accepts only
`cargo`/`nextest`), and the M0 spike had never exercised it. The mechanism, verified end
to end in `tests/spike/mutants/`, is package targeting plus a name filter:

    cargo mutants -p <mutated-crate> --test-package <harness-crate> --re <fn> --copy-target true -- <test-name-filter>

**M4 correction: it is `--copy-target true`, not `--gitignore false`.** The earlier
mutants spike's own recommendation ("pin `--gitignore false` explicitly") is falsified by
M4's real runs, on two counts, both recorded in docs/m4-findings.md: (1) `--gitignore`'s
own *default* is already off (confirmed directly against cargo-mutants 27.1.0's test
suite, `gitignore_off_by_default`) — passing it explicitly changes nothing; (2) there is a
second, separate skip `--gitignore` cannot reach at all: cargo-mutants' own copy step
(`copy_tree.rs`) unconditionally prunes any directory literally named `target` sitting
*directly at the copy root*, before the walk even considers `.gitignore` — Ply's harness
crate at `<crate_dir>/target/ply/fuzz/<name>` sits exactly one level inside that directory,
so every `mutate` run hit it (`cargo build failed in an unmutated tree`, the harness
crate's `Cargo.toml` reported missing) even with `--gitignore false` passed. The earlier
spike's own `harness-genloc` fixture never actually exercised this: its harness sat one
level deeper (`lib/target/ply/fuzz/`, `lib` itself a subdirectory of the spike's copy
root), so the top-level-target prune never matched it — an accident of that spike's
fixture depth, not evidence of this placement's general safety. `--copy-target true` is
the fix, and cargo-mutants' own CLI will not accept it alongside `--gitignore` at all
(both share a mutually exclusive argument group) — since the default already matches what
Ply wants, the adapter passes `--copy-target true` alone. The honest cost: this copies the
target crate's entire `target/` build cache into every scratch tree cargo-mutants
builds — measured at ~13s total (baseline + 2 trivial mutants) against a 189MB `target/`
in this session's `weakspec` fixture; a real, size-dependent cost, not a free fix, and an
open item for M5 (moving the harness crate to a location outside `target/` entirely would
remove the need for this flag, at the cost of its own git-ignore entry).

**Ply borrows the user's `Cargo.toml`; it does not keep it.** Package targeting above
only resolves if the generated harness is a member of the same workspace `cargo metadata`
sees, so on a crate that declares its own `[workspace]` table Ply adds the harness to that
table's `members` list before running any engine. That is an edit to a file the user owns
and did not ask to have changed, so it lasts exactly as long as the run that needs it: the
registration is held by a guard whose release — including on the error paths — writes the
original manifest back byte-for-byte. A run therefore leaves nothing in `git status`.

Two conditions keep the undo honest. It never restores over a manifest whose bytes changed
while the run was in flight; a file that moved under the guard is not the guard's to
rewrite, so it is left exactly as found. And because taking the harness out of `members`
would otherwise orphan it — a crate that is neither a workspace root nor a member of one
cannot be built at all — the same release rewrites the harness's own manifest into the
standalone shape. The counterexample test Ply just generated stays runnable from
`target/ply/fuzz/<name>/` afterwards. A counterexample you cannot run is one you have to
take on trust, which is the opposite of the point.

The one gap, stated rather than left to be found: a run killed outright (`SIGKILL`, a
crashed container) runs no guard, so the `members` entry survives it. The next run removes
it — the restore target is always the original *minus* the harness entry, and no human
hand-writes a member path under `target/ply/fuzz/`.

`W0502`'s surviving-mutant count is not a pure weak-spec measure: an *equivalent* mutant —
one whose change cannot alter observable behaviour — survives any spec, however strong.
The spike found one in a 14-mutant run on a three-line function. The diagnostic must not
imply every survivor is a specification gap.

There is no transparent runtime enforcement: generated tests call functions explicitly.
An `induct` check (Kani loop contracts, proving loops by invariant instead of unrolling)
is planned, not in v1: Kani's loop-contract support is experimental, and Ply has no
stable-Rust invariant attribute yet. A function's verdict is the strongest evidence its
passing checks earned; a failing check is a `violation` regardless of what else passed.
**A timeout is not a violation (MUST).** Kani's summary line renders a CBMC timeout and a
genuine contract failure identically as `VERIFICATION:- FAILED`. The distinction *is*
available — `--harness-timeout` reports exhaustion explicitly — so an adapter that
conflates them is careless, not unlucky. Two traps to avoid outright: never
`--output-format=old`, which reports a timeout as success, and never `--quiet`, which
exits 0 on failure (Kani issue #4745). An adapter that conflates them reports a counterexample-free
"violation" for a function that was merely slow — evidence that lies, the one failure
this project exists to prevent. Every adapter MUST distinguish engine exhaustion
(`timeout`, a status outside the evidence order per D6) from a falsified claim
(`violation`), MUST NOT emit a `violation` without a witness, and MUST carry the
distinguishing engine output into the diagnostic so the judgement is auditable.

**Checkability is a property of the body, not just the signature.** §5.4b's list bounds
what Ply will *attempt*; it does not promise the attempt finishes. The scale spike's own
first run is the proof: a `Vec<u8>` bounded to length **1** — a fully supported signature
— timed out because the body was an iterator chain (`.iter().map().sum()`), whose generic
dispatch CBMC unwound past 1150 iterations. The fixture was rewritten to a manual indexed
loop; Ply cannot rewrite a user's body. So a supported signature may still yield
`timeout`, and that outcome must be cheap, fast, and clearly reported rather than
prevented by promises.

**A harness that never ran is a tool error, not a result (2026-08-24 M4 review, D1).**
The `fuzz` and `test` checks share one generated harness crate, so a run that did not
succeed, did not time out, and named no failing test executed *zero* cases — most often
because that crate failed to compile (a user's `examples` entry that does not type-check
is enough, and §5.4a exempts those entries from the contract subset, so nothing validates
them earlier). Per §8, the adapter reports `X0901` carrying the compiler's own first
error, for **every** check in that harness, and the node's verdict is `tool_error`: never
a pass, because no evidence exists, and never a `violation`, because there is no witness.
The same rule covers a failure whose witness cannot be recovered at all: `X0901`/
`tool_error` is the honest report, never a witness-free `violation`. **What counts as
"cannot be recovered" narrowed on 2026-08-25.** A body that panics before its
postcondition is evaluated does escape Ply's own marker — but proptest catches that
panic, shrinks it, and prints the minimal failing input in its own report, which the
adapter must read rather than discard. So a panicking body earns a `violation` carrying
that shrunk input. Until it did, the fuzz tier's only two answers for a genuine crash bug
were "all green" and "Ply's harness had a problem" — a whole class of real defect that
could not be reported at any seed (docs/review-post-004-strategy.md's correction to
vetting 004's finding 4). `tool_error` remains for the case where neither source yields an
input.

**A comparison is widened to `i128` only when both sides are provably numeric
(2026-09-01).** The D7 table row above says the rendered assertion is "widened/checked
arithmetic" so `result == x + 1` at `x`'s maximum value reports the broken promise instead
of panicking on the overflow while checking it — but the widening that protects arithmetic
used to cast **every** leaf a comparison reached, including text, an `Option`, a struct, or
an enum, none of which can be cast `as i128` at all. Found pointing Ply at `semver`: the
author's own most natural phrasing of "the constructor stores the text it was given
verbatim" — comparing two `&str` values with `==` — could not compile
(`error[E0606]: casting &str as i128 is invalid`), and because `fuzz`/`test` checks in a
crate share one generated harness (immediately above), that one comparison's compile
failure reported `tool_error` for every other function still waiting on that harness too,
however correct. The fix narrows widening to a comparison whose *both* sides are provably
numeric — a numeric literal; a parameter or the result whose declared type is a plain
integer scalar, `bool`, `char`, or a float; a dereference, parenthesised form, or explicit
numeric cast of a numeric thing; arithmetic over numeric operands; or a nested comparison/
logical expression, always safe to cast since it is always `bool` regardless of what it
compares. Anything else — a method call, a field access, a path to a constant, an enum
variant — leaves that comparison exactly as written, which is always legal Rust: `rustc`
already accepted the function's own body with that comparison unwidened before Ply ever
ran, so declining to cast can never itself break compilation, only casting a non-numeric
leaf could.

**A `fuzz(n)` verdict names the run that produced it (2026-08-25).** The generated
harness's RNG is built from a seed Ply chooses and records — in the §8 envelope, on the
node whose verdict it produced — never from entropy, and proptest's own persisted-failure
replay is switched off so the run depends on nothing but that seed. `--seed <hex>` replays
a recorded run exactly. When no seed is given, one is derived from the function's own name
and contract text, so identical source always replays identically while two functions in
one run do not share a draw sequence. Vetting 004's finding 4 is what this closes: six
runs of identical source split three-and-three between a clean pass and a real panic, the
run that found the bug unreplayable and the run that missed it indistinguishable from a
real pass. **It buys replay and auditability, not detection power** — 256 uniform samples
that miss an overflow beginning at ~29% of the input range still miss it, now reliably.
The reliability story for this tier is the seed *plus* `mutate`'s kill signal, never the
seed alone: a seeded coin flip is still a coin flip.

**An abandoned fuzz run earns no verdict (2026-08-24 M4 review, D4).** When `requires`
rejects so much of the generated input space that proptest hits its own global-reject
limit and gives up, approximately no case was checked. That run is `unclaimed` with a
`W0503` naming the accepted/rejected counts — never `fuzzed(n)`, which would report *n*
cases of evidence that never happened. A warning beside an overstated number is still an
overstated number. The `n` in `fuzzed(n)` is the count the engine was asked for and
reached; a high-but-survivable rejection rate (the ordinary `W0503` case) does keep
`fuzzed(n)`, because proptest draws until it has *n* accepted cases — what is weak there
is their spread, not their count.

**A `fuzz(n)` verdict can be honest about its count and still overstate what was tested,
when the promise itself is an "either this, or that" (2026-09-02).** A high rejection rate
(above) is thrown away *before* the checked call ever runs. A different emptiness lives
*inside* the call: a postcondition whose top level is `||` is true the moment its first
side is true, and the real behaviour on the far side of it may never run. Found pointing
Ply at `semver`'s `Version::parse`, whose own promise is `!text.contains(' ') ||
result.is_err()` — most generated text contains no space at all, so the promise's first
side alone already decides most cases, and the whitespace-rejection rule the author
actually wrote the promise to check — `result.is_err()`, the only side ever reached on
the text that *does* contain one — ran on only a small minority of the 64 cases.
`fuzz(n)` reported this unqualified.

Ply now measures which side of a top-level `||` decided each case that held, and prints
the split unconditionally — a promise with no top-level `||` earns no split, and neither
does one whose structure this measurement cannot read; both get silence, never an
invented number. The count preserves `||`'s own left-to-right, short-circuit evaluation:
a side that never ran because an earlier one already decided the case is never credited
or blamed for what it "would have" said, so the measurement can never itself trigger a
side effect (a panic, say) the checked promise's own author relied on `||` to avoid. When
one side decides more than half of every case (the same threshold the high-rejection
warning above already uses), the verdict carries the `promise-lopsided` status — a
sibling of `partial-history`'s "narrower than it looks", never a reuse of it: that mark
describes an input the run could not build or a call it could not make, *before* the
checked call; this one describes which side of the promise the call's own result
satisfied, *after* it. Both real, both distinct facts about the same evidence. This
count is a fact about the promise's own text, never a claim about which lines of the
checked function ran — a `||`'s left side deciding every case says nothing about which of
the function's internal branches those cases exercised, and Ply's wording does not say
otherwise.

Per-harness time budget: every engine invocation carries a hard cap (`--engine-timeout`,
§6). Exceeding it yields `timeout`, never a silent hang and never a `violation`. The cap
is on the **whole invocation**, not on one phase of it: cargo-mutants copies the tree,
builds an unmutated baseline, and then runs the tests once per mutant, and its own `-t`
caps only that last phase — so Ply wraps the invocation in `timeout` as well (10× the
per-mutant budget, minimum 120s) and reports exit code 124 as `M0601`/`timeout`. A
`mutate` run that produced no mutant count at all (killed by that cap, engine missing, or
output Ply could not read) carries the `inconclusive` status, never `weak-spec`:
`weak-spec` asserts a finding, and no engine made one (2026-08-24 M4 review, D5). D14
records only results that earned evidence, so a timing-out function re-pays its cap on
every run — which is why the cap must be small by default and the status cheap to
re-report.

Default checks: shape-aware. `[bounded(2)]` when the fn has a contract **and** its
signature passes the §5.4b gate; `[fuzz(256)]` when it has a contract whose shape §5.4b
excludes; none otherwise. A flat `[bounded(2)]` default would route most contracted
functions in ordinary Rust into `unsupported` or a multi-minute timeout.

The default is resolved in one place for every consumer —
`ply_core::model::{effective_checks, component_default_checks}` — so `check`, `verify`,
`audit`, `worklist` and the renderer cannot disagree about which list governs a fn.
`verify` read a fn's own list and nothing else until 2026-08-25, which made a
component's declared default a line `check` resolved and `verify` silently ignored: one
document, two answers. The two listing commands went on reading it that way a few
commits longer, which was worse where it showed: a trust listing that misreads which functions are
checked misreports what a result rests on, and it did so in both directions — naming an
assumption `verify` never makes for a fn that inherits `fuzz`, and calling a fn nothing
checks when its component checks it.

**That default fires only where no checks list is written at all.** `checks: []` is a
written list, an empty one, and it means what it reads as: check nothing here. Nothing
runs, the verdict is `unclaimed`, and `W0515` says so in a sentence rather than leaving a
reader to notice a missing verdict. It is not an absence and never inherits: an empty
list on a fn overrides an ancestor component's default exactly as a full list does
(§5.1), and an empty list declared as a component default means the fns under it are
checked by nothing. Until 2026-08-25 `verify` tested the list for emptiness rather than
for presence, so `checks: []` put the shape-aware default back and proved the function
anyway — the document said "do not check this" and the tool answered `bounded(2)`, which
is the failure mode §1 exists to refuse, in the reassuring direction.

#### 5.4d Trusted claims

Some load-bearing properties live outside Ply's reach — cross-thread safety proven by a
loom test, a paper proof, an external audit. A `trusted` entry records such a claim with
its evidence: `{ claim, evidence }`. Trusted claims change no verdict and run no engine;
they exist so the tree is honest — without them, a node whose real correctness argument
is external renders indistinguishably green. They appear in `cargo ply audit` as part of
the trust surface and carry a distinct visual form (§7.1). An agent must never add or
edit a `trusted` entry on its own judgment; attestation is a human act.

**An attestation stops covering an item that changed.** An entry records the content hash
of the item it attests (item body and contract text, the same two inputs §5.2a hashes
first). When the item changes, the attestation no longer covers what it vouched for: the
entry draws the "no longer covers this" corner marker beside its shield, and `audit` lists
it as owed re-attestation. Without this a `trusted` entry outlives the code it described —
the shield renders identically fresh forever, and a human's word about last year's function
silently vouches for this year's. That is evidence lying, in the one construct built
entirely on trust. Re-attestation is a human act, and nothing in Ply clears it: unlike a
checked result, which §5.2a re-earns automatically the moment its fingerprint moves, an
attestation can only be renewed by the person who made it.

### 5.5 Modular composition (D5)

Verification runs callees-before-callers over the call graph: within a crate, claimed
functions are ordered topologically by their call edges, callees before callers, ties
(equal-rank items — no dependency between them either way) broken by node id so the
order is deterministic and a re-run cannot flap a golden. **A cycle cannot be ordered.**
`f` and `g` calling each other (directly, or through any chain back to one another) is
not a failure of the ordering, it is the fact the second branch below exists to catch:
every claim in a cycle falls back to it, for every same-crate contracted callee it
reaches, because the one thing branch one needs — this claim's callees already verified
— cannot be established for a claim with no place in the order (built 2026-08-26).

**The fallback reaches further than the cycle's own members, and this is easy to read
past.** "No place in the order" is a property of the *ordering*, not of cycle membership:
a claim's turn only arrives once every claimed callee it reaches has already been placed,
so a claim that calls into a cycle — however many calls removed, and without being part of
any cycle itself — never gets a turn either, and falls back on every edge exactly as a
cycle member does. The same is true of a claim whose callee is not in this run's ordered
set at all. So the set denied branch one is: **every claim on a cycle, plus every claim
that transitively reaches one.** That is deliberately coarser than refusing edge by edge
— a claim three calls downstream of a cycle is denied credit it might, on a finer rule,
have been entitled to — and the coarseness is the safe direction, chosen because the
finer rule requires establishing that the cycle member's own proof did not itself rest on
the assumption the downstream call would grant. Written down 2026-08-30 after review found
that the implementation had always behaved this way while no artifact said so.

To verify fn `f` that calls fn `g`, the split is on **what `g` offers**, in three
branches — the first two keyed on the evidence behind `g`'s contract, the third on there
being no contract to key on:

- `g` passed its own Kani contract proof this run, and `g` is in the same crate →
  generate `f`'s harness with `#[kani::stub_verified(g)]`. Clean verdict: `f` is not
  marked `conditional` for `g` and owes no evidence for it. **A reused result counts as
  proved**, not merely as a shortcut: since commit c650e55 the record's fingerprint
  covers the code a check actually runs, not just the checked function's own lines, so a
  matching stored `bounded(k)` for `g` is exactly as sound a foundation as one earned
  fresh this run — the honesty condition that reuse feature exists to make true. What
  does *not* qualify: a `g` whose own verdict is itself `conditional` (standing on a
  further assumption) is never treated as proved here, or the debt it carries would be
  laundered out of view the moment something stubs it out — this branch requires `g`
  clean all the way down, not merely `bounded`-shaped.
  **The bound this branch reports is capped at the weakest link.** `f`'s own proof holds
  only *given* `g` meets its contract, and that was only established to `g`'s own depth —
  so if `f` is declared at `bounded(5)` but stands on a `g` proved only to `bounded(2)`,
  the honest composed verdict is `bounded(2)`, never `f`'s own declared number. Reporting
  the deeper one would be exactly the "evidence stronger than what it rests on" overclaim
  §1 exists to refuse. **A clean verdict is not a standalone one**: `f`'s tree entry
  still names every same-crate proof it stood on and the bound each earned (`W0517`,
  `info` severity — nothing here is wrong or owed, only worth recording) so a reader
  never mistakes "not conditional" for "proved in isolation".
  **Branch one requires `g`'s own proof to cover `g`'s whole parameter domain, not
  merely the caller's specific argument** (added 2026-08-26, adversarial review): a
  `bounded(k)` proof over a length-indexed parameter — `Vec<u8>`, a slice, `BTreeSet`,
  an array — only ever builds values up to length `k`, so it says nothing about a
  caller passing a longer one. Composing against it anyway is the overclaim §1 exists
  to refuse in its purest form: reproduced live, a callee proved only over vectors of
  length ≤ 2 returns a value breaking its own postcondition at length 3, and a caller
  always passing length 3 still composed to a clean `bounded(2)`, exit 0. A callee with
  any such parameter is therefore never eligible for branch one, whatever its own
  verdict — it falls back to branch two exactly like a cycle or an unclean callee does.
  This is narrowed, not proved: a fixed-size `[T; N]` array is conservatively excluded
  too, even though its size is part of the type and an argument-containment argument
  might one day admit it — that argument is not made here (`RustType::is_full_domain`,
  `crates/ply-core/src/harness.rs`).
  **`f`'s own record depends on what it composed against, not only on `g`'s source.**
  §5.2a's fingerprint gained a twelfth input for exactly this branch: the same-crate
  callees a claim stands on, each with the bound it earned (`verified_bounds`,
  2026-08-26, found by adversarial review of this branch before it landed and *again*
  after, independently, on a second reviewer's own fixture — editing only `g`'s declared
  `checks:` with no source touched anywhere left `f`'s stored, now-stale deeper bound
  untouched the first time this was tried). A second, quieter defect went with it and
  took a separate fix: the record's own "is this a verdict the checks could earn"
  integrity check (`W0516`) predates this branch and assumed a `bounded(k)` check could
  only ever produce `bounded(k)` verbatim, so a caller's *genuinely* composed
  `bounded(j)` for `j < k` looked exactly like tampering and was refused, silently
  re-verified from scratch, on every run after the one that earned it — `f` paid full
  engine cost forever, not once. A same-day widening of that fix — accepting any
  `bounded(j)` with `j <= k` — was itself found unsound by a second adversarial review:
  a hand-edited `bounded(4)` for a claim whose real composed value was `bounded(2)`
  passed, because it still sat under `f`'s own declared `bounded(5)`. The rule now
  pins the exact value the composition must equal — `min(f`'s declared bound, the
  shallowest bound among every callee it stands on`)` — and requires equality against
  that number, not merely a ceiling (§5.2a states it in full); a reused `g` supplies
  its bound the same way a freshly proved one does, since either way it is `g`'s own
  recorded, re-hashed verdict that answers the question.
- Anything else *that still has a declared contract* — `g` merely fuzzed or tested, `g` in
  another crate, `f` and `g` in a cycle, or `g` carrying no verification at all but a
  contract declared for it in `ply.yaml` (§5.4's external-spec route) → verify `f`
  assuming `g`'s contract, stub `g` out of the proof, and mark `f`'s verdict
  `conditional` (`W0511`), listing each assumed contract. The contract has to *say*
  something first: a promise nothing can satisfy, or one true of every value, is caught
  before the proof runs (`E0502`/`E0503`, below). **A same-crate contracted `g` reached
  through this branch is stubbed with `#[kani::stub_verified(g)]` too, mechanically
  identical to the first branch** — Kani's plain `#[kani::stub]` cannot target a function
  that carries its own contract at all (Kani issue #4591: a compile error, "Failed to
  find contract closure", killing the whole crate; reproduced against both the pinned
  toolchain and Kani's own `main` in `tests/spike/kani-pin`, and again directly against
  this feature 2026-08-26). What tells the two branches apart is never anything Kani
  checks — its own existence gate for `stub_verified` is purely syntactic either way
  (`tests/spike/FINDINGS.md` item 4: it "works, but is unenforced" — no check that the
  named proof harness ever ran or passed) — it is entirely Ply's own bookkeeping: whether
  the ordering above actually established `g` clean this run. A callee still reached
  through `ply.yaml`'s external-spec route (no inline contract at all) keeps the older,
  hand-built stand-in function and plain `#[kani::stub]`, which works fine there — Kani's
  limitation is specifically about stubbing a target that itself carries `#[kani::requires]`/
  `#[kani::ensures]`, and a boundary-contract callee never does.
- **No contract is declared for `g` anywhere** — not inline, not in `ply.yaml` → Ply
  **refuses to descend into `g`'s body**. `f`'s `bounded` check earns no evidence:
  verdict `unclaimed`, diagnostic `W0512`, and the run fails by default (§1's
  absence-of-evidence principle, §6's exit table).

**The third branch is where all legacy code lands** (vetting 004, 2026-08-25). The first
two branches both presuppose a contract; a two-year-old module has none, so before this
branch existed no rule applied at all and the outcome was whatever the engine did with
the inlined body — measured on 004's `tier_fee_cents`: `timeout` at a 120s budget and
again at 600s (11m23s wall clock), against `bounded(2)` in 1m20s for the identical
function with the boundary call removed. The refusal is decided from the call graph
(D11's extractor) before any engine starts, so it costs milliseconds rather than the
whole budget.

Three honesty conditions attach to the branch, and they are what make it a rule rather
than a shortcut:

1. **The diagnostic names the callee that was not descended into**, and where it is
   called — never only the caller. A verdict that says "`tier_fee_cents` could not be
   checked" and never mentions `ledger::fees::bps_for_tier` tells a reader nothing they
   can act on (§8's non-result rule). `W0512`'s `fixes` offer the two real options:
   declare a contract for the callee, or drop the check to `fuzz(n)`, which crosses the
   boundary by simply running the code.
2. **Ply never inlines an unclaimed body into a caller's proof, at any call site in the
   caller's own body.** Descending is not the more honest option, only the slower one: it
   either exhausts the budget and reports nothing, or it proves the caller *against a body
   nobody claimed*, yielding a `bounded` verdict whose meaning silently includes code no
   contract vouches for. The qualifier is exact and not decorative: this condition holds
   for the call sites this rule inspects, which are the ones written in the function being
   checked. An unclaimed callee *below a contracted callee* `g` was a gap that stayed open
   until D5's first branch landed (2026-08-26, closed for either of its branches): a
   same-crate `g` is now always either stubbed or refused, never inlined, whichever
   branch reached it, so whatever `g` itself calls never travels into the caller's
   proof at all. **"Refused" is not a hedge**: a same-crate contracted `g` whose exact
   shape Ply cannot build a stand-in for — a tuple-pattern parameter, a `self`
   parameter, an unparseable contract attribute — used to fall through this rule
   silently (no stub, no refusal, no diagnostic, `g`'s real body inlined) until an
   adversarial review found it 2026-08-26; it is refused by name now (`W0512`), the
   same way an unclaimed callee already was. **Still open**:
   a contracted `g` reached through a path dependency (a different crate) is still
   inlined exactly as before — cross-crate `stub_verified` is out of scope for v1 (below),
   so this gap survives there specifically. See this section's limits below for what else
   is still open.
3. **An assumed boundary contract is owed evidence until something exercises it.** A
   contract declared in `ply.yaml` for an unclaimed callee is trusted, and trust that is
   never checked is green paint. The assumption is auditable (`cargo ply audit`'s trust
   surface), part of the caller's own fingerprint so that editing the promise re-runs
   every caller resting on it (D14, §5.2a), and — the part that makes it better than
   trust — checkable by the cheap tier: `fuzz` has no trouble crossing the boundary, so a
   declared contract on a legacy callee can be fuzz-checked against the real legacy body.
   Until it is, the caller's node carries the `owed-evidence` status, and **`cargo ply
   audit` lists it**: the callee, the promise, the caller resting on it, and what would discharge it
   (2026-08-25, Phase 1b — before that both commands were unbuilt and this paragraph
   described them in the present tense anyway, which made the enforcement loop an IOU).
   `cargo ply worklist` lists the same thing from the other side: the *assumption* is
   permanent trust surface, and the *evidence owed on it* is work that closes, so `audit`
   carries the first and `worklist` the second. `conditional` is the *normal*
   state of a legacy-extension codebase, so it
   must read as routine and legible rather than as an alarm — the annotation carries the
   trust story, and a user who learns to skip it has lost it.
   **This condition covers branch two's *inline*-contracted callees too, not only its
   `ply.yaml`-declared route** — found not to, by adversarial review, 2026-08-26: both
   commands read only the declared-contract map, so a same-crate callee assumed through
   its own `#[ply::requires]`/`#[ply::ensures]` (this branch, reached whenever that
   callee is not itself an independently bounded-checked claim) reported `conditional`/
   `owed-evidence` correctly at `verify` time while `audit`'s trust surface and
   `worklist`'s count both stayed silent about it — this honesty condition not holding
   for a whole class of assumption. Fixed the same day, narrowly: listed whenever the
   callee carries no `bounded` check anywhere in the document. **Known gap, not solved
   here**: a same-crate callee that *is* claimed `bounded` elsewhere but still lands on
   branch two at `verify` time (inside a cycle, or behind an unclean run) needs the same
   ordering computation `verify` itself does to tell "stood on" from "assumed" — these
   two listing commands do not attempt that and under-report exactly that case.

**What this rule reaches, and what it does not.** It applies to `bounded` only: Kani
descends into a callee's real body, so a caller's proof silently acquires that body's
meaning, while proptest merely *runs* the callee — which is why the fuzz tier crosses a
legacy boundary happily and needs none of this. Method calls on a receiver
(`x.min(10_000)`, `v.len()`) are not call sites for this rule — they are overwhelmingly
`std`, and flagging them would fire on every ordinary line of Rust while telling a user
nothing they could act on.

Within `bounded`, the rule keys on **what the callee is, never on how the call is
spelled**. That sentence is load-bearing, and it was false as written until 2026-08-25:
the resolver read only the caller's own top-level `fn` items, so `use rates::legacy_rate;`
plus a bare-name call classified *unresolved* — and unresolved meant descend. One `use`
line converted the refusal into a clean `bounded(2)`, zero diagnostics, exit 0, over an
unclaimed first-party body (adversarial review of the post-004 fixes, D1). Resolution
therefore follows the crate's own structure: `use` declarations including renames
(`as`), nested groups (`use a::{b, c::d}`) and globs; inline `mod`s; file modules
(`mod foo;` → `foo.rs` or `foo/mod.rs`); re-exports at the entry of each file; and the
same walk again inside the `src/lib.rs` of a **path dependency** declared in the crate's
`Cargo.toml`.

Resolution has three outcomes, not two, and the third is what keeps the rule honest:

- **Resolved** → the three-way split above decides it.
- **Outside the workspace** — `std`, `core`, a registry crate — no source Ply can read,
  and none it should expect to. These are left alone and Kani still descends into them.
  **This is a real gap**, stated here rather than left to be discovered: a `bounded`
  verdict can still include a body Ply never examined.
- **Opaque**: Ply followed the path into first-party source and could not read it — a
  `mod` whose file is missing or unparseable, a path dependency whose `src/lib.rs` will
  not open, or a bare name that could only have come from a glob import of one of those.
  Not being able to look is not the same as there being nothing there, so this **refuses**
  (`W0513`, verdict `unclaimed`) rather than descending. A glob Ply *can* see through is
  resolved exactly like a named import; a glob into a crate outside the workspace
  (`use std::cmp::*;`) is left alone like any other call into it.

One convention is deliberate: a bare name beginning with a capital is treated as a type
or enum-variant constructor (`Some(x)`, `Ok(v)`, `Wrapper(t)`), not a free function, and
never triggers the glob refusal. Firing the boundary rule on `Some(x)` would tell a
reader nothing they could act on — the same reason method calls are not call sites here.

One first-party gap remains open (recorded in TODO.md rather than papered over): calls
Ply's reader cannot see at all — generated by a macro, routed through a `#[path = "..."]`
module attribute, or made through a function pointer or trait method — are not call
sites for it. The gap this used to name for a same-crate contracted callee `g` (`g`
inlined rather than stubbed, so an unclaimed callee one level below it travelled into the
caller's proof unnamed) closed 2026-08-26 when D5's first branch landed — see above.

A `ply.yaml` fn entry that declares `requires`/`ensures` and asks for no `checks` is a
**boundary contract declaration**, not a claim: it exists so callers can assume something
about that function. It contributes an assumption and earns no node of its own, in this
crate or another — reporting it as an `unclaimed` claim would say the opposite of what
was written.

**A declared promise that constrains nothing is a defect in the document, and never
counts as an assumption.** This is §1's rule one turn further in: an *absence of real
assumption is not a pass* either. Under the design Ply is built for, a language model
writes one promise per piece of old code a new feature calls, so an empty, tautological or
self-contradictory promise is not a hypothetical — it is the realistic failure, and it is
the one way per-function promises can quietly lie. Before running a proof that stubs a
callee, Ply asks the engine two questions about each declared clause, over the clause
alone with no function body anywhere in the harness: **can any value satisfy it**, and
**can any value break it**. Both are answered exhaustively over the value space (CBMC
solves them symbolically, not by sampling), and each costs well under a second once the
crate is compiled — measured 2026-08-25.
**This gate is about branch two's clauses, never branch one's** (made explicit
2026-08-26, after an adversarial review found the gate firing on branch one too): a
callee proved clean this run (`stub_verified`, branch one) is not standing on an
assumption at all — its inline contract is real evidence, established by its own
passing proof, not a promise this run is trusting sight-unseen. Interrogating it for
vacuity is asking the wrong question of the wrong branch, so the gate now runs only
over the clauses branch two actually assumes.

- **Unsatisfiable** — no value satisfies it. Ply hands the clause to the engine as an
  assumption, so the caller's proof holds *vacuously* and anything at all is provable
  under it. Measured on `tests/fixtures/emptypromise`: a caller whose own postcondition is
  plainly false came back `bounded(2)`, exit 0, with the impossible promise listed beside
  the verdict as though it were carrying weight. Ply now **refuses to run the proof**:
  `E0502`, verdict `unclaimed`, and the diagnostic quotes the clause back.
- **Trivially true** — true of every value the type can hold (`|result| true`, or
  `|result| *result >= 0` on an unsigned integer). It constrains nothing, so the callee
  was in effect replaced by an arbitrary value. The caller's verdict **stands and is
  honest** — an unconstrained value is the *weaker* assumption, not the stronger one, so
  the result holds whatever that callee returns — but the report called it an assumption
  owed evidence, which sends a reader off to discharge a debt that does not exist.
  `E0503`, error severity so the run fails, the clause quoted back with the type it ranges
  over, and `W0511`'s `conditional` sentence names the empty clause rather than counting
  it among the promises that are owed evidence.
- **Neither answer** — a clause whose parameters have a type the bounded codegen cannot
  build an arbitrary value for, a clause Ply cannot parse, or an engine that timed out.
  Reported as unchecked (`W0514`), never as sound: the verdict beside it still assumes the
  promise, and the diagnostic says so.

What this does not reach, stated rather than left to be discovered: it decides
*emptiness*, not *strength* — a promise that excludes one value out of four billion is
neither unsatisfiable nor trivially true and passes this gate while carrying almost no
information; it says nothing about whether the real callee *honours* the promise, which
is the separate `owed-evidence` debt discharged by fuzzing the callee against it; it asks
about each declared promise alone, not about the harness as a whole, so vacuity arising
from the interaction between a caller's own `requires` and a stub's assumptions is not
caught here (Kani's `kani::cover` is the instrument for that, and is not built); and it
does not look at a verified function's own inline `#[ply::ensures]`, where a weak spec is
the `mutate` tier's question (`W0502`) rather than this one's.

The verdict tree shows each verdict's assumption chain; `conditional` propagates upward as a
status (D6).

**§5.5's limits**, gathered in one place rather than left scattered across the section
they were each stated in:

- **Branch one excludes a callee with any length-indexed parameter** — `Vec<u8>`, a
  slice, `BTreeSet`, an array — because its own `bounded(k)` proof only ever covers
  values up to length `k`, not the type's full value space, and a caller's argument is
  never checked against that limit. Stated in full above, where the branch is defined;
  gathered here because it is a domain-coverage gap, the same shape as the others in
  this list. Narrowed rather than solved: excluding every non-full-domain type is
  conservative (a fixed-size array is excluded too, though its bound is arguably part
  of its type), because no argument-containment check exists yet to admit any of them
  safely. Adding one is future work, not a defect in what shipped.
- **Ply cannot build a stand-in for every same-crate contracted callee's exact shape**
  — a tuple-pattern parameter, a `self` parameter, a contract attribute Ply cannot
  parse. Before 2026-08-26 this fell through silently: no stub, no refusal, no
  diagnostic, and Kani inlined the callee's real body — contradicting this section's
  own "always stubbed, never inlined" condition. Refused now (`W0512`) exactly like an
  unclaimed callee is, naming the callee and why Ply could not build its stand-in.
- **Cross-crate `stub_verified` is out of scope for v1.** A callee reached through a path
  dependency is left exactly as before this feature (full descent, Kani inlines its real
  body) even when it carries its own contract — neither branch one nor branch two's
  same-crate `stub_verified` mechanism applies across a crate boundary. `tests/spike`'s
  own item 5 found a sound *workaround* (a caller-local `#[kani::proof_for_contract]`
  naming the remote `pub` function by qualified path), but it needs a second harness
  declared per consuming crate with no cross-crate proof caching, and D5 does not
  generate it automatically. Decide whether a later milestone does.
- **A call outside the workspace** (`std`, `core`, a registry crate) is left alone and
  Kani still descends into it — stated above, repeated here because it is the same shape
  of gap: a `bounded` verdict can still include a body Ply never examined.
- **A call Ply's reader cannot see at all** — generated by a macro, routed through a
  `#[path = "..."]` module attribute, or made through a function pointer or trait method —
  is not a call site for this rule at all, in either direction: such a call is neither
  refused (branch three) nor stubbed (branches one and two), because Ply never extracted
  it from the body to begin with.
- **Branch one requires `g` clean, never merely `bounded`-shaped.** A `g` whose own
  verdict carries `conditional` — because it, in turn, stands on a further assumption —
  never qualifies as proved here, however deep its own declared bound. This is a
  deliberate, conservative reading of "passed its own Kani contract proof": composing
  branch one across more than one hop of assumption is a real question (does `f`
  inherit `g`'s own owed-evidence debt, transitively, and how is that shown on `f`'s
  node?) that this design does not answer. Recorded as a known gap rather than guessed
  at — see TODO.md.
- **The cycle fallback is decided per claim, not per edge.** A claim inside an
  unorderable cycle falls back to branch two for *every* same-crate contracted callee it
  reaches, including ones outside the cycle that could, in principle, still have been
  ordered relative to it. The coarser rule is what §5.5's own ordering text states and
  what this section's own test suite pins; a finer per-edge version is a possible
  future refinement, not a defect in what shipped.
- **The stub-verified mechanism's own soundness rests entirely on Ply's scheduler, never
  on Kani.** `tests/spike/FINDINGS.md` item 4 found Kani's own compile-time check for
  `#[kani::stub_verified]` purely syntactic — it confirms *some* `#[kani::proof_for_contract]`
  harness exists for the named target, never that the harness ran, or passed, in this
  invocation or any other. Every honesty condition above (ordering, "clean not merely
  bounded-shaped", the reused-record rule) exists because of this: if Ply's own ordering
  or its "clean" gate were ever wrong, nothing downstream — not Kani, not CBMC — would
  notice or refuse.

### 5.6 Underspecification

`ply::unresolved!(147, "employee discount undecided")` marks a decision nobody has made
yet. It expands to `unimplemented!("unresolved #147: employee discount undecided")` —
always, dev and prod alike. Simple, honest, greppable. `ply worklist` lists every marker
(macro or `ply.yaml` registry) with its span, enclosing component, and blocking status.
A fn containing `unresolved!` is capped at check `test`, flagged `W0521`.

**Implemented 2026-08-25 (Phase 1b), with one half missing.** The macro exists in
`ply-attrs` and expands exactly as stated; `worklist` lists markers from both sources,
merged by id so one decision written in both places is one item, each carrying its
`file:line:col`, the `component::fn` node it sits in (or the function alone, where no
claim names it) and what it blocks. **The cap is not enforced**: nothing applies `W0521`,
so `cargo ply verify` still runs whatever the claim asks for against a body that panics at
the marker. `worklist` says so on every marker line and in its `coverage.not_checked`,
which makes the gap visible rather than closed.

### 5.7 Synth mode (M6, experimental)

`mode: synth` on a fn claim means the model writes the body. `cargo ply synth <fn>`:
1. Assemble the generation contract: signature, contracts, examples, the enclosing
   component's caps/edges/profile (as prose constraints plus the raw `ply.yaml` excerpt),
   and the signatures + contracts of everything the fn may call.
2. Invoke the model (pluggable; default is a `claude` CLI subprocess; configured in
   `ply.toml`).
3. Write the body; run the check pipeline for this fn only; on failure, feed the
   Diagnostic JSON back; loop at most N times (default 5).
4. On success, mark the fn `#[ply::derived(spec_hash = "...")]` and record it in the
   verdict tree. When a derived body is later hand-edited (its hash no longer matches), `ply check` warns
   `W0531` and the fn silently becomes `mode: check`.

No streaming, no IDE integration, no multi-fn synthesis in v1.

---
## 6. CLI

```
cargo ply render [path]      # draw ply.yaml as SVG; needs no code or Cargo project
                             # --output/-o writes a file; otherwise prints to stdout
                             # --depth/--focus/--collapse control folding
                             # --json emits a navigable declaration-only visual envelope
cargo ply check              # schema + anchors + architecture. Fast, no engines.
                             # IMPLEMENTED: schema + anchors only (see below).
cargo ply verify [path|fn]   # run checks via engines, callees first; write cex artifacts
                             # (reuses a recorded result whose fingerprint still matches,
                             #  D14/§5.2a; a mismatch re-runs the check)
cargo ply tree               # verdict tree, worst-of aggregation, assumption chains
cargo ply worklist           # unresolved markers + weak specs (W0502)
                             # IMPLEMENTED: markers + owed evidence, no engines (see below).
cargo ply audit              # trust surface: profile escapes, assumed contracts, derived fns
                             # IMPLEMENTED: six tiers, no engines (see below).
cargo ply doctor             # engine presence + versions vs pins; prints the exact
                             # install command for each missing engine, never installs
cargo ply synth <fn>         # M6
cargo ply skill              # (re)generate docs/PLY.skill.md from schema + diag registry
```

**`render` is the pre-code command.** It accepts a `ply.yaml` file or a directory that
contains one, and defaults to the current directory. It parses the same document model
and calls the same SVG renderer used by published visual artifacts. It runs no checks,
reads no result record, creates no `target/ply` files, and needs no `Cargo.toml`. An
output path writes one SVG file; without one, the command writes SVG to standard output.
With `--json`, it writes the same visual-envelope shape used by editor clients, populated
from declarations alone. Every item is `unclaimed`; no code is read and no check runs.

**`check` implements two of its three tiers (2026-08-25, Phase 1a), and says so in its own
output.** Schema (the document against `schema/ply.schema.json` — `E0201`, `E0204` — then
every document-local rule that needs no code behind the anchors) and anchors (every fn
claim resolved through the same resolver `verify` anchors with — which is also the one
that classifies calls (§5.2, §5.5) — so the two commands never disagree about which
claims point at real code; `E0301` names the nearest item-index name, and where the
function is real but unreachable from a crate-root harness because it or a module above
it is private, the diagnostic says *that* instead of "not found"). The architecture tier
is M2. There is no staleness tier left to implement: §5.2a settles a recorded result by
re-hashing it at the moment `verify` uses it, so no command has a "possibly out of date"
state to report. That gap is carried in the `--json` envelope as a `coverage.not_checked`
array and printed under "What this command did NOT check", because a command that reports
only findings lets a clean run read as full coverage — the same failure as an absence of
evidence reported as a pass (§1). `check` runs no engines, so every node in its envelope carries the verdict
`unclaimed`: that is the command reporting no evidence of its own, not a judgement about
the code, and the human surface says so in as many words. `check`'s exit codes are 0
clean or advisory-only, 1 any error-severity finding, 2 tool error; `--fail-on` is not
wired to it yet (its `evidence` default is meaningless for a command that gathers none),
and neither are `--only-changed` or `--engine-timeout`.

**`worklist` ships two tiers as of 2026-08-25 (Phase 1b), and names the one it cannot.**
It lists **unresolved markers** (§5.6) — `ply::unresolved!` in the code and the `ply.yaml`
registry, merged by id so one decision written in both places is one item, each with its
span, its enclosing function and what it blocks — and **owed evidence** (§5.5): an assumed
boundary contract nothing has yet run the real callee against, with the one line of
`ply.yaml` that would close it. That second tier is the same fact `audit` reports, read
the other way round, and the split is the point: `audit` lists what a codebase rests on
permanently, `worklist` lists what somebody recorded and means to finish. An environmental
assumption (§5.1's `entry:`) is therefore on `audit` and never here — nobody can discharge
it, so counting it as owed would pressure a user into deleting an honest declaration.
**`worklist` exits 0 whether or not it has items**, for the same reason: a command that
failed a build for containing a `TODO` would make deleting the `TODO` the cheapest fix.
Its `coverage.not_checked` carries two things: `W0502` weak specs need a `mutate` run,
which is engine work this command does not start, and it does not read the recorded
results either (§5.2a), so a weak spec found by an earlier `verify` is not listed here;
and §5.6's cap of a marked function at check `test` (`W0521`) **is not enforced by this
build**, so each marker's blocking line says what §5.6 intends rather than what Ply stops
you doing. The items ride in the envelope as `open_items`, an additive §8 field. The
`ply::unresolved!` macro itself now exists in `ply-attrs` (it expands to
`unimplemented!("unresolved #<id>: <note>")` unconditionally, exactly as §5.6 states);
before Phase 1b it did not, so no code containing a marker could compile.

**`audit` ships as of 2026-08-25 (Phase 1b), and reports none of its findings as a
failure.** The one-line description above predates most of the surface it now covers, so
the command lists, in this fixed order: **assumed boundary contracts** (§5.5) — each
naming the callee, the promise `ply.yaml` declares for it, the caller whose verdict rests
on it, and the `owed-evidence` that promise carries until something exercises it;
**environmental assumptions** (§5.1's `entry:`); **`trusted` claims** with their evidence
(§5.4d); **helpers called from contracts** (§5.4a); **`#[ply::allow(...)]` escapes**
(§5.3); and **derived bodies** (§5.7). Three of those have no owed state and never will:
an escape, an environmental assumption and an attestation are permanent trust surface, and
counting one as an open item would pressure a user into deleting an honest declaration —
the opposite of what the surface is for. The command exits 0 with a surface to report;
only a document that will not load fails it (1), and a missing one is a tool error (2).
What it cannot see rides in `coverage.not_checked`, the same way `check` carries its
missing tiers: **whether an attestation still covers its item** (§5.4d) is not computed,
so every attestation is listed undated, and **assumption discharge** needs a verdict this
command does not produce and does not read from the record (§5.2a), so every assumption is
listed owed; **helper evidence** needs a verdict, and this command produces none; call
sites Ply's reader cannot see (§5.5's own gaps) are absent from the assumed-contract list;
and the **architecture bans** an escape suppresses are M2, so today an escape switches
nothing off. Like `check`, `audit` runs no engines, so every node in its envelope reads
`unclaimed`, and its last line says so. The surface itself rides in the envelope as
`trust_surface`, an additive §8 field.

Inspection and verification flags: `--json` (schema §8, the agent surface),
`--engine-timeout=<s>` (shape-aware default, not a flat number — see below),
`--only-changed` (scope to the git diff), `--fail-on=warn|evidence|error` (default
`evidence`, see the exit codes below), `--seed=<hex>` (replay a recorded `fuzz(n)` run —
§5.4c).

**The engine-timeout default is shape-aware, not a flat number (M4 correction).** A flat
60s default (this section's own earlier text) does not fit every §5.4b-supported shape:
the M3 review measured a `bounded(8)` proof over an 8-element `Vec` — a shape §5.4b lists
as supported — timing out at 60s in 3/3 runs (62–63s each), while the M3 e2e suite's own
fixture for exactly that case passes `150` explicitly to make it pass at all
(docs/m3-slice-findings.md). A default that cannot finish a supported shape makes
`timeout` the ordinary outcome rather than the exceptional one, which is how a status
meant to mean "engine exhausted" decays into noise a user learns to skip. The fix (M4,
`crates/ply-cli/src/verify.rs::default_engine_timeout_secs`) makes the budget depend on
the declared check's shape instead of raising one flat constant for everything: a
scalar-only `bounded(k)` harness keeps the original 60s (nothing in the M3 findings shows
it insufficient there); a `Vec`-typed `bounded(k)` harness gets `30 + 15·k` seconds. **The
split is derived; the coefficients are fitted, and are not a measurement** (2026-08-24 M4
review, O1). Derived: within the implemented §5.4b subset, `Vec` is the only shape whose
CBMC unwind cost grows with the bound, so it is the only shape scaled. Fitted:
`150 = base + rate·8` is one equation in two unknowns (`0 + 18.75·k` fits it equally
well), and the 150 is the M3 e2e fixture's own generous constant rather than a measured
requirement — docs/m3-slice-findings.md finding 3 measured the *identical* harness from
~1s to ~107s across runs, variance that dominates any k-linear model. Nothing claims more
than that. `fuzz`/`test`/`mutate` checks keep a flat 60s: proptest and plain `cargo test`
do not carry Kani's `Vec`-unwind cost profile, so nothing here shows a shape-aware scaling
is needed for them yet — except that a `mutate` run is many test runs, so Ply caps the
whole cargo-mutants invocation separately (§5.4c). Passing `--engine-timeout` explicitly
always overrides the default, for every check kind, exactly as before.

**A stubbed `bounded` harness gets a floor of 300s (2026-08-25).** §5.5's second branch
replaces a callee with its declared contract, so where the real body returned one of a
handful of concrete values the stub returns a *symbolic* one constrained only by
`ensures` — strictly less information for the solver, and more work. Like the `Vec` split
this is derived from something Ply knows *before* the run (the harness either carries a
stub or it does not); unlike it, the constant is fitted to a single measurement, and a
second measurement says the cost is not the stub's alone: vetting 004's `tier_fee_cents`
needs **201.77s** stubbed, while the `boundarycontract` fixture — same rule, same stub,
smaller body — verifies in **9.72s**. So a stub does not imply 200s; it implies the
expensive direction, and 60s is not a budget this feature can live at. Until this landed
it did not: `tier_fee_cents` is scalar-signature, so plain `cargo ply verify` gave it 60s
and reported `timeout`, and the diagnostic that should have carried the assumption never
appeared — the tranche's headline capability, dead at the tool's own defaults
(adversarial review of the post-004 fixes, G1). 300 is 201.77s plus room for the
run-to-run CBMC variance docs/m3-slice-findings.md measured on an identical harness
(~1s–107s). **Body cost is not, and cannot be, in the default**: `arraycard`'s array
parameter costs 0.036s to construct and its body ~139s, and no signature-shaped rule can
know that (§5.4c: "checkability is a property of the body, not just the signature"). What
a user gets there is `timeout` plus `K0601`, whose first fix is to raise the budget. A
timed-out proof that *was* stubbed says so, and says why, in `K0601` itself.

The default path is exercised end-to-end by one e2e (`boundarycontract_fixture`, which
passes no `--engine-timeout` at all) plus unit tests on the formula; every other fixture
passes the flag explicitly, which is how the default went unobserved for a milestone.

Exit codes: 0 clean, 1 violations or failures — **including a run in which any node
carries an absence of evidence** (`timeout`, `unsupported`, `tool_error`, `unclaimed`,
`engine-missing`, `inconclusive`), **as its verdict or as a status beside it** (§1: an
absence is a name, not a slot) — 2 tool error, 3 missing engine for an explicitly
requested check. The code is chosen by the absence, wherever it was recorded: a missing
engine is 3 whether it arrived as a `bounded` check's verdict or as a status on a fn
whose fuzz check passed.

**`--fail-on` relaxes that default; it never enables it.** Three values, from strictest to
loosest:

| `--fail-on` | the run fails when |
|---|---|
| `warn` | any diagnostic of warning severity or worse was emitted — including the ones that sit beside a real verdict (`W0502` weak spec, `W0503` narrow spread) |
| `evidence` (default) | any node carries an absence of evidence — as its verdict or as a status — or any error-severity diagnostic was emitted |
| `error` | only an error-severity diagnostic was emitted (a violation, an unresolvable anchor, a tool error) |

`error` is the pre-2026-08-25 behaviour, kept as the documented opt-out for a codebase
mid-adoption where absences are expected and tracked elsewhere. Choosing it is a
statement that this run's green means less than the default's, which is why it has to be
typed.

**The printed tree carries statuses, not only verdicts (2026-08-25).** §7.1 gives
statuses their own visual channel on the diagram — corner markers beside the fill, never
a change to the fill, because a status is a different kind of fact (D6). The terminal had
no such channel: a node whose verdict rested on an assumed contract printed as a bare
`bounded(2)`, indistinguishable from one standing on checked code, with the qualifier
reachable only in `--json` or in the diagnostic paragraph below the tree. The most-read
surface was the one where the assumption was invisible, which defeats the point of §5.5.
A node's line now ends with `[assumed]` where it carries `conditional` and
`[evidence owed]` where it carries `owed-evidence` — plain words, not the D6 names, per
CLAUDE.md's newbie bar — and the sentence explaining each is printed once beneath the
tree, only when the tree carries that mark. The marks travel upward exactly as the
statuses do, so a qualified leaf is legible at the root without expanding anything: the
job the corner marker does on the diagram.

Housekeeping: Ply owns everything under `target/ply/` and every generated test whose name
starts with `ply_cex_`; `verify` deletes generated artifacts whose claims no longer
exist.

## 7. Recursion & aggregation (the zoom model)

The model is a tree: workspace → components → nested components → fns. Every node
carries `{ id, kind, anchor, content_hash, verdict, statuses, worst_descendant,
open_items }`. `verdict` is the node's own claim status; `worst_descendant` implements
D6 over the evidence order; `statuses` and `open_items` (unresolved markers, weak specs,
conditional or owed-evidence verdicts) propagate upward as counts. A node whose result
was reused rather than re-run additionally carries `reused: true` (§5.2a) — a fact about
when the run happened, not a status: it stays on the node that earned it, propagates
nowhere, and enters neither the evidence order nor any exit code.

Aggregation rules the verdict kernel (`tools/kernel`) checks exhaustively and the tool
must preserve:

- **Kinds only.** The evidence order compares the six kinds; the `n`/`k` parameters of
  `fuzzed(n)`/`bounded(k)` are reported in the verdict, never compared. Two claims of
  the same kind are the same rung — the ladder's pressure is toward a stronger kind,
  not a bigger number.
- **Only claimable items contribute their own evidence.** fns fold their own verdict
  into `worst_descendant`; containers (workspace, components) fold over children only.
  A container with no claimable descendants reads `unclaimed`. (The literal uniform
  fold would make every container read `unclaimed` forever, since containers carry no
  claims of their own.)
- **`open_items` counts flag instances, not flagged nodes**: a node carrying two
  statuses contributes 2. The count is triage pressure and must not lose information.
- **`conditional` structurally carries its assumptions** (D5): a conditional status
  without an assumptions list is unrepresentable in the kernel, not validated against.

`tree --json` emits this tree (D10). Human-facing `tree` renders it as an indented
tree, one line per node, worst first; `--depth N` limits depth and `--focus <id>`
descends into a subtree. This is the human review surface: at depth 1 you see which
subsystem is weakest, then zoom. A future canvas UI renders the same JSON; nothing in
this repo should special-case it beyond keeping the schema stable and treelike.

### 7.1 The visual grammar

The grammar is designed to be drawn. Every construct in `ply.yaml` has exactly one
visual form, and the mapping is total in both directions: a diagram of the model shows
nothing that was not declared, and everything declared can be shown. This bijection is a
design gate: **a proposed grammar feature with no clear visual form does not enter the
grammar.**

| Construct | Visual form |
|---|---|
| component (nesting) | box; nested boxes for nested components; anchor as subtitle |
| fn claim | leaf chip inside its component's box |
| `->` edge | solid arrow between boxes |
| `~>` data flow | dashed arrow labeled with the type |
| deny | barred red arrow between the matched patterns; a `*` pattern draws its own per-rule "any" marker — wildcards have no shared identity, so unrelated rules never appear connected |
| `owns` | header line under the anchor, `owns T, U` — the types this component is sole mutator of |
| `state:` | a header line under the anchor, `state T — N of M shown`, then one row per field named in `show:`: its shape glyph, its field name, and its type as the source spells it. Both numbers are counted from code, never from the document, and the count is omitted entirely when there was no code to count. The type column is one column for the whole box, set by its longest field name — a ragged column is read a row at a time, an aligned one is read as a column. Rows sit below the capability badges and above the fn chips — state is what the component *is*, chips are what it *does* |
| a state field's shape | one glyph per row, drawn in ink only — **no new colour channel**, since every hue is already spoken for (green earned, red violation, violet authorship, the grey ceiling ramp). Seven forms, each a distinct silhouette at 12px: **scalar** a filled cell; **text** a cell with a written line across it; **list** three stacked equal bars (order carried); **map** a narrow key cell beside a wide value cell, twice; **set** three loose discs, unaligned (each once, no order); **might be missing** a dashed cell (`Option`); **a shape of your own** two overlapping outlined cells (a struct or enum — there is more inside). Proposal sheet: `docs/state-shapes.svg` |
| a state field Ply cannot build | the same diagonal hatching unclaimed code already carries, on the glyph itself — no eighth form, so a hatched field still shows *which* shape cannot be built. Two details are load-bearing and both were found by measuring rather than by review. The hatch is drawn at a **finer pitch than the ceiling hatch** (4 units against 8): the ceiling pattern inside a 12-unit glyph is about one stripe and does not read as hatching at all. And on the two outline-only forms it becomes the glyph's **fill** rather than replacing its ink, because those two — a structure of your own, and a shape Ply has no vocabulary for — are the commonest unbuildable fields there are, so a hatch that could not reach them could not say the thing it exists to say. "Cannot build" is the sampling engine's own answer, not "the parser gave up": `BTreeMap<u64, Level>` parses perfectly well as a map and still cannot be built |
| capabilities / `pure` | badge row on the box; `pure` = a sealed border, no badges |
| profile | tag on the box |
| checks list | a readable second row on the fn chip, in declaration order: `test`, `fuzz: n cases`, `bounded: loop≤k`, `prove`, `mutate`. The tooltip and transcript expand each check and state what its number measures. |
| verdict | fn-chip fill by discrete display state, never a continuous ordinal ramp — see the amendment below. Five states: `declared` (no evidence resolved, or evidence that settled nothing — drawn exactly as a chip with no evidence always has), `earned` (green — the one hue reserved for evidence a run actually produced), `violated` (red — the only red besides `deny`/finding), `unanswered` (a run tried and could not say; distinct, never red), `stale` (a stored result exists but the code moved since; distinct, never red). "Earned on assumptions" (§5.5's `conditional`) is a marked variant of `earned`, not a sixth hue — the mark is an attached character, never a new colour |
| statuses | corner markers on the node (conditional, weak-spec, …) — `tools/render`'s own SVG realizes the colourblind-safety half of this today folded into the five display states above (`✓`/`✓†`/`✗`/`?`/`↻` beside the fn's check labels, not yet one marker per individual status kind) |
| worst_descendant | a collapsed box takes its weakest descendant's fill — D6 made visible |
| assumption chain | thin dotted arrows from a verdict to the contracts it assumed |
| unresolved marker | numbered pin on the fn or component |
| trusted claim | hollow shield badge on the node — attested by named evidence, not machine-checked |
| contract clauses (`requires`/`ensures`) | a contract mark on the fn chip — a solid ink bar the full height of the chip's left edge (a gutter mark: "this row carries something binding"); the tooltip lists each clause verbatim, and inside a `--focus` target the clauses are additionally *drawn* under the fn name, one line each, prefixed `Input (requires)` and `Postcondition (ensures)`. Drawn only at focus, never at overview: the overview answers "where does attention go", the focused view answers "what exactly is promised here", and clause text at overview zoom buries the first question. Ancestors of the focus target are excluded for the same reason — they stay expanded only to show the path down. This draws the §7.2 watermark per function: marked = a promise stands at the mark; bare = signature only. The static renderer sees YAML clauses; the §8 envelope adds inline attributes from a completed `cargo ply verify` run. |
| declared ceiling | component fill, on the **neutral grey** ramp (never green — see the channel-discipline amendment of 2026-08-29), stepped by the verdict ordinal scale: the strongest verdict the component's declared checks *could* earn — per fn the strongest check kind (`test`→tested … `prove`→proved; `mutate` strengthens, never lifts; no checks = unclaimed, unfilled), folded worst-of by the kernel's container rule. A ceiling is a promise, not proof: it is drawn in grey, never in the green reserved for evidence a run has earned, and its tooltip says none of it has run |
| `strict` | a solid ink triangle notch in the box's top-right corner — the "flagged, zero tolerance" instinct; tooltip already explains errors-not-warnings |
| `mode: synth` | the fn chip's fill turns light violet — violet is hereby the authorship channel, its single meaning "machine-written": the body below the watermark is synthesized from the contract, with the checks holding the line. Tooltip says exactly that |
| `examples` | a gray `e×N` token in the chip's annotation area (next to the `T=...` note); the tooltip already counts and the `test` check runs them |
| hollow component (derived, like findings) | a component that declares nothing inside — no fns, no nested components — draws with a dashed border: a sketch outline, nothing to zoom into yet. Tooltip says so plainly. Derived from absence rather than declared; the natural state of every box in top-down authoring, expected to solidify as claims arrive |
| finding (tool-computed, not declared) | the offending item drawn in error red with an `E####` badge; its tooltip leads with the diagnostic. A finding with no drawable item attaches a red count to the workspace title. A document with findings still renders — a picture that refuses to draw hides the problem it should be showing (origin: fault-injection demo, where a faulted toolchain drew `bounded(0)` as legitimate evidence) |
| external | a solid-bordered, unfilled, anchor-less, badge-less box **outside the workspace frame** — position extends its one declared meaning (containment) from "inside the box = part of the component" to "inside the frame = part of the system"; no new channel. Never on the verdict scale, not even `unclaimed`. Tooltip: "⟨name⟩ — a system or person outside this codebase: ⟨note⟩. Ply draws it so the boundary is visible, but checks nothing about it — every arrow touching it is a declaration, not a verified fact." |
| `entry:` (derived edge) | a dashed arrow, labeled `entry`, from the reachable fn to the external that can reach it — crossing the frame border like any `~>` edge. Not a declared edge (fn claims are not edge endpoints, §5.3); the renderer derives it the same way it derives the ceiling fill. Tooltip names the fn, the external, and lists each `requires` clause now standing as an environmental assumption |

**Amended 2026-08-30 (verdict fill is now real, and it is discrete, not a ramp).**
Until `9ab9fb7` a verdict could only append hover text; the `verdict` row above ("node
fill on the ordinal scale") was therefore aspirational, describing a future canvas
rather than anything `tools/render` actually drew. That refactor made a run's evidence
a real input to the SVG renderer (`EvidenceView`, consulted inline as each component box
and fn chip is drawn), and this amendment is what the renderer does with it: five
discrete display states — `declared`, `earned`, `violated`, `unanswered`, `stale` — never
a continuous green ramp from `violation` to `proved`. The generative rule behind all
five: no display state may exist unless the evidence handed to the renderer holds the
fact it displays. `earned` still means exactly what the channel-discipline paragraph
above says (green = evidence actually earned), but it is now one fixed hue rather than a
scale — "earned on assumptions" (§5.5's `conditional`) adds an attached mark, never a
second green. This retraction is scoped to `verdict`; `declared ceiling`'s neutral grey
ramp (a promise, never proof) is unchanged by it, and a collapsed box's fill still comes
from that same ceiling, not from evidence — a collapsed box states its
earned-over-promised fraction as a plain count (`"6 of 10 earned"`) alongside that fill,
never as a second fill or a percentage, per the rejected-meter finding in
`docs/visual-language-research.md`'s own header: summing evidence into one proportion is
exactly what the kernel's worst-of rule forbids.

A collapsed component is one solid-bordered box (never dashed — hollow means *nothing*
inside; collapsed means *plenty* inside, folded) showing its name, anchor, a contents
line (`N components · M fns`), and its worst-descendant ceiling/verdict fill — and it
draws as a **stack**: one offset card edge behind the box, the "pile of cards" instinct
for folded content. The three depth-states are preattentive: flat card = fully shown,
stacked card = content folded behind, dashed = nothing inside at all. Three
things never fold away: capability badges (union of the subtree's — a collapsed box
containing `net` still shows `net`), the unresolved-pin count, and the finding count.
`ply-render --depth N` and `--focus <component>` mirror the tree CLI.

Zooming is collapse/expand over the §7 tree and mirrors `tree --depth`/`--focus`. A
renderer's only input is the §8 envelope — no side channel. The renderer itself stays
out of scope; this section fixes what any renderer must show. The archi-techture bundle
in `.archi/` is the working example of the style. One deliberate exception: a minimal
static renderer (`tools/render/`, `ply.yaml` → SVG, no GUI) exists to prove this mapping
is total — every construct drawable, nothing undrawable admitted. It is a spec-validation
tool, not the canvas.

**Channel discipline (the instinct rule).** The diagram must be readable at a glance by
someone who has learned nothing: every visual channel carries exactly one meaning, and
each meaning borrows an instinct the viewer already has. Hue: red = forbidden or wrong;
green = evidence **actually earned by a run**; amber = a human's attention (owed, or
vouched); ink = structure — and neutral grey, in an ordered ramp, = evidence *promised*.
Saturation: depth within the grey ramp = how strong the promise is.

**Amended 2026-08-29.** This paragraph previously read "Saturation: pastel = promised,
saturated = earned", putting promise and evidence on one hue and separating them by
saturation alone. That is retracted, on two grounds. First it did not hold in practice:
the top of the promise ramp was a saturated green, so a project where not one check had
ever run rendered as a field of healthy green — the absence-of-evidence failure (§1)
drawn in pixels, by the tool that exists to prevent it. Second it could not hold in
principle: a pastel-versus-saturated step on one hue is exactly the fine discrimination
that does not survive a glance, so the rule asked the reader to do the one thing the
channel cannot do. Promise and evidence are now separated by *hue*, which does survive
a glance. Red is likewise reserved: a declared capability is neither forbidden nor
wrong, so capability tags are neutral, and red is left free for the one thing that must
stay loudest.

The ramp stays ordered rather than collapsing to a single quiet state, because the
evidence ladder is this tool's main lever: "barely tested" and "proved" must not draw
identically merely because neither has failed.

**Position and size, named (added 2026-08-29).** The channels above were disciplined and
position was not, which left the strongest channel on the canvas carrying whatever a
reader chose to read into it. Measured evidence that this is not hypothetical: on the
London Underground map, the drawn geometry influenced travellers' route choices roughly
*twice* as strongly as their own experienced journey times, with about 30% taking a
slower route the map merely drew as shorter. A reader trusts the geometry whether or not
it was meant to mean anything. So each is now named, including the ones that mean nothing:

- **Vertical position = call depth.** Callers sit above what they call. This is real and
  may be relied on.
- **Horizontal order = declaration order in `ply.yaml`, and nothing else.** It is not
  importance, not sequence, not layering. If a future feature wants this channel it must
  claim it here first.
- **Box size = how much is declared inside it, and nothing else.** A large box is a box
  with many functions, never a more important or riskier one.

**Absence is drawn, never implied (added 2026-08-29).** A component promising nothing was
filled plain white, and blank space reads as background rather than as a state — so the
riskiest thing on the canvas, code the diagram is showing you while saying nothing about
it, was also the quietest. It is now filled with a diagonal hatch: a positive mark, which
a glance can catch, where absence could not be. The dashed border keeps its narrower
meaning ("nothing inside at all yet"), so an empty sketch and a populated box that
promises nothing are no longer the same picture.

**Every colour-coded meaning also carries a mark (added 2026-08-29).** Roughly 8% of men
cannot separate red from green reliably, so no meaning may rest on hue alone: a forbidden
call carries its crossbar, an item owed a human carries its number. Checked in CI, along
with a colour-distance floor under simulated red-green colour blindness — the floor is a
blunt instrument and is documented as such, the marks are what actually make the palette
safe.

**Lines must not cross shallowly, and must never lie along each other (added
2026-08-29).** The one layout property with a large, replicated experimental effect on
reading speed is lines crossing — ranked well above symmetry, bends, or node placement in
controlled studies. The useful form is the refinement rather than the headline: eye
tracking finds a crossing near a right angle is essentially ignored, while a shallow one
sends the eye back and forth and costs accuracy, the penalty fading as the angle opens.
So crossings are not forbidden — forbidding them would over-constrain the layout for no
measured gain — but shallow ones are, and CI holds it. Two lines drawn *along* each other
are worse than any crossing and are a defect in their own right: the reader sees one line
where the document declared two, so a rule goes invisible. Four such overlaps were found
among the forbidden-call routes when this invariant was first written; fixed by giving
every routed forbidden-call and external/`entry:` line its own nested lane — each further
one from the same obstruction pushed one step further out than the last, in the same
order that already keeps them from crossing — and the ratchet now holds at zero.

**One alternative palette, never a theming hook (added 2026-08-29).** These diagrams are
read where dark is a common default, and the render paints its own near-white background,
so a dark reader got a bright panel rather than a diagram. A second palette now follows
`prefers-color-scheme`. It is deliberately *not* configurable: the meanings above are
enforced — red must belong to something forbidden or wrong, nothing un-run may be green,
absence must carry a mark — and a palette a reader could redefine would make every one of
those guarantees unenforceable. A diagram whose colours mean whatever was configured
cannot also be a diagram whose colours can be trusted. So: two expressions of one set of
meanings, both held to the same tests, including the colour-blindness floor — which
caught the first dark red proposed, at 0.200 against ordinary structure, inside the range
where real confusions live.

**The strip (added 2026-08-29).** Every render opens with one line stating what the
document declares and how much of it promises nothing — `11 components · 14 functions ·
2 promise nothing`. It counts promises only, never results, because nothing has run; a
strip that reported results would be inventing them. It exists because those numbers were
always in the document and a reader had to scan every box to recover them. Border: dashed = hollow sketch,
solid = specified, double = sealed pure. Edge dash: solid = may call (checked),
dashed = **declared, not machine-checked**, red-barred = must not. The dashed meaning
was originally stated narrower ("data flows"); externals' `~>` edges and derived
`entry:` edges are that same declared-not-checked fact about a different kind of
crossing, so the wider statement *subsumes* the original rather than adding a second
meaning to the channel — a flow was always exactly "declared, not machine-checked",
this just names what was already true. Small marks: solid square = a contract stands; numbered
amber pin = decision owed; hollow shield = human-attested. A new visual form must draw
from these channels consistently or claim an unused channel and name its single meaning
here — reusing a channel for a second meaning is refused the way an undrawable
construct is. The acceptance check is the **squint test**: blurred, the picture must
still say where the system is weak (pale), wrong (red), unfinished (dashed), and
waiting on a human (amber). A form whose meaning dies under squint is decoration, not
grammar. Words live in tooltips and the frame tooltip; the canvas carries no legend by default —
the picture must work on instinct alone. `ply-render --legend` opts into a compact
legend strip appended below the frame (for docs, onboarding, print), generated from the
same style constants the renderer draws with, so legend and drawing cannot drift apart.

Gate debt: **none as of 2026-08-24** — and this time drawn, not merely assigned. Every
construct in the grammar has a visual form that the renderer actually emits and a test
pins: the last three were `strict` (ink corner notch), `mode: synth` (violet chip fill),
and `examples` (`e×N` token); externals and the derived `entry:` edge followed the same
day, gated on a vetting re-run (vetting 003's "external-elements gate" section) rather
than assumed from the table row alone. The earlier version of this paragraph claimed
closure when those forms existed only as table rows; the gate asks whether a reader of
a real diagram can see the construct, so a row is not enough.

### 7.1a The transcript: the same grammar, written out

`ply-render --text` (and `cargo ply render --text`) writes the document as prose instead
of as a picture: the §7.1 grammar serialised into sentences rather than into shapes.

It is **not** merely the drawing transcribed. It states two things the drawing does not,
both from §5.4c: whether a function's checks were written on it or inherited, and which
ancestor they were inherited from. Those are invisible in a picture whose only channel
for "checked this strongly" is a fill, and the earlier version of this section claimed
the text "carries no information the drawing lacks" while the code carried a comment
saying the opposite — corrected 2026-08-30.

It exists because of a measurement, not a preference. On the committed trading-system
diagram, 474 characters of text are drawn on the canvas and 9,923 are reachable only by
hovering — 95% of what the render says, and all of the reasoning: why a box sits where it
does on the ladder, what a check actually does, which ancestor a promise was inherited
from. A reader who cannot hover gets the labels and none of the meaning. A model reading
the document cannot hover at all.

Reading `ply.yaml` instead is not equivalent, and the gap is the point: the source says
what was **written**, the transcript says what is **true after the rules are applied**.
A missing `checks:` line inherits from the nearest ancestor; a written empty one inherits
nothing and means "check nothing here" (§5.4c). Those look nearly identical and mean
opposite things, and a reader who resolves them wrongly states a confident falsehood
about what is verified.

Three properties are load-bearing:

- **Deterministic by construction.** The renderer takes the parsed document and returns
  a string: no filesystem, no clock, no environment, no locale, no randomness is in
  scope. Every collection walked is insertion-ordered, so iteration is document order.
  There are no sorts and no computed non-integers, so there is no float formatting to
  pin.
- **Generated, never hand-written.** It is produced from the document like a compiler's
  output; no one edits it. A copy is committed beside each vetting scenario, for the same
  reason the drawings are: a change to the wording then arrives in review as a diff a
  person can read, rather than as an invisible shift in what the tool tells people. Those
  copies are gated against a live render, because a stale one would do the opposite of
  what it is for. Nothing in the build depends on them.
- **One derivation where there is one fact.** The sentences both views share come from
  shared functions — the check glosses, the ceiling line, the profile rules, the deny
  rules, the open-question sentence — so those cannot drift. This is a discipline, not a
  mechanism: nothing stops a future sentence being written twice, and the `pure` sentence
  was worded differently in the two views for a day before review caught it. Where a fact
  belongs to only one view (the drawing's compact check row; the text's inherited-from
  attribution) there is nothing to share.

The two views do not agree verbatim, and should not. The picture keeps compact labels such
as `bounded: loop≤2`, `fuzz: 1024 cases`, `e×1`, `⛉`, and `*`; the text expands their
meaning. Demanding verbatim agreement would force the text to be as terse as the picture.
The invariant that holds instead is stronger and drives from the document: **every
component, function, check, contract clause, capability, owned type, profile rule,
default `checks:` list, trusted claim, worked example, entry point, machine-written
marker, seal, `strict` flag, edge, forbidden rule, external and open question in the
document is findable in the transcript.** A construct added to the grammar later cannot
quietly skip it, because the walk binds every field of `Component` and `FnClaim` by name
with no rest pattern: a new field stops the test compiling.

That last sentence was written before it was true. The first version of the walk read the
fields its author remembered and silently skipped four — `pure`, `strict`, `mode` and
`examples` — so deleting the entire worked-examples block, or the seal sentence, left
every test green. It was found by review on 2026-08-30, along with the graver failure it
was supposed to prevent: a function that inherited an empty `checks:` list was told it
had *written* one, collapsing the very §5.4c distinction quoted above. Thirteen deliberate
breakages now run against these tests; before the repair, one died.

`--text` cannot be combined with `--depth`, `--focus` or `--collapse`. Those fold parts of
a drawing away to fit a screen; the text has no screen to fit and always states the whole
document. A run that asks for both is refused rather than silently narrowed, because a
reader handed a quietly-folded transcript would believe they had the complete view.

### 7.2 The watermark

The system has three strata:

1. **Declarative** — everything `ply.yaml` and the attributes express: components,
   edges, capabilities, contracts, checks. Fully drawable (§7.1), fully checkable.
2. **The watermark** — where declaration stops: a function's signature plus its
   contract. Below the mark lies the body. The mark is per-function, and movable in one
   direction only: `mode: synth` lets the model write the body *down from* the mark,
   with the check pipeline holding the line (D8, §5.7).
3. **The floor** — the imperative interior of Rust that no grammar will ever express:
   algorithms, loop bodies, data-structure manipulation. Ply *verifies* below the
   watermark; it never *specifies* there.

A grammar extension may push the watermark lower — express more declaratively — only if
it stays visually depictable (§7.1) and clearly above the floor. Attempting to specify
the floor itself means building a verification language, the abandoned path this project
exists to avoid (§1).

Honesty note: today the declarative stratum expresses exactly two altitudes —
architecture claims and whole-function contracts. The strata between them (state
machines, temporal rules, cross-function queries) are not yet expressible, and this
section must not be read as implying they are. They are candidate watermark-lowering
extensions, admitted one at a time through the §7.1 gate.

The strata make the system's kinds of *unspecified* distinct — **four**, not three:
the floor is permanently unspecifiable, by design. Below the watermark nothing is
owed either: the body is not declared, it is verified against the contract above it.
An `unresolved!` marker (§5.6) is different in kind: specification that is **owed but
missing** — a tracked, numbered hole in the declarative stratum, expected to close. It
sits physically in a body, but the missing decision usually belongs above the mark
(the answer becomes a contract clause, an example, or a branch condition). This is why
§5.6 caps such a fn at check `test`: with a decision unresolved, the contract cannot be
complete, so the watermark is not yet a promise worth proving against. An **external**
(§5.1) is a fourth kind, distinct from all three: **out of scope by ownership**, not by
incompleteness. A component nobody has specified yet and a system somebody else
operates used to render identically — both simply absent from the model — and absence
already means something (`unclaimed`), so the second case had no honest slot. An
external will never be claimed, and that is correct rather than pending: it is not the
floor (nothing here is imperative Rust), not a below-watermark body (there is no
watermark — no signature, no contract), and not `unresolved!` (nobody owes a decision;
the boundary is simply not this codebase's to specify).

## 8. Result JSON

Every command emits one envelope:
`{ "command": "...", "ply_version": "...", "root": <node tree §7>, "diagnostics": [<Diagnostic>...] }`.
Stability rule: additive changes only after M3; the goldens in tests/ui are the contract.

A node whose result was **reused** rather than re-run carries `reused: true` (§5.2a);
the field is absent otherwise, never `false`. It is set only after the node's fingerprint
was recomputed from today's inputs and matched the recorded one, so its presence is a
statement that the recorded result is about the code in front of you. Everything else on
such a node — verdict, statuses, `evidence`, and the diagnostics carrying its node id —
is exactly what the run that earned it emitted.

A node whose verdict came from a sampling engine that **actually ran** additionally
carries `evidence: { "engine": "proptest", "seed": "<64 hex chars>", "cases": 256 }` —
§1's requirement that every verdict name what produced it concretely enough to reproduce
it. `cargo ply verify <path> --seed <hex>` replays that exact run.

`evidence` describes a run that happened, never a run that was declared. It is **absent**
when nothing ran — an `unsupported` shape, a harness that failed to compile — and `cases`
alone is absent when the run happened but its count is neither the declared number nor
knowable: a run cut short by its time budget, or stopped at its first failing case.
`cases` is what the engine reached, so on a run proptest abandoned to its own reject limit
it is the small number it accepted, beside a verdict of `unclaimed`. (Corrected
2026-08-25: `evidence` was attached whenever `fuzz(n)` was declared, so a check that never
ran a single case still reported `cases: n` — adversarial review of the post-004 fixes,
D5. The declared count remains visible on the diagnostic's `check` field.)

One Diagnostic schema for all engines:

```json
{
  "code": "K0502",
  "severity": "error",
  "phase": "verify",
  "engine": "kani",
  "check": "bounded(3)",
  "node_id": "pricing::quote",
  "content_hash": "b3f9…",
  "title": "ensures may fail: |p| p.bid <= p.ask",
  "primary_span": {"file": "crates/pricing/src/lib.rs", "start": [41, 5], "end": [41, 38]},
  "counterexample": {
    "inputs": {"inst": "Instrument { id: 0, tick: -1 }"},
    "kani_witness": "target/ply/playback/pricing_quote_01.json",
    "cargo_test": "crates/pricing/src/ply_generated_cex.rs",
    "trace": [ {"span": "…", "detail": "bid = tick * 2 = -2"} ]
  },
  "assumptions": [{"kind": "assumed_contract", "fn": "parser::parse", "verdict": "fuzzed(256)"}],
  "fixes": [{"title": "add requires inst.tick > 0", "edits": [{"span": "…", "insert": "#[ply::requires(inst.tick > 0)]"}]}],
  "open_item": null
}
```

Three further top-level fields are **additive and command-specific** (2026-08-25, Phase
1b): `trust_surface`, an array of `{kind, subject, node_id, statuses, where?, detail}`
emitted by `audit`; `open_items`, an array of `{kind, id?, node_id, where?, blocking,
detail}` emitted by `worklist`; and `not_carried_forward`, an array of
`{node_id, because}` emitted by `verify`, naming each claim that had a recorded result it
could not use and the inputs that moved (§5.2a). The last is omitted when empty, because
"every result was carried forward" and "nothing was recorded to carry" are the same
absence to a reader of the tree. Each is present only on its own command, and **absent is not the
same as empty**: an empty array means the command looked and found nothing, while an
absent one means it never got to look (a document that failed the schema). `detail` is the
plain sentence the human surface prints; `statuses` carries the D6 names an item bears —
`owed-evidence` on an assumption nothing has exercised, `staleness-unknown` on an
attestation Ply cannot date.

`kani_witness` (renamed from `kani_playback` — §8's stability rule permits this once,
pre-M3) is present whenever the witness-extraction step ran: the exact failing input,
byte-level and engine-version-bound. It is input storage — replaying it exercises the body
but does not re-check the contract, so it is never the failing reproduction. `cargo_test`
is that reproduction: present only when the inputs rendered as stable Rust source (D7),
else `W0541` with a `reason` of `inputs_unrenderable` or `expression_unrenderable`. Its
path is in-crate (D2's generated-module mechanism), not a `tests/` integration file — M3
implements this as `<crate>/src/ply_generated_cex.rs`, one file per crate, declared via a
`mod` line the same way the proof harness is (docs/m3-slice-findings.md).

**A non-result is still feedback.** `timeout`, `unsupported`, and `engine-missing` carry
no counterexample, but the consumer is usually an agent mid-repair, and §1's second
finding applies to them too: feedback without a handle performs little better than none.
So every such diagnostic MUST name the cause in the same concrete terms a fix would need
— the offending construct, why this engine chokes on it, and what would change the
outcome — and SHOULD populate `fixes` with the options. Worked example, the one that
motivated this rule: a function that clones a `BTreeSet` in a loop makes Kani's encoding
intractable at any bound (observed on Ply's own kernel, 2026-08-23). The honest
diagnostic names the type and the encoding, then offers: lower the bound, drop to
`fuzz(n)`, or restructure the hot path to a fixed-size array.

The boundary is absolute: **Ply proposes, never rewrites.** A `fixes` entry is a
suggestion the caller may apply and a human may review as a diff. Ply must never reshape
a user's data structures to suit an engine — a proof about a program the user does not
run is worth less than no proof, and the agent, not the tool, owns that trade.

Diagnostic codes live in one exhaustive enum: `E02xx` config/schema, `E03xx/W03xx`
anchoring, `A04xx/W04xx` architecture and resolution, `E05xx/V05xx/W05xx` contracts and
verification, prefixes `K/P/M/R` reserved for engine-specific codes, `W01xx` environment,
`X09xx` internal errors. Adapters never pass engine stderr/stdout through raw: they parse
it, or fail with `X0901` attaching the raw output for debugging.

## 9. Testing strategy

**A test nobody has watched fail is not evidence.** This is §1's absence-of-evidence
principle turned on Ply's own suite, and it is stated here because Ply has repeatedly
failed it. Every test in this repository must be observed red — against the defect it
names, with a failure message that names that defect and not merely "assertion failed" —
before it is made green. A test written after the code it checks proves only that the
test agrees with the code, which is exactly the reasoning Ply refuses to accept from a
user about their own program.

The evidence for the rule, recorded rather than asserted: D5's first branch (2026-08-26)
shipped with 315 passing tests and six defects, four of them found by adversarial review
rather than by the suite. Two of those were nearly dismissed as environment flakiness.
The worst was a **false clean verdict** — a caller reported `bounded` while standing on a
callee whose proof never covered the arguments it was actually passed — and the commit
before it had honestly reported a timeout, so the feature converted an honest absence of
evidence into evidence. Every fixture its author wrote used scalar parameters; the defect
needed a length-indexed one. The suite was not weak by accident: it was blind in exactly
the shape nobody thought to write.

Two consequences follow, and both are requirements:

1. **A defect found by review enters the suite as a fixture of its own shape**, permanently
   — not as a spot-check on the code path that happened to be wrong. The shape is what was
   missing; the line was only where it surfaced.
2. **Green is not a merge argument.** A passing suite is evidence that the shapes it
   covers still behave; it is never evidence that a feature is correct. Where a feature
   crosses into a subsystem it was not written against — the record and reuse path is the
   one that has caught Ply out twice — the crossing gets its own test, or the feature is
   narrowed until it does not cross.

- **Fixture e2e**: each feature ships a minimal fixture project + `ply.yaml` + expected
  `--json` output as insta goldens. Review goldens like API diffs.
- **Cex validity oracle**: every rendered `cargo_test` must FAIL under `cargo test` in
  the fixture before the fix — with failure output that states the contract, pinned by
  substring — and PASS after; every `kani_witness` artifact must *replay* under
  `cargo kani playback` with the pinned version and its decoded inputs must equal
  `counterexample.inputs` (replay is not required to fail — ADR-0003 caveat 3). A
  rendered test that does not fail, or a witness that does not replay, is `X0902` (an
  adapter bug). This is the primary correctness check of the whole tool — implemented and
  green in M3 on the `clamp` fixture (docs/m3-slice-findings.md); the witness-replay half
  of the oracle (via `cargo kani playback`) is **not yet wired into the M3 e2e suite** —
  recorded honestly as NOT RUN, not skipped silently.
- **Schema goldens**: `schema/ply.schema.json` is golden-tested; a fixture set of valid
  and invalid `ply.yaml` documents pins validation behavior and E0201 pointer paths.
  Implemented in `crates/ply-core/tests/fixtures/schema/` — each invalid document is
  filed beside a `.expected` golden holding the exact diagnostics it must produce, and
  the valid ones must produce none and load. Three further tests hold the schema to the
  code rather than to itself: every schema object with a fixed key vocabulary sets
  `additionalProperties: false`, every key the schema declares is a key the serde model
  actually reads, and the two regex/matcher pairs agree over a corpus.
- **Extraction differential**: property-test call-graph extraction against a naive
  AST-walk reference on generated fixture modules; any disagreement is a bug. Assert
  resolution coverage (D11) on fixtures with known-unresolvable sites.
- **Engine-absence matrix**: run every e2e once per engine with that engine masked out,
  asserting graceful downgrade (`W0110` / `engine-missing` rather than a violation, a
  weak-spec finding or a crash) **and exit 3, never exit 0** — the check did not fail, and
  it also did not happen. One masked-engine case is built (`mutate` with cargo-mutants
  masked, `tests/e2e/tests/mutate_engine_missing.rs`); the full matrix is not.
- **Self-hosting**: golden tests for `skill` output, and `cargo ply check` runs clean on
  this repo (this workspace gets its own `ply.yaml` from M2 onward, kept green in CI).
  Once M5 lands, `cargo ply verify` runs over this workspace too: contracts on our own
  core functions (config merge, anchor resolution, verdict aggregation), verified by our
  own pipeline, in CI.

## 10. Milestones (accretive; every prior acceptance test stays green)

**M0 — feasibility spike** (~2 sessions; nothing else starts until ADR-0003 exists)
- One throwaway fixture crate; Kani version pinned. Exercise: a contracted private free
  function; a contracted method; a user-defined input type; a same-crate stubbed callee;
  a cross-crate callee (expect failure — document it); cargo-mutants with a custom test
  command running a generated harness; a real contract violation with
  playback replay; one attempt at rendering that cex as a plain `#[test]`; the
  `cfg_attr` emission from a prototype `ply-attrs`; the in-crate proof-module mechanism.
- Accept: ADR-0003 records, per item, works / fails / needs-flag, and lists every spec
  amendment that follows. The fixture and a script to re-run it are kept under
  `tests/spike/`.

**M1 — schema + model + anchors** (~3 sessions)
- `schema/ply.schema.json`; serde model; validation with pointer→line mapping;
  multi-file merge; micro-syntax parsers; E02xx suite (goldens). ADR-0002 written.
- syn-based item index; anchor resolution with suggestions (E0301); fingerprint
  recording and result reuse (D14, §5.2a — engine fields filled as engines land);
  `check` and `worklist` (markers only); `--json` envelope everywhere.
- Accept: a fixture with 2 crates, nested components, one broken anchor, one claim whose
  recorded result is reused on a second run, and one whose fingerprint moved and is
  re-run — golden output exact; invalid-yaml fixture set pinned.

**M2 — architecture engine** (~4 sessions)
- Crate tier from `cargo metadata` (A0401/A0405 as errors). Item tier behind
  `trait Extractor`: calls, caps via capmap, mutates, unresolved-call accounting;
  warnings by default, `strict` upgrade; W0411/W0412; profile bans with allow-escapes;
  `audit` (escapes only at this stage). ADR-0001 written.
- Self-host: this workspace gets its own `ply.yaml`, checked in CI.
- Accept: a fixture with a crate-tier violation (error), an item-tier cross-boundary
  call (warning, then error under `strict`), a pure violation, an owns violation, and a
  profile escape — goldens for each; a clean fixture producing zero diagnostics; a
  fixture with a deliberately unresolvable call asserting the coverage metric and W0412.

**M3 — Kani adapter + counterexample pipeline** (~8–10 sessions; shaped by ADR-0003)
- ply-attrs emitting `cfg_attr(kani, ...)`; harvest of pre-existing cfg_attr-kani
  attributes; `ply.yaml` contract merge; spec-subset validation (E0501); supported-
  signature gate (V0505 / `unsupported`); in-crate proof-module generation for
  `bounded`; run and parse Kani; witness artifacts (input storage) + rendered
  contract-asserting tests where possible (docs/plans/d7-replayable-tests.md); witness
  budget policy decided there; Diagnostic assembly with suggested fixes for the
  mechanical cases (a missing `requires` derived from a failed side condition).
- Callees-first order; `stub_verified` under D5's conditions; `conditional` verdicts
  with assumption chains.
- Accept: a fixture where a cex is found, its playback reproduces, and its rendered test
  fails then passes, JSON golden; a two-fn fixture proving modular verification (caller
  stubs proved callee; assumption chain empty); a caller-of-fuzzed-callee fixture
  earning `conditional`; an unsupported-signature fixture earning `unsupported`, not a
  build failure.
- **M3 thin-slice status (docs/m3-slice-findings.md)**: the vertical slice
  `ply.yaml` + contracted fn → Kani harness (with mandatory unwind emission) → run →
  parse → `bounded(k)`/`violation`+witness/`timeout` → D7 rendered test → §8 envelope is
  built and green end to end (`crates/ply-attrs`, `crates/ply-core`, `crates/ply-cli`,
  4 fixtures under `tests/fixtures/`, e2e oracle tests under `tests/e2e/`). NOT yet
  built, left for the rest of M3: callees-first/`stub_verified`/`conditional` (D5),
  `ply.yaml` contract merge (only inline attributes are read), the mechanical-fix
  suggestions, `impl`-method/generic/`check_with` support, and the witness-replay half
  of the §9 oracle (`cargo kani playback` is implemented as an adapter function but not
  yet wired into the e2e suite).

**M4 — fuzz + test + mutate checks** (~4 sessions)
- proptest harness generation over the supported-signature set (ints biased small,
  structs field-by-field, Vec length 0–8; `requires` as rejection filter with a
  high-rejection warning); shrink → rendered test.
- `test` check: example tests + generated direct contract cases.
- cargo-mutants scoped `--re` per fn with the test/fuzz kill signal (D12); E0504;
  weak-spec detection (W0502) wired into `worklist`.
- Accept: a seeded-bug fixture shrunk to a minimal cex; a vacuous-ensures fixture
  flagged weak; a strong-spec fixture earning `·spec-strong`; a mutate-without-tier
  fixture producing E0504.
- **M4 status (docs/m4-findings.md)**: built and green end to end —
  `crates/ply-core/src/{fuzz_gen,harness_crate,engines/fuzz,engines/mutants}.rs`, the
  shape-aware default-check routing (`ply-cli/src/verify.rs::default_checks_for`), and 5
  new fixtures under `tests/fixtures/` (`fuzzbug`, `weakspec`, `strongspec`, `mutatetier`,
  `btreeset`) with e2e tests under `tests/e2e/tests/`, covering all four of this
  milestone's own acceptance fixtures plus the fifth (a `BTreeSet<u8>` fixture) that is
  the point of the whole milestone: a shape §5.4b excludes from `bounded` earning an
  honest `fuzzed(256)` verdict via the shape-aware default route. Fuzz-found violations
  render through the *same* `contract_rt` renderer the Kani path uses (D7's design,
  now both consumers wired) — confirmed on the `fuzzbug` fixture, FAIL-then-PASS. Struct
  parameters ("field-by-field" fuzzing) are **not implemented** this session — a
  deliberate scope cut recorded in docs/m4-findings.md, not a silent gap: `BTreeSet` was
  used for the Kani-excluded acceptance shape instead (the spec's own "recursive, or a
  `BTreeSet`" alternative). §6 and §5.4c amended for two falsified claims: the
  engine-timeout default was flat, not shape-aware, and the mutate mechanism's own
  `--gitignore false` guidance is superseded by `--copy-target true` (a second,
  previously-undiscovered copy-skip, unconditional on gitignore).

**M5 — verdict tree, aggregation, skill, polish** (~3 sessions)
- Verdict tree with worst-of, status propagation, and open items; `--depth`/`--focus`;
  `audit` completed (escapes + assumptions + derived); `skill` generation embedding the
  schema; `--only-changed`.
- **Partly landed early, 2026-08-25** (docs/post-004-fixes.md): D5's third branch and its
  `ply.yaml`-declared-contract sibling, `conditional`/`owed-evidence` propagating upward
  as statuses, and §8's `assumptions` array — brought forward because vetting 004 showed
  the boundary *is* the product for the fragment-first thesis. Still M5's own work:
  callees-first scheduling (so D5's first branch, `stub_verified`, can fire at all),
  `audit`/`worklist`, `--depth`/`--focus`, `--only-changed`.
- Accept: a nested fixture where one weak leaf drags down the root verdict and a
  `conditional` mid-tree node propagates as a status; tree JSON golden; skill file
  golden.

**M6 — synth (experimental)** (~3 sessions)
- The §5.7 loop with a pluggable model command; derived marking; hand-edit detection.
- Accept: a fixture where a synth fn with 2 examples + 1 ensures converges within 5
  rounds against the M3/M4 pipeline. Record the transcript as a fixture artifact. The
  pipeline mechanics are what's under test, so provide a `MockModel` that replays a
  canned transcript, and allow marking the live-model test `#[ignore]` in CI.

**M7 — Verus prove check + optional Miri** (**optional**: do not start without an ADR,
informed by M3–M5 experience, arguing the translation is worth its cost; expect it to be
compiler-scale work, not a thin adapter)
- Translate the spec subset to Verus for a profile-restricted fragment; adapter; verdict
  `proved`; document the fragment's limits honestly in SCHEMA.md.
- Accept: a gcd-style fixture proved end to end; a fixture outside the fragment failing
  with a clear diagnostic naming the unsupported construct.

Out of scope entirely: concurrency verification (loom, weak-memory models, lock-free
data structures — deliberately excluded, not forgotten; the engines here check
single-threaded behavior), canvas/GUI, data-flow (`~>`) checking, LSP/editor integration,
model-based two-state specs beyond `old()` (sequence histories, FIFO ordering — `old()`
itself is in scope, §5.4a), the `induct` check, cross-crate `stub_verified`,
proof-backed mutation, async fns in verified components, and the internals of crates
outside the workspace.

## 11. Session protocol

Start each session from this spec (by § reference) plus the current milestone's
acceptance list. End with `cargo test` green, goldens reviewed, and `cargo ply check`
green on this repo (M2 onward). Never advance a milestone while the cex validity oracle
(§9) is red. Record any decision this spec does not cover as a one-paragraph ADR in
`docs/adr/`; amend the spec rather than contradicting it.
