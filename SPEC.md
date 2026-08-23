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
| concrete playback | Kani's mechanism for replaying a counterexample: it emits the raw input bytes and re-runs them through the harness via `cargo kani playback`. Tied to the Kani version and flags that produced it. |
| Verus | A deductive verifier for Rust: it proves claims for *all* inputs, with no bound, in exchange for a restricted language subset and more annotation work. |
| proptest / property testing | Running a function on hundreds of generated random inputs and checking its contract on each. *Shrinking* reduces a failing input to a minimal one. |
| Arbitrary | The trait (in both Kani and proptest, separately) that lets an engine construct values of a type. A function is only checkable if its inputs are constructible. |
| mutation testing | Planting small deliberate bugs (*mutants*) in the code and checking whether the specs catch (*kill*) them. A surviving mutant means the spec is weak. `cargo-mutants` is the engine. |
| vacuous spec | A specification that verifies while constraining nothing. |
| Miri | An interpreter that detects undefined behavior in unsafe Rust. |
| harness | A small generated program that exercises one function under one engine. Ply generates these; users never write them. |
| check | (Ply) One verification method applied to a function: `test`, `fuzz(n)`, `bounded(k)`, `prove`, or `mutate`. A function declares a *checks list*. |
| verdict | (Ply) The evidence level a function's passing checks earned: `tested`, `fuzzed(n)`, `bounded(k)`, `proved`. |
| status | (Ply) A qualifier on a verdict: `conditional` (rests on assumed contracts), `stale`, `weak-spec`, `unsupported`, `engine-missing`, `timeout`, `inconclusive`. |
| component | (Ply) A named architectural unit, declared in `ply.yaml` and anchored to a crate or module. |
| capability (cap) | (Ply) A coarse effect a component is allowed: `net fs db time rand proc unsafe`. |
| anchor | (Ply) The real code item (crate, module, or function) a claim attaches to. |
| fingerprint | (Ply) The recorded hash of everything a verdict depended on: item body, contract text, callee contracts, engine name + version + flags, features, target. A claim whose fingerprint no longer matches is *stale*. |
| verdict tree | (Ply) The aggregated per-node verdicts for the whole workspace, rendered by `cargo ply tree`. |
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
   went from 68% to 97% in one year; Verus/Rust sits at 44% and climbing.
2. **Feedback quality is the dominant variable.** Structured feedback that includes a
   counterexample moves agent success rates from ~0% to ~80% on identical models. Every
   falsified executable claim Ply reports therefore MUST carry a concrete failing input
   and, where possible, a replayable test. (Architecture violations, timeouts, tool
   errors, and surviving mutants have no input witness; they carry spans and evidence of
   their own kind.)
3. **Local success does not compose.** Models that verify single functions at 95%+
   collapse below 5% on multi-function programs that require cross-boundary reasoning.
   Verification is therefore modular by construction: Ply checks every function against
   its own contract, and checks callers against their callees' *contracts*, never their
   bodies.
4. **Machine-written specs go vacuous.** A spec can verify while claiming nothing.
   Mutation testing is therefore a first-class check: a contract that fails to kill
   mutants is flagged as weak.

Ply's job: one schema for claims, one router to engines, one JSON result schema, one
worklist. The engines already exist and others maintain them. **We build glue and UX,
never solvers.** A session that finds itself implementing SMT encoding, model checking, or
proof search has gone off-spec — stop.

## 2. Fixed design decisions (do not relitigate)

