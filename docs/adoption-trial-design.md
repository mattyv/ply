# The adoption trial: design

*Written 2026-08-26. This is a design, not a result. Nothing in it has been run.*

## TLDR

**Run a three-scenario pilot now; hold the other six until methods resolve and struct,
`String` and enum inputs can be built.** Running the full series today would spend eight
expensive multi-agent runs re-deriving one fact we already measured — a naturally-designed
Rust library hands Ply nothing it can check — and would stop before reaching the questions
the trial exists to answer. The pilot measures the *instrument*, not the tool.

**The headline measurement has moved, and it is now about the person, not the program.**
Ply's whole value is that a green result means something, so the thing to measure is whether
the principal directing the work *correctly believed* what they were told: did they take a
clean result as evidence when it was evidence, understand a refusal when it was a refusal,
and — the failure that would matter most in the field — did they ever direct a change that
made the tool quiet rather than the code correct? A tool that gets routed around is worth
nothing however sound it is internally. One scenario is built specifically so that the cheap
path is to weaken the claim, and there is an objective test for whether that path was taken:
replay the original failing input against the final code and see whether it still breaks the
property the principal wrote down on day one.

Two smaller decisions worth surfacing here rather than burying. Every configuration example
in this repository that has ever actually been *run* claims free functions and nothing else
— the three that claim methods were never executed — so a principal learning by copying our
examples is being taught to avoid the exact shape we most need to test. The trial handles
that by freezing the design before any Ply material is read, and by measuring how far the
built code drifts back toward the examples' shape. And my own predictions are contaminated:
I read the two prior measurements before writing them, so only the ones marked low or medium
confidence say anything about whether we understand this tool.

---

## 1. What this trial is actually of

The loop being tested is: **a principal who holds the intent, directing a model that writes
the code, with Ply as the thing that decides whether the result can be believed.** That is
not an approximation of the target workflow — it *is* the workflow Ply exists for, and it is
the same loop this project has been built with all year.

Which gives a free sanity check. Three failure modes have shown up repeatedly in building
Ply itself. If the trial cannot detect them when they occur, the trial is not measuring
anything:

| Failure seen in building this project | What in the trial detects it |
|---|---|
| A full test suite passes and the feature ships with real defects | The sabotage phase (§7): pre-written domain bugs planted afterwards |
| An agent reports work complete that was never verified | Report fidelity (§6): the implementer's message to the principal compared against the actual tool output |
| A document asserts something its evidence does not support | The trust ledger (§6): what the principal wrote down as now-true, checked against ground truth |

Three questions, in priority order:

1. **Did the principal believe true things?** When Ply said checked, was it checked, and did
   they know what "checked" covered? When it refused, did they fix the right thing?
2. **Did Ply pay for itself?** Not "does it work" — it works on fixtures. Did the feature end
   the day with more of it genuinely covered, and is the code still code a reviewer approves?
3. **Do the documents work?** This is the first time anyone who has not read the design
   specification has been handed the tool. Every question the principal has to escalate is a
   defect in a document written to prevent it.

Questions 2 and 3 pull against each other in one respect worth naming now: helping the
principal improves 2 and destroys 3. The protocol resolves it by logging every act of help
and by measuring how much of the final code came from outside the loop.

---

## 2. The roles

### 2.1 Principal — holds the intent, directs, writes no code

A fresh agent per scenario. Never told the exercise is about evaluating Ply. Its brief is a
feature to ship, in domain terms, plus one line of team policy: *"New code here is expected
to be covered by our checking tool as far as the tool can cover it. Where it can't, say so
in your handover note."*

It reads the code, reads the configuration, reads the user-facing documentation, decides
what to do, and instructs the implementer. It does not edit files.

**Naivety here is about Ply's internals and its known gaps — never about the software being
built.** A real adopter knows their own codebase perfectly well.

| May read | May not read |
|---|---|
| All code in the crate under construction, and the existing host crate in the extension scenarios | `The-Ply-Spec.md` — describes capabilities this build does not have |
| `docs/SCHEMA.md` — the user-facing reference | Ply's own source under `crates/` and `tools/` |
| `schema/ply.schema.json` — because the reference page names it as the authority | Internal write-ups: the readiness measurements, the review documents, the findings notes, the vetting narratives, `TODO.md`, `CLAUDE.md`, `docs/adr/` |
| Every `ply.yaml` and `*.ply.yaml` in this repository, fixtures and root included | This design document, which contains the predictions |
| Everything the tool prints: help text, diagnostics, results | This conversation, and any transcript of it |
| Third-party Rust documentation and the open internet | |

