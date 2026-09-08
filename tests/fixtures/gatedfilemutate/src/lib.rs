pub mod pipeline;

// This unrelated impl makes the fingerprint conservatively cover the whole
// crate. It must not erase the function/file identities mutation selection
// can still resolve.
pub struct Counter;

impl Counter {
    pub fn current(&self) -> u32 {
        0
    }
}
