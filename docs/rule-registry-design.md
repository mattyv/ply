# The rule registry — one page for review

*Design proposal, 2026-08-31. Nothing here is built. It needs a decision before it is.*

## The measurement that justifies it

Every rule Ply can report is identified by a code. Those codes are written down twice: once
in the source that emits them, once in `The-Ply-Spec.md` and `docs/SCHEMA.md`. Counted today:

| | |
|---|---|
| distinct codes across both | **72** |
| in both — emitted *and* documented | **38** |
| emitted, explained nowhere | **21** |
| documented, never emitted by anything | **13** |

Barely half of what Ply talks about is described where a user would look, and thirteen rules
are promised by documents that no code enforces.

The thirteen are the dangerous half. A reader of the spec learns that Ply checks a rule; it
does not. That is the failure this whole project exists to refuse — describing intent as
fact — occurring in the documents that define it.

The twenty-one are the quieter problem: a diagnostic fires on a user's terminal carrying a
code that appears in no document they can search.

This is not new, and that is the argument. It has been found and closed by hand at least
three times: §8 of `docs/SCHEMA.md` claimed none of its rules were enforced when two of them
were; the enforcement matrix was written and then found to be missing a row; and this
session alone produced three sentences that claimed more than they should. Each was fixed
individually. The count above is what "fixed individually" leaves behind.

## What the registry is

One table, as data, listing every rule exactly once. Each row carries:

- **code** — `A0401`, `W0511`, and so on
- **tier** — which level of checking it belongs to (crate, item, function)
- **status** — `enforced`, `declared-only`, or `planned`
- **severity** — error, warning, info
- **gloss** — one plain sentence, in the newbie-bar voice, saying what the rule means
- **spec anchor** — where the reasoning lives, for a reader who wants it

Everything that currently names a rule derives from that table instead of restating it: the
checker, the "what was not checked" output, the enforcement matrix in `SCHEMA.md`, the
diagram and the text form, and the planned agent-facing `skill` command — which the spec
already specifies as *generated from schema plus diagnostic registry*, so this is its data
source rather than a new idea.

## What it makes impossible, by construction

Two invariant tests, in the style of `every_painted_element_resolves_a_style_rule` — walk
the real artifact, fail on the first unexplained item:

1. **Every code the source emits has a row.** A new diagnostic without a registry entry
   fails the build. This closes the twenty-one.
2. **Every row marked `enforced` has a site that emits it.** A rule that stops being
   enforced, or was never enforced, cannot keep its status. This closes the thirteen.

Neither is a lint anyone can forget to run. They are the gate.

A rule that is *not* enforced can still be documented — that is often honest and useful —
but it renders as `declared, not enforced` wherever it appears, in every surface, because
they all read the same field. Nobody has to remember to write the caveat.

## What it does NOT fix, stated plainly so it is not oversold

**The registry would not have caught a single one of this session's three false sentences.**
Those were: a claim that no constructor existed when one did; a claim that `verify` does not
read a contract it demonstrably reads; a claim that a test had been written where none had.
Every one carried a correct code with a correct status. The registry governs *whether a rule
is described as enforced*. It says nothing about whether a sentence is true.

That is a different class and it needs a different instrument — the exact-string wording
tests already in use, plus reviewers running the repository's own fixtures rather than
crates written for the purpose. Worth saying because the registry is exactly the kind of
structural fix that invites the belief that a whole category of error is now closed.

## Four decisions I need from you

1. **Where the table lives.** My recommendation: a `const` table in `ply-core`, not a data
   file read at build time. The compiler can then require every match over it to be
   exhaustive, so adding a rule forces every consumer to handle it. A TOML file is friendlier
   to edit and gives up that guarantee. I would take the guarantee.

2. **Whether `SCHEMA.md`'s tables are generated.** The repository already has this pattern
   working for the committed drawings: regenerate in CI, `git diff --exit-code`, and a stale
   document fails the build. Extending it to the rule tables is the only way the documents
   stay true without anyone remembering. It means those tables stop being hand-editable.

3. **What happens to the thirteen.** Each is a rule the docs promise and nothing enforces.
   They are not one thing: some are unbuilt features, some may be genuinely dead. They need
   triaging into *implement* or *retract* before the registry can carry them honestly, and
   that triage is a judgment call per rule. Roughly a session's work, and the registry is
   blocked on it.

4. **Whether it lands before or after the two reach defects.** The registry is the larger
   piece; the reach defects (a promise cannot mention its receiver; a comparison nested in a
   promise) are small, measured, and block the most natural things a promise can say. My
   recommendation is reach defects first, on the grounds that they are cheap and the registry
   is not, but they compete for the same session and it is your call.

## Cost, honestly

The table and its two invariant tests are perhaps a session. Triaging the thirteen is
another, and cannot be delegated cleanly because each one is a decision about whether a
promised rule should exist. Rewiring the consumers — checker output, both views, the schema
tables — is a third, mostly mechanical once the table exists.

So: three sessions, of which one is judgment and two are typing. It closes the sixth clause
of the seven-clause definition of production, which is otherwise unmet and cannot be closed
by review, because review is what has been failing to close it.