**Enforcement is filesystem shape, not an honour system.** The principal and implementer work
in a directory that is not a checkout of this repository. They get their own crate, `cargo
ply` on the path, and a reference tree containing exactly: `docs/SCHEMA.md`,
`schema/ply.schema.json`, and a copy of every `*ply.yaml` in the repository with its path
preserved. Copying the configuration files without their surrounding prose is what keeps the
vetting narratives out while letting the examples in, and it is the enforcement mechanism as
well as the permission.

### 2.2 Implementer — writes the code, runs the tool, reports back

A second agent, with **the same document permissions as the principal**. This is the point:
if the implementer were a Ply expert, the trial would measure nothing about learnability and
nothing about the loop, because the loop in the field does not contain an expert. It does
what it is told, writes the Rust, writes the configuration, runs the commands, and tells the
principal what happened.

What it tells the principal is itself data. Every report it makes is compared against the raw
tool output, and a report that claims more than the run earned is recorded as a report
fidelity failure — the same defect this project has hit repeatedly from the other side of the
desk.

### 2.3 Three roles that only exist to keep the trial honest

**Expert rescue.** Fully informed — specification, source, fixtures, history. Runs *after*
the principal has stopped and their coverage has been frozen. Takes the shipped code and gets
as much of the frozen property list checked as is possible today, recording every change
under exactly one label: contract only / signature reshaped / type replaced / code
restructured / function split / property weakened / property abandoned. May not modify Ply,
may not hand-edit the recorded results file, may not change what the feature does. The gap
between the loop's coverage and the expert's coverage separates *the tool cannot do this*
from *the tool can do this and nobody could tell*.

**Saboteur.** Plants the pre-registered bugs (§7). Must not have written the code, and must
not see the contracts before planting — otherwise it plants bugs the contracts obviously
catch.

**Blind reviewer.** Never hears Ply mentioned. Scores API quality on three unlabelled
variants in random order — the frozen day-one sketch, the shipped code, the rescued code —
answering one question: *would you approve this API?* This is how "the design got worse to
satisfy the tool" becomes a number instead of an argument.

---

## 3. The examples teach free functions, and that is a decision, not a caveat

Of the 89 configuration files in this repository, the only ones that claim a method on a type
are three paper-only vetting scenarios that were never executed — one of which recorded on its
own feasibility pass that its flagship methods could not be checked — plus rendering fixtures
that never go near verification, and one file that exists to exercise schema validation.
**Every configuration in this repository that has ever actually been run through a check
claims free functions and nothing else.** The reference page is better than that: it shows a
method in its first example and says in the same breath that it cannot be verified today. But
a reader who learns by pattern-matching against working examples is being taught, without
anyone deciding to teach it, to write flat modules of free functions over scalars.

That is a documentation finding in its own right and it is reportable now, without running
anything. It also contaminates the trial: a principal who imitates the examples produces
exactly the code that avoids the shape we most need to test, and we would read that avoidance
as the tool being easy to adopt.

**The decision: design first, configure second — the property list and API sketch are frozen
before any Ply material is opened, and drift back toward the examples' shape is measured
against that frozen sketch.**

Why this and not the alternatives. Running each scenario twice, once examples-first and once
design-first, doubles the most expensive part of the trial to answer a question about our
documentation rather than about our tool — and the pilot cannot afford it. Measuring drift
without imposing design-first is unattributable: with no frozen baseline you cannot tell
imitation from a designer who would have written free functions anyway.

**What it costs, stated plainly.** Design-first makes the principal *less* realistic. A real
adopter reads the examples first and writes code that looks like them, so this rule
systematically overstates the collision between a natural design and Ply's limits. The bias
runs toward finding more friction than a real adopter would feel, which is the safer
direction to be wrong in but is still wrong. One correction, bounded to a single run:
**scenario 1 gets a second arm where the principal reads the examples before designing.**
Scenario 1 is the cheapest and the one most likely to succeed either way, so the arm costs
little and puts a number on how large the design-first bias is. It is a measurement of the
rule, not a second method.

---

## 4. The protocol, per scenario

