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
| status | (Ply) A qualifier on a verdict: `conditional` (rests on assumed contracts), `owed-evidence` (an assumed contract that nothing has yet checked against the real code — it travels with `conditional` and is what turns "we assumed this" into "and here is the debt"), `stale`, `weak-spec`, `unsupported`, `engine-missing`, `timeout`, `inconclusive`. |
| component | (Ply) A named architectural unit, declared in `ply.yaml` and anchored to a crate or module. |
| capability (cap) | (Ply) A coarse effect a component is allowed: `net fs db time rand proc unsafe`. |
| anchor | (Ply) The real code item (crate, module, or function) a claim attaches to. |
| fingerprint | (Ply) The recorded hash of everything a verdict depended on: item body, contract text, callee contracts, engine name + version + flags, features, target. A claim whose fingerprint no longer matches is *stale*. |
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
| D6 | Verdicts aggregate upward as **worst-of** over the evidence order `violation < unclaimed < tested < fuzzed < bounded < proved`. Statuses (`conditional`, `owed-evidence`, `stale`, `weak-spec`, `unsupported`, `engine-missing`, `timeout`, `inconclusive`) do not sit in that order; they propagate upward as flags and open-item counts alongside it. `owed-evidence` is the debt half of `conditional` (§5.5, added to this list 2026-08-25 — it was being emitted before it was defined): `conditional` says the verdict rests on an assumed contract, `owed-evidence` says nothing has yet checked that contract against the real body. They are two facts, and a run that discharges the second keeps the first. | A proof in one corner must not hide a merely-tested boundary in another; and a timeout is not a weaker proof, it is a different kind of fact. |
| D7 | Every counterexample is stored as **Kani witness data** (the exact input bytes, engine-version-bound — input storage, not a reproduction: Kani's playback replays the function body only and never re-evaluates contract closures, so an `ensures` violation replays green) and, whenever the inputs can be rendered as stable Rust source, additionally as an ordinary `#[test]` that **asserts the postcondition explicitly** and therefore fails under plain `cargo test` — the only red artifact, and D7's repair target. The assertion is rendered overflow-safe (widened/checked arithmetic), so it fails by stating the contract, never by re-triggering an incidental panic inside the check. When rendering fails, the diagnostic says so (`W0541`) — inputs are never fabricated. **The red-test promise is qualified to failures arising in the function's own body.** A failure that depends on a *stubbed* callee's invented return (§5.5's assumed-contract branch) has no faithful plain-Rust reproduction: the rendered test calls the real callee, which never produces the stub's value, so it is emitted green — verified in `tests/spike/kani-pin/FINDINGS.md`, and unfixable by engine version. Such a failure reports `W0541` with reason `stub_substituted`, carrying the fabricated value and a proposed contract tightening, because its repair target is the declared contract rather than the code (docs/plans/d7-stub-failures.md). Rendering a red test against a rewritten body is refused: it would fail for a program the user does not run. | Playback is exact but body-only (ADR-0003 caveat 3); the portable test is the agent-friendly repair target and carries the red-test promise alone. Implemented and verified end-to-end in M3 (docs/m3-slice-findings.md): the `clamp` fixture's rendered `#[test]` fails before a contract fix and passes after, on the pinned Kani 0.67.0. |
| D8 | `synth` mode (the model writes the function body) is orchestration over the check pipeline: prompt assembly, a retry loop, and marking the output as derived. It ships last and adds no checking machinery. | Thin by design. |
| D9 | Implementation language Rust; three crates (§4); engines run as subprocesses with version detection at startup, never linked as libraries. The Kani version is pinned in `ply.toml` and recorded in every fingerprint. | Engine version churn must not break our build or silently invalidate old evidence. |
| D10 | `node_id` is the component path + item path only. The blake3 `content_hash` is a sibling field, never part of the ID. All JSON output shares one envelope (§8) containing a tree of nodes plus a flat diagnostics list. | IDs must survive edits so external consumers (a future canvas UI) can track nodes across runs. Design for the canvas now; build it never (out of scope). |
| D11 | Extraction may be incomplete but never silently so: every call site the extractor cannot resolve is counted and reported (W0412 plus a per-component coverage metric in `check` output). | Visibility is what makes the advisory tier (D4) and any future `strict` opt-in meaningful. |
| D12 | A function declares a **checks list**, e.g. `checks: [bounded(3), fuzz(256), mutate]`. `mutate` requires a `test` or `fuzz` entry in the same list (else `E0504`) and uses only those as its mutant-kill signal, scoped per function with cargo-mutants' `--re`. | One base check could not express "bounded plus a fuzz-backed mutation tier". Running Kani once per mutant costs minutes per mutant per function; proof-backed mutation needs an opt-in budget, which is out of scope. |
| D13 | **Spike before build** (milestone M0): every engine-facing detail in this spec is provisional until a hands-on spike, with the pinned Kani version, records in ADR-0003 what actually works — attribute emission, in-crate harness modules, `stub_verified`, playback, input construction. The spec is then amended to match reality. | The engine surface is the highest-risk part of the design; paper decisions there are guesses. |
| D14 | `ply.lock` records, per claim, a **fingerprint**: item token-stream hash, merged contract text, callee contract hashes, engine name + version + flags, active features, target triple. A verdict is `stale` when any part changed. `cargo ply accept` re-blesses fingerprints and refuses (`E0303`) nodes whose last run failed. The fingerprint is also the verification cache key: `verify` skips any node whose fingerprint matches a recorded passing verdict and reuses it; `--force` reruns. | An old success must not bless changed assumptions or a different toolchain; without an accept verb, staleness warnings accumulate until they mean nothing; and without fingerprint-keyed skipping, every CI run re-pays full engine cost and the tool becomes nightly-only. |

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
                            # accept|synth|skill
  templates/                # harness code templates (plain format strings)
  tests/
    ui/                     # ply.yaml + Rust fixtures + golden JSON (insta)
    fixtures/               # small cargo projects used as check targets
    e2e/                    # end-to-end: fixture → engines → expected verdicts
  docs/
    SCHEMA.md               # user-facing reference for ply.yaml (generated ok)
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
`entry:` list is `W0410` — nothing in the document says how it connects. Externals
have no staleness machinery: there is no body, no contract, no evidence string to
fingerprint, so the tooltip and the (future) audit line always say plainly
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

### 5.2 Anchoring & staleness

Every component anchors to a real crate or module; every fn claim anchors to a real
function. `ply check` resolves anchors via the extractor. An unresolvable anchor →
`E0301` with nearest-name suggestions (edit distance over the item index). **A renamed
function must break CI, not silently orphan its claims.**

Each claim's fingerprint (D14) lives in `ply.lock` (committed). When any part of
the fingerprint no longer matches, `ply check` reports `W0302 claim may be stale` and the
node carries status `stale`. `cargo ply accept [node_id ...|--all]` re-records
fingerprints once a human (or an agent, after verifying) confirms the claims still hold;
it refuses (`E0303`) nodes whose last verify run failed.

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

#### 5.4a Spec expression subset (contracts only)

This subset applies to `requires`/`ensures` — the expressions sent to proof engines.
`examples` entries are exempt: they are arbitrary Rust `==` expressions, compiled as
plain `#[test]`s and never translated for an engine.

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
substitute the snapshot. `old()` in `requires` is meaningless and rejected (`E0501`).
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

v1 supports functions whose parameters and return type are, recursively:

- **integers, `bool`, `char`, `Option<T>`, `Result<T,E>`** of supported types — cheap
  unconditionally (~0.1s);
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
- **`&T`/`&[T]`** of the above (built from an owned value in the harness).

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
non-exhaustive or private-field types without a user-supplied generator — yields status
`unsupported` with diagnostic `V0505` naming the offending type. Unsupported is a
reported fact, never a harness build failure. A user-supplied generator hook (a
`pure`-marked constructor function named in `ply.yaml`) lifts a type into the supported
set; its design is validated in the M0 spike.

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

Per-harness time budget: every engine invocation carries a hard cap (`--engine-timeout`,
§6). Exceeding it yields `timeout`, never a silent hang and never a `violation`. The cap
is on the **whole invocation**, not on one phase of it: cargo-mutants copies the tree,
builds an unmutated baseline, and then runs the tests once per mutant, and its own `-t`
caps only that last phase — so Ply wraps the invocation in `timeout` as well (10× the
per-mutant budget, minimum 120s) and reports exit code 124 as `M0601`/`timeout`. A
`mutate` run that produced no mutant count at all (killed by that cap, engine missing, or
output Ply could not read) carries the `inconclusive` status, never `weak-spec`:
`weak-spec` asserts a finding, and no engine made one (2026-08-24 M4 review, D5). D14
caches passing verdicts only, so a timing-out function re-pays its cap on every run —
which is why the cap must be small by default and the status cheap to re-report.

Default checks: shape-aware. `[bounded(2)]` when the fn has a contract **and** its
signature passes the §5.4b gate; `[fuzz(256)]` when it has a contract whose shape §5.4b
excludes; none otherwise. A flat `[bounded(2)]` default would route most contracted
functions in ordinary Rust into `unsupported` or a multi-minute timeout.

#### 5.4d Trusted claims

Some load-bearing properties live outside Ply's reach — cross-thread safety proven by a
loom test, a paper proof, an external audit. A `trusted` entry records such a claim with
its evidence: `{ claim, evidence }`. Trusted claims change no verdict and run no engine;
they exist so the tree is honest — without them, a node whose real correctness argument
is external renders indistinguishably green. They appear in `cargo ply audit` as part of
the trust surface and carry a distinct visual form (§7.1). An agent must never add or
edit a `trusted` entry on its own judgment; attestation is a human act.

**A trusted claim goes stale like any other evidence.** An entry records the content hash
of the item it attests (D14's fingerprint, item body and contract text). When the item
changes, the attestation no longer covers what it vouched for: the entry is marked `stale`
and draws the stale corner marker beside its shield, and `audit` lists it as owed
re-attestation. Without this a `trusted` entry outlives the code it described — the shield
renders identically fresh forever, and a human's word about last year's function silently
vouches for this year's. That is evidence lying, in the one construct built entirely on
trust. Re-attestation is a human act too: `accept` does not clear it.

### 5.5 Modular composition (D5)

Verification runs callees-before-callers over the call graph. To verify fn `f` that calls
fn `g`, the split is on **what `g` offers**, in three branches — the first two keyed on
the evidence behind `g`'s contract, the third on there being no contract to key on:

- `g` passed its own Kani contract proof this run, and `g` is in the same crate →
  generate `f`'s harness with `#[kani::stub_verified(g)]`. Clean verdict.
- Anything else *that still has a declared contract* — `g` merely fuzzed or tested, `g` in
  another crate, `f` and `g` in a cycle, or `g` carrying no verification at all but a
  contract declared for it in `ply.yaml` (§5.4's external-spec route) → verify `f`
  assuming `g`'s contract, stub `g` out of the proof, and mark `f`'s verdict
  `conditional` (`W0511`), listing each assumed contract.
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
   checked. An unclaimed callee *below a contracted callee* is a gap that stays open until
   D5's first branch stubs the contracted one — see this section's limits below, where it
   is stated rather than implied.
3. **An assumed boundary contract is owed evidence until something exercises it.** A
   contract declared in `ply.yaml` for an unclaimed callee is trusted, and trust that is
   never checked is green paint. The assumption is auditable (`cargo ply audit`'s trust
   surface), stale-able (D14), and — the part that makes it better than trust — checkable
   by the cheap tier: `fuzz` has no trouble crossing the boundary, so a declared contract
   on a legacy callee can be fuzz-checked against the real legacy body. Until it is, the
   caller's node carries the `owed-evidence` status, and **`cargo ply audit` lists it**:
   the callee, the promise, the caller resting on it, and what would discharge it
   (2026-08-25, Phase 1b — before that both commands were unbuilt and this paragraph
   described them in the present tense anyway, which made the enforcement loop an IOU).
   `cargo ply worklist` does not list it yet. `conditional` is the *normal*
   state of a legacy-extension codebase, so it
   must read as routine and legible rather than as an alarm — the annotation carries the
   trust story, and a user who learns to skip it has lost it.

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

Two first-party gaps remain open, and are recorded in TODO.md rather than papered over:
(a) the rule inspects the **claimed function's own body**, so a caller that calls a
*contracted* callee `g` still acquires whatever `g` itself calls — until D5's first
branch (`stub_verified`) lands, `g` is inlined rather than stubbed, and an unclaimed
callee one level below `g` travels into the caller's proof unnamed; (b) calls Ply's
reader cannot see at all — generated by a macro, routed through a `#[path = "..."]`
module attribute, or made through a function pointer or trait method — are not call
sites for it.

A `ply.yaml` fn entry that declares `requires`/`ensures` and asks for no `checks` is a
**boundary contract declaration**, not a claim: it exists so callers can assume something
about that function. It contributes an assumption and earns no node of its own, in this
crate or another — reporting it as an `unclaimed` claim would say the opposite of what
was written.

The verdict tree shows each verdict's assumption chain; `conditional` propagates upward as a
status (D6). Cross-crate `stub_verified` (Kani's wrapper/double-stub workaround) is out
of scope for v1.

### 5.6 Underspecification

`ply::unresolved!(147, "employee discount undecided")` marks a decision nobody has made
yet. It expands to `unimplemented!("unresolved #147: employee discount undecided")` —
always, dev and prod alike. Simple, honest, greppable. `ply worklist` lists every marker
(macro or `ply.yaml` registry) with its span, enclosing component, and blocking status.
A fn containing `unresolved!` is capped at check `test`, flagged `W0521`.

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
cargo ply check              # schema + anchors + staleness + architecture. Fast, no engines.
                             # IMPLEMENTED: schema + anchors only (see below).
cargo ply verify [path|fn]   # run checks via engines, callees first; write cex artifacts
                             # (skips fingerprint-fresh passes, D14; --force reruns)
cargo ply tree               # verdict tree, worst-of aggregation, assumption chains
cargo ply worklist           # unresolved markers + weak specs (W0502) + stale claims (W0302)
cargo ply audit              # trust surface: profile escapes, assumed contracts, derived fns
                             # IMPLEMENTED: six tiers, no engines (see below).
cargo ply accept [id|--all]  # re-record fingerprints in ply.lock (§5.2)
cargo ply doctor             # engine presence + versions vs pins; prints the exact
                             # install command for each missing engine, never installs
cargo ply synth <fn>         # M6
cargo ply skill              # (re)generate docs/PLY.skill.md from schema + diag registry
```

**`check` implements two of its four tiers (2026-08-25, Phase 1a), and says so in its own
output.** Schema (the document against `schema/ply.schema.json` — `E0201`, `E0204` — then
every document-local rule that needs no code behind the anchors) and anchors (every fn
claim resolved through the same `discover_fn` `verify` uses, so the two commands never
disagree about which claims point at real code; `E0301` names the nearest item-index
name, and where the `use`-following resolver can see the function somewhere this slice
cannot verify from, the diagnostic says *that* instead of "not found"). Staleness needs
`ply.lock`, which nothing writes yet; the architecture tier is M2. Both gaps are carried
in the `--json` envelope as a `coverage.not_checked` array and printed under "What this
command did NOT check", because a command that reports only findings lets a clean run
read as full coverage — the same failure as an absence of evidence reported as a pass
(§1). `check` runs no engines, so every node in its envelope carries the verdict
`unclaimed`: that is the command reporting no evidence of its own, not a judgement about
the code, and the human surface says so in as many words. `check`'s exit codes are 0
clean or advisory-only, 1 any error-severity finding, 2 tool error; `--fail-on` is not
wired to it yet (its `evidence` default is meaningless for a command that gathers none),
and neither are `--only-changed` or `--engine-timeout`.

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
missing tiers: **trusted-claim staleness** and **assumption discharge** both need
`ply.lock` (Phase 1c), so every attestation is listed undated and every assumption is
listed owed; **helper evidence** needs a verdict, and this command produces none; call
sites Ply's reader cannot see (§5.5's own gaps) are absent from the assumed-contract list;
and the **architecture bans** an escape suppresses are M2, so today an escape switches
nothing off. Like `check`, `audit` runs no engines, so every node in its envelope reads
`unclaimed`, and its last line says so. The surface itself rides in the envelope as
`trust_surface`, an additive §8 field.

Global flags: `--json` (schema §8, the agent surface — every command supports it),
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

Housekeeping: Ply owns everything under `target/ply/` and every generated test whose name
starts with `ply_cex_`; `verify` deletes generated artifacts whose claims no longer
exist.

## 7. Recursion & aggregation (the zoom model)

The model is a tree: workspace → components → nested components → fns. Every node
carries `{ id, kind, anchor, content_hash, verdict, statuses, worst_descendant,
open_items }`. `verdict` is the node's own claim status; `worst_descendant` implements
D6 over the evidence order; `statuses` and `open_items` (unresolved markers, weak specs,
conditional, owed-evidence or stale verdicts) propagate upward as counts.

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
| capabilities / `pure` | badge row on the box; `pure` = a sealed border, no badges |
| profile | tag on the box |
| checks list | glyph row on the fn chip |
| verdict | node fill on the ordinal scale: `violation` red → `proved` deepest green; `unclaimed` unfilled |
| statuses | corner markers on the node (conditional, stale, weak-spec, …) |
| worst_descendant | a collapsed box takes its weakest descendant's fill — D6 made visible |
| assumption chain | thin dotted arrows from a verdict to the contracts it assumed |
| unresolved marker | numbered pin on the fn or component |
| trusted claim | hollow shield badge on the node — attested by named evidence, not machine-checked |
| contract clauses (`requires`/`ensures`) | a contract mark on the fn chip — a solid ink bar the full height of the chip's left edge (a gutter mark: "this row carries something binding"); the tooltip lists each clause verbatim. This draws the §7.2 watermark per function: marked = a promise stands at the mark; bare = signature only. (The renderer sees only YAML-declared clauses; inline attributes join when `cargo ply` emits the §8 envelope.) |
| declared ceiling | component fill, low-saturation, on the verdict ordinal scale: the strongest verdict the component's declared checks *could* earn — per fn the strongest check kind (`test`→tested … `prove`→proved; `mutate` strengthens, never lifts; no checks = unclaimed, unfilled), folded worst-of by the kernel's container rule. A ceiling is a promise, not proof: it never uses the full-saturation fill reserved for earned verdicts, and its tooltip says none of it has run |
| `strict` | a solid ink triangle notch in the box's top-right corner — the "flagged, zero tolerance" instinct; tooltip already explains errors-not-warnings |
| `mode: synth` | the fn chip's fill turns light violet — violet is hereby the authorship channel, its single meaning "machine-written": the body below the watermark is synthesized from the contract, with the checks holding the line. Tooltip says exactly that |
| `examples` | a gray `e×N` token in the chip's annotation area (next to the `T=...` note); the tooltip already counts and the `test` check runs them |
| hollow component (derived, like findings) | a component that declares nothing inside — no fns, no nested components — draws with a dashed border: a sketch outline, nothing to zoom into yet. Tooltip says so plainly. Derived from absence rather than declared; the natural state of every box in top-down authoring, expected to solidify as claims arrive |
| finding (tool-computed, not declared) | the offending item drawn in error red with an `E####` badge; its tooltip leads with the diagnostic. A finding with no drawable item attaches a red count to the workspace title. A document with findings still renders — a picture that refuses to draw hides the problem it should be showing (origin: fault-injection demo, where a faulted toolchain drew `bounded(0)` as legitimate evidence) |
| external | a solid-bordered, unfilled, anchor-less, badge-less box **outside the workspace frame** — position extends its one declared meaning (containment) from "inside the box = part of the component" to "inside the frame = part of the system"; no new channel. Never on the verdict scale, not even `unclaimed`. Tooltip: "⟨name⟩ — a system or person outside this codebase: ⟨note⟩. Ply draws it so the boundary is visible, but checks nothing about it — every arrow touching it is a declaration, not a verified fact." |
| `entry:` (derived edge) | a dashed arrow, labeled `entry`, from the reachable fn to the external that can reach it — crossing the frame border like any `~>` edge. Not a declared edge (fn claims are not edge endpoints, §5.3); the renderer derives it the same way it derives the ceiling fill. Tooltip names the fn, the external, and lists each `requires` clause now standing as an environmental assumption |

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
green = evidence; amber = a human's attention (owed, or vouched); ink = structure.
Saturation: pastel = promised, saturated = earned. Border: dashed = hollow sketch,
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
anchoring/staleness, `A04xx/W04xx` architecture and resolution, `E05xx/V05xx/W05xx` contracts and
verification, prefixes `K/P/M/R` reserved for engine-specific codes, `W01xx` environment,
`X09xx` internal errors. Adapters never pass engine stderr/stdout through raw: they parse
it, or fail with `X0901` attaching the raw output for debugging.

## 9. Testing strategy

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
  recording and staleness (W0302, D14 — engine fields filled as engines land); `accept`;
  `check` and `worklist` (markers only); `--json` envelope everywhere.
- Accept: a fixture with 2 crates, nested components, one broken anchor, and one stale
  claim (then blessed via `accept`) — golden output exact; invalid-yaml fixture set
  pinned.

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
