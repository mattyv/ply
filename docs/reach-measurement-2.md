# What Ply can check of a second library's stated properties — `semver`

*Measurement, 2026-08-30. A repeat of `docs/invariant-reachability.md`'s method on a second,
independently chosen library, run as a measurement and not as the acceptance gate.*

*Read: all 2,117 lines of `semver` 1.0.28's library source (`src/lib.rs`, `parse.rs`,
`eval.rs`, `impls.rs`, `display.rs`, `identifier.rs`, `error.rs`) and its four test files,
vendored from `/root/.cargo/registry/`; then `crates/ply-core/src/harness.rs`,
`crates/ply-cli/src/verify.rs`, `crates/ply-cli/src/check.rs`, and The-Ply-Spec.md §5.1,
§5.4b–§5.4c.*

*Ran: `cargo ply check` and `cargo ply verify` (human and `--json`) against a vendored copy
of `semver` in five configurations, plus four minimal reproduction crates built to isolate
blockers. No product code was written or changed. The vendored copy lives outside this
repository and nothing here modifies `semver` upstream.*

---

## The crate was chosen before reading what Ply supports

This ordering is the only thing that stops the result being cherry-picked, so it is
recorded first and in full.

**Candidates considered:** `semver`, `smallvec`, `base64`, `fixedbitset`, `num-integer`,
`humantime`, `rangemap`.

**Picked: `semver` 1.0.28** (David Tolnay), on the four selection criteria in order:

1. **Real and depended upon.** Cargo itself uses it; it is in the dependency tree of most
   of the Rust ecosystem. Not a toy, not written for this exercise.
2. **Readable in full.** 2,117 lines of library source, under the 3,000-line budget.
3. **The author wrote the properties down.** The doc comments are unusually explicit: a
   "# Total ordering" section on three separate types, a per-operator expansion table
   giving `^`, `~`, `=`, `>`, `<`, `<=` and `*` their exact equivalents, a numbered list of
   parse rejections with an example string each, and a counterintuitive pre-release
   matching rule spelled out in a full paragraph. The SemVer spec's own wording is quoted
   inline in the comparison code. Four test files carry law-shaped tests
   (`test_spec_order`, `test_caret`, `test_tilde`, `test_wildcard`, `test_pre`).
4. **The shape the pitch targets.** Pure logic, no I/O, no threads, no mutable state, and a
   wrong answer means Cargo resolves a dependency to a version that should never have
   matched.

**Disclosure of partial prior knowledge, because "blind" should mean what it says.** The
brief required reading `docs/invariant-reachability.md` first, and that document names the
*previous* supported-type list. So I knew, before choosing, that integers and `Duration`
were buildable and that floats and structs were not. I did **not** read `harness.rs`, the
spec's supported-signature list, or `TODO.md` before committing. The knowledge I did carry
pointed *away* from `semver`, not toward it: its whole public surface is `&str` and its own
struct types, which the old list suggested would be hostile. It turned out to be hostile
for entirely different reasons than the old list would have predicted, which is the
substance of this document.

---

## TLDR

**Of the sixteen properties `semver`'s author wrote down, Ply checks one — and it checks
that one only because the author happened to type `-> Self` rather than `-> Version`.**
Rewriting that return type to name the type explicitly, a change no compiler and no reader
would call a change at all, turns the single passing verdict into `unsupported`. Zero of
the sixteen are out of this tool's shape: every one is a pure single-function property with
no threads, no sequences and no hidden state. That is the most favourable library this
project is ever likely to meet, and reach is 1 in 16.

**Bad news first, in the order a stranger would hit it.** Ply generated a harness that does
not compile, on a nine-line reproduction, for any method whose parameter has the same type
as its receiver — `a.same_as(b)`, `merge`, `union`, `cmp`. It printed a false sentence:
"it has no associated function … that builds a `A` value … and none was found", about a
type whose `A::new` Ply demonstrably calls three lines away in the same crate. It accepted
six contracts written the documented out-of-source way, reported "6 of 6 fn claims point at
a function Ply can find", then ran none of them and said so only in a warning that
contradicts a second warning in the same run. And when it did find a real bug, the terminal
said "proptest shrank a failing case to this minimal example" and then showed no example —
the values exist, but only in `--json` and in a file Ply wrote into the crate's `src/`
without mentioning it.

