/// Reached only through a local `use ... as` rename -- `T` is never
/// `Till`'s own name anywhere in this file, so a plain identifier
/// comparison against `type_name` cannot see this one at all without
/// resolving the alias back to what it actually names.
use crate::till::Till as T;

impl T {
    pub fn alias_bump(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
