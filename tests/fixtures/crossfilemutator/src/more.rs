use crate::till::Till;

/// A second, ordinary `impl Till` block in a file other than the one
/// `Till` is declared in -- the only place `Till`'s state ever changes.
impl Till {
    pub fn take(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
