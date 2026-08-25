# Pre-registered region promises

**Written and committed before a single Kani run.** This file is the whole bias control:
the clauses below were written from each legacy module's *own* point of view — what its
owner could honestly say about all of it — not from which callers were about to fail.
`run.sh` re-checks the hashes at the end of every run, and `FINDINGS.md` reports whether
they still match.

A "region promise" here is exactly one thing: **one clause, written once, applied as the
postcondition of the stub generated for every function in the region.** Mechanically it is
a per-callee `ensures` that happens to be shared. Nothing else about the harnesses differs
from `tests/spike/havoc/`.

---

## Region A — `ledger::fees` (the `given004` legacy crate)

> **A1.** Every rate this schedule returns is a rate in basis points: at most 10,000,
> which is one hundred percent.

One clause. **Covers 1 function** (`bps_for_tier`) — see the limit recorded below.

### Why the region is `ledger::fees` and not `ledger`

`ledger` as a whole **cannot carry a single clause, and this is a type-level fact settled
before any run, not a verification outcome.** Its public surface returns `u64` (a sequence
number), `i64` (a balance, legitimately negative), `Vec<Entry>`, `Vec<AccountId>`, and
`u32` (a rate). There is no expression that is simultaneously a meaningful promise about a
signed balance, an unbounded counter and a heap vector. The largest honest region here is
the `fees` submodule, which today holds exactly one public function.

That is recorded **now**, before running, because it is the natural limit of the idea and
it must not look like a result that was discovered late.

## Region B — `catalog` (the `natural` legacy crate)

> **B1.** Nothing in this module returns a value above 1,000,000. Everything here is a
> configured operational number — a tax rate in basis points, a list price in cents, a
> count of bands or manifest lines, a batch size, an account spend limit — and the largest
> of them is an account's spend limit, which the finance team caps well below that.

One clause. **Covers 6 functions**: `vat_bps`, `unit_price_cents`, `band_count`,
`batch_size`, `manifest_lines`, `spend_limit_cents`. The clause text is identical in all
six stubs; `1_000_000` is a valid literal for both `u32` and `usize`, which is why one
clause spans the region's two return types.

### What was deliberately NOT promised, and why

An owner writing B1 is tempted to add **"and every count here is at least 1."** It is
false. `spend_limit_cents(0)` returns `0` for a brand-new account — the module's body says
so and its own unit test `a_new_account_has_no_limit` asserts it. A region promise is a
promise about *every* function in the region, so a clause that is true of five of them and
false of the sixth cannot be written. Refusing it here, in advance, is the honest move;
adding it later because a caller failed would be reverse-engineering.

### The ceiling question, settled in advance

`1_000_000` is a round number with headroom over the module's real maximum, which is what
a region-wide promise is written with — it has to hold for every function and survive next
quarter's price list. A reader may still object that a looser ceiling was chosen to make
the experiment fail. So a second, **tightest-still-true** variant is pre-registered here
and run as its own row:

> **B1-tight.** Nothing in this module returns a value above 500,000.

500,000 is `spend_limit_cents`'s own configured limit and therefore the module's actual
maximum today; **anything tighter than 500,000 is false of `catalog` as written.** If the
result at B1 and at B1-tight is the same, the ceiling was not the variable.

## The per-callee column (the route that already works, §5.5 branch two)

Written now, from each function's own meaning, so the comparison column is pre-registered
too and not tuned after the region rows land:

| callee | per-callee `ensures` |
|---|---|
| `vat_bps` | `result <= 10_000` (a rate in basis points) |
| `unit_price_cents` | `result <= 100_000` (a list price in cents, under EUR 1,000) |
| `band_count` | `result >= 1 && result <= 64` (there is always at least one band) |
| `batch_size` | `result >= 1 && result <= 10_000` (a batch holds at least one item) |
| `manifest_lines` | `result >= 1 && result <= 1_000` (a manifest has at least one line) |
| `spend_limit_cents` | `result <= 1_000_000` (a limit in cents; zero is a real value) |

That is **6 clauses for 6 functions**, and 4 of them say something B1 cannot: a lower
bound. Whether they buy anything B1 does not is what the run measures.

---

## Hash pins

Recorded before the first run. The two files that carry the sample are byte-for-byte the
ones `tests/spike/havoc/FINDINGS.md` pre-registered, so the comparison is like-for-like:

```
c0cf3136d6dcc095de2eb53d3417e31afad6c42764d104e83bf14fb2d25bbf4a  natural/feature/src/lib.rs
b109337d248e9c6c9c79cb6ba733fb2d6594f6c23826065970e3e26d99b95bac  natural/legacy/src/lib.rs
```

Both match the hashes on record in the havoc findings. `given004/` and every `Cargo.toml`
are also copied unchanged, with one exception: `natural/feature/Cargo.toml` gains a
`tight` feature so B1-tight can be run without editing the committed harness.
