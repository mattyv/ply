# Module boundaries and acceptance evidence

Baseline: `fc10db9` (2026-09-09).

This plan delivers two independent changes as separate review increments. Acceptance
evidence lands first because it can reproduce the observed zero-record failure without
waiting for module resolution. Module-boundary enforcement follows on the same mainline.

## Increment 1: named acceptance evidence

### Question answered

Acceptance evidence answers whether a named production path met a user requirement for
specific committed inputs. It does not strengthen a function proof, and a proof cannot
erase an acceptance failure. A required acceptance failure withholds overall success.

### Declaration

Add a top-level `acceptance:` mapping keyed by a stable snake-case claim id:

```yaml
acceptance:
  decimal_response_maps:
    requirement: "A venue response with valid decimal prices maps to the expected records"
    component: mapping
    entry: rmpoc_mapping::map_response
    test:
      package: rmpoc-mapping
      target: venue_response
      name: decimal_strings_map_to_expected_records
    inputs: [tests/fixtures/venue-response.json]
    expected: [tests/fixtures/venue-response.expected.json]
    required: true
```

The first version runs Rust integration tests only. It accepts Cargo package, integration
test target, and exact test name as separate fields; arbitrary shell text is not accepted.
`inputs` and `expected` are regular files relative to the named Cargo package. Published
results normalize them to portable verification-root-relative paths, so the same child document
keeps working both alone and when composed through a parent workspace. `component` must
resolve to one declared component. `entry` records the production entry point exercised;
the test remains responsible for calling it.

`required` is independent of architecture profiles in this first version. It applies whenever
the claim is admitted to this invocation; it does not import claims belonging to components
outside a selected linked subtree. Ply has architecture ban profiles but no selected
verification-profile CLI concept, so this field must not overload `profiles:`. A later profile
selector may refine when a claim is required without changing the result shape.

### Execution and attribution

Ply first asks Cargo to build the named integration-test target with bounded build time and
reads Cargo's JSON artifact messages. It accepts exactly one artifact matching both Cargo
package identity and integration-test target identity. A missing or ambiguous artifact, or a
target with `harness = false`, is a tool/configuration error rather than acceptance evidence.
Ply then asks Cargo, from the package directory, to launch that package and target with an exact
libtest name filter and a bounded run time. This preserves member-specific Cargo configuration,
build-script library paths, package environment, and toolchain selection instead of recreating
Cargo's runtime contract. A successful process counts only when the final libtest summary reports
one executed, non-ignored test; the exact filter makes that one test the declared name.
Zero matches and ignored tests are “not run,” never passes. Tests cover relative fixture paths
from a workspace member, custom harnesses, missing exact names, ignored tests, and timeouts.
Nested or otherwise ambiguous libtest output is a tool error rather than inferred evidence.

Each claim produces a top-level, additive acceptance result with its id, requirement,
component, entry, test identity, input paths, required flag, outcome, and observation
detail. Outcomes are `passed`, `failed`, `timeout`, `tool_error`, and `not_run`. Contract
nodes and their evidence stay unchanged. Required outcomes other than `passed` make
`verify` unsuccessful. Optional failures remain visible but do not redefine function
evidence.

Acceptance has its own overall gate. A document that requests no contract claims but whose
required acceptance claims pass succeeds without changing the empty contract tree's verdict.
An empty contract tree fails for missing evidence only when contract evidence was requested.
A required acceptance non-pass fails under every `--fail-on` setting; an optional non-pass is
reported but does not change the exit status. Tests pin acceptance-only success, combined
proof success plus acceptance failure, optional failure, and every required outcome.

Linked verification selects only acceptance claims whose declared component is the linked
target or one of its declared descendants. Result identity is qualified by source document and
claim id; selected component ids and verification-root-relative display paths are rebased into the root
invocation. All admitted results are grafted before the acceptance gate is evaluated. Claims
belonging to unselected sibling components are not run and are disclosed as outside the composed
invocation, so a required child failure can neither disappear nor be attributed to the root.

The first version always executes acceptance tests freshly. No result is written to
`ply.lock`. Any future reuse must cover the production source, test and oracle source,
fixture and expected files, dependency graph, compiler, and active configuration.

### Fixture and provenance

The repository fixture contains a small production pipeline: raw response bytes enter the
real deserializer, pass through record mapping, and return domain records. Expected output
is committed separately and derived from the fixture's stated protocol rules, never from
the implementation under test. A provenance sidecar states whether the response is captured
or synthetic, the source operation and schema version where known, the date, and every
redaction or transformation. Tests distinguish an empty valid response, malformed input,
an upstream error envelope, and partial record rejection.

### Visual form

Declaration-only drawings list acceptance claims as declared and not run. Evidence-coloured
drawings show their independent outcome. They do not colour function chips or raise the
component's proof rung. Existing consumers see additive fields and retain their old view.

## Increment 2: module-boundary enforcement

### Active scope