| # | Decision | Rationale |
|---|---|---|
| D1 | Plain stable Rust is the only executable artifact. Specs never compile to code, except Ply-generated harnesses, proof modules, and tests. | Zero migration cost; everything works with bare cargo. |
| D2 | Contracts are written as **`#[ply::requires(...)]` / `#[ply::ensures(...)]`** attributes on the function. The `ply-attrs` macro re-emits the original function unchanged, adding `#[cfg_attr(kani, kani::requires(...))]` (and the ensures equivalent). Under plain cargo the attributes vanish; under `cargo kani` they instrument **the real function** — never a copy. Proof harnesses are generated into a `cfg(kani)`-gated module inside the target crate so they see private items. Pre-existing `#[cfg_attr(kani, kani::requires(...))]` attributes are harvested and merged by conjunction. | Kani's `proof_for_contract` verifies the function the attributes annotate; contracts on a generated copy would verify a different symbol. `cfg_attr` keeps bare `cargo build` working (D1). The in-crate module mechanism (generated file + one module declaration, or an equivalent include) is settled by the M0 spike; the fallback is verifying `pub` items only from a sibling harness crate. |
| D3 | Architecture claims, checks, capabilities, ownership, profiles, and the unresolved registry live in **`ply.yaml`**, validated against a normative JSON Schema (§5). | These claims are cross-cutting and have no natural attribute location. YAML plus a schema needs no parser of our own, and agents emit YAML reliably. The schema, not prose, is the formal definition. |
| D4 | Architecture enforcement has two tiers. **Crate-level dependency rules** (from `cargo metadata`, which is exact) are errors and default-deny between declared components. **Item-level rules** (calls, capabilities, ownership — from syn, which is approximate) are warnings by default; a component opts into item-level errors with `strict: true`. | Default-deny is only honest over facts that are sound. Crate dependency data is sound today; syn-derived call data is not (no name resolution, no macro expansion). Advisory-until-strict gives teeth without theater. |
| D5 | Verification is modular and evidence-honest. Kani's `stub_verified(g)` is used only when `g` itself passed a Kani contract proof this run, in the same crate. Any weaker case — callee merely fuzzed or tested, callee in another crate, cycle in the call graph — verifies the caller against an *assumed* contract and marks the verdict **`conditional`**, listing the assumptions. A conditional verdict never reads as plain `bounded`. | Stubbing is a soundness claim; fuzzing does not license it. Kani contracts are crate-local (the cross-crate wrapper/double-stub workaround is out of scope for v1). Never inline a contracted callee's body. |
| D6 | Verdicts aggregate upward as **worst-of** over the evidence order `violation < unclaimed < tested < fuzzed < bounded < proved`. Statuses (`conditional`, `stale`, `weak-spec`, `unsupported`, `engine-missing`, `timeout`, `inconclusive`) do not sit in that order; they propagate upward as flags and open-item counts alongside it. | A proof in one corner must not hide a merely-tested boundary in another; and a timeout is not a weaker proof, it is a different kind of fact. |
| D7 | Every counterexample is stored as **Kani playback data** (exact, engine-version-bound) and, whenever the inputs can be rendered as stable Rust source, additionally as an ordinary `#[test]` that fails under plain `cargo test`. When rendering fails, the diagnostic says so (`W0541`) — inputs are never fabricated. | Playback is what Kani actually provides; the portable test is the agent-friendly repair target and is emitted whenever honestly possible. |
| D8 | `synth` mode (the model writes the function body) is orchestration over the check pipeline: prompt assembly, a retry loop, and marking the output as derived. It ships last and adds no checking machinery. | Thin by design. |
| D9 | Implementation language Rust; three crates (§4); engines run as subprocesses with version detection at startup, never linked as libraries. The Kani version is pinned in `ply.toml` and recorded in every fingerprint. | Engine version churn must not break our build or silently invalidate old evidence. |
| D10 | `node_id` is the component path + item path only. The blake3 `content_hash` is a sibling field, never part of the ID. All JSON output shares one envelope (§8) containing a tree of nodes plus a flat diagnostics list. | IDs must survive edits so external consumers (a future canvas UI) can track nodes across runs. Design for the canvas now; build it never (out of scope). |
| D11 | Extraction may be incomplete but never silently so: every call site the extractor cannot resolve is counted and reported (W0312 plus a per-component coverage metric in `check` output). | Visibility is what makes the advisory tier (D4) and any future `strict` opt-in meaningful. |
| D12 | A function declares a **checks list**, e.g. `checks: [bounded(3), fuzz(256), mutate]`. `mutate` requires a `test` or `fuzz` entry in the same list (else `E0504`) and uses only those as its mutant-kill signal, scoped per function with cargo-mutants' `--re`. | One base check could not express "bounded plus a fuzz-backed mutation tier". Running Kani once per mutant costs minutes per mutant per function; proof-backed mutation needs an opt-in budget, which is out of scope. |
| D13 | **Spike before build** (milestone M0): every engine-facing detail in this spec is provisional until a hands-on spike, with the pinned Kani version, records in ADR-0003 what actually works — attribute emission, in-crate harness modules, `stub_verified`, playback, input construction. The spec is then amended to match reality. | The engine surface is the highest-risk part of the design; paper decisions there are guesses. |
| D14 | `ply.lock` records, per claim, a **fingerprint**: item token-stream hash, merged contract text, callee contract hashes, engine name + version + flags, active features, target triple. A verdict is `stale` when any part changed. `cargo ply accept` re-blesses fingerprints and refuses (`E0303`) nodes whose last run failed. | An old success must not bless changed assumptions or a different toolchain; and without an accept verb, staleness warnings accumulate until they mean nothing. |

