---
name: ply-audit
description: Report what a codebase's green results actually rest on — the promises taken on trust, the evidence still owed, and the decisions nobody has made — without running engines or treating an absence of findings as a clean answer.
---

# Ply Audit

A verdict says what was checked. This says **what it stood on**. Both commands here are
fast and run no engines.

The question this skill answers is not "did it pass" — `$ply-verify` answers that. It is
"if this is green, what would have to be true for that green to be meaningless?"

## The two commands

```bash
cargo ply audit path/to/crate --json
cargo ply worklist path/to/crate --json
```

| Command | Answers |
| --- | --- |
| `audit` | What this codebase's evidence rests on that Ply never checks |
| `worklist` | What is owed: unresolved decisions, and assumed promises nothing has verified |

Read both. They overlap by design and neither is a superset: `audit` lists standing trust,
`worklist` lists outstanding work.

## What lands on the trust surface, and why each matters

These are the exact kinds `audit` reports — not a general list of things that could go
wrong, and not everything Ply leaves unchecked:

| Kind | What it means | Why it is worth reporting |
| --- | --- | --- |
| `assumed_contract` | A proof used a callee's declared promise instead of its real body | If that promise is wrong, everything resting on it is wrong, and nothing has checked it |
| `environmental_assumption` | A function an outside party can reach; its preconditions are assumptions about the world | Nothing inside the codebase ever checks them |
| `trusted_claim` | Someone attested to it with named evidence; no machine checked it | It is a person's word, correctly recorded as a person's word |
| `contract_helper` | A promise calls a helper function, so the promise is only as true as that helper | The helper is part of the specification and usually nothing checks it |
| `profile_escape` | An `#[ply::allow]` suppressing a finding on one item | Someone decided to permit this; it is a decision, not an absence |
| `derived_fn` | A body generated from its own contract | The contract is the only thing standing behind it |

`worklist` reports two more: unresolved markers — decisions nobody has made, recorded in
place — and assumed promises still waiting on evidence.

## How to report it

1. **Lead with the count and the shape**, not a list. "Eleven things are taken on trust;
   four are assumed promises about legacy code, and those are the ones that would change a
   verdict if they turned out false."
2. **Name the ones that would change a result.** An assumed promise a proof rested on is a
   different order of thing from an escape someone deliberately wrote. Say which is which
   rather than presenting one flat list.
3. **Give each item its cheapest discharge.** An assumed promise is discharged by checking
   the callee against that same promise — usually a `fuzz` check on the callee, which runs
   the real body and needs no proof. Say that, with the function named.
4. **Never present an empty list as a clean bill of health.** An empty trust surface on a
   codebase with no claims in it means nothing was searched, and the command says so
   itself: "That is a fact about what is declared, not a verdict about the code."
5. **Carry the command's own limits through.** Both commands end with a section naming
   what they did not look at. That section is part of the answer, not a footer — dropping
   it turns a bounded report into an unbounded-sounding one. Report at least the limits
   that bear on what you were asked.

## The distinction that matters most

**A conditional verdict and an owed one are two facts, not one.**

- *Conditional* says the result rests on an assumed promise. That is permanent information
  about how the result was obtained.
- *Owed* says nothing has yet checked that promise against the real code. That is a debt,
  and it can be paid.

Discharging the debt does not make the verdict unconditional — it still rested on the
promise. Report both, and do not let one stand in for the other.

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

| target | authority |
| --- | --- |
| implementation | ask-first |
| contract | ask-first |
| declared_check | ask-first |
| architecture_contract | ask-first |
| unresolved_marker | ask-first |

**This skill reports; it does not repair.** Discharging an assumption means adding a check
or changing code, and both belong to the developer. Propose the specific change with the
function named and the evidence quoted, then stop. Removing an unresolved marker without
making the decision it records is deleting the record of an open question, which is worse
than leaving it.
