# Adversarial review: structs, enums, and running on an ordinary crate (everything since `ae9f901`)

*Read-only review, 2026-08-28. No product code was written or changed; `cargo test
--workspace` was never run. Every claim below was settled by running the already-built
`target/debug/cargo-ply` (built 01:10, after the last commit that touched code — the final
commit changed only the specification) against twenty throwaway crates in `/tmp/rev5/`, plus
copies of the rate limiter and of the published `semver` crate. Commands and recipes are at
the end. Scratch build output has been deleted.*

---

## TLDR

**There is a fourteenth, and it is the twelfth wearing a different hat.** A method whose
promise is false after **one** ordinary call gets a green pass, exit 0, written into the
committed record and reused on the next run — because any earlier call whose argument
Ply cannot build is dropped from the object's history without a word, and the sentence
printed beside the green verdict says the opposite: *"every value this run saw was
reachable by calling the type's own code, nothing else, so nothing here was assumed."*
Three ordinary argument types open this door: a piece of borrowed text, a slice, and a
struct. I proved the promise false by running the program: it prints 5 against a promise
of "always 0".

**Second, in the other direction, and this is the answer to "can Ply build a value the
real program cannot": yes, and it reports a violation from it.** Write the most ordinary
fallible constructor in Rust — the one that returns a result and rejects bad arguments —
leave the fields public so callers can read them, and Ply ignores the constructor
entirely, assembles a value the constructor exists to prevent, and tells you your correct
function breaks its own promise. The same thing happens, with a constructor Ply *can*
read and *does* honour elsewhere, purely because the type is declared in one file and its
constructor written in another. The disclosure fires, but the disclosure is not enough:
its stated ground is "there was no constructor to call", and in both cases there was one.

**Third, the good news, and there is a lot of it.** The blocker from the last review is
genuinely gone: I ran Ply inside a member of an ordinary two-crate workspace, it found the
planted bug, it wrote nothing into any manifest, the root build still works afterwards,
and `cargo clean` no longer breaks the crate. The twelfth false clean is really fixed —
both flavours the last review named now come back red. And struct and enum parameters work
on every shape I could build out of parts Ply already supports: text inside a struct, a
list inside a struct, an enum carrying data, three levels of nesting, a struct argument and
an object to call the method on at the same time, a precondition that reads a struct's
field, and every variant of an enum reachable including the first and the last.

**The refusal rate has not moved on the yardstick: still 0 of 39.** On a real published
crate I picked instead (`semver`), 1 of 13 promises earned a verdict. The single biggest
cause on that crate is not on anyone's caveat list: **borrowed text is not supported.**
Owned text is; `&str`, the commonest parameter type in Rust, is refused by name.

**Could a user use this on their own code today, with eyes open?** The layout wall is
down, so the answer is no longer "you cannot start". But it is still no: a green verdict on
a method cannot be trusted without reading the generated harness to see which calls were
dropped, and a red verdict on a function taking a struct may be an artifact of a value the
program cannot produce.

---

## Ranked by whether a user could be misled

| # | | Why it ranks here |
|---|---|---|
| **1** | Green pass on a promise that is false after one call | A false clean, exit 0, recorded and reused, reachable through the commonest argument types in Rust, under a sentence that asserts the opposite |
| **2** | Correct code reported as broken | Ply builds a value the constructor exists to forbid; the fallible-constructor shape that triggers it is everywhere |
| **3** | One awkward type turns the whole crate into a tool error | Five separate ordinary shapes do it; every other function's check dies with it, and the message is raw compiler output |
| **4** | A crash in a function taking one struct loses its witness | Ply found a real crash and threw the input away, reporting it as its own problem |
| **5** | Borrowed text is refused | Six of thirteen on a real crate; absent from the caveat list |
| 6 | Smaller wording and hygiene items | Listed at the end; none of them change what a verdict means |

---

## 1. The fourteenth false clean: an operation Ply cannot build an argument for is dropped in silence

The whole crate:

