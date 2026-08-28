/// The plain, bare spelling after an explicit `use` -- the shape the
/// original fix (docs/review-silent-narrowing.md finding 1) already
/// covers. Kept here, alongside the four qualified/aliased spellings, so a
/// future change to the qualified-path resolution cannot silently regress
/// this one out from under the others.
use crate::till::Till;

impl Till {
    pub fn bare_bump(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
