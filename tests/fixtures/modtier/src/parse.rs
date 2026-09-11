//! Reads input. The design says it may use `shared`, and may not reach
//! into `exec` -- parsing is not allowed to start running things.

pub fn parse(input: &str) -> usize {
    let n = crate::shared::normalise(input);

    // THE VIOLATION. A direct call into `exec`, which the document forbids.
    crate::exec::run(n)
}