## 3. Toolchain

Rust stable, edition 2024. Cargo workspace of three crates (§4).

Internal dependencies: `syn` (features `full`, `visit`) + `proc-macro2`;
`serde`/`serde_json`; a maintained serde-YAML implementation (`serde_yaml_ng` at time of
writing — confirm at M1, record in ADR-0002); `jsonschema`; `clap`; `insta`; `walkdir`;
`toml`; `similar`; `blake3`.

External engines run as subprocesses and are detected at startup. Each is optional: a
missing engine downgrades the checks that need it, with warning `W0110` and status
`engine-missing`. It never fails the run.

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
golden-tested. `cargo ply skill` embeds it in the generated skill file.

Everything is declarative data. The only embedded syntaxes are:
1. **check strings** — `test | fuzz(N)? | bounded(K)? | prove | mutate`.
   Schema-validated by regex, parsed in ply-core.
2. **edge strings** — `A -> B` declares that component A may call component B (checked).
   `A ~> B : path::Type` declares a data flow (parsed and rendered, NOT checked in v1).
3. **deny strings** — `PAT -> PAT [except C1, C2]` where `PAT := IDENT | *`.
4. **Rust expressions** — contract and example strings parsed with `syn::Expr` (§5.4a).

### 5.1 Document structure

```yaml
ply: 1                           # schema version, required

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
        unresolved:              # optional registry links for markers in this fn
          - { id: 147, note: "employee discount undecided" }

edges:
  - pricing -> parser
  - "pricing ~> risk : pricing::Quote"

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
diagnostics. Duplicate component names across merged files → `E0202` (nested names are
qualified by their parent, so only siblings can collide). A string that passes the schema
regex but fails the real micro-syntax parser → `E0203`, stating the expected form.
`mutate` without a `test` or `fuzz` entry in the same checks list → `E0504`.

### 5.1a Strictness & lexical rules

The schema must encode all of the following; the goldens (§9) pin them.

1. **Unknown fields are errors.** Every object in the schema sets
   `additionalProperties: false`; an unrecognized key → `E0204` with a
   nearest-known-key suggestion. A typo must be caught, never ignored.
2. **Identifiers.** Component and profile names match `[a-z][a-z0-9_]*` (snake_case,
   ASCII). In edge and deny strings, tokens are separated by one or more spaces; the
   parser accepts any run of whitespace and the canonical form uses single spaces.
3. **Code paths.** Anchors and fn keys are plain segment paths: `IDENT(::IDENT)*`, where
   a segment may also be a type name in `Type::method` position. No generics, no
   trait-qualified paths (`<T as Trait>::f`), no lifetimes. An anchor or fn key outside
   this form → `E0304 unsupported path form`, naming the construct.
4. **Numeric bounds.** `fuzz(N)`: 1 ≤ N ≤ 1_000_000. `bounded(K)`: 1 ≤ K ≤ 64. Out of
   range → `E0203`.
5. **Unresolved ids** are positive integers, unique across the whole merged workspace
   (registry and fn entries together); a duplicate → `E0205`.

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
- `calls_dyn(from, trait)` — dynamic dispatch; reported `W0311` if the trait has
  implementations in a component the caller may not reach.
- `calls_unresolved(from, span)` — call sites the extractor cannot place (D11).
- `touches_cap(item, cap)` — capability approximation: uses of
  `std::net`/`std::fs`/`std::process`/`std::time`, rand crates, and `unsafe` blocks,
  plus a user-extensible `capmap.toml` mapping crate paths to caps.
- `mutates(item, TYPEPATH)` — mutation of a named type.

Item-tier rules (each `W`-severity by default, `A`-severity error under `strict`):
1. A call crosses two declared components with no `->` edge → `A0402`.
2. A `pure` component touches any capability → `A0403` (names the cap, spans the item).
3. A component reaches a capability outside its `uses` set through its own code, rather
   than through a declared `->` edge into a component that has the cap → `A0404`.
4. `owns T`: an item outside the owning component mutates `T` → `A0406`.
5. Profile bans (syntactic checks over the component's items — these are reliable and
   always errors) → `A0407`. The per-item escape
   `#[ply::allow(ban_name, reason = "...")]` suppresses the error and is recorded in the
   audit list.