```rust
#[derive(Debug, Clone, Copy)]
pub struct Amount { pub cents: u32 }        // all fields public

pub struct Till { total: u32 }
impl Till {
    pub fn new() -> Self { Till { total: 0 } }

    /// The only way the till ever changes -- and it takes a struct.
    pub fn take(&mut self, a: Amount) -> u32 {
        self.total = self.total.saturating_add(a.cents);
        self.total
    }

    #[ply::ensures(|result| *result == 0)]   // FALSE after one `take`
    pub fn total(&self) -> u32 { self.total }
}
```

```
Till::total — fuzzed(256)
EXIT=0
```

I ran the real program to be sure the promise is false rather than merely suspicious:

```
let mut t = Till::new();
t.take(Amount { cents: 5 });
t.total()                       ->  5
```

Three lines, no cleverness, and Ply is green. The generated harness says why: the pool of
"the type's own operations" it may run before the checked call contains exactly one entry,
and that entry is the checked method itself.

```rust
match __ply_op_choice {
    0 => { let _ = Till::total(&__ply_receiver); }
    _ => unreachable!(...)
}
```

`take` is gone. It was removed because Ply cannot build an `Amount` *as an operation
argument* — and this is deliberate: the source comment introducing struct parameters says
so outright, on the ground that "a real function keeps checking without that particular
mutator in its sequence, rather than the harness failing to compile". The cost of that
choice is a green verdict on a false promise, and nothing anywhere on screen says an
operation was dropped.

**It is not about structs.** I put four types in one crate, each with exactly one mutating
operation, differing only in that operation's argument type:

| the mutator takes | pooled? | verdict on a promise that is false after one call |
|---|---|---|
| `Amount` (a struct with public fields) | no | `fuzzed(128)`, exit 0 |
| `&str` | no | `fuzzed(128)`, exit 0 |
| `&[u32]` | no | `fuzzed(128)`, exit 0 |
| `Option<u32>` | yes | violation, caught |

A mutating method that takes a string is not an exotic shape. `Log::record(&mut self, name: &str)`
is the shape, and it produces a clean green pass on a promise that is false the first time
anyone calls it.

**The disclosure makes it worse rather than better.** Printed beside the green verdict:

> `Log::count` needs a `Log` value to call it on, and Ply built one itself: it called
> `Log`'s own constructor (`Log::new`), then ran up to 3 more calls to `Log`'s own
> operations before the checked call — each run picking a random number of steps from 0 to
> 3, repeating `Log::count` itself. **Every value this run saw was reachable by calling
> `Log`'s own code, nothing else, so nothing here was assumed.** But this run only covers
> receivers reached in at most 3 such calls from a freshly built one — a bug that only
> shows up on the 5th call is outside what this run checked …

Every clause of that is true and the whole is misleading. "Nothing here was assumed" is
exactly wrong: Ply assumed the operation it could not call does not exist. The clause that
carries the real news — "repeating `Log::count` itself" — reads as a parenthetical about
sampling, not as "one of this type's methods was excluded". The last sentence invites the
reader to worry about the fifth call when no first call was ever made.

**And it persists.** `ply.lock` records `fuzzed(128)` with `"cases": 128` and an evidence
block naming the seed, and the next run prints `[reused]`. A reviewer reading that diff
sees 128 cases of evidence for a promise that is false in three lines.

This is the twelfth false clean's exact shape — a state-changing call that can never
happen, a green pass, and a disclosure that says the history was explored. The twelfth was
closed by admitting `&mut self` operations and operations with different argument lists
into the pool, and I confirmed both fixes hold. This is the same hole, reached through the
argument *type* instead of the argument *count*, and it is not recorded as a known gap
anywhere outside a source comment.

**What would make it honest without building anything:** if an operation is dropped,
refuse the check by name — the same way an unbuildable *parameter* is refused — or, at
minimum, name the dropped operation in the disclosure and downgrade the verdict. A
sentence that lists the operations Ply *did* run is not the same as one that names the one
it could not.

---

## 2. Correct code reported as broken: Ply builds a value the constructor forbids

The public-fields route is the risk the brief named, and it fires in two ordinary
situations. In both, the type has a constructor that enforces an invariant, and in both
Ply never calls it.