The first version analyzes the selected non-test Rust library package under the feature set
Cargo reports as enabled for the invocation and the host target triple. Source-tier coverage
records package id, target, enabled features, and each conditional scope as active, inactive,
or unknown. Source under a simple inactive `cfg(test)` or `cfg(feature = "...")` is excluded
only when that condition is known false. Unknown attributes, target predicates, `cfg_attr`, and
configuration-dependent module paths remain unknown and prevent their dependent references
from becoming definite violations.

Crate-tier coverage remains a separate Cargo-metadata statement. It records the metadata
invocation and target filtering actually used; unfiltered platform dependency information is
never labelled as the same host configuration as source analysis.

The analyzer does not execute project code. Build scripts, macro expansion, `include!`,
dynamic dispatch, trait dispatch, indirect calls, and unsupported `cfg` expressions create
named coverage gaps. A clean result with gaps says “no forbidden reference found in the
analyzed subset.”

### Ownership

Resolve each anchor into `(crate identity, module path)`. A module component owns items
declared in that module subtree. The most specific explicit descendant anchor owns its own
subtree; its ancestor owns the remainder. A crate-root component therefore owns residual
modules around narrower descendants.

Two unrelated components may not own the same module or overlapping module subtrees.
Declared component containment may overlap only when its Rust anchors have the same
ancestor relationship. Imports and re-exports never transfer ownership. A method belongs
to the module containing its `impl`; its reference to the implemented type is a separate
type-reference edge. Package identity, Rust privacy, and component ownership remain
separate facts.

Missing or ambiguous anchors are errors. Unassigned first-party modules are counted and
named; Ply never invents a root component to consume them. Existing ambiguous crate
identity and duplicate crate-root diagnostics remain.

Crate-tier ownership is a deliberate projection, not the module index with its path removed.
Only an exact crate-root anchor owns that crate's Cargo dependency edges. Documents containing
only crate-root anchors therefore retain their existing findings and exit behaviour. A module
anchor never claims its entire crate at the Cargo tier, and multiple disjoint module anchors do
not become duplicate crate owners. Once a crate is split into module components, Cargo metadata
alone cannot say which module uses a package dependency: without an exact root anchor those
dependencies are reported as unassigned package-level facts; with a root anchor they are still
reported as package-level facts rather than attributed to the root component's residual modules.
Source references provide the component attribution. No component is chosen by declaration
order.

### Observed references

The source index records:

- origin item and declaration module;
- resolved destination item and declaration module;
- source span;
- kind: call, function value, type, import, or re-export;
- active configuration identity; and
- resolution outcome: resolved, ambiguous, or unresolved with a reason.

The supported slice covers ordinary inline and file modules; nested `crate`, `self`, and
`super` paths; named and renamed imports; re-export chains; lexical local shadowing; direct
free-function calls; function items used as values; explicit imports; and type paths in
function signatures, struct fields, and type aliases. `#[path]` modules are either read at
their declared path or reported as outside the analyzed scope; Ply never silently tries the
conventional filename instead.

The existing call resolver is reusable for declaration lookup and import expansion, but it
does not retain origin-module scope, type references, or local bindings. The feasibility
gate is a fixture test across all supported reference kinds. If the shared syntax index
cannot resolve that fixture without guessing, stop the expansion and document a deliberate
compiler-backed route and its toolchain cost.

Reference-bearing syntax outside the supported slice is not silently ignored. Enum payloads,
local type annotations, constants and statics, initializers, patterns, casts, associated items,
macros, trait/generic dispatch, and every other visited form either yields resolved references
or a named coverage gap for its affected module. The feasibility fixture includes negative and
mixed cases—including shadowing through a re-export—so “unsupported” is exercised as an
observable outcome rather than a catch-all promise.

### Policy and severity

Package dependencies continue through the exact Cargo-metadata tier. Module references
use the existing component permission, containment, wildcard, and deny rules. An explicit
deny always wins even when an edge or containment would otherwise permit the crossing.

A resolved forbidden reference is advisory by default and an error when the source
component declares `strict: true`, matching the existing item-tier contract. An unresolved
reference or incomplete source scope is a coverage gap, not a guessed violation. Policy
endpoints that resolve to nothing remain configuration errors.

Coverage reports package dependencies, resolved module references, unresolved references,
unassigned modules, and configuration identity as separate facts. It uses counts, not a
percentage with an invented denominator.

### Acceptance fixture for the analyzer

One single-crate application contains `parsing`, `domain`, and `execution` modules. Its
tests pin direct and renamed calls, re-exports, type references, function values, shadowing,
inline/file equivalence, nested ownership, duplicate anchors, deny precedence, visible
unsupported constructs, selected feature paths, unowned modules, and unchanged multi-crate
behavior. At least one end-to-end command test must report the exact same-crate forbidden
call site.

## Delivery gates

Each increment updates the schema, prose specification, explanations, JSON envelope, visual
consumer, and tests together. Rendered changes are rasterized at their own dimensions and
inspected. Astra reviews the design, each implementation increment, and the final branch.
The real trial application is changed only in an isolated copy and only after the fixture
demonstrates the intended boundary.
