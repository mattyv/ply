//! A promise about what a method *changes* must be checkable, and must
//! catch the bugs a rule about the whole value cannot see.
//!
//! Round 3 of the A/B vetting planted three bugs in this exact type. Only one
//! broke `available <= capacity`; the other two left it perfectly true, and
//! nothing in Ply could state them. Across two stateful scenarios four of six
//! bugs were that shape, which is the measured gap this closes.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

/// A refill of nothing that silently tops the bucket back up. It preserves
/// the whole-value rule exactly -- proved so in
/// `tests/spike/verus-component`, where the invariant proof passes at 6
/// verified, 0 errors with this bug present.
#[test]
fn a_refill_of_nothing_that_silently_refills_is_caught() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    let broken = src.replace(
        "        let room = self.capacity - self.available;",
        "        if tokens == 0 {\n            self.available = self.capacity;\n            return;\n        }\n        let room = self.capacity - self.available;",
    );
    assert_ne!(src, broken, "the refill body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    assert_eq!(
        run.json["root"]["verdict"], "violation",
        "`refill` promises to add exactly what was asked for, and a refill of nothing now \
         fills the bucket -- a whole-value rule cannot see this, which is why the promise \
         exists: {}",
        run.json
    );
}

/// A take that succeeds one token short. This one a whole-value rule can
/// reach, and the promise must reach it too -- a new instrument that loses
/// what the old one caught is not an improvement.
///
/// The planted bug is deliberately **panic-free**. The first version of this
/// test used `self.available + 1 >= tokens`, which underflows on the very
/// next line, and an adversarial review showed the test passed with
/// `try_take`'s promise replaced by `|result| true`: the panic was doing the
/// work, not the promise. That is the same mis-crediting the Verus spike
/// nearly made, landed. Hence the saturating arithmetic below, and hence the
/// assertions on *which* diagnostic and *which* node rather than on the root
/// verdict alone.
#[test]
fn a_take_that_succeeds_one_token_short_is_caught_by_the_promise() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    let broken = src
        .replace(
            "        if self.available >= tokens {",
            "        if self.available.saturating_add(1) >= tokens {",
        )
        .replace(
            "            self.available -= tokens;",
            "            self.available = self.available.saturating_sub(tokens);",
        );
    assert_ne!(src, broken, "the sufficiency test must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let diags = run.json["diagnostics"].as_array().unwrap();

    let broke = diags
        .iter()
        .find(|d| {
            d["node_id"] == "tokenbucket::TokenBucket::try_take"
                && d["title"].as_str().is_some_and(|t| {
                    t.contains("fails its own contract") || t.contains("postcondition")
                })
        })
        .unwrap_or_else(|| {
            panic!(
                "`try_take` promises it succeeds exactly when there are enough tokens, so the \
                 promise itself has to be what fails -- a panic reported as \"it never returned\" \
                 would mean this test is measuring the arithmetic and not the promise: {}",
                run.json
            )
        });
    assert!(
        !broke["title"]
            .as_str()
            .unwrap_or("")
            .contains("does not return at all"),
        "caught as a panic rather than a broken promise: {broke}"
    );

    // Bug isolation: `refill` is correct here, and must not be blamed. It
    // shares a receiver with the buggy `try_take` in every generated
    // sequence, which is exactly how a wrong name gets onto a report.
    assert!(
        !diags.iter().any(|d| {
            d["node_id"] == "tokenbucket::TokenBucket::refill" && d["severity"] == "error"
        }),
        "`refill` is untouched and must not be reported as failing: {}",
        run.json
    );
}

/// The honest half: the untouched fixture earns its evidence rather than
/// reporting a refusal. Without this, both tests above would pass against a
/// tool that refused every claim.
#[test]
fn the_untouched_bucket_earns_evidence_for_both_of_its_mutators() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");
    let run = run_verify(&cargo_ply, fixture.path(), 300);

    assert_eq!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "a method that changes something has to be checkable, not refused: {}",
        run.json
    );
}

/// The false clean this refusal closes, reproduced end to end (2026-09-07).
///
/// A promise that takes its "before" reading through a method which itself
/// changes the bucket is evaluated by running that reading first -- so the
/// reading resets the very state the promise is about, and the planted
/// refill-of-nothing bug becomes unreachable. Before the refusal this run
/// came back `fuzzed(256)` with no diagnostic at all: a clean result over a
/// broken bucket.
///
/// The bug is planted as well as the promise rewritten on purpose. Refusing
/// a promise that happened to be over correct code proves much less than
/// refusing one that was about to certify a real bug as fine.
#[test]
fn a_promise_that_reads_through_a_mutating_method_is_refused_not_reported_clean() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    let rigged = src
        // A reading that also empties the bucket -- the shape a real cache
        // `get` (touching recency) or a `level_and_reset` gauge has.
        .replace(
            "    pub fn capacity(&self) -> u32 {",
            "    pub fn take_reading(&mut self) -> u32 {\n        let n = self.available;\n        self.available = 0;\n        n\n    }\n\n    pub fn capacity(&self) -> u32 {",
        )
        // `refill`'s promise now takes its before-reading through it.
        .replace(
            "        == old(self.available()).saturating_add(tokens).min(old(self.capacity())))]",
            "        == old(self.take_reading()).saturating_add(tokens).min(old(self.capacity())))]",
        )
        // ... over a bucket that silently refills to the top on a refill of
        // nothing: the round-3 bug, live.
        .replace(
            "        let room = self.capacity - self.available;",
            "        if tokens == 0 {\n            self.available = self.capacity;\n            return;\n        }\n        let room = self.capacity - self.available;",
        );
    assert_ne!(src, rigged, "the fixture must have been rewritten");
    fixture.write_lib_rs(&rigged);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let diags = run.json["diagnostics"].as_array().unwrap();

    let refusal = diags
        .iter()
        .find(|d| d["node_id"] == "tokenbucket::TokenBucket::refill")
        .unwrap_or_else(|| {
            panic!(
                "a promise that changes what it reads has to be refused by name -- with no \
                 diagnostic at all this is the false clean itself, a broken bucket reported as \
                 checked: {}",
                run.json
            )
        });
    assert_eq!(
        refusal["title"].as_str().unwrap(),
        "Ply cannot check `refill`: its promise calls `take_reading`, and that method changes \
         the `TokenBucket` it is called on. Ply works out what a reading was before the call by \
         running that reading first -- so a reading taken through a method that changes the \
         value would alter the very thing the promise is about, and the check could come back \
         clean while the bug it was written to catch is still there. Use a method that only \
         reads (one taking `&self`) in the promise, or add one.",
        "the sentence a reader sees is reviewed like code: {refusal}"
    );
    assert_ne!(
        run.json["root"]["verdict"], "fuzzed(256)",
        "the run must not report evidence it did not earn: {}",
        run.json
    );
}
