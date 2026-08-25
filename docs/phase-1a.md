# Phase 1a — the validation layer becomes the product

Run 2026-08-25, branch `claude/project-concept-eval-6soxfl`. Three slices, three commits:
the promotion, the schema, and `cargo ply check`. No engine work, no `ply.lock`, no
architecture tier.

Two things were wrong at the start of this phase, and both were the same kind of wrong:
a claim that had never been true.

1. **The real `ply.yaml` semantics lived in `tools/`, and the product had a copy.**
   `crates/ply-core/src/config.rs` carried a hand-rolled four-struct subset of the format
   with a `TODO(M1)` on it saying so. `tools/model` and `tools/check` carried the whole
   grammar — checks inheritance, `E0202`, `E0203`, `E0206`, `E0209`, `E0304`, `W0409`,
   `W0410` — with the tests. Two readers of one document is precisely the defect §5.1a
   rule 1 was amended to name after vetting 004 finding 7, where a team's external
   `ensures:` reached no engine and raised no warning.
2. **`schema/ply.schema.json` did not exist**, while §5 called it "the NORMATIVE
   definition of ply.yaml" and §5.1a said "the schema must encode all of the following".

---

## 1. What moved, and why that direction

`tools/model/src/lib.rs` → `crates/ply-core/src/model.rs`.
`tools/check/src/lib.rs` → `crates/ply-core/src/check.rs`.
`crates/ply-core/src/config.rs`'s duplicate model → deleted.

`tools/model` is gone as a package. `tools/check` is now binary-only: the CLI contract it
tests (exit codes, "never emits SVG") lives in `main.rs`, and the rules it prints come
from `ply_core::check`. `tools/render` depends on `ply-core` instead of `ply-model` +
`ply-check`, and keeps its `pub use ply_core::model` re-export so every `ply_render::model::…`
call site is unchanged.

Both `tools/` crates reach into `crates/ply-core` by path dependency. That is the honest
direction: the product owns the `ply.yaml` model, and the spec-validation tooling
consumes it. The reverse — the product depending on `tools/` — would have made the
shipping binary depend on a workspace §4 describes as predating it.

### Held fixed on purpose

Three things could have drifted silently under a move. Each was pinned rather than
noticed later:

- **Iteration order.** The promoted model is `IndexMap`, not `BTreeMap`: the renderer
  lays components out in the order the author declared them, so a document's reading
  order and its picture agree. `verify` read a `BTreeMap` and therefore emitted nodes and
  diagnostics in name order, which the e2e goldens pin. `verify` now sorts explicitly, at
  the two loops that used to get it for free, with the reason at the use site.
- **`E0504`'s wording.** Both copies of D12's rule existed with different sentences, and
  `verify` recovered the code by splitting its own message on the first colon — wording
  and code coupled by accident. There is now one predicate (`check::mutate_lacks_kill_signal`)
  and one sentence (`check::mutate_kill_signal_message`), and `verify` names the code
  itself. The surviving sentence is `tools/check`'s, because that is the one three
  exact-string tests already reviewed.
- **`E0203`'s wording.** Same story, smaller: the product's terser `E0203: …` gave way to
  the plain-language sentence with the code appended, per CLAUDE.md's newbie bar ("a code
  may follow a plain sentence, never replace one").

### Is it behaviour-preserving?

Yes, with one deliberate exception and one measured caveat.

| | before | after |
|---|---|---|
| `tools` workspace tests | 145 | 118 |
| product unit tests (engine-free) | 97 | 142 |
| **total** | **242** | **260** |

The 27-test drop in `tools` is `tools/model`'s inline test module moving with its file
into `ply-core` — the same tests, the same names, run by the other workspace. Nothing was
deleted. The product's growth is those 27 plus the schema tier (9), `check` (8), and one
new `E0203` wording test.

