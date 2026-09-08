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

/// A precondition that reads the value the method is called on has to work
/// in a real run, not merely appear in the generated text (2026-09-08).
///
/// It did not: the filter was written out above the line that builds the
/// value, so the generated check named something that did not exist yet and
/// died as a compiler error. A test that only inspected the generated source
/// would not have caught that, which is why this one runs the tool.
#[test]
fn a_precondition_that_reads_the_bucket_is_checked_rather_than_failing_to_compile() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    // "only check a take when there is something in the bucket" -- the
    // ordinary thing a caller writes, and the shape that used to be
    // unwritable.
    let gated = src.replace(
        "    #[ply::ensures(|result| *result == (old(self.available()) >= tokens))]",
        "    #[ply::requires(self.available() > 0)]\n    #[ply::ensures(|result| *result == (old(self.available()) >= tokens))]",
    );
    assert_ne!(src, gated, "the precondition must have been added");
    fixture.write_lib_rs(&gated);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let node = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "TokenBucket::try_take")
        .unwrap_or_else(|| panic!("`try_take` must still be in the report: {}", run.json));

    assert_eq!(
        node["verdict"], "fuzzed(256)",
        "a precondition that reads the bucket must leave the method checked -- a refusal or a \
         build failure here is the defect this closes: {}",
        run.json
    );
}

/// The honest other half: the precondition must actually *reject* cases,
/// rather than being quietly dropped on the floor. A filter nobody applies
/// would pass the test above while doing nothing.
///
/// A precondition no case can satisfy has to end the run with Ply saying so,
/// not with evidence it did not earn.
#[test]
fn a_precondition_no_case_can_satisfy_is_reported_rather_than_earning_evidence() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    // A bucket never holds more than its capacity, so this is false for
    // every value the constructor can build.
    let impossible = src.replace(
        "    #[ply::ensures(|result| *result == (old(self.available()) >= tokens))]",
        "    #[ply::requires(self.available() > self.capacity())]\n    #[ply::ensures(|result| *result == (old(self.available()) >= tokens))]",
    );
    assert_ne!(src, impossible, "the precondition must have been added");
    fixture.write_lib_rs(&impossible);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let node = run.json["root"]["children"][0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "TokenBucket::try_take")
        .unwrap_or_else(|| panic!("`try_take` must still be in the report: {}", run.json));

    assert_eq!(
        node["verdict"], "unclaimed",
        "no case reached `try_take` at all, so reporting 256 cases' worth of evidence would be \
         a false clean -- the filter has to be doing something: {}",
        run.json
    );
    assert_eq!(
        node["evidence"]["cases"], 0,
        "and the recorded case count has to be the real one: {}",
        run.json
    );
    let said_so = run.json["diagnostics"].as_array().unwrap().iter().any(|d| {
        d["node_id"] == "tokenbucket::TokenBucket::try_take"
            && d["title"].as_str().is_some_and(|t| {
                t.contains("thrown away by the function's own `#[ply::requires]` precondition")
                    && t.contains("no fuzz evidence at all")
            })
    });
    assert!(
        said_so,
        "and Ply has to say why the run came back with nothing, naming the precondition as what \
         threw every input away -- a weaker verdict with no explanation leaves the reader to \
         guess: {}",
        run.json
    );
}

/// The honesty gap this closes (2026-09-08): "Ply never reports a broken
/// promise it cannot show you the input for" was untrue for a method that
/// changes the value it is called on.
///
/// The report named the failing call's arguments and said nothing about how
/// the value got into the state where the call broke its promise -- which,
/// for this kind of promise, is most of the input. The constructor call and
/// the sequence of operations were both right there in the generated test
/// and simply never printed.
#[test]
fn a_broken_transition_promise_shows_how_the_value_reached_the_failing_state() {
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
    let diag = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["node_id"] == "tokenbucket::TokenBucket::refill" && d["counterexample"].is_object()
        })
        .unwrap_or_else(|| {
            panic!(
                "the broken refill must be reported with a counterexample: {}",
                run.json
            )
        });

    let history = diag["counterexample"]["receiver_history"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "a promise about what a call changed is broken by a *history*, not only by the \
                 call's own arguments -- without it the report cannot show the input it says it \
                 always shows: {diag}"
            )
        });
    // An independent oracle, not a shape check (2026-09-08). An adversarial
    // review planted a bug that reported every argument as `0`, and the
    // previous version of this test -- which asserted the history started
    // with the constructor, named some calls, and contained a digit --
    // passed. A history is a claim about how the value got somewhere, so
    // the honest assertion is to follow it and see where it lands.
    //
    // `refill`'s promise is broken by the planted bug only on a bucket that
    // is not already full: a full bucket refilled by nothing stays full and
    // the promise holds. So a history that replays to a full bucket cannot
    // be the history of this failure, whatever shape it has. Under that
    // planted all-zeroes bug the recipe reads `new(0), then try_take(0)`,
    // which replays to a full bucket -- and fails here.
    assert!(
        history.starts_with("TokenBucket::new("),
        "the history has to start where the value did: {history}"
    );
    let (mut capacity, mut available) = (None::<u64>, 0u64);
    for call in history.split(", then ") {
        let (name, rest) = call
            .split_once('(')
            .unwrap_or_else(|| panic!("every step must be a call: {history}"));
        let arg = rest.trim_end_matches(')');
        let n: Option<u64> = if arg.is_empty() {
            None
        } else {
            arg.parse().ok()
        };
        match (name, n) {
            ("TokenBucket::new", Some(c)) => {
                capacity = Some(c);
                available = c;
            }
            ("TokenBucket::try_take", Some(k)) => {
                if available >= k {
                    available -= k;
                }
            }
            ("TokenBucket::refill", Some(k)) => {
                let cap = capacity.expect("the constructor comes first");
                available = available.saturating_add(k).min(cap);
            }
            ("TokenBucket::available", None) | ("TokenBucket::capacity", None) => {}
            _ => panic!("the history names a call this oracle does not know: {call} in {history}"),
        }
    }
    let capacity = capacity.expect("checked above that the constructor comes first");
    assert!(
        available < capacity,
        "replaying the reported history leaves a full bucket ({available} of {capacity}), and a \
         full bucket refilled by nothing keeps its promise -- so this cannot be how the reported \
         failure happened. A recipe that does not reproduce the failure is worse than no \
         recipe: {history}"
    );
}