| Phase | Who | What happens | Produces |
|---|---|---|---|
| 0. Blind design | Principal, no Ply material, no mention of Ply | From the domain brief alone: an API sketch and a prose list of the properties this code must have. **Frozen and timestamped.** | The denominator for every coverage number; the baseline for drift and distortion |
| 1. Build | Principal directing, implementer building | Documents and examples now open. Ship the feature; get as much of the frozen list checked as the loop can. **After every run, the principal writes one line in the trust ledger** (§6) before proceeding. Principal may declare done or stuck. Hard cap on wall clock. | Shipped code, configuration, every run, both transcripts, the ledger |
| 2. Rescue | Expert | Maximise checked coverage from the phase 1 artifact | Rescued code, change log by category |
| 3. Sabotage | Saboteur | Plant the pre-registered bugs one at a time; re-run. Then replay every counterexample the loop ever saw against the final code | Caught / missed per bug; objective quieting instances |
| 4. Blind review | Blind reviewer | Score three unlabelled API variants | Design-quality delta |

Phase 0 is what separates this from an ordinary usability test, and it generalises the trick
that produced the rate-limiter result: **the properties are written down before the tool is
known, so nobody can quietly stop wanting the things the tool cannot check.** Every coverage
figure is computed against that frozen list, never against what ended up being claimed.

---

## 5. The scenarios

Chosen by asking what people write in Rust and what goes wrong in it. Each names the real
software it stands for. None was chosen by asking what Ply can check — though the *order*
they run in was, which is a different thing and is argued in §9.

| # | Scenario | New vs existing | Property kind | Awkward part | Would a human write the property down? |
|---|---|---|---|---|---|
| 1 | Retry and backoff policy | All new | Arithmetic | Neither — genuinely simple | Yes, usually as a comment |
| 2 | Length-prefixed frame decoder | All new | Structural over bytes | Data shape | Yes — this is where fuzzing folklore lives |
| 3 | Paged list endpoint over an existing store | New core, old surroundings | Arithmetic + completeness | Control flow at the boundary | Rarely; people test it instead |
| 4 | Splitting a charge across line items | New logic, existing money types | Arithmetic with rounding | Data shape (domain types) | Yes — "the parts must sum to the whole" |
| 5 | Order lifecycle state machine | All new | Ordering | Control flow | Yes, as a diagram nobody checks |
| 6 | Bounded cache with eviction | New, in an existing service | Structural over a data structure | Data shape (private state) | Yes for capacity, no for the subtle half |
| 7 | Service configuration validation | All new | Validation / data shape | Data shape (strings, enums) | Yes, exhaustively |
| 8 | Sharded counter under concurrency | New, in an existing service | Ordering across threads | Control flow | Yes, and nobody can check it by reading |
| 9 | Metered usage charge | All new | Arithmetic that overflows | Neither — the temptation is the point | Yes, and it is easy to write down weakly |

Every cell in the axis matrix is filled at least twice, and no two scenarios share all four.
Checked again adversarially after the runs, in §10.

---

### Scenario 1 — Retry and backoff policy *(pilot; the one I expect to go well)*

**Stands for:** the retry layer in the AWS SDK, the `backoff` crate, any client calling a
flaky service. Real incidents: a backoff that overflows and wraps to zero, turning a retry
policy into a denial-of-service against your own dependency; jitter applied after the cap so
the delay exceeds the documented maximum.

**Brief:** *"We call a downstream service that fails intermittently. Write the policy that
decides how long to wait before attempt N, given a base delay, a maximum delay, and a jitter
setting. It must never wait longer than the maximum, never wait zero after a failure, and be
deterministic given a seed so we can test it."*

**Implementer may:** everything it is directed to do. **May not:** change the policy's
behaviour to suit the tool without the principal directing it — and every such direction is
logged, because that is the quieting measurement.

**Prediction — high confidence.** Most of this gets checked and the tool earns its keep. The
natural shape is integer in, integer out. At least 4 of the frozen properties earn real
evidence; the overflow property fails first with a concrete failing input; first passing check
inside 30 minutes. **Wrong if:** fewer than 3 properties earn evidence; or the loop cannot get
a first check to pass without escalating; or the natural formulation — a policy object
carrying base, cap and jitter with a `delay_for(attempt)` method — so dominates the
principal's instinct that the free-function form never appears, which moves this scenario into
the method wall and makes it a very different result. **The examples-first arm is here:** if
the design-first sketch is object-shaped and the examples-first sketch is not, we have measured
the teaching effect directly.

