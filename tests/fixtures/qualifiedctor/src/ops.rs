//! `Quota`'s own constructor, in the only spelling this file can use.

impl super::Quota {
    /// The private field is never zero, which is what `refill_per_second`
    /// promises about every value anyone can construct.
    pub fn new(per_second: u32) -> Self {
        super::Quota {
            per_second: per_second.max(1),
        }
    }
}
