//! Runs things. May use `shared`; may not reach back into `parse`.

pub fn run(n: usize) -> usize {
    crate::shared::normalise("x") + n
}
