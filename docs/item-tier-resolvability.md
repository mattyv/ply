# Item-tier resolvability: what fraction of call sites can a source-only reader name?

**A source-only reader can name the callee at roughly one in five call sites — 20.6% of production
code in `crates/` (1,158 of 5,613), 15.2% in `tools/` (453 of 2,988) — and everything else needs a
type checker Ply does not have.** Including test code nudges both numbers up a couple of points
(22.6% and 17.5% respectively) but does not change the picture: an item-level tier built on syntax
alone would report on a minority of the program in both codebases measured, and the minority is the
same shape in each place — plain function calls — while the majority is method calls, which no
syntactic walk can follow.

## How this was measured

A throwaway `syn`-based classifier (not part of the Ply workspace; lived in a scratch directory and
was deleted after this report) parsed every `.rs` file under `crates/` and under `tools/`
separately, walked every function body (free functions, inherent/trait impl methods, trait default
methods, and `const`/`static` initializers), and classified every call-shaped expression into one
of the buckets below. It reuses the same distinctions `crates/ply-core/src/callgraph.rs` and
`crates/ply-core/src/reach.rs` already draw in production:

- `callgraph.rs`'s `CallCollector` only ever collects free-function calls (`ExprCall` with a
  path callee) and explicitly says why it ignores method calls: "receiver-dispatched,
  overwhelmingly `std`, and flagging them would... tell a user nothing they could act on." This
  tool keeps that split (bucket 1 vs. bucket 2 below) instead of inventing a different one.
- `reach.rs`'s whole reason for existing is that a syntactic walk cannot follow a method call, an
  operator that might dispatch to a first-party `impl`, or a macro — and that **any** `impl` block
  anywhere in first-party source is enough to make Ply give up and hash the whole crate rather than
  guess. This tool's operator bucket makes the same call: it does not try to distinguish `a + b` on
  two `u32`s from `a + b` on a first-party type with `impl Add`, because Ply's own reach-walker
  doesn't either — it just refuses the moment an `impl` exists anywhere. The count below is the
  total, with that caveat stated rather than hidden.
- `reach.rs`'s `mentioned_paths` is why bucket 5 (function-as-value) exists at all: "`helper(x)`
  and `map(helper)` both put `helper`'s body in the run, and only the first is a call expression."

Deliberate differences from those two modules, stated rather than left implicit: neither
`callgraph.rs` nor `reach.rs` needed to decide whether `T::assoc()` is resolvable when `T` is a
generic type parameter (their own resolver just looks up whatever name is written and lets
`Opaque`/`NotFound` fall out); this tool adds that distinction explicitly (bucket 1c) because it
changes the answer to "can a reader name the callee." Similarly, neither module needed to decide
whether `foo(x)` calls a named item or a local closure held in a variable named `foo`, because their
purpose (deciding what to *hash*) tolerates that ambiguity — the reach.rs module comment says so
outright: "A path that names nothing is ordinary further down the walk: most mentioned paths are
local variables." For this measurement, that ambiguity is exactly the question, so bucket 1b makes
it explicit instead of silently folding it into "resolvable."

## The full breakdown

Two numbers per cell throughout: with test code (`#[cfg(test)]` modules and `#[test]`/`#[…::test]`
functions) excluded, then included. Percentages are of that column's own total.

### `crates/` (ply-core, ply-cli, ply-attrs)

| Bucket | Excl. tests | Incl. tests |
|---|---|---|
| 1a/1c. Plain path call (`foo(..)`, `mod::foo(..)`, `Type::assoc(..)`, `Self::assoc(..)`) — **resolvable** | 1,158 (20.6%) | 1,749 (22.6%) |
| 1b. Call through a local variable/parameter of the same name, not a named item — **not resolvable** | 52 (0.9%) | 64 (0.8%) |
| 2. Method call (`receiver.method(..)`) — **not resolvable** | 3,042 (54.2%) | 3,817 (49.3%) |
| 3. Operator that could dispatch to a first-party `impl` (arithmetic/bitwise/shift/comparison, unary `-`/`!`/`*`, indexing `a[b]`) — **not resolvable**, primitive and custom not distinguished | 529 (9.4%) | 580 (7.5%) |
| 4. Macro invocation (`foo!(..)`) — **not resolvable** | 686 (12.2%) | 1,394 (18.0%) |
| 5. Function-as-value (bare name, matching a known fn, passed as an argument) — **counted as not resolvable at that site** (see below) | 8 (0.1%) | 8 (0.1%) |
| 6. `?` operator (implicit `Try`/`From::from` dispatch) — **not resolvable** | 138 (2.5%) | 138 (1.8%) |
| 6. Call through a non-path expression (field/index/computed callee) | 0 | 0 |
| 6. `T::assoc()` where `T` is a generic type parameter | 0 | 0 |
| **Total call sites** | **5,613** | **7,750** |
| **Resolvable share** | **20.6%** | **22.6%** |

