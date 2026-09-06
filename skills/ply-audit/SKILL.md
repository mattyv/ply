---
name: ply-audit
description: Report what a codebase's green results actually rest on — the promises taken on trust, the evidence still owed, and the decisions nobody has made — without running engines or treating an absence of findings as a clean answer.
---

# Ply Audit

A verdict says what a run checked. This audit lists **declared trust and possible evidence
gaps**. Both commands here run no engines and do not establish what a completed proof used.

The question this skill answers is not "did it pass" — `$ply-verify` answers that. It is
"if this is green, what would have to be true for that green to be meaningless?"

## The two commands

```bash
cargo ply audit path/to/crate --json
cargo ply worklist path/to/crate --json
```

| Command | Answers |
| --- | --- |
| `audit` | Declared trust, assumptions, and the limits of this static inspection |
| `worklist` | Unresolved decisions and assumed promises listed as needing evidence |

Read both. They overlap by design and neither is a superset: `audit` lists standing trust,
`worklist` lists outstanding work. Neither command reads verification records to learn
whether a listed callee already passed. Do not call an item unpaid merely because it
appears here; use completed verification evidence for that question.

## Read the document first, if you have not

Both commands report against what the document declares, so a thin document produces a
short trust surface for reasons that have nothing to do with the code being trustworthy.
When you do not already know what is declared:

```bash
cargo ply render path/to/crate --text
```

That explains the YAML declarations. A screenshot omits tooltip text, so prefer this
form for declaration review. It does not include Rust contract attributes; use source
or a completed run when those matter. Inspect the drawing too if visual layout is part
of the request.

## What lands on the trust surface, and why each matters

These are the exact kinds `audit` reports — not a general list of things that could go
wrong, and not everything Ply leaves unchecked:

| Kind | What it means | Why it is worth reporting |
| --- | --- | --- |
| `assumed_contract` | Static inspection identifies a callee promise that a declared proof may assume | Check a completed run to see whether it was assumed or supported by callee evidence |
| `environmental_assumption` | A declared external can reach a function whose preconditions constrain its inputs | The declaration does not establish that the outside caller satisfies those conditions |
| `trusted_claim` | Someone attested to it with named evidence | Ply does not validate the attestation; its artifact may itself be a specialised test |
| `contract_helper` | A promise calls a helper function, so the promise is only as true as that helper | The helper is part of the specification and usually nothing checks it |
| `profile_escape` | An `#[ply::allow]` suppressing a finding on one item | Someone decided to permit this; it is a decision, not an absence |
| `derived_fn` | A body generated from its own contract | The contract is the only thing standing behind it |

`worklist` lists unresolved markers — decisions nobody has made, recorded in place —
and assumed promises that its static scan marks as owed. Carry its limitation about
unread verification records into the report.

## How to report it

1. **Lead with the count and the shape**, not a list. "Eleven things are taken on trust;
   four are promises about legacy code that declared checks may assume. The audit does
   not say whether those callees have already been checked."
2. **Distinguish potential dependencies from recorded ones.** A declared assumption is
   different from evidence that a particular proof used it. Cite a completed run if you
   say a result rests on that promise; otherwise describe it as a declared dependency.
3. **Give actionable next steps where possible.** For a callee promise, identify a
   suitable check against the real body. Fuzzing can provide sampled evidence, but it
   does not substitute for a bounded proof when that is required. An external assumption
   cannot be discharged by checking this codebase alone; do not promise every item can
   be removed.
4. **Never present an empty list as a clean bill of health.** An empty trust surface on a
   codebase with no claims in it means nothing was searched, and the command says so
   itself: "That is a fact about what is declared, not a verdict about the code."
5. **Carry the command's own limits through.** Both commands end with a section naming
   what they did not look at. That section is part of the answer, not a footer — dropping
   it turns a bounded report into an unbounded-sounding one. Report at least the limits
   that bear on what you were asked.

## The distinction that matters most

**A conditional verdict and an owed one are two facts, not one.**

- In a completed verification run, *conditional* says a result rests on assumed promises.
- *Owed* identifies evidence still needed under that run's rules.

These static commands cannot determine which evidence has since been earned. Nor is
conditionality permanent across runs: a caller can stand on a callee's clean bounded
proof from the same run without `conditional` or `owed-evidence`. Report the actual
statuses from the completed run rather than deriving them from the audit list. A saved
older run still records its original assumptions.

## Reading a code

Anything either command reports carries a short code. Decode it with:

```bash
cargo ply explain <CODE>
```

That says what the code means, who reports it, and — importantly here — whether it is a
rule this build actually enforces or one that is only described. A rule that nothing emits
must never be reported as a check that passed.

## Data boundary

| resource | access |
| --- | --- |
| audit_json | read |
| worklist_json | read |
| ply_lock | forbidden |
| target_ply_internals | forbidden |

Read the public JSON of these two commands. Do not open `ply.lock` or reconstruct the trust
surface from records, and do not derive a verdict here — this skill reports standing
assumptions, and a verdict comes from a run.

## Change authority

The table states defaults when the task has not already authorized the change. Honor
existing user authorization; do not ask again for work already approved. A request to
review alone does not authorize changing requirements.

| target | authority |
| --- | --- |
| implementation | ask-first |
| contract | ask-first |
| declared_check | ask-first |
| architecture_contract | ask-first |
| unresolved_marker | ask-first |

**This skill reports; it does not repair.** Propose the specific change with the function
named and evidence quoted. For an audit-only request, end with the findings. If the task
already authorizes repairs, continue under the authoring or verification skill without
asking again. Removing an unresolved marker still requires resolving the decision it
records; deleting it is not evidence that the question was answered.
