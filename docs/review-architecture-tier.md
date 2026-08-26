# Adversarial review: the architecture crate tier (2026-08-26)

**No — this does not merge as it stands, and the single thing that must change is that a
run where the architecture check did not happen must stop exiting 0.** Break any manifest
in the workspace, remove cargo from the path, or introduce the exact dependency cycle the
new `ply.yaml` says the checker exists to catch, and Ply prints "No problems found",
buries the words "NOT CHECKED" in a coverage paragraph, and exits 0. There is no
diagnostic, no status on any node, and no test anywhere in the repository covering that
path — the branch that turns a failed check into a green run is the one branch nobody
watched fail.

Two more findings are serious enough to name in the same breath. A component whose anchor
names a crate that does not exist owns nothing and is never reported, so renaming a crate
turns the architecture description into fiction while the run stays green — the same
failure the tool refuses to allow for a renamed *function*. And crates in the workspace
that no component claims are invisible: Ply's own workspace has one today, and even a
`deny` rule written as broadly as `* -> core` does not reach it.

The good news, and it is real: the classification logic itself is correct everywhere I
could reach it. Containment, declared edges, bans that override an edge, bans with
exceptions, the library-versus-binary identity fix, and target-gated dependencies all
behave the way the spec says, and the invariant test that was found tautological last
round genuinely goes red now when containment is broken. What is wrong is not the rule.
It is everything the rule is never shown.

---

## Findings, worst first

### 1. When the dependency graph cannot be read, the run passes anyway

The architecture check only runs if `cargo metadata` succeeds. When it does not, the
result is recorded as a sentence inside the coverage report and nothing else: no
diagnostic, no status on any node in the machine-readable output, and no effect on the
exit code. The command prints "No problems found in the document." and exits 0.

This is not a rare path. Three ordinary reproductions:

**(a) The exact violation the new `ply.yaml` was written to catch.** Its comment says the
library "must never reach up into the command-line layer" and names "the cycle a
well-meaning refactor would create". Because the command-line layer already depends on the
library, any such refactor is a package cycle — and cargo refuses to produce a graph for a
package cycle. Reproduced on a faithful copy of this workspace's shape (same crate names,
same binary-only top crate, this repository's own `ply.yaml` verbatim; the baseline copy
reports the same "1 real crate dependencies … 1 permitted" line the real repo does):

```
$ cargo-ply check /tmp/plyrev/plymirror
EXIT CODE = 0
cargo ply check — /tmp/plyrev/plymirror/ply.yaml

  schema        The document against schema/ply.schema.json, then every rule that can be
                settled from the document alone.
  anchors       0 of 0 fn claims in this crate point at a function Ply can find.

  No problems found in the document.

What this command did NOT check:
  architecture  NOT CHECKED. Ply could not get this crate's real dependency graph, so
                neither the crate-level nor the item-level part of the architecture check
                ran: `cargo metadata` failed in /tmp/plyrev/plymirror: error: cyclic package
                dependency: package `ply-cli v0.1.0 (/tmp/plyrev/plymirror/crates/ply-cli)`
                depends on itself. ...
```

**(b) A typo in any manifest in the workspace.** A bad version string in one crate:

```
$ cargo-ply check /tmp/plyrev/ws7
EXIT=0
  ...
  No problems found in the document.

What this command did NOT check:
  architecture  NOT CHECKED. ... failed to parse the version requirement `!!!` for
                dependency `serde` ...
```

**(c) Cargo not on the path** (a container without a toolchain, a CI step ordering
mistake):

```
$ env PATH=/usr/bin:/bin cargo-ply check /tmp/plyrev/ws1
EXIT=0
  ...
  architecture  NOT CHECKED. ... could not run `cargo metadata` in /tmp/plyrev/ws1:
                No such file or directory (os error 2)
```

In the machine-readable output there is nothing an agent can key on except prose:
`"diagnostics": []`, every node's status list empty, and the failure recorded only as
free text inside a coverage entry. That is precisely the shape the mission section warns
about after the last time this happened — a missing engine recorded in a field the
pass/fail rule did not read. The rule was restated over *names* so the next absence
recorded in a new field would be caught by vocabulary rather than by a special case. This
absence is recorded in a new field, and it is not caught.

Nothing in the repository tests this path. Searching the whole tree, `Unavailable` appears
four times and all four are production code.