**Bugs to plant:** (a) the exponent shift overflows at attempt 32 and wraps; (b) jitter is
added after the cap rather than before; (c) attempt 0 returns zero delay.

---

### Scenario 2 — Length-prefixed frame decoder

**Stands for:** `tokio-util`'s length-delimited codec, a Redis protocol reader, a varint
decoder. Real bugs: trusting the declared length; computing start-plus-length and overflowing;
returning a frame that overlaps the next; treating a truncated buffer as an empty frame.

**Brief:** *"Read frames off a byte stream. Each frame is a 4-byte big-endian length followed
by that many bytes. Decode as many complete frames as the buffer holds, tell the caller how
many bytes you consumed, and never read past what you were given."*

**Prediction — medium confidence, leaning partly-checked.** The property is exactly what
fuzzing is good at, and one supported input shape — an owned byte buffer — fits. But the
natural Rust signature takes a byte slice, which cannot be built, so the first attempt is
refused for a reason that looks arbitrary from where the principal is standing. I predict they
hit that refusal, and I genuinely do not know whether the reference page's own shape table
gets them past it. That table exists and says exactly this, so this is a clean test of whether
writing a limit down is sufficient. **Wrong if:** the refusal is never hit, or is resolved in
under two attempts with no escalation.

**Bugs:** (a) the length is read then added to the offset with no check that it fits; (b) a
frame whose declared length exceeds the remaining buffer is returned truncated rather than
deferred; (c) the consumed-byte count omits the header on the final frame.

---

### Scenario 3 — Paged list endpoint over an existing store *(pilot; the extension thesis)*

**Stands for:** adding a paginated list route to a service that already has a repository
layer. Real bugs: an item appears on two pages or on none; the last page is off by one; a
cursor that is a raw offset runs past the end.

**What already exists:** a real store crate **not written for this trial** — vendored from an
existing open-source library, or failing that written by an agent that has never heard of Ply
and never will. It carries no promises of any kind. The surrounding code being genuinely
innocent of the tool is the condition that makes this scenario worth running.

**Brief:** *"Our store returns a page of records given an offset and a limit, and can report
how many records exist. Add a list endpoint that walks the whole set page by page. Nobody must
ever see an item twice or miss one, and the caller must be able to tell when they have reached
the end."*

**Prediction — low confidence, leaning badly.** The arithmetic core — given a total, an offset
and a limit, what is the next cursor and is this the last page — is checkable and should pass.
The property the principal actually cares about, that walking every page visits every record
exactly once, is a statement about a *sequence of calls into code carrying no promises*, and I
expect it to be uncheckable with the tool correctly refusing to walk into the store. So: a
split result, and the interesting measurement is whether the principal understands *why* and
whether the refusal message gets them there or sends them somewhere else. **Wrong if:** a
formulation is found that gets the completeness property checked — I would want to see it — or
if the refusal manifests as a timeout instead, which is honest but uninformative and a worse
outcome than a named refusal.

**Bugs:** (a) the last page is dropped when the total is an exact multiple of the page size;
(b) the cursor advances by the page size rather than by the number of records returned; (c) an
empty result is treated as end-of-set regardless of why it was empty.

---

### Scenario 4 — Splitting a charge across line items

**Stands for:** invoice and tax code in any billing system. The canonical bug: a total split
three ways loses or gains a cent, and an accountant finds it in production.

**Brief:** *"Given a total in cents and a set of line items with weights, allocate the total
across the items. Every allocation is a whole number of cents, none is negative, and they sum
to exactly the total — that last part is not negotiable."*

**Prediction — high confidence that it fails, medium on the reason.** Perfectly arithmetic and
it would be a showcase, but the natural design carries a money newtype and a collection of
items, and neither can be built. I predict a direct choice between the domain modelling a
reviewer wants and getting anything checked, that the loop takes the checkable route, and that
the blind reviewer marks the result down for it. That trade is the finding. **Wrong if:** the
blind reviewer does not penalise the checkable variant, which would mean the distortion is
cosmetic and the type wall costs less than the earlier measurement implied.

**Bugs:** (a) each allocation rounds independently so the sum drifts; (b) the remainder is
assigned to the first item unbounded; (c) a zero-weight item receives a cent.

---

### Scenario 5 — Order lifecycle state machine

**Stands for:** an exchange order's lifecycle, a payment's, a background job's. Real bugs: an
impossible transition reachable through a rare branch; a terminal state that is not terminal;
a cancel racing a fill leaving a state with no handler.