### `tools/` (check, kernel, render, schedule)

| Bucket | Excl. tests | Incl. tests |
|---|---|---|
| 1a/1c. Plain path call — **resolvable** | 453 (15.2%) | 841 (17.5%) |
| 1b. Call through a local variable/parameter — **not resolvable** | 10 (0.3%) | 15 (0.3%) |
| 2. Method call — **not resolvable** | 1,479 (49.5%) | 2,323 (48.3%) |
| 3. Operator that could dispatch to a first-party `impl` — **not resolvable** | 887 (29.7%) | 1,080 (22.4%) |
| 4. Macro invocation — **not resolvable** | 147 (4.9%) | 539 (11.2%) |
| 5. Function-as-value — **counted as not resolvable** | 2 (0.1%) | 2 (0.0%) |
| 6. `?` operator | 10 (0.3%) | 11 (0.2%) |
| 6. Non-path call / generic-param assoc call | 0 / 0 | 0 / 0 |
| **Total call sites** | **2,988** | **4,811** |
| **Resolvable share** | **15.2%** | **17.5%** |

Parsing had zero failures in both trees — every `.rs` file under `crates/` and `tools/` parsed and
was classified; nothing was skipped because the tool couldn't read it.

**Choices made explicit, as asked:**

- **`Type::assoc(..)` (bucket 1a/1c)**: counted as resolvable — same as a plain function call —
  *unless* the qualifying segment is a generic type parameter in scope at that call (`T::assoc()`
  inside `fn f<T: Trait>(...)`), in which case it is unresolvable for the same reason a method call
  is: which `impl` runs depends on what `T` gets monomorphized to, and the source doesn't say.
  `Self::assoc()` is always counted resolvable — unlike a generic parameter, `Self` inside an
  `impl` block names exactly one written function body regardless of what the impl is generic over.
  Neither codebase measured has *any* generic-parameter-headed associated calls (`grep`-confirmed:
  zero `T::` call patterns in either tree), so bucket 1c is 0/0 here — not a bug, a real property of
  this codebase's own restrained style, and a reason this measurement may be optimistic for a more
  generically-written Rust codebase (see the representativeness caveat below).
- **Operators (bucket 3)**: reported as one combined total, not split into "primitive" vs.
  "first-party impl," because — as stated above — that split needs type information this tool
  doesn't have, and Ply's own `reach.rs` doesn't attempt it either. Every operator site is counted
  as unresolvable.
- **Function-as-value (bucket 5)**: **not** counted as resolvable. A bare name passed as an
  argument is not itself a call to a specific callee at that source location — the actual
  invocation, if any, happens somewhere else the tool never sees (inside whatever received the
  value), possibly through several more layers of indirection. It is reported as its own bucket
  rather than merged into either side, but it counts toward the *unresolvable* side of the headline
  ratio, because — from that exact site — no callee can be named. It is rare in both trees (8 and
  2 occurrences), so this choice barely moves the headline number either way.

## Validation: the counter checked against a hand count

`crates/ply-attrs/src/lib.rs` (122 lines, the smallest real source file in the repo suitable for
hand-counting) was counted by hand before being run through the tool. Hand count: 28 call-shaped
sites — 3 plain calls (1 `expand_unresolved`, 2 `String::new`), 15 method calls, 10 macro
invocations, 0 operators, 0 function-as-value, 0 other. The tool's output for that file matched
exactly: 28 sites total, same 3/15/10/0/0/0 split, same lines.

The check also caught something worth reporting on its own: the file's one test,
`a_marker_expands_to_an_unconditional_unimplemented_naming_the_id_and_the_note`, contains

```rust
assert_eq!(
    expand_unresolved(tokens).to_string(),
    "unimplemented ! (\"unresolved #147: employee discount undecided\")"
);
```

`expand_unresolved(tokens)` (a plain call) and `.to_string()` (a method call) sit right there in
the source text, but both are inside the arguments of `assert_eq!`, which `syn` treats as an opaque
token stream rather than parsed Rust — so neither is counted anywhere, by hand or by tool. This is
the exact caveat `reach.rs`'s own module comment gives for macros ("a macro's expansion is not in
the tokens the walk reads"), confirmed here to apply to a macro's *arguments* too, not just its
expansion. Both counts came out at 28 with this pair excluded; the true count of calls *written in
the source text* is 30. See the caveats below for what that means for the headline number.

## What this means for the item tier

**Do not build it as a syntax-only item tier.** Four out of five call sites in `crates/` — and five
out of six in `tools/` — cannot be resolved to a specific callee by reading source text, before any
type checker runs. A tier that reports at the item level would be reporting on the smallest of the
categories measured here (plain function calls) while staying silent on method calls, which are the
single largest category in both trees by a wide margin (54.2% and 49.5% of all call sites,
excluding tests). That is not a tier with a gap at the edges; it is a tier whose blind spot is
bigger than its coverage. Nothing about test-vs-production code, or about which of the two trees is
looked at, changes that conclusion — the resolvable share stays in the 15–23% band throughout every
cut in the table above.