**What must change:** a run in which this tier could not look must not exit 0. Either the
failure becomes an error-severity finding, or the exit code learns about
`coverage.not_checked`, or both.

---

### 2. An anchor that names a crate which does not exist is silently ignored

Component ownership is decided by matching an anchor's first path segment against a crate
identity. A miss is not a finding — the entry simply never appears in the ownership map,
and every rule mentioning that component quietly stops applying.

Reproduced as a crate rename, which is the way this happens in practice. Same mirror of
this workspace, this repository's own `ply.yaml` unchanged, `ply-core` renamed to
`ply-kernel` and the command-line crate's dependency updated with it:

```
$ cargo-ply check /tmp/plyrev/plymirror
EXIT=0
  architecture  No crate here depends on another crate that belongs to a different declared
                component, so there was nothing to check.

  No problems found in the document.
```

Ply's own architecture description has just become fiction, the one dependency it used to
check is gone, and the tool reports a clean run in a slightly *more* reassuring sentence
than before. Contrast the rule the same command already applies to function claims: a
renamed function must break CI, not silently orphan its claims. Component anchors get no
such rule. Note also that this is not the tier's own code — it is the document tier's
missing check — but this tier is the first consumer that turns it into a false statement
about real code.

Three siblings of the same defect, each reproduced:

**Two components anchored at the same crate.** The first in document order wins; the
second owns nothing, and any ban attached to it never fires:

```yaml
components:
  a_public:   { anchor: ws6_a }
  a_internal: { anchor: ws6_a }
  b:          { anchor: ws6_b }
  c:          { anchor: ws6_c }
edges: ["b -> a_public", "c -> a_public"]
deny:  ["c -> a_internal"]
```
```
EXIT=0
  architecture  2 real crate dependencies cross ... 2 permitted ... 0 not permitted
  No problems found in the document.
```

`c` really does depend on the crate the user is trying to fence off. The ban is inert and
nothing says so.

**A component anchored at a module rather than a crate.** The anchor's first segment is
taken and the rest discarded, so a module-anchored component claims the *whole* crate if
it is declared first — and owns nothing if it is declared second. Declared first, the
crate-anchored component `a` goes dead and every finding names a module as though it were
the owner, advising the user to write an edge that would encode something false:

```
  A0401 crate `ws6_b` depends on crate `ws6_a`. ... `ws6_a` belongs to `a_inner` ...
        Add "b -> a_inner" under `edges:` if this is intended
```

Declared second, its ban is inert and the run is clean, exactly as in the duplicate case
above.

**An edge or ban naming a component that does not exist.** Ambiguity is caught honestly
(`E0206`, verified below in the clean list). A name that resolves to *nothing* is not:

```yaml
deny: ["b -> nosuch"]
```
```
EXIT=0
  No problems found in the document.
```

A typo in a ban is a ban that is not there, reported as a clean codebase.

---

### 3. Workspace crates that no component claims are invisible, and cannot be reached even by a wildcard

An undeclared crate is treated exactly like `serde` — out of scope, not counted, not
mentioned. That is right for a third-party dependency. For a member of the same workspace
it means part of the program is undescribed and the run says nothing about it.

Add a crate to a workspace that depends on both declared components and change nothing
else:

```
$ cargo-ply check /tmp/plyrev/ws1     # ws1-new depends on ws1-a and ws1-b; neither declared edge covers it
EXIT=0
  architecture  1 real crate dependencies cross between two differently-declared components:
                1 permitted by a declared edge or by nesting, 0 not permitted
  No problems found in the document.
```

This is live in this repository right now. `tests/e2e` is a workspace member; `ply.yaml`
declares three components and it is not one of them. I would call the omission **honest in
wording and a hole in effect**: the file's comment is carefully scoped ("Anything else
between these three is a violation"), so it does not lie. But there is no way for the
description to say "and there is a fourth crate here on purpose", and no way for the tool
to notice a fifth. Reproduced on the mirror with `tests/e2e` depending on the library:

```
$ cargo-ply check /tmp/plyrev/plymirror
EXIT=0
  architecture  1 real crate dependencies cross ... 1 permitted ... 0 not permitted
  No problems found in the document.
```

And the escape hatch does not help. With the broadest ban the grammar allows added to this
repository's own `ply.yaml`:

```yaml
deny:
  - "* -> core except cli"
```
```
EXIT=0
  architecture  1 real crate dependencies cross ... 1 permitted ... 0 not permitted
  No problems found in the document.
```