**Resolution visibility (D11)**: `check` output reports, per component, the share of call
sites the extractor resolved. An unresolved call site whose textual candidates include an
item in a component that would need an undeclared edge → `W0312 possible undeclared edge
(call unresolved)`. Unresolved sites with no such candidate are counted but not itemized.
SCHEMA.md must state plainly that item-tier facts are approximate and how `strict`
changes severity.

Edges constrain **direct** calls only: `A -> B` and `B -> C` neither grants nor requires
`A -> C`.

### 5.4 Contract semantics

The canonical contract source is the inline `#[ply::requires]`/`#[ply::ensures]`
attributes on the function (D2). `requires`/`ensures` entries in `ply.yaml` are ANDed in,
for teams that prefer external specs. Because the attributes annotate the real function,
rustc type-checks contract expressions whenever the crate builds under `cfg(kani)`; under
plain cargo they are inert.

#### 5.4a Spec expression subset (accepted everywhere)

Boolean Rust expressions over the function's parameters and `result`; literals (integer,
bool, char, string); calls to `pure`-marked helper fns; `==,!=,<,<=,>,>=`; `&&,||,!`;
arithmetic; field access; `.len()`; `.is_ok()/.is_err()/.is_some()`; `matches!()`. The
list is closed: any construct outside it — indexing, other method calls, paths to
constants, closures other than the `|result| expr` ensures form, `old()` (pre-state
references, noted as planned) — is rejected with `E0501` naming the construct. No side
effects (checked syntactically). Identifiers must resolve to parameters or `result`,
sanity-checked against the anchored signature.

#### 5.4b Supported signatures

An engine can only check a function whose inputs it can construct. v1 supports functions
whose parameters and return type are, recursively: integers, `bool`, `char`, structs and
enums of supported types with Ply-derivable `Arbitrary` (public, invariant-free fields),
`Option`/`Result`/`Vec` of supported types, and `&T`/`&[T]` of the above (built from an
owned value in the harness). Everything else — trait objects, generics, smart pointers,
non-exhaustive or private-field types without a user-supplied generator — yields status
`unsupported` with diagnostic `V0505` naming the offending type. Unsupported is a
reported fact, never a harness build failure. A user-supplied generator hook (a
`pure`-marked constructor function named in `ply.yaml`) lifts a type into the supported
set; its design is validated in the M0 spike.

#### 5.4c Check → engine mapping

| check | engine | verdict on success |
|---|---|---|
| test | generated `#[test]`s from `examples`, plus generated direct contract cases (concrete inputs run through the real function, contract asserted) | `tested` |
| fuzz(n) | proptest harness, n cases (default 256), shrinking on; `requires` as a rejection filter, with a warning when the rejection rate is high | `fuzzed(n)` |
| bounded(k) | Kani contract proof (`proof_for_contract`), loop bound k (default 2) | `bounded(k)` |
| prove | Verus translation (M7, optional) | `proved` |
| mutate | cargo-mutants scoped `--re <fn>`; kill signal = the `test`/`fuzz` checks in the same list (D12) | appends `·spec-strong`, or flags `W0502 weak spec (N surviving mutants)` |

There is no transparent runtime enforcement: generated tests call functions explicitly.
An `induct` check (Kani loop contracts, proving loops by invariant instead of unrolling)
is planned, not in v1: Kani's loop-contract support is experimental, and Ply has no
stable-Rust invariant attribute yet. A function's verdict is the strongest evidence its
passing checks earned; a failing check is a `violation` regardless of what else passed.
Default checks: `[bounded(2)]` if the fn has any contract, else none.