**The single capability that would unblock the most is `&str` parameters — it blocks 11 of
the 16.** But the sharpest number in this measurement is a different one: **no single
capability unblocks even one additional property.** Every one of the fifteen unreached
properties is held by between two and four independent blockers, so any one fix shipped
alone moves reach from 1 to 1.

**This contradicts the first measurement's ranking almost point for point.** Floating point
was ranked first there; `semver` contains no floats at all, so it would unblock zero.
Structs and enums were ranked last there with explicitly "zero effect"; here they are the
gateway to twelve of the sixteen. Mutable output parameters and generic instantiation,
ranked third and fourth there, unblock zero here. And the blocker that dominates this
library — Ply refusing a function because of its *return* type — never appeared on the
first list at all, because in the rate limiter everything was already blocked at the
parameters and nothing ever reached the return check. One library's ranking does not
generalise, and now that is measured rather than assumed.

| | count | which |
|---|---|---|
| Checkable today | **1** | #13 (`Version::new` leaves pre-release and build metadata empty) |
| Out of this tool's shape entirely | **0** | — every property is single-function |
| Single-function in substance, blocked | **15** | all the rest |
| Unblocked by any *one* capability shipped alone | **0** | blockers come in chains of 2–4 |

---

## What Ply can do here, stated plainly

So the reasoning below can be followed without opening anything:

- A promise is attached to **one function**: a condition required of its inputs, a condition
  guaranteed of its result. Ply then either proves it exhaustively for small inputs, or
  throws hundreds of random inputs at it.
- To do either it must **build every argument from nothing**. It can now build: the
  integers, `bool`, `char`, `Option`, `Result`, fixed arrays, `Vec`, `BTreeSet`, the
  `NonZero` family, `Duration`, `f32`/`f64` (random inputs only), `String` (random inputs
  only), and a struct or enum of the user's own — the last built by calling the type's own
  constructor where one exists, or by naming its fields where they are all public.
- It cannot build **`&str`**. `String` is supported and `&str` is not; the reference is
  looked through and the bare `str` behind it is not a type Ply knows.
- It refuses a function whose **return type** it cannot itself construct, even though it
  never constructs one — the real call produces it. The one exception is the literal word
  `Self`.
- It does not check **trait methods** — so nothing behind `Ord`, `Default`, `Display` or
  `FromStr` is reachable, which in this library is most of the interesting behaviour.
- It does not check **private functions**, and says so clearly.

One structural fact dominates everything below: **`semver` is a string parser and a
comparison library.** Every value it operates on is produced from a string, and every
question it answers is answered by an ordering. Ply can hand it no string and cannot accept
an ordering back.

---

## The sixteen properties

Each is quoted or paraphrased from where the author stated it, with a pointer.