`*` means "any *declared* component", not "anything". Nothing in the output or in
`docs/SCHEMA.md` says so.

The fix is not a fourth component. It is a count: say how many crates in this workspace
belong to a declared component and how many do not, and name the ones that do not.

---

### 4. Test-only and build-time dependencies are dropped, undisclosed, and untested — and the coverage sentence is then false

The graph keeps only ordinary runtime dependencies. A `dev-dependencies` or
`build-dependencies` entry is discarded. That is a defensible choice; what is not
defensible is that nothing says it, and that the sentence printed in its place asserts
something untrue.

Build-dependency crossing a boundary with no edge permitting it:

```
$ cargo-ply check /tmp/plyrev/ws3   # crate_b has [build-dependencies] on crate_a
EXIT=0
  architecture  No crate here depends on another crate that belongs to a different declared
                component, so there was nothing to check.
  No problems found in the document.
```

That sentence is a statement about the workspace, and it is false: one crate here does
depend on another crate that belongs to a different declared component.

Test-only dependency running the other way round from a declared edge — the shape that
would put a cycle in the intent of this repository's own description:

```
$ cargo-ply check /tmp/plyrev/ws2   # crate_a dev-depends on crate_b; crate_b depends on crate_a
EXIT=0
  architecture  1 real crate dependencies cross ... 1 permitted ... 0 not permitted
  No problems found in the document.
```

Two real crossings exist; the report says one, and calls it "real crate dependencies"
without qualification.

The exclusion has **no test at all**. Deleting the filter entirely — so that test and
build dependencies are all treated as ordinary ones — leaves the whole suite green:

```
### dev/build-dep filter deleted
   ply-core + ply-cli unit tests:  ok. 167 passed; 0 failed
   arch_crate_tier_command e2e:    ok. 4 passed; 0 failed
   archtierbin_fixture e2e:        ok. 3 passed; 0 failed
```

---

### 5. Optional dependencies that are off by default are invisible

The module's own comment claims the advantage of walking cargo's resolved graph is that it
"reflects which optional dependencies actually got activated". It reflects which ones are
activated *under the default feature set*. A dependency behind a non-default feature is
absent entirely:

```
$ cargo-ply check /tmp/plyrev/ws4   # crate_b has an optional dependency on crate_a behind feature "extra"
EXIT=0
  architecture  No crate here depends on another crate that belongs to a different declared
                component, so there was nothing to check.
```

Again that sentence is false, and the proof is one flag away:

```
$ cargo metadata --format-version=1            -> ws4-b deps: []
$ cargo metadata --format-version=1 --all-features -> ws4-b deps: ['crate_a']
```

Build the crate with `--features extra` — which anyone with that feature in CI does — and
the boundary is crossed by code that ships. This is the same family as finding 4 and
should be settled the same way: either look at all features, or say in the output which
configuration was looked at.

---

### 6. Two workspace crates sharing a library name are conflated, and findings then name the wrong crate

Crate identity is the library target's name, or the normalised package name for a
binary-only crate. Two different packages in one workspace may legally carry the same
library name. They collapse to one identity, and the reports that follow are wrong in both
directions.

Four crates: `ws8-left` and `ws8-right` both expose a library called `shared`; `ws8-user-l`
depends on left, `ws8-user-r` depends on right. The document anchors one component at
`shared` and permits only the left user:

```
EXIT=1
  A0401 crate `ws8_user_r` depends on crate `shared`. `ws8_user_r` belongs to the `user_r`
    component and `shared` belongs to `leftside`, and no `->` edge in this document says
    `user_r` may depend on `leftside` ...
  A0405 crate `ws8_user_r` (component `user_r`) depends on crate `shared` (component
    `leftside`), and this matches the rule "user_r -> leftside" under `deny:`, which forbids
    it.
```

`ws8-user-r` does not depend on `leftside` at all. Both findings state a fact about this
codebase that is not true, and a ban fires against a dependency that does not exist. The
mirror image — a second component anchored at the same shared name — is finding 2's
duplicate-anchor case, so the false-negative direction is available too.

Rarer than the others and ranked accordingly, but it is the only place I found where the
tier says something positively false about the code rather than staying silent, which is
the worse of the two failure modes.

---

### 7. The tests

The tautological invariant test was genuinely fixed. Removing containment entirely makes it
fail, along with the two containment spot-checks:

