//! A correct function must never be reported as breaking its promise
//! because Ply's own generator could not reach it.
//!
//! The guard added on 2026-09-05 was right about the gap it closed: each
//! generated case returns early when the precondition rejects it, Rust
//! reports an early return as a passing test, and passing tests were
//! counted as executed cases -- so a precondition no generated value
//! satisfies earned `tested` on a function never called once.
//!
//! The way it closed that gap was wrong. It asserted inside a generated
//! test, and `verify` reads a failing generated test as a broken contract,
//! so an obviously-correct function came back as "a real, reproduced
//! violation, not a probabilistic one". It also told the reader to add a
//! worked example -- advice that changed nothing, because the check counted
//! only generated boundary values and never looked at examples. Reported by
//! external review 2026-09-06 and reproduced exactly as described.
//!
//! Both halves are pinned here: no violation, and an example that satisfies
//! the precondition really does earn the evidence the message promises.

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn find_fn<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    if node["kind"] == "fn" && node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|c| find_fn(c, id))
}

#[test]
fn a_correct_function_the_generator_cannot_reach_is_not_called_a_violation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("narrowprecond");

    let run = run_verify(&cargo_ply, fixture.path(), 120);

    let unreached = find_fn(&run.json["root"], "only_at_42")
        .unwrap_or_else(|| panic!("no node for only_at_42 in {}", run.json));
    assert_ne!(
        unreached["verdict"], "violation",
        "`only_at_42` returns exactly what it promises. Ply not being able to generate an \
         input that satisfies `x == 42` is a fact about Ply, and reporting it as a broken \
         promise accuses the author's code of something untrue: {}",
        run.json
    );
    assert_eq!(
        unreached["verdict"], "unclaimed",
        "nothing reached the function, so it earned nothing -- which is neither a pass nor a \
         failure but an absence, and the evidence order already has a word for that: {}",
        run.json
    );

    // The absence has to be visible from the top, or a reader is told
    // everything is fine about a promise nobody checked.
    let root_statuses: Vec<&str> = run.json["root"]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(
        root_statuses.contains(&"inconclusive"),
        "the workspace must carry a flag saying a check in it reached no conclusion, statuses \
         were {root_statuses:?}: {}",
        run.json
    );

    // No error-severity diagnostic *about the two correct functions*. The
    // third one in this fixture promises something false on purpose and
    // must produce exactly such a diagnostic, so this is scoped by node
    // rather than counting the whole run.
    let errors: Vec<&serde_json::Value> = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["severity"] == "error")
        .filter(|d| d["node_id"] != "narrowprecond::broken_but_exampled")
        .collect();
    assert!(
        errors.is_empty(),
        "a function nobody could reach is an absence of evidence, not an error: {errors:?}"
    );

    // And the reader is told what actually happened, in words that name the
    // real cause rather than a generated test's name.
    let w = run.json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "W0542")
        .unwrap_or_else(|| panic!("no W0542 in {}", run.json));
    let title = w["title"].as_str().unwrap();
    assert_eq!(
        title,
        "`only_at_42` was never called, so its promise has not been checked on a single input. \
         Ply builds test inputs by trying boundary values for each parameter -- for `x: u32` \
         that is 0, 1, small numbers and the maximum -- and its precondition `x == 42` rejects \
         every one of them. Nothing here is broken and nothing here is proven. Add an \
         `examples:` entry that satisfies the precondition and the promise gets checked on \
         that input. (W0542)",
        "the sentence a reader gets must name the cause and give advice that works (W0542)"
    );

    let paired = find_fn(&run.json["root"], "only_at_42_and_true")
        .unwrap_or_else(|| panic!("no node for only_at_42_and_true in {}", run.json));
    assert_eq!(
        paired["verdict"], "tested",
        "the example supplies the only input that satisfies `x == 42 && flag`, so it must \
         reach the body and the promise must be checked on it: {}",
        run.json
    );

    // A passing example is not a contract check. `broken_but_exampled`
    // promises zero and returns one; its example asserts only that the call
    // returns one, which is true. Between the morning fix and the evening
    // one this reported `tested`, exit 0 -- evidence that lies, and the one
    // thing this tool exists not to do. The example's input now feeds the
    // generated contract cases, so the promise is asserted at 42 and fails.
    let lying = find_fn(&run.json["root"], "broken_but_exampled")
        .unwrap_or_else(|| panic!("no node for broken_but_exampled in {}", run.json));
    assert_eq!(
        lying["verdict"], "violation",
        "the promise is broken at the only input anything reaches, and a passing example \
         says nothing about the promise -- reporting anything but a violation here is \
         evidence that lies: {}",
        run.json
    );

    // The advice the message gives has to be advice that works: the same
    // function with a satisfying example earns real evidence.
    let reached = find_fn(&run.json["root"], "only_at_42_with_example")
        .unwrap_or_else(|| panic!("no node for only_at_42_with_example in {}", run.json));
    assert_eq!(
        reached["verdict"], "tested",
        "a worked example that satisfies the precondition does call the function, so the \
         promise really was checked on a real input and must earn evidence -- otherwise the \
         message above is telling the reader to do something that does not help: {}",
        run.json
    );
}