| # | Property (author's words, abridged) | Where stated | Single-function? | What stops Ply | Author flagged? |
|---|---|---|---|---|---|
| 1 | Major, minor, patch "compared numerically from left to right, lexicographically ordered as a 3-tuple of integers"; `1.5.0 < 1.19.0` | `Version` doc, "# Total ordering" | Yes | derived trait method (no body to anchor); `Ordering` return | Yes — worked example given |
| 2 | "a pre-release version is considered less than the ordinary release" | same | Yes | same as #1 | Yes |
| 3 | Digit-only identifiers compared numerically; letter/hyphen in ASCII order; "any numeric identifier is always less than any non-numeric" | `Prerelease` doc + spec quoted inline in `impls.rs` | Yes | trait method; `Ordering` return; `Prerelease` unbuildable; `&str` | Yes — a "*Tip:*" warns users about `alpha11 < alpha2` |
| 4 | "A larger set of pre-release fields has a higher precedence than a smaller set, if all of the preceding identifiers are equal" | spec quoted verbatim in `impls.rs` | Yes | same as #3 | Yes — quoted from the spec |
| 5 | "Build metadata is ignored in evaluating `VersionReq`; it plays no role in whether a `Version` matches" | `BuildMetadata` doc | Yes, as a contract calling `matches` twice | `VersionReq` unbuildable; `&str` | Yes |
| 6 | `cmp_precedence` compares "disregarding build metadata. Versions that differ only in build metadata are considered equal." | `Version::cmp_precedence` doc + doctest | Yes | `Ordering` return; `&Self` not resolved; duplicate-import bug | Yes — a full sorting doctest |
| 7 | Build metadata's own order keeps leading zeros significant: `0 < 00 < 1 < 01 < 001 < 2 < 02 < 002 < 10` | comment in `impls.rs`; `demo.090` example in doc | Yes | trait method; `Ordering` return; unbuildable receiver; `&str` | Yes — hand-written table |
| 8 | `^I.J.K` (I>0) ≡ `>=I.J.K, <(I+1).0.0`; `^0.J.K` (J>0) ≡ `>=0.J.K, <0.(J+1).0`; `^0.0.K` ≡ `=0.0.K` | `Op` doc, "# Op::Caret" | Yes, on `Comparator::matches` | `Comparator` unbuildable (`Op` is `#[non_exhaustive]`, `pre` is private-field); `&str` | Yes — six cases enumerated |
| 9 | `~I.J.K` ≡ `>=I.J.K, <I.(J+1).0`; `~I.J` ≡ `=I.J`; `~I` ≡ `=I` | `Op` doc, "# Op::Tilde" | Yes | same as #8 | Yes |
| 10 | `I.J.*` ≡ `=I.J`; `I.*` ≡ `I.*.*` ≡ `=I`; `=I.J` ≡ `>=I.J.0, <I.(J+1).0`; `>I.J` ≡ `>=I.(J+1).0` | `Op` doc, four operator sections | Yes | same as #8 | Yes |
| 11 | "in order for *any* `VersionReq` to match a pre-release version, the `VersionReq` must contain at least one `Comparator` that has an explicit major, minor, and patch version identical to the pre-release being matched, and that has a nonempty pre-release component" | `VersionReq::STAR` doc; repeated as a comment in `eval.rs` | Yes | `VersionReq` unbuildable; `&str` | **Emphatically** — the word "Counterintuitively" and a full paragraph |
| 12 | "The default VersionReq is the same as `VersionReq::STAR`" ≡ `parse("*")` ≡ `>=0.0.0` | `impl Default` doc; `test_default` | Yes | trait method; `VersionReq` return type | No |
| 13 | "Create `Version` with an empty pre-release and build metadata" | `Version::new` doc | Yes | **nothing — this one reaches** | No |
| 14 | Parse rejects, each with an example: too few components, leading zero, unexpected character, empty pre-release or build, characters outside `0-9 A-Z a-z -`, `u64` overflow; and "Whitespace is not allowed anywhere" | `Version::parse` "# Errors", seven bullets; `Version` "# Syntax" | Yes | `&str`; `Result<Version, Error>` return type | Yes — seven named failure modes |
| 15 | An accepted identifier is stored verbatim: `as_str`, `len`, `is_empty` and equality all agree with the input string | `tests/test_identifier.rs::test_new`, a loop over lengths 0–280 and one at 20,000 | Yes | `&str`; `Result<Prerelease, Error>` return type | **Yes** — the author wrote an actual length-swept property test here, the only one in the crate |
| 16 | A requirement may hold at most 32 comparators; beyond that, "excessive number of version comparators" | `MAX_COMPARATORS` in `parse.rs`; `test_multiple` | Yes | `&str`; `Result<VersionReq, Error>` return type | Yes — an explicit denial-of-service bound |

**Zero out of sixteen are out of the tool's shape.** The first measurement reported four of
eleven out of shape and rightly called that a result to state proudly. `semver` has no
counterpart: no concurrency, no sequences of operations, no hidden state, no
trace-equivalence claims. Every property here is one pure function, its arguments, and its
return value. This is the shape the tool was designed for, met in the wild, and it reaches
one of them.