```
### MUTANT: containment removed entirely
   containment_permits_a_dependency_on_a_descendant_with_no_edge ... FAILED
   containment_permits_a_descendant_depending_back_on_its_ancestor ... FAILED
   every_cross_component_dependency_is_either_permitted_or_flagged_never_both ... FAILED
```

I found no second test of the same kind — no assertion in this tier that compares two
values derived from the same code path. What I found instead is a different weakness:
several decisions the tier makes are pinned by nothing at all. Each mutation below leaves
the entire relevant suite green (unit tests for both product crates, plus both
architecture end-to-end suites):

| Deliberate break | Suite result |
|---|---|
| Test/build dependency filter deleted | all green |
| A dashed flow edge now permits a real crate dependency | all green |
| Duplicate anchors: last declaration wins instead of first | all green |
| An anchor's *last* path segment is taken as the crate instead of its first | all green |
| An unknown dependency package falls back to its raw cargo id | all green |

The second row is worth dwelling on. Every solid arrow in a Ply diagram is a checked claim
and every dashed one is explicitly declared-not-checked; if a dashed flow edge started
permitting a real dependency, that would be a false clean of exactly the kind this project
refuses. Today the behaviour is correct — I verified it at the command line, a dashed edge
does not permit the dependency and the finding still fires — but it is correct by one
pattern match that no test defends.

**One fixture does restate an implementation assumption as a requirement.** The
binary-crate fixture's top crate really depends on two crates; only one is declared. Its
first end-to-end test asserts `diagnostics.len() == 1` with the comment "the undeclared
`dual` crate must not also show up". That is finding 3's behaviour promoted to a pinned
expectation. If undeclared workspace crates ever start being reported — which is the fix I
am recommending — this fixture fails, and the reason it fails will look like a regression
rather than the correction it is.

---

### 8. Claims that the artifacts do not support

**"Verified in both directions before landing."** True, and I reproduced both directions.
But it covers one of the three invariants the file states, because it is the only one with
a dependency behind it. Of the three:

- *The macro crate stands alone* — enforced. A dependency from it into the library is
  reported correctly (verified: names both crates, both components, exits 1).
- *The library must never reach up into the command-line layer* — **not enforced**. Any
  such dependency is a package cycle, cargo refuses the graph, and finding 1 applies: exit
  0, clean.
- *Nothing may depend on the command-line layer* — enforced only against crates somebody
  remembered to declare (finding 3), and not at all from the library (the cycle again).

So the description's headline example — "the cycle a well-meaning refactor would create" —
is the one case the checker cannot see. That deserves saying in the file rather than being
discovered here.

**"The tier that would has been cancelled on measurement."** The measurement
(`docs/item-tier-resolvability.md`) is careful, honest about its own bias, hand-validated
against a file counted by hand, and its own conclusion is correctly scoped: *"Do not build
it as a syntax-only item tier"* on the strength of call-site resolvability. But the
commit message stretches it to cover `pure`, `uses` and `owns`. Capability detection is a
path-mention check (uses of the filesystem, the network, processes, time, randomness,
unsafe blocks) and profile bans are syntactic checks the spec itself calls "reliable and
always errors". Neither was measured. The cancellation is well-evidenced for calls and
unevidenced for capabilities; the file should say which half it decided.

**"Capabilities are deliberately absent."** This one holds up, and it is the right call for
the stated reason.

**Self-hosting is not wired to anything.** The testing strategy says this workspace gets
its own description "kept green in CI". CI runs formatting, lint, the unit tests and the
end-to-end tests. It does not run `cargo ply check` on this repository, and no test
asserts that this repository's own description passes. Combined with finding 2, the
description can rot to nothing without a single red build.

**The specification is stale by three commits.** It still says `check` "implements two of
its three tiers" and lists the architecture tier as unbuilt future work, and the command
summary still reads "IMPLEMENTED: schema + anchors only". That stopped being true when the
tier landed. The user-facing schema document was not updated either: it describes the crate
tier in the future tense and mentions neither of the two findings a user most needs (that
test and build dependencies are excluded, and that undeclared crates are ignored).

**The running list is stale.** It records the architecture tier as carrying a known defect,
"blind to binary-only crates … Fix in flight". That fix landed in `a4c8675`, three commits
ago and in the same session. There is no entry at all for this repository declaring its own
architecture.