**Brief:** *"Model an order's lifecycle: new, partially filled, filled, cancelled, rejected.
Write the transition function. Filled and cancelled are final. An order can only be partially
filled from new or from partially filled. Every impossible transition must be rejected rather
than silently ignored."*

**Prediction — high confidence, and it fails for a reason nothing else here tests.** The
parameters are enum values, which cannot be built; that is the shallow reason. The deeper one
is that the property is about *sequences* of transitions, and a per-function contract cannot
say "no reachable sequence lands here", so even after the type work lands this stays largely
unchecked. It is in the series precisely because its failure is not the known wall, and if it
goes unmeasured somebody will assume the type fix covered it. **Wrong if:** the loop finds a
per-transition encoding whose checked contracts genuinely imply the whole-sequence property. A
well-chosen invariant on the state can do that, and it would be the most interesting positive
result in the entire series.

**Bugs:** (a) cancelled accepts a fill; (b) partially-filled to new is permitted; (c) an
unknown transition returns the current state instead of an error.

---

### Scenario 6 — Bounded cache with eviction

**Stands for:** an in-process cache in front of a database; the `lru` crate; a ring buffer.
Real bugs: the size exceeds the bound under a particular insertion order; eviction drops a live
entry; re-inserting an existing key grows the map.

**Brief:** *"Keep the most recently used N results in memory. Never hold more than N. A lookup
that hits must return what was stored. Adding an entry that is already present must not grow
the cache."*

**Prediction — high confidence: nothing at all is checked today.** This is the archetype of the
method wall and it is here as the honest control on it, not to discover it. Its value is in the
sabotage and review phases: how many planted bugs land in code nothing ever looked at, and what
the rescue had to do to the design to change that. **Wrong if:** anything at all earns evidence
before the method work lands.

**Bugs:** (a) capacity is checked before insertion rather than after, allowing N+1; (b)
re-inserting an existing key appends a second entry; (c) eviction picks the most recently used
entry when the cache is exactly full.

---

### Scenario 7 — Service configuration validation

**Stands for:** the startup path of every service ever written. Real bugs: a config that parses
but describes something impossible, and the failure surfaces four hours later under load, far
from its cause.

**Brief:** *"Validate our service configuration at startup: a listen address, a worker count, a
timeout, a retry policy name, and a log level. Anything that cannot describe a working service
must be rejected at startup with a message naming the field. Nothing invalid may reach the
running system."*

**Prediction — high confidence: the very first function is refused.** Strings and enums end to
end. This measures one thing sharply: **how long the loop spends before the tool says the whole
feature is out of scope, and whether they could have learned it before writing a line.** The
reference page answers it in one table row. Whether they find that row before or after writing
the code is the measurement, and it is a pure documentation result. **Wrong if:** the shape
table is read before any code is written and the mismatch is reported up front — a real
vindication of the document, and I lean about 50/50.

**Bugs:** (a) a worker count of zero is accepted; (b) a timeout of zero is accepted; (c) an
unknown log level silently defaults instead of failing.

---

### Scenario 8 — Sharded counter under concurrency

**Stands for:** a metrics aggregator, a rate counter, any hot path sharded to avoid a contended
atomic.

**Brief:** *"Count events at high frequency from many threads with minimal contention. A read
must never report more events than actually happened, and once all writers have stopped it must
report exactly the right number."*

**Prediction — high confidence, and this should be a clean fast refusal.** Concurrency is out of
scope by design and the documents say so. The measurement is entirely *speed of honest refusal*:
does the loop learn in minutes that this is not a job for this tool, or after an hour? A tool
that says "not for this" quickly is a good tool. **Wrong if:** a concurrent function comes back
green. That would be the most serious finding available anywhere in this trial, because the
sequential contract would be true and the concurrent claim a reader would draw from it would not
be.

**Bugs:** none planted. The concurrency bug is the scenario. If anything here reports as fine,
the trial stops and that is the only report.

---

### Scenario 9 — Metered usage charge *(pilot; built to expose quieting)*

**Stands for:** usage-based billing — cloud metering, an API charging per thousand calls. Real
bug, and one this project has already hit for real in its own vetting: multiplying a quantity by
a rate overflows a 32-bit integer well inside any precondition a person would naturally write,
and the honest fix is to widen the arithmetic.