---

## What the tool actually said

### Run 1 — six properties in `ply.yaml`, `semver`'s source untouched

The documented out-of-source way to attach a promise (§5.1: `requires:` / `ensures:` under
`fns:`) is the only way that makes sense for a crate you did not write. `check` was
encouraging:

```
  anchors       6 of 6 fn claims in this crate point at a function Ply can find.
```

`verify` then produced six `unsupported` verdicts in 0.56s, and this pair of warnings about
the same function in the same run:

```
[W0510] semver::Version::new — the `requires:`/`ensures:` declared for `Version::new` in
  ply.yaml is used where §5.5 needs it -- callers of `Version::new` may assume it at a
  boundary -- but it is **not** yet ANDed into `Version::new`'s own checks, which §5.4 says
  it should be. So this run checked `Version::new` against its inline
  `#[ply::requires]`/`#[ply::ensures]` only. (W0510)

[V0505] semver::Version::new — `Version::new` declares `fuzz` but has no `#[ply::ensures]`
  and no `examples:` entries -- there is nothing to check its result against, so nothing was
  run. Add an `#[ply::ensures]` clause naming what `Version::new` promises about its result,
  or add `examples:` entries naming concrete calls to assert.
```

The first says the contract exists and was used. The second says there is no contract. Both
are about the same function, and the reader has to reconcile them alone. The actionable
truth — *contracts written in `ply.yaml` do not reach the engines; only source attributes
do* — is stated in neither, and is the single most important thing a new user needs to know
before they write a line of configuration. `check`, the command people run first, reports
"6 of 6" and gives no hint of it at all.

The other four refusals:

```
[V0505] semver::Prerelease::new — Ply cannot check `Prerelease::new`: parameter(s) text: str
  use a type neither the bounded (Kani) nor the fuzz (proptest) codegen builds inputs for.
  This is reported as unsupported, not attempted -- it never silently hangs.

[V0505] semver::Version::cmp_precedence — Ply cannot check `Version::cmp_precedence`:
  parameter(s) other: Self use a type neither the bounded (Kani) nor the fuzz (proptest)
  codegen builds inputs for. ...

[V0507] semver::VersionReq::matches — Ply cannot build a receiver for `VersionReq`: it has
  no associated function in the file it is declared in that builds a `VersionReq` value and
  takes only types Ply's checkers already know how to build -- constructing a receiver needs
  a constructor to call, and none was found.

[V0507] semver::VersionReq::default — `VersionReq::default` is defined in a trait
  implementation (`impl ... for VersionReq`). Ply checks inherent methods and free
  functions, not trait methods, yet.
```

Two of these are good. `V0507` on `default` is exactly right: names the reason, names the
boundary, no jargon. The `VersionReq::matches` message names the missing thing precisely.

Two are not. `text: str` is not what the user wrote — the source says `&str`, and a reader
searching their file for `str` as a parameter type will not find it. And `other: Self` names
a spelling rather than a type: the user's mental model is "it takes another `Version`", and
Ply reports a keyword.

### Run 2 — one inline annotation added to `Version::new`

Forking the crate to add `#[ply::ensures(|result| result.pre.is_empty() && result.build.is_empty())]`
is what a user must actually do. It works:

```
    Version::new — fuzzed(256)
```

Everything else stayed `unsupported`. The first run took 21.3s (building the generated
harness crate); repeat runs are 0.5s. Nothing hung, nothing crashed, nothing took absurdly
long.

### Run 3 — the same code, with `-> Self` written as `-> Version`

```
    Version::new — unsupported
[V0505] semver::Version::new — Ply cannot check `Version::new`: none of its declared checks
  apply to this function's shape. This is reported as unsupported, not attempted.
```