**The readiness document does not mention this tier.** It is dated today, it is titled as a
measurement of whether Ply can be handed to someone else, and its ordered list of what
stands in the way covers only the verification path. An always-on part of `check` shipped
the same day and is absent from it, gaps and all.

**The circularity the project already flagged.** The record notes as a known risk that a
model-drafted architecture description someone skims and approves is "architecture-as-vibes
laundered through a deterministic checker", and rejected an earlier plan for being circular.
This `ply.yaml` was drafted by the same agent that wrote the checker, in the same session.
The three rules in it happen to be true — I checked each against the manifests — so the
outcome is fine. The process is the one that was ruled out.

---

## What I checked and found clean

These I attacked and could not break. Recording them because a clean area is information.

- **Target-gated dependencies** (`[target.'cfg(...)'.dependencies]`) are seen and
  classified, both when the condition matches the host and when it does not. I expected
  this to be a hole and it is not.
- **Renamed dependencies** (`package = "..."`) are transparent, because the graph walk
  works from cargo's package identities rather than the name written in the manifest.
  Every fixture in the tier uses a rename, and the mirror workspaces I built do too.
- **Library-versus-binary identity**, the thing the follow-up fix was about. A crate with
  both a library and a binary is named by its library; a binary-only crate is named by its
  normalised package name and never by its binary. Both are pinned by tests that fail when
  broken.
- **Ambiguous component names** in an edge or a ban are reported honestly, naming both
  candidates and telling the user how to disambiguate.
- **A dashed flow edge does not permit a real dependency** — correct today, though defended
  by no test (finding 7).
- **Containment in both directions**, a declared edge permitting exactly its own pair, a
  ban firing even when an edge would otherwise permit the dependency, a ban's exception
  list exempting only the component it names — all behave as specified, and all are pinned
  by tests that fail when the logic is broken.
- **Dependencies on crates outside the workspace** are correctly out of scope.
- **The wording of both findings passes the newbie bar.** They name both crates, both
  components, state plainly that nothing permits the crossing, and give the two ways to
  fix it. No jargon that needs the spec open.
- **No arithmetic hazard** in the coverage tally: the permitted count cannot underflow,
  because a violation is only ever counted after the crossing that produced it.
- I found **no case where the tier reported a violation that was not there**, apart from
  finding 6's conflated names.

---

## Is a crate-level-only architecture checker worth having?

Yes — but its case is narrower than the commit messages suggest, and it is not yet made.

The honest accounting: at crate granularity this tier enforces something cargo already
half-enforces. You cannot create a cross-crate dependency without editing a `Cargo.toml`,
and a `Cargo.toml` diff is short, rare and reviewable by eye. On this repository the entire
checked surface is one dependency. So the tier is not buying detection of a change nobody
would notice; it is buying two other things.

The first is that **intent becomes a file**. "The macro crate stands alone because it is
compiled for the host" is a sentence that today lives in one person's head and tomorrow
lives in a diff. That is worth having independently of enforcement, and it is why the
description should exist even where the checker is weak.

The second is that **this is the only tier that can ever be sound**. The measurement that
cancelled the item tier is convincing: four call sites in five cannot be resolved from
source, and a tier whose blind spot is larger than its coverage would spend its silence as
approval. The crate tier has the opposite property — cargo knows the answer exactly, so
when it fires it cannot be wrong, and when it is quiet it can in principle say precisely
what it looked at. For a project whose entire thesis is that a green result means
something, having one architecture check that is exact is worth more than having a broader
one that is approximate.

But that second argument is a promise this build does not yet keep. Six of the eight
findings above are the tier being quiet about something it did not look at: a graph it
could not fetch, a crate nobody claimed, a dependency class it drops, a feature
configuration it did not consider, an anchor that matched nothing. Every one of them
currently reads as approval. A sound tier that is silent for unsound reasons is not better
than an approximate one — it is worse, because its soundness is what earns it the right to
be believed.

So: keep it, and keep the description. But it does not earn its place until "no problems
found" means "I looked at your whole workspace and found nothing", rather than "I looked at
whatever I could reach and did not tell you what I could not". Fix finding 1, fix finding
2, and give the coverage line a denominator — how many crates in this workspace belong to a
declared component, and which ones do not. Those three changes are small, and they are what
turns this tier from a thing that happens to be right into a thing that can be trusted when
it is quiet.