### The constructor returns a result

```rust
#[derive(Debug, Clone, Copy)]
pub struct Window { pub start: u32, pub end: u32 }

impl Window {
    pub fn new(start: u32, end: u32) -> Result<Self, BadWindow> {
        if start > end { return Err(BadWindow); }
        Ok(Window { start, end })
    }
}

/// TRUE of every `Window` this program can build.
#[ply::ensures(|result| *result)]
pub fn well_formed(w: Window) -> bool { w.start <= w.end }
```

```
well_formed — violation          EXIT=1
```

Ply builds `Window { start: 8, end: 0 }` field by field and reports the function as
breaking its own contract. That is a fallible constructor written the way Rust
documentation tells you to write one. The result-returning constructor is already a known
gap, filed as "still refused" — it is no longer merely refused; it now silently changes
which of Ply's own three rules applies, and the second rule produces a false alarm.

### The constructor lives in a different file from the type

```rust
// src/types.rs
pub struct Window { pub start: u32, pub end: u32 }

// src/build.rs
impl Window {
    #[ply::requires(start <= end)]          // written in Ply's own notation
    pub fn new(start: u32, end: u32) -> Self { … }
}
```

Same false violation, with the witness `start=8, end=0`. Ply searches for a constructor
only in the file where the type is declared, so splitting types and their implementations
across modules — an ordinary way to organise a crate — silently demotes the type from
rule one to rule two. The precondition Ply itself would have honoured, written in the
notation Ply itself defines, is never read.

### Is the disclosure enough?

No, for one specific reason: the disclosure's justification is false in both cases. It
says the value was built by filling in fields *"(every one of them is already public, so
nothing here restricts what a caller could build)"*, and offers as the risk that "a type's
own methods can maintain a relationship between public fields that nothing in the type
itself enforces". In both crates above, something *does* enforce it — a constructor, in
the same crate, which in one case carries a machine-readable precondition Ply can read.
The sentence describes a weaker situation than the one the user is in.

I am not arguing the route should be refused outright: for a type with public fields and
no constructor at all, it is sound, and the fixture that pins it (`Point`) is a fair
example. Two narrowings would remove both false alarms without losing that:

- look for the type's constructors across the crate, not only in its declaring file — the
  index that finds the type already walks every file;
- when a type has *any* constructor Ply found but could not use (a result wrapper, a
  private one), do not fall through to field-filling silently; either refuse by name or
  say in the disclosure that a constructor exists and was skipped, and why.

For contrast, the constructor route itself is in good shape. A constructor with a
precondition is honoured on the parameter path exactly as it is for an object — I read the
generated harness and the precondition is emitted as a rejection filter before the
constructor call — so the false violation the last review found on the object path has not
reappeared here.

---

## 3. One awkward type turns every check in the crate into a tool error

Five ordinary shapes make Ply generate a harness that cannot compile. In each case Ply had
enough information to refuse by name and did not, and the message the user sees is raw
compiler output about Ply's own generated code.

| shape | what the user sees |
|---|---|
| the type is `pub(crate)`, fields all `pub` | `error[E0603]: struct `Hidden` is private` |
| a **private** constructor exists beside public fields | `error[E0624]: associated function `make` is private` |
| the struct has **13 or more** public fields (12 is fine) | `the trait bound (…13 types…): Strategy is not satisfied` |
| `#[non_exhaustive]` on a **variant** rather than the enum | `error[E0639]: cannot create non-exhaustive variant using struct expression` |
| the type lives in a private module behind a `pub use` facade | `error[E0603]: module `quota` is private` |

The private-constructor one is the most common and the most annoying, because Ply chose it
in preference to a route that would have worked: a struct with all-public fields *and* a
private helper constructor is refused entirely, when field-filling was available. I
confirmed the same blindness on the object-construction path, so this is one shared gap,
not two: neither scan looks at whether the constructor it picked is callable from outside
the crate.