A cosmetic edit to the return type — same function, same body, same behaviour, accepted by
the compiler without comment — deletes the only verdict this library earns. And the
explanation degrades at the same moment: the message that had named a parameter type now
names nothing at all. `unsupported_shape_diag` inspects only parameters, so when the blocker
is the return type it falls back to a sentence carrying no information a user can act on.
A neighbouring doc comment in the same file already calls this wording "simply false" in an
adjacent case; here it is merely useless, which is not much better.

### The check is real — it goes red when the function is broken

Making `Version::new` return a non-empty pre-release for `major == 7 && minor == 7`:

```
    Version::new — violation
[P0502] semver::Version::new — `Version::new` breaks its own postcondition
  `|result|result.pre.is_empty() && result.build.is_empty()` for at least one input --
  proptest shrank a failing case to this minimal example. (P0502)
```

It found it, and the generated input strategy (`prop_oneof![3 => 0u64..=16u64, 1 =>
any::<u64>()]`) makes that a real catch rather than luck. The `--json` envelope carries
`{"major": "7", "minor": "7", "patch": "0"}` and a replay seed, and Ply wrote a runnable
failing test to `src/ply_generated_cex.rs` with a genuinely good panic message.

**Two things are wrong with how that is delivered.** The terminal sentence ends "…to this
minimal example." and then shows no example; the values are in the JSON envelope, which the
default invocation does not print. And Ply added a file to the crate's `src/` and a `mod
ply_generated_cex;` line to `lib.rs` without saying so anywhere in its output. On a crate
the user wrote that is a surprise; on a vendored third-party crate it is an edit to someone
else's source that only `git status` will reveal.

### One thing you have to know that nothing tells you

`--help` prints:

```
Usage: cargo-ply [OPTIONS] <COMMAND>
```

The literal word `ply` you must type between the binary and the command appears nowhere in
that line, and putting a global option where the usage string implies it goes fails:

```
$ cargo run -p ply-cli -- --json ply verify <dir>
error: unrecognized subcommand 'ply'
```

`--json` has to go *after* the directory. Small, but it is the first wall a newcomer hits,
and the help text points them straight at it.

---

## Three defects found by running it rather than reasoning about it

Each is reduced to a minimal reproduction. None of these is a `semver` quirk.

### 1. A method whose parameter type equals its receiver type generates a harness that does not compile

Nine lines:

```rust
pub struct Pair { pub a: u64 }
impl Pair {
    pub fn new(a: u64) -> Self { Pair { a } }
    #[ply::ensures(|result| *result == (self.a == other.a))]
    pub fn same_as(&self, other: &Pair) -> bool { self.a == other.a }
}
```

```
    Pair::same_as — tool_error
[X0901] dupimport::Pair::same_as — `Pair::same_as`'s `fuzz(64)` check ran zero cases: the
  test harness Ply generates for it failed to compile, so nothing was checked at all. This is
  reported as a tool error -- never as a pass, because no evidence was gathered, and never as
  a violation, because there is no failing input to show. The compiler's own first error was:
  error[E0252]: the name `Pair` is defined multiple times. (X0901)
```

The codegen emits `use dupimport::Pair;` once for the receiver and once for the parameter.
**The diagnostic itself is excellent** — it refuses to call this a pass or a violation and
quotes the compiler — but the shape it breaks on is `equals`, `merge`, `union`, `min`,
`max`, `cmp`: most binary operations on a type. Six of `semver`'s sixteen properties are
this shape.

### 2. A `Result<Self, E>` constructor is accepted for a parameter and refused for a receiver

The same type, the same constructor, in one crate:

```rust
pub struct OpaqueErr { _k: u8 }
pub struct A { v: u64 }
impl A {
    pub fn new(v: u64) -> Result<Self, OpaqueErr> { Ok(A { v }) }
    #[ply::ensures(|result| *result >= 0)]
    pub fn get(&self) -> u64 { self.v }          // receiver
}
#[ply::ensures(|result| *result >= 0)]
pub fn a_as_param(a: A) -> u64 { a.v }            // parameter
```

```
    A::get — unsupported
    a_as_param — fuzzed(16)
