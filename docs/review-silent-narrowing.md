# Adversarial review: is silent narrowing one disease, and is a structural invariant the cure?

*Read-only review, 2026-08-28. No product code was written or changed; `cargo test
--workspace` was never run. Every claim about behaviour below was settled by running the
already-built `target/debug/cargo-ply` (built 03:15, containing the just-landed "excluded
operations named honestly" work and the tree-mark change that landed alongside it while
this review was running) against four throwaway crates in `/tmp/rev6/` and one rebuilt
from the previous review's `/tmp/rev5/`. Commands are at the end. Scratch crates deleted
afterwards.*

---

## TLDR

**The proposal is wrong in the form you stated it, and it is wrong for a reason you can
measure in twenty minutes: the dominant failure is not "Ply excluded something and forgot
to say so", it is "Ply never looked, so there was nothing to say".** A rule that nothing
may be excluded without appearing in the verdict can only bind exclusions the code
noticed. I found three more green passes on false promises this afternoon, all in the
subsystem that was patched this morning, all of them exclusions no ledger would ever have
held: the type's only mutating method lives in a second file; the type's only mutating
method is a trait method; the type has a second constructor Ply never calls. All three
print the sentence the fourteenth's fix was written to stop being false — *"every value
this run saw was reachable by calling the type's own code, nothing else, so nothing here
was assumed"* — and all three exit 0.

**The diagnosis is mostly right and needs one correction that changes what to build.** The
fourteen are not one shape, they are four, and only about nine of them are silent scope
reduction. Two of the others are a different disease with a different cure: one is
"evidence about a different function than the claim names" (nothing was narrowed — the
wrong thing was measured), and one is "evidence outliving the thing that justified it"
(the verdict was not assembled at all; it was replayed from a record). Building the
exclusions invariant would leave both untouched, and by your own list they are among the
worst two of the fourteen.

**What I would build instead is the same idea with the arrow reversed, and this repo
already contains the pattern twice.** Do not ask each check to list what it left out —
open-world, self-reported, unfalsifiable. Ask each check to state positively what it
covered, make *incomplete* the default that costs nothing to reach, and let completeness
be earned only by comparison against an inventory computed independently of the code that
does the narrowing. That is exactly how the reuse hash already works (it either bounds the
walk or declares it could not and hashes the whole crate, and it says which), and exactly
what the cheap command already prints under "What this command did NOT check". The
proposal is that mechanism, moved from the command down to the individual result — not a
new idea, an existing one finished.

**One thing found while running that outranks the design question and is not on any list:
this morning's fix did not reach the crate it was written for.** Re-running the previous
review's own four-type crate with today's binary reproduces the fourteenth verbatim — the
old, false "nothing here was assumed" sentence, with a green verdict — because the stored
result still matched and was carried forward, diagnostics and all. Ply's own version is
one of the inputs that decides that, and it is a constant that has never moved. Delete the
stored file and the same binary reports the exclusion correctly. So today, fixing a
false-clean bug does not fix it for anyone who has already run Ply once.

---

## 1. Is the diagnosis right?

Mostly, and the exceptions are load-bearing. Sorting your own eight named examples, plus
the ones the four reviews and the running-state list name around them, by *mechanism*
rather than by symptom gives four groups, not one.

### Group A — a success criterion satisfied by an empty set ("nothing failed")

- a test filter matched no tests, and "no failing test" read as success
- a method with a receiver generated no tests on one check path, and that read as success
- a declared check that ran nothing sat beside one that passed, and the passing one won
- the mutation check found no mutants to run, and "none survived" read as a strong spec
- the failing-test parser could not find what it was looking for and reported zero failures

These share the shape "a positive conclusion drawn from an absence". Nothing was
*narrowed* in the sense of a domain being shrunk — the domain was empty, or the observation
channel was blind. The cure that actually shipped for these is the right one and is not an
exclusions ledger: a pass must now prove that at least one case executed. That is a
*positive-evidence* rule, and it generalises cleanly.

### Group B — a check that really ran, over a smaller world than the sentence beside it claims