**The amplifier is what makes this rank third rather than sixth.** All of a crate's
generated tests share one harness, so when it fails to compile, *every* function reports a
tool error — including functions that are entirely fine and would have failed loudly. In a
two-function crate where one takes a `pub(crate)` struct and the other has a plainly false
promise, both come back as tool errors and the real bug is never named. Put promises on
twenty functions, have one of them mention a type that is one keyword short of public, and
the whole run is red for a reason that has nothing to do with any of them.

The wording of the message is right about its own uncertainty ("Ply could not tell whether
this function's own generated code is what broke it") and that is the correct behaviour
given a broken shared harness. The fix belongs earlier: these five shapes are all
detectable before codegen.

---

## 4. A crash in a function whose only parameter is a struct loses its witness

```rust
pub struct Window { pub start: u32, pub end: u32 }

#[ply::ensures(|result| *result >= 0)]
pub fn width(r: Window) -> u32 { r.end - r.start }   // panics on start > end
```

```
width — tool_error
proptest reported a failing case for `width`, but Ply could not recover the failing
input from the run … so there is no counterexample to show you.
EXIT=2
```

The engine had the input. Running the generated harness by hand prints
`minimal failing input: ( 3, 0, )`. Ply reads that report and then discards it, because it
requires the number of recovered values to equal the number of *declared parameters* —
here two values for one parameter. A plain two-argument function that panics is reported
perfectly (I checked: a real violation with the witness), so this is specific to the new
shape.

It is a seam, and a visible one: the same recovery code was taught, in this same window,
to carry an object's shrunk history through as one opaque field rather than dropping it —
with a comment explaining exactly why dropping it would be wrong. The parameter branch two
lines below did not learn the same lesson.

Two smaller relatives of the same mismatch:

- A struct with exactly **one** field slips through the count check and its field value is
  printed as though it were the struct: for `k: Divisor`, the counterexample reads
  `k = 0`, which is not a value of that type.
- Where one parameter contributes zero values (a constructor that takes no arguments, or a
  unit struct), the counts can line up again by accident. In the case I built, proptest's
  own nesting saved it — the witness came out as `l = ()`, `p = (0,0)` — so I did not see a
  wrong value attributed to a wrong parameter. The guard protecting that is arithmetic
  coincidence, not intent.

---

## 5. The refusal story: it has not moved

### The yardstick: 0 of 39, unchanged

I rebuilt the last review's measurement rather than trusting it. Every function in the rate
limiter got a promise (a trivially true one, so that shape is the only thing being
measured), every claim got a real check asked for, all 39 ran.

| what happened | claims |
|---|---|
| declared inside a generic implementation block | 14 |
| a trait method, or a method of a trait implementation | 10 |
| its module is private, so the generated harness cannot name it (5 refused by name, 3 as a harness that would not compile) | 8 |
| the claim points at nothing findable | 2 |
| no promise to check, or a check that does not apply to the shape | 3 |
| an object Ply cannot construct | 1 |
| a parameter whose type Ply cannot build | 1 |
| **checked** | **0** |

The two commits under review move nothing here, and the commit message says so honestly
("real verdicts stay at zero there"). Worth stating plainly all the same: after four
rounds of type work, the fair sample still produces no evidence at all, and the causes are
overwhelmingly *not* type support — 24 of 39 are generics and traits, 8 are the standard
private-module-with-a-facade layout, and exactly 1 is a parameter type.

One thing did improve: the cheap command no longer only prints a reassuring coverage
number. It still says "32 of 39 fn claims in this crate point at a function Ply can find",
but it now lists every one of the refusals underneath and exits non-zero. A user who runs
it first will now see the bad news.

### A real published crate: 1 of 13

Because the rate limiter is a fixture, I also took `semver` 1.0.28 out of the local
registry — an ordinary, widely used library nobody wrote with Ply in mind — and put a
promise on all thirteen of its public inherent methods.

| what happened | claims |
|---|---|
| a `&str` parameter | 6 |
| no constructor Ply can call for the object (all four are result-returning) | 6 |
| **checked** | **1** (`Version::new`) |

**Borrowed text is the headline, and it is not on the caveat list.** Owned text works —
`fn owned_len(s: String)` is checked, the planted bug is found, and the control-character
disclosure fires correctly. `fn borrowed_len(s: &str)` is refused with "parameter(s)
`s: str` use a type neither … builds inputs for". The specification's own supported list
says a reference to a supported type is supported, built from an owned value in the
harness; that is exactly what would be needed here and it is exactly what does not happen.
The same refusal covers `&[u32]`. A reference to a user's own struct, by contrast, *does*
work — I checked, and a false promise on `fn f(p: &Point)` is caught.

The refusal wording for the six objects is also still wrong about the obstacle, unchanged
from the last two reviews: "it has no associated function … that builds a value and takes
only types Ply's checkers already know how to build", said of `BuildMetadata::new(text: &str) -> Result<Self, Error>`,
which exists, is public, and is refused for two reasons the sentence never mentions.

---

## 6. The isolated-workspace harness: it works, with one hygiene defect

This is the part of the branch I tried hardest to break and mostly could not.

- **An ordinary crate** (`cargo new --lib`, no workspace line): runs, finds planted bugs,
  writes nothing into the manifest. Confirmed by reading the file afterwards.
- **A real two-crate workspace**, run from inside a member: runs, finds the planted bug,
  the root manifest and the member manifest are both untouched, and `cargo build` at the
  root still works afterwards. This is the wall the last review said made the answer
  "wait", and it is down.
- **A second run** reuses the recorded result in 0.026s and says why.
- **`cargo clean`** in an ordinary crate no longer breaks anything: the crate still
  builds, and the next Ply run re-checks and explains that the outside-world versions
  changed. The old breakage is a straight consequence of editing the manifest, so it
  disappeared with the edit.
- **A `.cargo/config.toml`** that redirects the build directory: works.
- **A pre-existing `target/`**: works.

Two things to know:

**A crate that already has a workspace line is still edited, and `cargo clean` still
breaks it.** The old behaviour is unchanged for that case by design, but the consequence is
worth writing down: two crates that differ by one line in a manifest get very different
treatment, and on the one with the line, `cargo clean` followed by `cargo build` fails with
`No such file or directory`. Since every multi-crate repository's *root* has that line, a
user who runs Ply at a root rather than in a member gets the old footgun back.

**In a multi-crate workspace, Ply writes into a directory the conventional ignore rule does
not cover.** The harness goes to `crates/<member>/target/ply/…`, but Cargo puts build output
at the workspace *root's* `target/`, so a member's `target/` never normally exists — and the
`.gitignore` `cargo new` writes is `/target`, anchored at the root. I ran the experiment: in
a git repository with that exact ignore file, after one Ply run, `git add -A` staged **475
files, 187 MB** of build artifacts. Ply creates the directory itself and puts nothing in it
that would stop this — no cache tag, no ignore file of its own. One `.gitignore` containing
`*`, written beside the harness, would close it.

**Running at a virtual manifest root** (a workspace root with no source of its own) is now
an honest refusal — "could not find the function this claim anchors to … in `./src/lib.rs`"
— rather than a stack trace. It is still a refusal, so there is no way to check a whole
workspace in one command; the working pattern is one run per member crate.

**Mutation testing** on a crate with no workspace of its own is refused by name, in a
paragraph that explains what the layout cannot support and why Ply will not add a workspace
line automatically, and the run does not pass. That is the right shape of answer. One
detail: the tree above it still prints a green `fuzzed(64)` at the root while the run exits
non-zero, so a reader skimming the tree alone sees green.

---

## 7. Is three operations enough, and is it theatre?

Three is honest and exactly three — I measured it. A promise that is false after three
state-changing calls is caught; the same promise moved one call further out is not, and
the sentence beside the green verdict says a bug needing more calls is outside what ran.

But three is not the constraint that bites. The last review found the pool could never
contain a state-changing call at all; that is fixed, and I confirmed both flavours it named
now come back red — a `&mut self` mutator is pooled, and so is a `&self` mutator whose
argument list differs from the checked method's. What replaced it is finding 1: the pool
silently loses any operation whose argument Ply cannot build. So for a real type the
question "is three enough" is premature. The question is whether the type's mutators are
in the pool at all, and for anything that takes a string, a slice, or one of the user's own
types, they are not — with no way to tell from the output.

Two further limits I did not see stated anywhere: the operations are found only in the file
where the type is declared (the same single-file scan that causes finding 2), and an
operation's own arguments are drawn without regard to that operation's precondition.

---

## 8. Where I went looking and found nothing wrong

Stated plainly because it is a lot, and because every item here was either a promise
written to be **false** or a measurement — a passing check on a true promise proves
nothing, and that error has hidden three of these bugs.

- **Struct and enum parameters work, on every shape I could assemble from supported
  parts.** A struct holding a `String`; a struct holding a `Vec<u32>`; an enum carrying
  data in some variants and not others; a chain of three user types nested through
  constructors; a struct parameter and an object to call the method on in the same
  signature; a precondition that reads a struct parameter's field; a struct with twelve
  public fields. Every false promise was caught, and the by-hand read of the generated code
  matched what was reported.
- **Every enum variant is reachable.** I wrote one promise false only for the first variant
  and one false only for the last, in enums of four and three variants. Both caught.
- **A constructor's precondition is honoured for a parameter**, not only for an object. I
  read the generated harness: the precondition is emitted as a rejection filter before the
  constructor call, and a promise that is only true because of that precondition passes
  cleanly rather than crashing.
- **The twelfth false clean is really fixed**, both flavours, verified with the last
  review's own two crates rebuilt from its description.
- **A check that ran nothing is a tool error, per check.** The worked-examples check on a
  function taking a struct generates no cases, and says so; declaring it *alongside* a
  sampling check no longer lets the passing one hide it — both are reported.
- **A near-impossible constructor precondition does not launder into a pass.** 1025 of 1025
  draws rejected, verdict "no evidence at all", exit 1.
- **The refusals that are supposed to be by name are by name**, and they read well: a type
  with no usable constructor and private fields; a tuple struct; a `#[non_exhaustive]`
  enum; an enum with a tuple variant; `Option<Point>` and `Vec<Point>`.
- **The cheap command no longer denies what the real one can do** — on a crate whose
  functions take structs, enums and constructed objects it reports no problems, and the
  real run then checks all three.
- **Ply writes nothing into the user's source tree on a clean run** other than the record
  file: I listed the crate's files after every run.

---

## 9. Smaller items

None of these change what a verdict means; all of them would be noticed on a first day.

- **A promise on a trait method breaks the user's build with no explanation.** Putting
  `#[ply::ensures]` on a trait's method declaration — the normal shape, no body — is a hard
  compile error, `expected curly braces`, pointing into the user's own file. Ply has a
  perfectly good sentence for this situation ("Ply checks inherent methods and free
  functions, not trait methods, yet") but you cannot reach it, because the crate no longer
  compiles. This is how I discovered it: it broke the rate limiter when I put promises on
  everything.
- **A rejection message names the wrong precondition.** When a *constructor's* precondition
  throws draws away, the report says they "were thrown away by the function's own
  `#[ply::requires]` precondition" — of a function that has none. The reader goes looking
  for something that is not there.
- **A disclosure says "by value" for a reference.** `fn f(p: &Point)` earns the assumption
  notice worded "`f` takes `Point` by value". Small, but this is the sentence whose job is
  to be exact about what was assumed.
- **Ply reports its own generated crate to the user as an architectural unknown**: "1 of 2
  crates in this workspace belong to a declared component. Not declared, and so invisible
  even to a wildcard deny rule: flowgate_ply_harness."
- **The witness for a struct parameter is raw internal machinery.** `w = __ply_leaf_p_w_start_start=8, __ply_leaf_p_w_end_end=0`.
  The accompanying sentence is honest that Ply cannot write it as Rust, but the two names
  in it exist nowhere in the user's program.

---

## Reproductions

All under `/tmp/rev5/` (build output deleted; sources retained), all against
`/home/user/ply/target/debug/cargo-ply` as built at the branch head. Each scratch crate
depends on the attribute crate by absolute path. Crates marked *plain* carry no
`[workspace]` line — the ordinary layout.

| # | what | where | result |
|---|---|---|---|
| 1 | mutator takes a struct | `/tmp/rev5/opstruct` | `fuzzed`, exit 0; real value 5 against a promise of 0 |
| 1b | mutators taking `&str`, `&[u32]`, `Option<u32>` | `/tmp/rev5/opstruct` | first two green, third caught |
| 2 | result-returning constructor + public fields | `/tmp/rev5/resultctor` | violation on correct code |
| 2b | constructor in a different file from the type | `/tmp/rev5/splitimpl` | violation on correct code, witness `start=8, end=0` |
| 3 | `pub(crate)` type with public fields | `/tmp/rev5/seamcompile` | both functions tool_error, real bug unreported |
| 3b | private constructor, parameter path | (rebuilt, deleted) | whole crate tool_error; field-filling was available |
| 3c | private constructor, object path | (rebuilt, deleted) | same |
| 3d | 13 public fields (12 is fine) | (rebuilt, deleted) | harness will not compile |
| 3e | `#[non_exhaustive]` on a variant | (rebuilt, deleted) | harness will not compile |
| 4 | one struct parameter, body panics | `/tmp/rev5/impossible` | tool_error, witness discarded; harness prints it by hand |
| 4b | plain two-argument panic, baseline | `/tmp/rev5/crashwit` | violation with witness, correct |
| 4c | one-field struct, witness label | `/tmp/rev5/onefield` | `k = 0` for a parameter of type `Divisor` |
| 4d | zero-value parameter beside a two-value one | `/tmp/rev5/mislabel` | nesting saves it; `l = ()`, `p = (0,0)` |
| 5 | rate limiter, promise and check on all 39 | (copied, deleted) | 0 checked; table in section 5 |
| 5b | `semver` 1.0.28, promise on all 13 | (copied, deleted) | 1 checked |
| 5c | `String` vs `&str` | `/tmp/rev5/strtest` | owned checked, borrowed refused |
| 5d | `&u32`, `&[u32]`, `&Point`, `Option<Point>`, `Vec<Point>` | `/tmp/rev5/refs` | scalar and struct references work; the rest refused |
| 6 | ordinary crate end to end | `/tmp/rev5/sfield` | three false promises caught; manifest untouched |
| 6b | two-crate workspace, run in the member | (rebuilt, deleted) | bug found; root build intact; nothing written |
| 6c | second run | (rebuilt, deleted) | reused in 0.026s |
| 6d | `cargo clean`, plain crate | (rebuilt, deleted) | build fine; Ply re-checks |
| 6e | `cargo clean`, crate with a workspace line | (rebuilt, deleted) | `cargo build` fails |
| 6f | `git add -A` after a run in a workspace member | (rebuilt, deleted) | 475 files, 187 MB staged |
| 6g | `.cargo/config.toml` redirecting the build dir | (rebuilt, deleted) | works |
| 6h | run at a virtual manifest root | (rebuilt, deleted) | honest refusal, no stack trace |
| 6i | mutation testing on a plain crate | (rebuilt, deleted) | refused by name, run does not pass |
| 7 | receiver bound measured at 3 and 4 calls | `/tmp/rev5/bound3` | 3 caught, 4 missed and said so |
| 8 | struct with `String`, struct with `Vec`, enum with data | `/tmp/rev5/sfield` | all three caught |
| 8b | three-deep nesting; object + struct parameter; precondition on a field | `/tmp/rev5/combo` | all three caught |
| 8c | constructor precondition on a parameter | `/tmp/rev5/ctorparam` | honoured; harness read by hand |
| 8d | first and last enum variant reachable | (rebuilt, deleted) | both caught |
| 8e | worked-examples check alone and beside a sampling check | `/tmp/rev5/testcheck` | tool error in both, per check |
| 8f | near-impossible constructor precondition | (rebuilt, deleted) | no evidence, exit 1 |
| 8g | cheap command on structs, enums, objects | `/tmp/rev5/checkcmp` | no false denial |
| 9 | promise on a trait method declaration | (rebuilt, deleted) | user's crate stops compiling |