[V0507] ctorprobe::A::get — Ply cannot build a receiver for `A`: it has no associated
  function in the file it is declared in that builds a `A` value and takes only types Ply's
  checkers already know how to build -- constructing a receiver needs a constructor to call,
  and none was found.
```

The sentence is false. `A::new` is exactly such a function, it takes a `u64`, and Ply calls
it successfully for `a_as_param` in the same run. `Result<Self, E>` was deliberately
admitted as a constructor shape on 2026-08-28 (`ctor_return_kind`'s `CtorReturn::ResultSelf`,
and `docs/review-structs-enums.md` finding 2) — the widening reached the parameter path and
not the receiver path. An error type that is itself buildable (`Result<Self, String>`)
behaves identically, so the error type is not the cause.

This matters here beyond the wording: **every constructor in `semver` returns `Result<Self,
Error>`, and every `semver` type is used as a receiver.**

### 3. `check` and `verify` still explain the same refusal differently, and `check` is the wrong one

```
check:  V0507 Ply found `VersionReq::matches` but cannot yet build a value of `VersionReq`
        to call it on -- constructing a receiver is not supported yet.

verify: [V0507] Ply cannot build a receiver for `VersionReq`: it has no associated function
        ... and none was found.
```

"Constructing a receiver is not supported yet" is false — it is supported, and a shipped
fixture depends on it. This is the same defect an adversarial review caught on 2026-08-27;
the fix (`check_does_not_deny_a_receiver_verify_can_actually_build`) closed the branch where
`verify` *can* build the receiver and left the blanket sentence standing on the branch where
it cannot. A newcomer who runs `check` first — which the tool's own output recommends — is
told a feature that exists does not.

---

## The ranking: which single capability unblocks the most

Counted as "appears in this property's blocker set", over the sixteen:

| Rank | Capability | Properties it blocks | Unblocks alone |
|---|---|---|---|
| 1 | **`&str` parameters** | 11 — #3, #4, #5, #7, #8, #9, #10, #11, #14, #15, #16 | **0** |
| 2 | **Accept a return type Ply can observe but not construct** (`Ordering`, `Result<Self, E>`, a named own type) | 10 — #1, #2, #3, #4, #6, #7, #12, #14, #15, #16 | **0** |
| 3 | **Build a receiver from a `Result<Self, E>` constructor** (defect 2 above) | 8 — #3, #4, #5, #7, #8, #9, #10, #11 | **0** |
| 4 | **Trait methods** (`Ord`, `Default`, `Display`, `FromStr`) | 6 — #1, #2, #3, #4, #7, #12 | **0** |
| 5 | **Same-type-as-receiver parameter** (defect 1 above) | 6 — #1, #2, #3, #4, #6, #7 | **0** |
| 6 | **`&Self` resolved to the concrete type** | 6 — #1, #2, #3, #4, #6, #7 | **0** |
| 7 | **`#[non_exhaustive]` enums** | 3 — #8, #9, #10 | **0** |

**`&str` is the answer to the question as asked**, and it is the right answer for the right
reason: this is a string parser, and every value in the library is born from a string, so
`&str` is both the direct blocker on the parse properties and the root of the chain that
makes every one of the library's own types unbuildable. `String` being supported while
`&str` is not is also the cheapest entry on this list by a wide margin — the sampling engine
already generates a `String` with curated content and a length bound; a `&str` parameter
needs that same value, owned by the harness and lent.

**But the honest headline is the last column.** Shipping `&str` alone moves reach from 1/16
to 1/16, because all eleven of the properties it blocks are also blocked by something else.
The smallest bundle that moves the number at all is `&str` **plus** dropping the return-type
gate, which lands #14, #15 and #16 — the parse properties, including the one length-swept
property test the author actually wrote — taking reach to 4 of 16. Adding
`Result<Self, E>` receivers on top makes `Prerelease`, `BuildMetadata`, `Comparator` and
`VersionReq` constructible, which is the gateway to the remaining twelve.