## Every way this measurement could be wrong or misleading

- **Test code is included in `crates/`'s and `tools/`'s own inline `#[cfg(test)] mod` blocks by
  default in the "excl. tests" column's denominator's sibling column, and both are reported.** Test
  code was *not* excluded from the source trees scanned — it lives inside the same `.rs` files as
  production code (see `crates/ply-core/src/reach.rs`'s own `#[cfg(test)] mod tests` for a typical
  example) and stripping it would need attribute-aware filtering, which this tool does implement
  (tracking `#[cfg(test)]` and `#[test]`/`#[…::test]` down through nested modules) — hence two
  columns rather than one. The direction of the skew is the opposite of what might be assumed: test
  code pushes the resolvable share *up* slightly (test bodies often open with a plain call to the
  function under test), not down. Top-level integration tests under the repo's own `tests/`
  directory were excluded entirely, by scope (`crates/` and `tools/` only, as asked) — those were
  not measured at all, in either direction.
- **Macro arguments hide calls from this tool, confirmed above, not just asserted.** Anything
  written inside `assert!(...)`, `assert_eq!(...)`, `println!(...)`, `vec![...]`, and similar is
  invisible to `syn`'s expression walk — the validation file alone had 2 such hidden sites in a
  28-site file. This under-counts the *total* call-site count in both trees by an unknown amount,
  concentrated whichever way macro-heavy test assertions and formatting calls skew — likely more in
  test code (heavy `assert_eq!`/`assert!` use) than production code. Whether this makes the
  resolvable *percentage* look better or worse depends on what kind of calls are hidden, and this
  tool cannot see well enough to say which way that cuts.
- **Whether this repository is representative of what Ply targets is not something this tool can
  answer, and the honest answer leans "no" in one specific way**: Ply's own project rules push
  hard toward "plain Rust" — CLAUDE.md and `reach.rs`'s design both exist because the team already
  avoids `impl` blocks, generics-heavy dispatch, and operator overloading where it can. That shows
  up directly in these numbers: `T::assoc()` generic dispatch and non-path calls both measured
  zero across the entire codebase. A more typical Rust codebase — heavier on builder patterns,
  iterator chains, trait objects, derived `Display`/`Add`/`From` impls, and async trait macros —
  would likely show a **lower** resolvable share than what's reported here, not a higher one. If
  Ply's target audience writes code in this repo's own restrained style, this number is a fair
  estimate for them; if the target audience writes typical idiomatic Rust, this number is
  optimistic.
- **The function-as-value heuristic (bucket 5) is a name-matching guess, not a resolved fact.** A
  bare lowercase identifier passed as a call argument is flagged only if it also matches the name
  of *some* function declared anywhere in the same source tree, and is not itself a
  parameter/`let`/match-bound local name in the enclosing function. That both over- and
  under-counts: it can mistake an unrelated local variable for a function reference if some
  unconnected function elsewhere happens to share its name (over-count), and it misses a function
  passed by reference (`&helper`) or through a closure that merely calls it (under-count). The
  measured count is tiny either way (8 of 5,613, 2 of 2,988), so this doesn't move the headline
  number, but it means bucket 5's count specifically should not be trusted to the digit.
- **The local-binding-call check (bucket 1b) is deliberately over-inclusive and therefore
  conservative in the direction that only lowers the reported resolvable share, never raises it.**
  It flags every single-segment call whose name matches *any* pattern-bound identifier anywhere in
  the enclosing function — parameters, every `let`, every `match`/`if let` arm, every closure
  parameter — without tracking real lexical scope or shadowing order. That can occasionally
  misclassify a genuine call to a same-named top-level function as "local" if an unrelated binding
  with that name exists elsewhere in the same function body. The true resolvable share is therefore
  greater than or equal to the number reported, never less — a bias toward understating rather than
  overstating what a syntax-only reader could do.
- **`Type::assoc()` calls where the type segment is a trait name and the argument's concrete type
  is only known at runtime** (e.g. a hypothetical `Trait::method(x, ..)` where `x: &dyn Trait`)
  would be miscounted as resolvable by this tool's rule, which only checks whether the leading
  segment is a *generic type parameter*, not whether it's a trait name paired with a dynamically
  typed argument. No such call was observed in either codebase during spot-checking, but the rule
  doesn't rule it out in general — a genuine gap in the heuristic, not just an unexercised one.
- **Boolean short-circuit operators (`&&`, `||`) were deliberately excluded from the operator
  bucket and from every total**, on the grounds that they are not overloadable in Rust and
  therefore never dispatch to any function at all — they aren't call sites by any definition used
  here. If a reader expected them counted as "trivially resolvable" call sites, the totals above
  would look slightly different (larger denominators, same numerators, so a slightly lower
  resolvable percentage) — but they were excluded as not being calls in the first place, not as
  resolvable ones folded into the total.