- an operation whose argument could not be built was dropped from a receiver's history
- a receiver sequence could never call a mutating operation, so only the trivial history ran
- generated text never contains control characters, and nothing said so
- generated numbers never include the two special values, and something does say so
- a receiver history is at most three calls deep, and something does say so
- a generic function is checked at exactly one concrete type, and the verdict says which
- (read, not run) a mutation run counts a mutant whose tests ran out of time as neither
  caught nor survived, so a function can earn the strongest spec-quality mark the tool
  prints while some mutants were never actually killed; the same is true of mutants that
  did not compile. Both are defensible conventions borrowed from the engine, and neither
  appears anywhere in the result

This is your class, exactly as you described it, and it is real. Note that the last four
are the *same* mechanism already handled well — which is the strongest argument for your
instinct: when Ply discloses a narrowing, it does it well, and the failures are the ones
where it didn't think to.

### Group C — the wrong thing was measured

- an anchor resolved to a different function than the one the contract described
- a test filter matched by substring, so one function was blamed for another's failures

Nothing was excluded here. Evidence was gathered, in full, about an item other than the
one named in the verdict. An exclusions field would be empty and correct on both, and both
would still ship. The invariant these need is an identity round-trip: the item a result is
reported against must be the same item the harness called, checked by construction rather
than by two code paths agreeing. That fix is already how the anchor one was closed, and it
is a different fix from yours.

### Group D — evidence that outlived what justified it

- the reuse fingerprint omitted the helper code a check actually ran
- (new, measured today) a stored result replays a superseded diagnostic verbatim

Here no verdict is assembled at all: one is replayed. Your invariant lives on the assembly
path and cannot see this. The *principle* — the boundary of what a result covers is part of
the result — does generalise here, and in fact this is where this repo already implemented
it best. The mechanism does not.

### Verdict on the diagnosis

Nine of the fourteen are Group B or the narrow half of Group A. Calling all fourteen one
disease is a compression that would produce the wrong mechanism, because it would have you
build a ledger on the assembly path — which is exactly where Groups C and D are not.

What *does* unify all four, and is worth stating as the principle rather than the
mechanism: **Ply reports evidence as a strength and never as a domain.** `fuzzed(256)`
says how hard it looked. It has never said where. Every one of the fourteen is a place
where the reader supplied the missing "over what" and supplied it too generously. That is
one sentence, it covers all four groups, and it does not commit you to a ledger.

---

## 2. Is the proposal the right shape of fix?

No — but the target is right and the failure is in the lever, so this is a redesign rather
than a rejection.

### Why a self-reported exclusion list cannot be made structural

The single scan that builds a receiver has, in about ninety lines, seven places where a
candidate is passed over. As of this morning exactly one of them records what it dropped —
the one the last review found. The other six are ordinary `continue`s:

- the block is a trait implementation (skipped entirely)
- the block is generic (skipped entirely)
- the block is in a different file from the type's declaration (never read at all)
- a constructor-shaped function whose return type isn't recognised (passed over)
- a function whose parameter pattern the reader can't model (passed over)
- every constructor after the first usable one (never considered again)

A type system cannot help here. Making the verdict unconstructible without an exclusions
field forces the author to pass *something* — and the honest value at every one of those
six sites is an empty list, because the code genuinely does not know it excluded anything.
That is not a hypothetical. It is what the code does today, and the three new false cleans
below are what it costs.

The deeper problem is that "what was excluded" is an open-world question. You cannot
enumerate the complement of a set you never enumerated. An invariant test that walks every
check path and fails on an unrepresented exclusion can only walk the exclusions that got
represented; it is a test that the ledger is internally consistent, not that it is
complete.

### What would work, and it already exists here twice

Reverse the arrow. Instead of *"list what you left out"*, require *"state what you
covered, and how you know it was everything"*, with two rules:

1. **Partial is the default and is free to reach.** A path that says nothing about its
   coverage is partial. Completeness is an assertion someone has to make, and it is the
   assertion that has to be justified.