### 5.5 Modular composition (D5)

Verification runs callees-before-callers over the call graph. To verify fn `f` that calls
contracted fn `g`:

- `g` passed its own Kani contract proof this run, and `g` is in the same crate →
  generate `f`'s harness with `#[kani::stub_verified(g)]`. Clean verdict.
- Anything else — `g` merely fuzzed or tested, `g` in another crate, `f` and `g` in a
  cycle → verify `f` assuming `g`'s contract, and mark `f`'s verdict `conditional`
  (`W0511`), listing each assumed contract.

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
cargo ply verify [path|fn]   # run checks via engines, callees first; write cex artifacts
cargo ply tree               # verdict tree, worst-of aggregation, assumption chains
cargo ply worklist           # unresolved markers + weak specs (W0502) + stale claims (W0302)
cargo ply audit              # trust surface: profile escapes, assumed contracts, derived fns
cargo ply accept [id|--all]  # re-record fingerprints in ply.lock (§5.2)
cargo ply synth <fn>         # M6
cargo ply skill              # (re)generate docs/PLY.skill.md from schema + diag registry
```

Global flags: `--json` (schema §8, the agent surface — every command supports it),
`--engine-timeout=<s>` (default 60 per fn), `--only-changed` (scope to the git diff),
`--fail-on=warn|error`.

Exit codes: 0 clean, 1 violations or failures, 2 tool error, 3 missing engine for an
explicitly requested check.

Housekeeping: Ply owns everything under `target/ply/` and every generated test whose name
starts with `ply_cex_`; `verify` deletes generated artifacts whose claims no longer
exist.

## 7. Recursion & aggregation (the zoom model)

The model is a tree: workspace → components → nested components → fns. Every node
carries `{ id, kind, anchor, content_hash, verdict, statuses, worst_descendant,
open_items }`. `verdict` is the node's own claim status; `worst_descendant` implements
D6 over the evidence order; `statuses` and `open_items` (unresolved markers, weak specs,
conditional or stale verdicts) propagate upward as counts.

`tree --json` emits this tree (D10). Human-facing `tree` renders it as an indented
tree, one line per node, worst first; `--depth N` limits depth and `--focus <id>`
descends into a subtree. This is the human review surface: at depth 1 you see which
subsystem is weakest, then zoom. A future canvas UI renders the same JSON; nothing in
this repo should special-case it beyond keeping the schema stable and treelike.

## 8. Result JSON

Every command emits one envelope:
`{ "command": "...", "ply_version": "...", "root": <node tree §7>, "diagnostics": [<Diagnostic>...] }`.
Stability rule: additive changes only after M3; the goldens in tests/ui are the contract.

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
    "kani_playback": "target/ply/playback/pricing_quote_01.json",
    "cargo_test": "tests/ply_cex_pricing_quote_01.rs",
    "trace": [ {"span": "…", "detail": "bid = tick * 2 = -2"} ]
  },
  "assumptions": [{"kind": "assumed_contract", "fn": "parser::parse", "verdict": "fuzzed(256)"}],
  "fixes": [{"title": "add requires inst.tick > 0", "edits": [{"span": "…", "insert": "#[ply::requires(inst.tick > 0)]"}]}],
  "open_item": null
}
```

`kani_playback` is present whenever Kani produced playback data; `cargo_test` only when
the inputs rendered as stable Rust source (D7), else `W0541`.

Diagnostic codes live in one exhaustive enum: `E02xx` config/schema, `E03xx/W03xx`
anchoring/staleness/resolution, `A04xx/W03xx` architecture, `E05xx/V05xx/W05xx` contracts and
verification, prefixes `K/P/M/R` reserved for engine-specific codes, `W01xx` environment,
`X09xx` internal errors. Adapters never pass engine stderr/stdout through raw: they parse
it, or fail with `X0901` attaching the raw output for debugging.

## 9. Testing strategy

- **Fixture e2e**: each feature ships a minimal fixture project + `ply.yaml` + expected
  `--json` output as insta goldens. Review goldens like API diffs.