**The return-type gate deserves a separate note**, because it is second on the list and it is
the only entry that costs nothing to remove. Its own doc comment records that it was added as
"a deliberate, requested narrowing … not a fix for an observed compile failure", and states
that "empirically … such a return type does not itself break anything on either engine:
nothing in this codegen ever names or constructs a return type". A gate that the code
records as blocking nothing technically is, on this library, the second-largest blocker
there is — and it is what makes `-> Self` versus `-> Version` a verdict-changing
distinction.

---

## Does this replicate the first measurement's ranking, or contradict it?

**It contradicts it, and nearly item for item.** That is the more valuable result: it
converts "the ranking generalises" from an assumption into a measured falsehood.

| Ranked on the rate limiter | Properties it unblocks in `semver` |
|---|---|
| 1. Floating-point numbers | **0** — `semver` contains no float anywhere |
| 2. Building a receiver from the library's own constructors | 8 — but only via the `Result<Self, E>` shape, which is *built and broken* rather than missing |
| 3. Mutable output parameters | **0** — `semver` has no `&mut` parameter |
| 4. Pinning a generic type parameter | **0** — `semver`'s public surface is not generic |
| 5. Structs and enums — "zero effect" there | The gateway to **12** of the 16 here |

And the two blockers that dominate `semver` — `&str` parameters and the return-type gate —
appear nowhere on the first list. Not because the first measurement missed them, but because
in the rate limiter every function was already refused at its parameters, so no function ever
reached the return-type check and no property ever needed a string. A blocker only becomes
visible once the ones in front of it are gone.

Three lessons that do generalise, on two libraries rather than one:

1. **Rank against more than one library, or the roadmap is the shape of one codebase.** The
   two rankings share no top item and disagree about the bottom one.
2. **Blockers chain, and counting appearances overstates the value of a fix.** Both
   measurements report a per-capability count; only this one asked "if I ship exactly this,
   what becomes checkable?" and got zero for all seven. That second column is the number
   worth putting on a roadmap.
3. **The out-of-shape count varies enormously and is a property of the library, not the
   tool.** Four of eleven there; zero of sixteen here. Reporting it proudly is right, but it
   is not a stable share.

---

## Uncertainties, and what would settle them

**Is `&str` genuinely cheap, or does lending complicate the harness?** The fuzz codegen
already builds an owned `String` with a bounded length and curated content; a `&str`
parameter wants that value held by the harness and borrowed for the call, which is the same
pattern `&T` already uses. *Lean: cheap, a few lines in the type parser plus the existing
`&T` lending path.* *Settled by:* a fixture with `fn f(s: &str) -> usize` and a length
contract, before anyone puts it on a roadmap with a promise attached.

**Would dropping the return-type gate reintroduce something it was added to prevent?** Its
own comment says no, in as many words, and describes the real cause of the failure it was
mistaken for (a zero-parameter function's strategy being a bare `()`), which was fixed
separately. *Lean: it can be dropped, or narrowed to "the contract must be able to name the
return value", which is the real requirement.* *Settled by:* removing it behind the existing
fixtures and seeing whether any harness stops compiling.

**Is the duplicate-import bug one bug or two?** The reproduction is a receiver and a
parameter of the same type. Whether two *parameters* of the same user type collide the same
way was not tested. *Settled by:* a two-line addition to the same reproduction crate.

**Did I count #5 and #8–#10 too generously as single-function properties?** Each is stated
by the author as an equivalence between two requirements, which is naturally two or three
calls. I counted them as single-function because the contract language admits any expression
that parses, so the second call can live inside the contract — the same latitude the first
measurement found and recorded. If a future change actually enforces the spec's closed list
of allowed constructs, four of these sixteen become two-call properties and the out-of-shape
count goes from zero to four. *Flagged, not resolved:* the spec still describes a restriction
that nothing enforces, which the first measurement also flagged and which is still true.

**One thing I could not test.** Whether reach would have been better on a library whose
types are built from integers rather than strings. `semver` was chosen for the strength of
its documented properties, not for its argument types, and its string-centric surface is a
genuine property of the domain rather than bad luck — but a third measurement on an
integer-centric library would say whether `&str` is `semver`'s problem or everyone's.