2. **Completeness is earned by difference against an inventory computed elsewhere.** For a
   receiver: enumerate every inherent implementation block for the type across the crate
   (the item index that resolves anchors already walks every file), and any operation in
   that inventory not in the pool is an exclusion — computed, not remembered. If any block
   is generic, behind a trait, or macro-produced, completeness cannot be asserted and the
   result is partial with that reason. For the test filter: the harness generator knows how
   many tests it wrote for this function; anything less than all of them executed is a
   narrowing, computed by subtraction.

This is precisely the shape of the reuse walk, which is the best-behaved subsystem in the
tool for exactly this reason: it either bounds what a result stands on and names the whole
set, or it declares it could not bound the walk and hashes everything, and *which of the
two happened is itself part of the result*. It is also the shape of what the cheap command
already prints — "What this command did NOT check" — which is your proposal, shipped, at
the granularity of a whole command. The work is to bring it down to the granularity of one
result.

Two consequences worth accepting deliberately:

- Under this rule the receiver path is **partial on every crate that has a trait
  implementation or splits a type across files**, until the scan is widened. That is
  blunt, and it is also true. The alternative, which is where you are today, is a green
  verdict.
- It converts "did we remember?" into "can we prove?", which is the only version of this
  that a test can hold.

### On the alternative you named — better per-case discipline instead of a mechanism

I do not think this survives the evidence, for two reasons rather than one. The first is
the count: fourteen instances, plus three more I found this afternoon in the subsystem
that was corrected this morning. The second is stronger and is new: **a per-case fix does
not currently reach a user who has already run Ply.** The stored-result path replayed the
superseded sentence verbatim (section 6). Discipline that cannot be delivered is not
discipline.

---

## 3. What would it cost?

Larger than a field, smaller than a rewrite, and one part of it is genuinely a single
large change.

**There is no verdict type to make unconstructible.** A verdict is a `String` and a status
is a `Vec<String>`, compared with `starts_with` and ranked by a function over string
prefixes. There are about thirty places that construct one and about thirty that build a
result node. So the "unconstructible without an exclusions field" option begins with
introducing a typed verdict into the product, which does not exist there today — the typed
one lives in the standalone kernel and is deliberately not linked in. That is a real piece
of work, it is mechanical, and it must preserve the serialised form exactly because the
committed envelopes are the contract.

**The result envelope's own stability rule permits this.** Additive fields only, which a
per-node coverage claim is. So the envelope, the human tree and the machine output can gain
it incrementally, one check path at a time — and the human tree has already grown the mark
for the first one.

**What has to change, in order of increasing pain:**

1. A per-node coverage claim in the envelope, and a printer for it. Additive, small.
2. The receiver scan learns to compute its inventory crate-wide instead of per-file. This
   is also the fix for two of the three new false cleans, so it pays twice.
