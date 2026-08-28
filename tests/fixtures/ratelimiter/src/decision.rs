//! §2.3 Decisions — the outcome of asking a limiter "can this go through?".

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    /// The request is admitted now. `remaining` is how many tokens were
    /// left in the bucket immediately after this request was debited.
    Allowed { remaining: u32 },
    /// Not admitted right now. Waiting at least `retry_after` and then
    /// retrying the same request (assuming nobody else spends tokens from
    /// this bucket in the meantime) would succeed.
    Denied { retry_after: Duration },
    /// The request asked for more tokens than the bucket can ever hold at
    /// full capacity. No amount of waiting makes this request satisfiable;
    /// it is a caller bug (or a cost model mismatch), not a rate-limit
    /// event, and callers should not retry it unchanged.
    Unsatisfiable { capacity: u32, requested: u32 },
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allowed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_allowed_true_only_for_allowed() {
        assert!(Decision::Allowed { remaining: 3 }.is_allowed());
        assert!(!Decision::Denied { retry_after: Duration::ZERO }.is_allowed());
        assert!(!Decision::Unsatisfiable { capacity: 1, requested: 2 }.is_allowed());
    }
}