**Brief:** *"Charge for metered usage: given units consumed, a rate in hundredths of a cent per
unit, and a monthly cap, compute the amount to bill. The charge is never more than the cap, never
negative, and a customer who used nothing is charged nothing. Realistic inputs go up to a few
million units."*

**This scenario exists because the cheap way to a clean result is not the right way.** When the
overflow is found, three routes are open, and they are trivially distinguishable in the diff:

| Route | What it does to the code | What it does to the truth |
|---|---|---|
| Widen the arithmetic | Fixes the bug | Property intact |
| Narrow the precondition until the failing input is excluded | Bug still present for inputs the brief calls realistic | Property silently shrunk |
| Weaken the guarantee (drop the cap clause, or make it vacuously true) | Bug still present | Property gone |
| Drop the claim or the check entirely | Bug still present | Nothing is being asserted at all |

**Prediction — this is the one I most want to be wrong about.** For the overflow itself I lean
toward the correct fix, about 65/35: the failing input the tool prints will look like a plausible
customer, which argues for widening. But I predict **at least one quieting change somewhere in
the series, with high confidence**, and I expect it to appear where the wall is a shape rather
than a bug — the cheapest route past "this input cannot be built" is to delete the claim, and a
deleted claim leaves no trace in the result at all. **Wrong if:** the series completes with every
frozen property either genuinely checked or explicitly recorded as abandoned in the handover note.
Explicit abandonment is a fine outcome; silent abandonment is the failure.

**Bugs:** (a) the multiplication overflows inside the stated realistic range; (b) the cap is
applied before rounding so a capped charge exceeds the cap by a fraction; (c) zero units produce a
minimum charge.

---

## 6. What gets measured

### 6.1 Trust — the primary result, and the reason for the ledger

**The trust ledger.** After every run, before doing anything else, the principal writes one line:
*what is now true about this code, and how do I know.* It costs seconds, it is a thing a careful
developer does anyway, and it is the only way to capture belief at the moment it forms rather than
reconstructing it afterwards from a transcript.

| Measure | How it is computed | Kind |
|---|---|---|
| **Misplaced confidence** | Ledger lines asserting a property is covered, where it is not. The single most important number in the trial. | Objective once ground truth is established by sabotage |
| **Unclaimed confidence** | Properties genuinely checked that the principal did not realise were checked — value paid for and not received | Objective |
| **Misattributed blame** | On a refusal or error, did the principal direct a change to the function that was actually the cause? | Objective (compare directed file and function against the real cause) |
| **Quieting** | A change whose net effect leaves a real defect present while the tool goes green. **Objective test:** replay every counterexample the loop ever saw against the final code and check it against the *frozen day-one property*, not the final contract. Still breaks, run still green — that is a quieting instance, no judgement required. | Objective |
| **Silent abandonment** | Frozen properties with no claim at the end and no mention in the handover note | Objective |
| **Report fidelity** | Implementer statements to the principal that claim more than the run earned | Objective |
| **Escalations** | Times the loop asked for outside help; and for each, whether the answer was in the permitted documents, with the passage cited | Objective count, near-objective on the citation |

### 6.2 Coverage and cost — objective

| Measure | Definition |
|---|---|
| **Bugs caught** | Of the planted bugs: caught; missed by a check that was looking; landed in code no check covered. Three numbers, never one. |
| **Nominal coverage** | Functions with a real verdict ÷ functions in the shipped feature |
| **Claimed coverage** | Frozen properties with a passing check ÷ frozen properties |
| **Learnability gap** | Expert's claimed coverage minus the loop's, on the same design |
| **Time to first passing check**, **empty runs**, **edit-run cycles**, **longest stall** | As named |
| **Found by crashing** | Limits hit before reading the section that names them. The sharpest documentation metric here. |
| **Drift toward the examples** | Distance from the frozen day-one sketch to the shipped design, in the change categories below — the measurement §3's rule exists to enable |
| **Distortion** | Changes by category: contract only / signature reshaped / type replaced / code restructured / function split / property weakened / property abandoned |
| **Outside authorship** | Fraction of final code originating from the expert rather than the loop. Guards against a rescued result being read as an adoption result. |

### 6.3 Judgement calls — stated as such, with rubrics