3. The other check paths each make their claim: the shared test harness (how many of the
   tests it generated for this function actually executed — the counter already exists),
   the prover path (which callees were descended into, which were replaced by a promise,
   which were left alone because their source is not readable — this last one is a
   documented gap that today appears in the specification and not in any run's output),
   and the mutation path.
4. The stored-result format carries the claim, and — this is the part that cannot be
   incremental — **every stored result written before the claim existed must stop being
   reusable.** The mechanism for that exists and is inert, because the version it keys on
   is a constant that has never moved (section 6). Deciding that is a prerequisite, not a
   follow-up.
5. Flipping the default from "complete unless recorded otherwise" to "partial unless
   proven" is one commit that turns a large number of currently-green fixtures partial at
   once, and every committed envelope changes in the same diff. There is no way to stage
   that honestly: staging it means shipping a period in which some paths claim
   completeness they have not earned, which is the disease.

Plainly: steps 1–3 are incremental and could start today. Steps 4 and 5 are each a single
large change, and step 5 is the one that makes the whole thing mean anything.

---

## 4. Would it actually have caught the fourteen?

Taking the proposal literally — a ledger of exclusions the code records as it makes them —
against your own eight named examples, plus the four the reviews describe around them.
"Caught" means the run would have been prevented from reading as a clean pass.

| # | the bug | ledger as stated | coverage-by-difference |
|---|---|---|---|
| 1 | test filter matched no tests | **no** — nothing was recorded as excluded; the selection was simply empty | **yes** — the harness knew how many tests it wrote; zero of them ran |
| 2 | method generated no tests on one check path | **no** — same; no exclusion was made, a generator produced nothing | **yes** — same subtraction |
| 3 | operation with an unbuildable argument dropped | **yes** — this is the one case the ledger was designed from | yes |
| 4 | receiver sequence could never call a mutator | **yes** — the shape filter did exclude, and would have had to say what | yes |
| 5 | mutation fell back and reported every mutant survived / none to run | **no** — an empty mutant set is not an exclusion | **yes** — zero of the mutants the engine listed were exercised |
| 6 | anchor resolved to a different function | **no** — nothing was excluded; the wrong item was measured | **no** — a coverage claim about the wrong function is still complete |
| 7 | reuse fingerprint omitted the helper code a check ran | **no** — no verdict was assembled; a stored one was replayed | **partly** — only because this class is where the pattern was invented; the mechanism needs the claim to be part of the stored result and checked on replay |
| 8 | proof refused, reported as a pass on another path | **yes**, if a declared check that produced no verdict counts as an exclusion | yes |
| 9 | text generation excludes control characters, undisclosed | **yes** — the generator knows what it excludes | yes |
| 10 | float generation excludes two values | already disclosed | already disclosed |
| 11 | receiver history bounded at three | already disclosed | already disclosed |
| 12 | one generic instantiation | already disclosed in the verdict's own name | already disclosed |
| 13 | correct function blamed for another's failing tests (substring filter) | **no** — nothing excluded | **no** — a superset was measured, not a subset |
| 14 | (new, section 5) mutator in another file / behind a trait / a second constructor | **no** — never seen, so never excluded | **yes** for the first two, **no** for the third without a separate rule about constructors |

Honest score for the proposal as stated: **three of the eight you named**, four if the
per-check reading of item 8 is granted. Coverage-by-difference scores **six of eight**, and
covers three of the four new ones. Neither touches the two in Group C, which need the
identity invariant instead.

An invariant that catches three of its own eight motivating examples is not a bad
invariant; it is the wrong invariant for six of them. That is the number I would want
before committing to it.

---

## 5. Is there a fifteenth? Yes — three, and they are in the file that was patched this morning

I went to the receiver scan because that is where the fourteenth lived and because its fix
records exclusions only at the single point where the code *knows* it is excluding. Every
other place it passes something over is invisible to that record. Three of those are
reachable with completely ordinary Rust.

**(a) The type's only mutator lives in a different file.** The scan reads one module file.

```rust
// src/till.rs
pub struct Till { pub(crate) total: u32 }
impl Till {
    pub fn new() -> Self { Till { total: 0 } }
    #[ply::ensures(|result| *result == 0)]        // FALSE after one `take`
    pub fn total(&self) -> u32 { self.total }
}

// src/more.rs
impl Till {
    pub fn take(&mut self, cents: u32) -> u32 {   // a plain integer argument
        self.total = self.total.saturating_add(cents); self.total
    }
}
```

```
workspace — fuzzed(128)
  otherfile — fuzzed(128)
    till::Till::total — fuzzed(128)
EXIT=0
```

and the disclosure beside it: *"repeating `till::Till::total` itself. Every value this run
saw was reachable by calling `Till`'s own code, nothing else, so nothing here was
assumed."* I ran the real program to be sure the promise is false rather than merely
suspicious: `Till::new(); take(5); total()` prints **5** against a promise of "always 0".
No exclusion is recorded, because nothing was excluded — the file was never opened. The
argument type is one Ply builds perfectly well.

**(b) The type's only mutator is a trait method.** Same file, plain integer argument, and
the scan skips trait implementations outright.

```rust
pub trait Fill { fn take(&mut self, cents: u32) -> u32; }
impl Fill for TraitTill { fn take(&mut self, cents: u32) -> u32 { … } }
```

`fuzzed(128)`, exit 0, same "nothing here was assumed" sentence. Real program: **5**
against a promise of "always 0".

**(c) The type has a second constructor and Ply calls only the first.**

```rust
impl TwoCtor {
    pub fn new() -> Self { TwoCtor { n: 0 } }
    pub fn preloaded(n: u32) -> Self { TwoCtor { n } }
    #[ply::ensures(|result| *result == 0)]        // FALSE for anything built by `preloaded`
    pub fn value(&self) -> u32 { self.n }
}
```

`fuzzed(128)`, exit 0, and the same sentence — which is false in a second way here: the
values reachable by calling this type's own code include everything `preloaded` can make,
and none of them were built. Real program: `TwoCtor::preloaded(7).value()` is **7**.

All three are the fourteenth's exact shape — a state the checked promise is false in,
unreachable by construction, under a verdict that says the opposite. None of them would be
caught by a ledger of what the scan decided to leave out, because in all three the scan
made no decision.

**Where I would look next, having not looked.** Two places, both in different subsystems
from the receiver scan, both read rather than run:

- **The prover path's treatment of a callee whose source Ply cannot read.** The
  specification records that such a call is left alone, so a proof verdict can include a
  body Ply never examined. That is written in the specification and, as far as I can tell
  from reading, is not attached to the verdict the way an assumed promise is. It is the
  shape that would matter most, because it sits under the strongest word the tool prints.
- **The mutation check's arithmetic.** The rule that decides whether a function's promise
  is strong enough asks only that nothing survived and that at least one mutant was caught.
  Mutants whose tests ran out of time, and mutants that failed to compile, are counted in
  neither column. So the mark can be earned over a strict subset of the mutants the engine
  produced, with the subset's size appearing nowhere. The zero-mutant case has already been
  closed — the guard is there and has a test — which is precisely the pattern: the
  narrowing to *empty* was fixed, the narrowing to *some* was not.

---

## 6. What it costs the user — and the thing that outranks it

### The disclosure is already too long, and it currently buys nothing

The receiver disclosure runs **193 words per method per run**. The previous review's
four-type crate prints four of them, one after another, each repeating the same explanation
of what a receiver is. A crate with forty methods prints forty.

And measured today: it does not change the verdict, the tree's own colour, or the exit
code. The run below reports the exclusion honestly *and* exits 0:

```
workspace — fuzzed(128)  [narrower than it looks]
  onlytill — fuzzed(128)  [narrower than it looks]
    Till::total — fuzzed(128)  [narrower than it looks]
EXIT=0
```

That is the worst of both: the cost of completeness is paid in words, and the benefit —
a run that does not read as success — is not collected. Continuous integration stays green
on a promise that is false in three lines of ordinary use.

### How I would resolve the tension

1. **The verdict line carries the fact; the paragraph carries the detail; the machine
   output carries the ledger.** One mark on the line (this already exists and reads well),
   one gloss printed once per run (also already there), and the per-item list in the
   structured output only. A person reading the tree needs to know *that* the result is
   narrower, not *which seven operations* — the second is what the machine surface is for.
2. **Rank the exclusions and print only the top ones in prose.** An operation that can
   change the object is news; a read-only one is not. "One mutating operation was never
   called: `Till::take`" plus "and 4 read-only operations" is one line and carries the
   entire message.
3. **Lead with the news.** The current paragraph puts the exclusion in the middle, between
   a description of how receivers are built and a bound about the fifth call, and closes on
   a sentence inviting the reader to worry about the wrong thing. Whatever survives should
   open with what was not covered.
4. **Make it gateable, and pick a defensible default.** `--fail-on=warn` already exits 1 on
   these runs today — I checked. But that level also fails on everything else advisory, so
   nobody will use it for this. My recommendation, offered as a recommendation because it
   is a specification decision and not an implementation one: **an excluded operation that
   can change the object is an absence of evidence for a promise about that object, and
   should fail the run by default**; an excluded read-only operation is a warning. That
   keeps the default honest without turning every narrowing into a broken build.

### The thing that outranks all of it

Running the previous review's own crate with today's binary, without deleting its stored
results, reproduces the fourteenth in full:

```
    Log::count — fuzzed(128)  [reused]
    Till::total — fuzzed(128)  [reused]
…
Every value this run saw was reachable by calling `Log`'s own code, nothing else,
so nothing here was assumed.
```

That is the superseded sentence, replayed word for word by the build that fixed it, with a
green verdict, because the stored result still matched and carries its own diagnostics.
Delete the stored file and the same binary prints the correct disclosure and marks the tree.

Ply's own version is one of the twelve inputs that decide whether a stored result may be
carried forward, and it is the crate version string, which has been `0.1.0` for every
commit on this branch. So the mechanism designed to make a fixed defect invalidate old
answers is present, wired, and inert. The running-state list carries this as an open
question ("whether every Ply release should invalidate every record"); on today's evidence
it is not an open question, it is the reason a false-clean fix does not ship. **Whatever is
decided about the invariant, this should be decided first** — otherwise every future
narrowing fix has the same fate: correct in the source, absent from the answer any existing
user gets.

---

## 7. What I would do, in order

1. Move the version that invalidates stored results, and decide the rule for moving it.
   Without this, nothing below reaches a user who has run Ply once.
2. Keep the positive-evidence rule for the empty-set group and state it once, as a rule
   rather than as four fixes: no result stronger than "nothing was claimed" may exist
   without a count of what actually executed. Enforce it where results are built, not per
   check path.
3. Do **not** build a ledger of self-reported exclusions. Build the coverage claim: partial
   by default, complete only by difference against an independently computed inventory,
   carried on the result, checked on replay. Start with the receiver scan, because that is
   where three known bugs live and where the inventory is cheap and exact.
4. Widen the receiver scan to the whole crate while you are in there. It closes two of the
   three new false cleans outright rather than merely disclosing them.
5. Leave the two identity bugs to their own invariant — the item a result names must be the
   item the harness called, established once rather than by two paths agreeing. It is a
   different fix and it should not be smuggled into this one.

One note on what is being built right now: the status being added for this — a name
specific to receiver histories — is the fourteenth patch generalised by exactly one step.
If a mechanism is going in, it should be the general one, and the name should be about
coverage rather than about receivers, or the fifteenth will need a fifteenth name.

---

## Reproductions

All under `/tmp/rev6/` (deleted after this review; sources reproduced in section 5 in
full), against `/home/user/ply/target/debug/cargo-ply` as built at 03:15 on 2026-08-28.
Each scratch crate is an ordinary `cargo new --lib` layout depending on the attribute crate
by absolute path, with a one-claim document asking for sampling.

| # | what | where | result |
|---|---|---|---|
| a | mutator in a second file, integer argument | `/tmp/rev6/otherfile` | `fuzzed(128)`, exit 0, "nothing here was assumed"; real program prints 5 against a promise of 0 |
| b | mutator is a trait method, integer argument | `/tmp/rev6/moregaps` | `fuzzed(128)`, exit 0, same sentence; real program prints 5 |
| c | a second constructor never called | `/tmp/rev6/moregaps` | `fuzzed(128)`, exit 0, same sentence; real program prints 7 |
| d | the fourteenth, disclosed, still green | `/tmp/rev6/onlytill` | `fuzzed(128)` + mark + 193-word disclosure, **exit 0** |
| e | same, with the stricter gate | `/tmp/rev6/onlytill --fail-on=warn` | exit 1 |
| f | the fourteenth replayed from a stored result | `/tmp/rev5/opstruct` with its existing stored file | old sentence verbatim, green, marked reused |
| g | same crate, stored file deleted | `/tmp/rev5/opstruct` | correct disclosure, tree marked, exit 1 (from an unrelated real violation in the same crate) |
| h | the cheap command on (a) | `/tmp/rev6/otherfile` | "No problems found in the document", exit 0 |

Code read rather than run: the receiver scan (its seven pass-over sites), the verdict
combination and ranking functions, the absence vocabulary and the exit-code rule, the
stored-result fingerprint inputs, and the reuse walk's two-mode design, which is the
pattern section 2 recommends copying.