/// The same refusal, with the offending clause moved into `ply.yaml`
/// (2026-09-08, external review of `7820a4b`).
///
/// The check ran while Ply read the method's own attributes. A document's
/// clauses are merged in afterwards, by a different caller, and nothing
/// re-checked them -- so writing `old(self.take_reading())` in the document
/// instead of the source walked straight past the refusal and produced the
/// clean-result-over-a-real-bug the refusal exists to stop. The bug is
/// planted here as well, so what this pins is a refusal of a promise that
/// was about to certify a broken bucket.
#[test]
fn a_promise_written_in_the_document_is_refused_the_same_way_as_one_in_the_source() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    let rigged = src
        .replace(
            "    pub fn capacity(&self) -> u32 {",
            "    pub fn take_reading(&mut self) -> u32 {\n        let n = self.available;\n        self.available = 0;\n        n\n    }\n\n    pub fn capacity(&self) -> u32 {",
        )
        // `refill` keeps only promises that say nothing, so the document's
        // clause is the whole contract and cannot be refused by proxy.
        .replace(
            "    #[ply::ensures(|result| self.available()\n        == old(self.available()).saturating_add(tokens).min(old(self.capacity())))]\n    #[ply::ensures(|result| self.capacity() == old(self.capacity()))]\n    pub fn refill",
            "    pub fn refill",
        )
        .replace(
            "        let room = self.capacity - self.available;",
            "        if tokens == 0 {\n            self.available = self.capacity;\n            return;\n        }\n        let room = self.capacity - self.available;",
        );
    assert_ne!(src, rigged, "the fixture must have been rewritten");
    fixture.write_lib_rs(&rigged);

    let yaml_path = fixture.path().join("ply.yaml");
    let yaml = std::fs::read_to_string(&yaml_path).unwrap();
    let with_clause = yaml.replace(
        "      TokenBucket::refill:\n        checks: [fuzz(256)]",
        "      TokenBucket::refill:\n        checks: [fuzz(256)]\n        ensures:\n          - \"|result| self.available() == old(self.take_reading()).saturating_add(tokens).min(old(self.capacity()))\"",
    );
    assert_ne!(
        yaml, with_clause,
        "the document must have gained the clause"
    );
    std::fs::write(&yaml_path, &with_clause).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let refusal = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["node_id"] == "tokenbucket::TokenBucket::refill"
                && d["title"]
                    .as_str()
                    .is_some_and(|t| t.contains("calls `take_reading`"))
        })
        .unwrap_or_else(|| {
            panic!(
                "a promise that changes what it reads has to be refused wherever it was \
                 written -- letting the document say what the source may not is the same \
                 false clean with an extra step: {}",
                run.json
            )
        });
    assert!(
        refusal["title"]
            .as_str()
            .unwrap()
            .contains("changes the `TokenBucket` it is called on"),
        "and it has to give the same reason: {refusal}"
    );
}

/// The gap left open when the history landed, now closed (2026-09-08).
///
/// The line carrying the history is written after the checked call returns,
/// so a call that *crashes* wrote none -- and that is the case where it is
/// worth most, because a crash leaves the reader with the raw generated
/// witness (`4, [(3,0,(),(),1)], 5`) and nothing else. The report said
/// which arguments crashed it and stayed silent about how the value got
/// into the state where they did.
#[test]
fn a_call_that_crashes_still_shows_how_the_value_reached_that_state() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("tokenbucket");

    let src = fixture.read_lib_rs();
    // Subtracting from the capacity underflows once anything has been
    // taken, so the crash needs a history to explain it: on a fresh bucket
    // `available == capacity` and nothing goes wrong.
    let broken = src.replace(
        "        let room = self.capacity - self.available;",
        "        let room = self.available - self.capacity;",
    );
    assert_ne!(src, broken, "the refill body must have been rewritten");
    fixture.write_lib_rs(&broken);

    let run = run_verify(&cargo_ply, fixture.path(), 300);
    let diag = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| {
            d["node_id"] == "tokenbucket::TokenBucket::refill" && d["counterexample"].is_object()
        })
        .unwrap_or_else(|| panic!("the crash must be reported with a witness: {}", run.json));

    let history = diag["counterexample"]["receiver_history"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "a crash is the case where the reader has least to go on, so it is the case \
                 that most needs the recipe -- and it was the one case that had none: {diag}"
            )
        });
    assert!(
        history.starts_with("TokenBucket::new("),
        "the history has to start where the value did: {history}"
    );
    assert!(
        history.contains(", then TokenBucket::"),
        "a fresh bucket cannot crash this, so the history has to name what happened to it \
         first, or it does not explain the crash: {history}"
    );
}