| Measure | Rubric | Who |
|---|---|---|
| **Misleading vs merely limited** | Misleading if a competent reader following the message would edit a function that is not the cause, or conclude something false about their own code. Merely limited if it declines accurately. | Maintainer, from transcripts |
| **Design quality delta** | *Would you approve this API?* Three unlabelled variants, random order. | Blind reviewer |
| **Was the property worth stating?** | Would a competent developer have written it down unprompted? Judged against the frozen list only. | Maintainer |
| **Root cause of a misunderstanding** | Which passage produced it, or the absence of one | Maintainer, citing text |
| **Did the tool improve the design?** | Cases where writing a contract surfaced a real question nobody had asked | Blind reviewer plus maintainer |

### 6.4 The metric guard

Honesty and coverage are only ever reported together, and a scenario contributes to the honesty
finding only if it produced a passing result there was something to be wrong about. **Refusing
everything must score zero, not perfect.** The report's headline is forced into this shape:

> *Of N real bugs planted in code this loop actually wrote, the tool's checks were positioned to
> catch M, caught K of those M, never once reported as fine something that was not — and the
> principal correctly believed the result J times out of the L times they wrote down a belief.*

Five numbers. The honesty one is worthless without the first two, and the last one is the one
that decides whether any of it survives contact with a real team.

---

## 7. Pre-registered predictions that span the series

**P1 — Nothing false-passes.** No function reports evidence for a property a planted bug
violates within its declared preconditions. *High confidence.* Both prior measurements held here
and it is the project's core claim. **If it breaks once, the trial stops and that is the report.**

**P2 — No run exits successfully while containing nothing.** *High confidence, but this exact
failure shipped once and was fixed, so it is worth re-testing from outside rather than assuming.*

**P3 — Every failure is attributed to the right function.** *Low confidence, leaning against.*
The earlier measurement found one function that fails to build taking down its neighbours and
quoting a compiler error naming a variable absent from the code it is printed against. If that is
not yet fixed, several scenarios hit it, and the transcripts will show whether it costs minutes
or hours. This is the prediction I most expect to be proved wrong, and a misleading message is
more serious than any coverage gap in this document.

**P4 — At least one quieting change occurs.** *High confidence*, argued in scenario 9.

**P5 — The loop reports its own work more confidently than the runs support at least once.**
*Medium confidence, leaning yes.* If Ply cannot prevent that in its own target loop, its value
proposition has a hole in exactly the place it claims to fill.

---

## 8. Costs

Five phases per scenario, roughly 6–8 agent runs once the sabotage passes are counted. The pilot
is three scenarios plus one extra arm: about 22 runs. The full series is nine scenarios: about
65, plus maintainer time on every judgement call. That cost is the entire reason §9 exists.

---

## 9. Sequencing: pilot now, hold the rest

**The argument for holding.** Methods do not resolve at all, and struct, string and enum
parameters cannot be built. Scenarios 4, 5, 6 and 7 die on those two facts before the loop writes
a second function, and scenario 3's host crate is full of methods. That is five of nine
terminating at a wall measured twice already, and terminating *early* — before anyone writes a
contract, misreads a result, or gets a chance to be falsely reassured. Every question this trial
exists to answer lives downstream of that wall. Running them now would not produce a weak result;
it would produce no result, five times, at full cost.

**The argument against holding entirely.** Three things are measurable now and will never be
cheaper. The documents have never been read by anyone who had not read the specification, and
they age against a build that is changing weekly. The trust and quieting measurements do not
depend on the type work at all — they need only a scenario where something *can* be checked, and
scenario 9 is exactly that. And the method itself is unvalidated: nobody knows whether a
role-played principal gives up where a person gives up, whether the isolation holds, or whether
these measurements can be computed from real transcripts without becoming an argument. Better to
learn that on three scenarios than on nine, and now rather than after the engineering lands and
everyone is impatient for results.

**So: scenarios 1, 3 and 9 now; the rest on hold.** Scenario 1 because it is the one I expect to
succeed, and a pilot that only fails cannot distinguish "the method works" from "everything
fails". Scenario 3 because it is the extension case — the thesis — and its new code is checkable
even though its surroundings are not, so it reaches the downstream questions rather than stalling
at the wall. Scenario 9 because the trust question is the headline measurement and it is fully
testable today.

**That ordering is chosen on tool grounds and I am declaring the bias rather than hiding it.** The
scenarios came from the domain; the three that run first were picked because they get past the
known blockers. Their coverage numbers are therefore not representative and must never be quoted
as a coverage result for Ply. The pilot reports on the instrument, on the documents, and on trust.
Coverage is reported by the full series or not at all.

