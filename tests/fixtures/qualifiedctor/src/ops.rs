//! `Quota`'s own constructor and one method, in the only spelling this file
//! can use for the type. The constructor returns the type by name rather
//! than `Self` -- ordinary Rust, and the second spelling that used to make
//! a perfectly usable constructor invisible.

impl super::Quota {
    /// The private field is never zero, which is what both the free
    /// function and the method below promise.
    pub fn new(per_second: u32) -> super::Quota {
        super::Quota {
            per_second: per_second.max(1),
        }
    }

    /// Needs a receiver, so Ply has to build a `Quota` before it can check
    /// anything about this at all.
    #[ply::ensures(|result| *result >= 1)]
    pub fn burst_ceiling(&self) -> u32 {
        self.per_second
    }
}