The **full suite ran once at the end**, engines and all: 169 passed, 0 failed
(142 product unit + 27 e2e, of which 4 are `check`'s new ones). Every Kani-driven e2e —
`boundarycontract` at 99s, `vecbound`, `timeout`, `arraycard` — passed unchanged.

The committed vetting SVGs are **byte-identical**: `git diff --exit-code -- vetting`
after re-rendering both documents, which is the check CI runs. `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` are clean in both workspaces.

The deliberate exception: `verify`'s `E0504` diagnostic `title` now carries the promoted
wording instead of its own. The e2e assertion on it is a substring check (`mutate`,
`test`, `fuzz`) and still holds; the change is a unification, and the wording that
survived is the tested one.

---

## 2. What the schema pins

`schema/ply.schema.json`, JSON Schema 2020-12, is the normative definition §5 has always
claimed it was. The interesting question was not *writing* it — it is a mechanical
rendering of §5.1 — but making it **load-bearing**, since a schema nothing reads is prose
with punctuation.

It is load-bearing in three ways:

- **The `E0204` key vocabulary is read out of it at runtime.** `ply_core::schema::known_keys`
  walks the schema's `properties` maps; the six Rust constants that used to hold that
  vocabulary (`DOC_KEYS`, `COMPONENT_KEYS`, …) are gone, along with the pointer-equality
  hack that turned one of them back into a human-readable level name. Delete a key from
  the schema and Ply stops accepting it.
- **Required fields come from its `required` arrays**, and the "this is missing"
  sentence quotes the schema's own `description` of the key — so `anchor:`'s purpose is
  written once.
- **Two constraints are declared as regexes and enforced by hand-written matchers**, so
  the shipping binary carries no regex engine. Each is held to the schema by an invariant
  test rather than by review.

### What it rejects that the product previously accepted

Everything below loaded silently before and is now `E0201` at load time — which means on
the `verify` path too, not only in `check`:

| | example that used to load | why |
|---|---|---|
| non-snake_case names | `components: { Pricing: … }`, `db-raw` | §5.1a rule 2, stated in the spec and checked by nothing |
| unknown capability | `uses: [network]` | §5.1's `uses:` vocabulary; a typo enforced nothing |
| unknown ban | `hot_path: [no_allocation]` | §5.1's ban list, same |
| unresolved id 0 | `unresolved: [{ id: 0, … }]` | §5.1a rule 5's "positive integers" |
| missing required field | a component with no `anchor:` | previously a serde message with no pointer |
| non-canonical check counts | `fuzz(0256)`, `fuzz(+5)`, `fuzz( 256 )` | see below |

That last row is the one the tests found rather than the one I set out to fix. The
invariant test comparing the schema's check-string regex against `parse_check` over a
corpus failed on `fuzz(0256)`: Rust's `u32::from_str` accepts leading zeros and a leading
`+`, and the parser inherited that. The schema is normative, so **the parser narrowed to
the schema**, not the other way round. The old `invalid fuzz(N) count in "fuzz(abc)"`
message — jargon, and untested — became a sentence that says what the brackets are for.

Checked before committing: every one of the 49 `ply.yaml` documents already in the repo
still passes, except `tools/render/tests/fixtures/unparseable.ply.yaml`, which is
deliberately unparseable. The e2e suite re-checks them all, since every fixture run loads
its own document through `config::load`.

### The goldens (§9)

`crates/ply-core/tests/fixtures/schema/` — 16 invalid documents, each beside a
`.expected` golden holding the exact diagnostics it must produce, and 3 valid ones that
must produce none *and* load. Every golden was read before it was written; the article
bug in "a component needs a `anchor:` line" was caught that way and fixed to "needs its
`anchor:` line" before anything was committed.

Four tests hold the schema to the code rather than to itself:

- every schema object with a fixed key vocabulary sets `additionalProperties: false` —
  stated as a walk over the whole document, so an object added later cannot skip the rule;
- every key the schema declares is a key the serde model actually reads;
- the check-string regex and `parse_check` accept the same language;
- the code-path regex and `is_valid_path_form` accept the same paths.

### What §5 asks for and this does not do

**The source line.** §5 says an `E0201` carries "the JSON-pointer path and source line",
and describes a position-marked YAML pass building a pointer → (line, col) index. The
pointer is exact and shipped. The index does not exist, so no diagnostic carries a line
number. A guessed line is worse than none — it sends a reader to the wrong place with
full confidence. §5 now says this instead of implying otherwise.

**Edge and deny strings carry no pattern.** Their language is not regular enough to state
twice without one copy lying, so the parser is the single definition and `E0203` is the
diagnostic. The schema says that in its own `description`, so a reader of the schema is
not left wondering.

---

## 3. What `cargo ply check` covers, and what it does not

§6: "schema + anchors + staleness + architecture. Fast, no engines." Two of the four.

**Covered — schema.** The document against `schema/ply.schema.json` (`E0201`, `E0204`,
each carrying its JSON pointer), then every document-local rule from `ply_core::check`:
`E0202` (a name declared both as a component and as an external — a duplicate *component*
name is not reachable yet, since YAML refuses a repeated key in one mapping and multi-file
merge does not exist), `E0203` micro-syntax, `E0205` duplicate unresolved ids, `E0206`
ambiguous references, `E0207`/`E0208` external edge rules, `E0209` unknown external in
`entry:`, `E0304` path forms, `E0504`, `W0409` redundant containment edges, `W0410`
unreferenced externals. A document that fails the schema stops there: the model would
refuse it anyway, and a pile of consequential errors on top of the real one helps nobody.

**Covered — anchors.** Every fn claim, resolved through the same `harness::discover_fn`
`verify` uses, so the two commands cannot disagree about which claims point at real code.
`E0301` names the nearest name in the item index (`harness::top_level_fn_names`, the same
set `discover_fn` searches — a suggestion naming a function `discover_fn` would then fail
to find would be worse than none). Where the resolution fails, the `use`-following
`callgraph::Resolver` is asked a second question — does this path resolve *anywhere* in
the crate? — purely so the diagnostic can distinguish "there is no such function" from
"it is inside a module this slice cannot verify from". Those are different problems with
different fixes, and only the first is a typo.

Components anchored to another crate are counted separately and produce no diagnostic:
`verify` is single-crate, so their anchors genuinely cannot be resolved from here, and
calling that an error would be wrong.

**NOT covered — staleness.** Needs `ply.lock` (D14), which nothing writes. Phase 1c.
**NOT covered — architecture.** M2.

Both gaps are in the `--json` envelope as `coverage.not_checked` and printed under "What
this command did NOT check", each saying what the user is therefore *not* being told —
"a claim whose function has changed since it was verified is not reported here"; "a call
that violates a `deny` rule will not be reported here". This is the part of the command
that needed designing rather than coding: a command that reports only findings lets a
clean run read as full coverage, which is the same failure as an absence of evidence
reported as a pass (§1).

`check` produces no verdicts and refuses to look as though it did. Every node in its
envelope reads `unclaimed`, and the last line of every human run says why: *"`check` runs
no engines, so it produces no verdicts: every claim in this run's `--json` envelope reads
`unclaimed` because this command gathered no evidence about it, not because the code is
unverified. `cargo ply verify` is what produces verdicts."*

Exit codes: 0 clean or advisory-only, 1 any error-severity finding, 2 tool error (a
missing or unparseable `ply.yaml` — there is no document to have findings about).
`--fail-on` is **not** wired to `check`: its `evidence` default is meaningless for a
command that gathers none, and every node being `unclaimed` would make it fail every run.
`--only-changed` is not wired either.

The four end-to-end cases run in **0.66s** with no engine installed, which is the
property §6 asserts when it calls the command fast.

---

## 4. The red-first failures, verbatim

Honest accounting: the schema slice was red-first throughout. The `check` command was
not — I wrote its shape first and its tests immediately after, and six of the eight
failed on the first run for a reason that had nothing to do with the code.

**Promotion.** A refactor has no new behaviour to go red on; the existing 242 tests are
the evidence, and they are what the slice was steered by.

**Schema module, before it existed:**

```
error[E0432]: unresolved imports `ply_core::schema`, `ply_core::schema`
 --> crates/ply-core/tests/schema.rs:8:15
```

**The one that changed a decision** — the regex/parser agreement invariant, on its first
run against the new schema:

```
"fuzz(0256)": schema regex says false, parse_check says true — the schema is the
normative definition, so the parser must agree with it exactly
  left: false
 right: true
```

That message named the actual defect, which is the bar CLAUDE.md sets: not "a test
failed" but "these two descriptions of one language disagree, here, about this string".

**Goldens, before they were written:**

```
no golden at .../invalid/component_name_not_snake_case.ply.expected — write it from a
reviewed run, never from a blind accept
```

**`check`'s tests, first run** — a fixture defect, not a code defect, and the message
said so exactly:

```
called `Result::unwrap()` on an `Err` value: /tmp/.tmpXmORmh/ply.yaml is not valid YAML,
so Ply could not read it as a ply.yaml at all: did not find expected key at line 7
column 9, while parsing a block mapping at line 4 column 5
```

The test YAML had been mangled by shell line-continuation when the file was written. The
failure pointed at line 7 column 9 of the document, which is where the damage was. Six
tests, one cause, no code change.

---

## 5. NOT RUN / NOT DONE

- **The full e2e suite** was run once at the end of the phase, not per slice — another
  agent was running Kani-heavy timing measurements on the same machine. During
  development the engine-free subset (`--exclude ply-e2e`) plus the three engine-free
  e2e tests (`mutatetier`, `unknown_key`, `check_command`) carried the signal. The final
  run was clean: 169 passed, 0 failed.
- **Multi-file discovery and merge** (§5's "files named `ply.yaml` or `*.ply.yaml` …
  merge into one model") is still not implemented. `config::load` reads exactly the path
  it is given, as before. `E0202`'s "unique across ALL merged files" is therefore still
  only enforceable within one document.
- **The JSON-pointer → (line, col) index** (§5). See above.
- **`cargo ply skill` embedding the schema** (§5's own sentence) — the command does not
  exist.
- **`check` on a bare `*.ply.yaml`** — the command takes a crate directory, like
  `verify`. `tools/check`'s binary is still what reads a loose document (the vetting
  scenarios have no crate behind them).
- **Self-hosting** (§9: "`cargo ply check` runs clean on this repo") — this workspace has
  no `ply.yaml` of its own yet; §9 dates that from M2.

---

## 6. TODO.md deltas — for the owner to apply

I did not edit `TODO.md` (another agent holds it). These are the deltas:

**Tick as landed:**
- Reconcile `tools/model` + `tools/check` with `crates/ply-core/src/config.rs` — "promote
  one, delete the other" (the `TODO(M1)` in `config.rs`) — done, `ceb52aa`.
- `schema/ply.schema.json` exists, is embedded, is load-bearing for `E0204`, and is
  golden-tested per §9 — done, `c8528ce`.
- `cargo ply check`: schema + anchor tiers, `--json`, exit codes — done, `5212cfa`.

**Add as new, open:**
- `check`'s staleness tier — blocked on `ply.lock` (Phase 1c). Its absence is currently
  reported in `coverage.not_checked`; that text comes out when the tier lands.
- `check`'s architecture tier — M2, same.
- JSON-pointer → (line, col) index for `E0201`/`E0204` (§5). Pointer shipped, line not.
- Multi-file `ply.yaml` discovery and merge (§5), and with it `E0202` across files.
- Wire `--fail-on` / `--only-changed` to `check` once there is a tier they can mean
  something for.
- `check` should accept a loose `*.ply.yaml` path, so `tools/check`'s binary can retire.
- `.archi/ply.json`'s "Tooling Today" diagram. Its two leftmost boxes were repointed at
  their new homes in this phase, but it still shows `cargo ply` as "not built yet" —
  stale since M3, not since this phase. Redrawing it against the product is its own job.

**KNOWN GAP, left open on purpose:**
- `discover_fn` sees only top-level functions in `src/lib.rs`. `check` inherits that
  limit deliberately, so it never passes an anchor `verify` would fail. The diagnostic
  now says which of the two failures a user has hit, which makes the limit visible
  instead of confusing — but the limit is still there.