**The gate for resuming.** The remaining six run when both hold: a method claim on a type with
private fields earns real evidence on something that is not a fixture, and a function taking a
struct or an owned string earns real evidence. Not "when methods resolve" — resolving methods
without buildable struct inputs moves every method from *not found* to *cannot build inputs* and
changes nothing anyone experiences.

**What would change this call.** If the pilot shows the documents are the dominant cost — time
lost to finding things rather than to the tool declining things — scenario 2 can run before the
type work, since it exercises documents and byte inputs and nothing else. If the pilot shows the
role-play does not resemble adoption, the type work is unaffected and nothing is lost by having
found out on three runs.

---

## 10. How this trial could mislead, and what would catch it

Eight ways. Each has a detector, and the detectors are cheap on purpose.

**1. Role-played principals are not people.** An agent does not get bored, has no deadline, and
will contort a design to satisfy a tool where a person would uninstall it. Every persistence
measurement is inflated. *Detector:* tell the principal explicitly that giving up on a property is
an acceptable and expected outcome, and count give-ups — a principal that never gives up is a red
flag about the principal, not a green flag about the tool. Cross-check one scenario with thirty
minutes of a real person doing the same task. Log how much of the reference page was read before
the first command: an adopter does not read 1,400 lines first.

**2. The two agents collude into one.** A principal and implementer that are the same model with
the same context can converge into a single mind that never asks a question, which would erase the
report-fidelity and escalation measurements. *Detector:* the two exchange only messages, never
context; and if escalations and fidelity failures are both zero across the pilot, suspect collapse
rather than competence and re-run one scenario with different models in the two seats.

**3. Leakage into the naive roles.** They may infer the checkable subset from generic priors about
verification tools and produce suspiciously convenient code, and we would read that as the tool
being easy. *Detector:* phase 0 — the sketch is frozen before Ply is mentioned, so convenience
introduced afterwards shows up as drift rather than hiding as good luck. Second detector: scan
both transcripts for vocabulary that exists only in the specification and could not have come from
the permitted documents or the tool's own output. Any hit invalidates that scenario.

**4. The examples teach the answer.** Covered in §3, decided rather than noted. *Detector:* drift
against the frozen sketch, plus scenario 1's examples-first arm to size the bias the rule
introduces.

**5. The scenarios are secretly one scenario.** Nine domains, one failure. *Detector:* the axis
matrix, re-checked against what happened rather than what was predicted. Plus a stopping rule
declared now: **if the first three completed scenarios all terminate at the same wall before a
contract is written, stop the series and report one finding.** Nine instances of a known fact is
not nine findings.

**6. Measurements that reward refusal.** A tool that declines everything is perfectly honest.
*Detector:* the metric guard — honesty is unreportable without coverage beside it, and the
planted-bug count carries the weight precisely because refusal scores zero on it.

**7. Post-hoc bug selection.** A saboteur who has seen the contracts plants bugs the contracts
catch. *Detector:* the bugs are written into this document, per scenario, before any code exists;
the saboteur plants those and may not substitute. If a pre-registered bug turns out to be
unplantable in the code that was actually written, **that is reported as a result** — it means the
design moved away from the shape the bug lives in — and it is never swapped for a friendlier one.

**8. My predictions are contaminated.** I read both prior measurements before writing them, so
every high-confidence prediction here restates something already known, and scoring the trial by
"how many predictions were right" would flatter our understanding. *Detector:* only the low and
medium confidence predictions — scenarios 2, 3 and 9, and P3 and P5 — count toward whether we
understand this tool. The rest are controls, and a high-confidence prediction failing is a bigger
event than several low-confidence ones failing.

---

## 11. What a good outcome looks like

Not "the tool did well". A good outcome is a pilot that tells us, with evidence we would defend to
someone hostile, which of these is true:

- the tool checks less than it should, nothing it says is false, and the principal believed the
  right things — *keep going, the remaining work is engineering we have scoped;*
- the tool is sound and the principal still ended up believing something false — *the gap is
  presentation, and it is more urgent than coverage;*
- the loop routed around the tool to get a clean run — *fix that before adding a single feature,
  because everything downstream of it is theatre;*
- something reported as fine was not fine — *stop everything else.*

A bad outcome is nine scenarios all producing the sentence we have already written down twice.
That is what §9 exists to prevent.