- **Cex validity oracle**: every rendered `cargo_test` must FAIL under `cargo test` in
  the fixture before the fix and PASS after; every `kani_playback` artifact must
  reproduce under `cargo kani playback` with the pinned version. The e2e harness asserts
  both. A counterexample that does not reproduce is `X0902` (an adapter bug). This is
  the primary correctness check of the whole tool.
- **Schema goldens**: `schema/ply.schema.json` is golden-tested; a fixture set of valid
  and invalid `ply.yaml` documents pins validation behavior and E0201 pointer paths.
- **Extraction differential**: property-test call-graph extraction against a naive
  AST-walk reference on generated fixture modules; any disagreement is a bug. Assert
  resolution coverage (D11) on fixtures with known-unresolvable sites.
- **Engine-absence matrix**: run every e2e once per engine with that engine masked out,
  asserting graceful downgrade (`W0110` / `engine-missing`), not failure.
- **Self-hosting**: golden tests for `skill` output, and `cargo ply check` runs clean on
  this repo (this workspace gets its own `ply.yaml` from M2 onward, kept green in CI).
  Once M5 lands, `cargo ply verify` runs over this workspace too: contracts on our own
  core functions (config merge, anchor resolution, verdict aggregation), verified by our
  own pipeline, in CI.

## 10. Milestones (accretive; every prior acceptance test stays green)

**M0 — feasibility spike** (~2 sessions; nothing else starts until ADR-0003 exists)
- One throwaway fixture crate; Kani version pinned. Exercise: a contracted private free
  function; a contracted method; a user-defined input type; a same-crate stubbed callee;
  a cross-crate callee (expect failure — document it); a real contract violation with
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
  warnings by default, `strict` upgrade; W0311/W0312; profile bans with allow-escapes;
  `audit` (escapes only at this stage). ADR-0001 written.
- Self-host: this workspace gets its own `ply.yaml`, checked in CI.
- Accept: a fixture with a crate-tier violation (error), an item-tier cross-boundary
  call (warning, then error under `strict`), a pure violation, an owns violation, and a
  profile escape — goldens for each; a clean fixture producing zero diagnostics; a
  fixture with a deliberately unresolvable call asserting the coverage metric and W0312.

**M3 — Kani adapter + counterexample pipeline** (~8–10 sessions; shaped by ADR-0003)
- ply-attrs emitting `cfg_attr(kani, ...)`; harvest of pre-existing cfg_attr-kani
  attributes; `ply.yaml` contract merge; spec-subset validation (E0501); supported-
  signature gate (V0505 / `unsupported`); in-crate proof-module generation for
  `bounded`; run and parse Kani; playback artifacts + rendered tests where possible;
  Diagnostic assembly with suggested fixes for the mechanical cases (a missing
  `requires` derived from a failed side condition).
- Callees-first order; `stub_verified` under D5's conditions; `conditional` verdicts
  with assumption chains.
- Accept: a fixture where a cex is found, its playback reproduces, and its rendered test
  fails then passes, JSON golden; a two-fn fixture proving modular verification (caller
  stubs proved callee; assumption chain empty); a caller-of-fuzzed-callee fixture
  earning `conditional`; an unsupported-signature fixture earning `unsupported`, not a
  build failure.

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

**M5 — verdict tree, aggregation, skill, polish** (~3 sessions)
- Verdict tree with worst-of, status propagation, and open items; `--depth`/`--focus`;
  `audit` completed (escapes + assumptions + derived); `skill` generation embedding the
  schema; `--only-changed`.
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

Out of scope entirely: canvas/GUI, data-flow (`~>`) checking, LSP/editor integration,
`old()`-style two-state contracts, the `induct` check, cross-crate `stub_verified`,
proof-backed mutation, async fns in verified components, and the internals of crates
outside the workspace.

## 11. Session protocol

Start each session from this spec (by § reference) plus the current milestone's
acceptance list. End with `cargo test` green, goldens reviewed, and `cargo ply check`
green on this repo (M2 onward). Never advance a milestone while the cex validity oracle
(§9) is red. Record any decision this spec does not cover as a one-paragraph ADR in
`docs/adr/`; amend the spec rather than contradicting it.
