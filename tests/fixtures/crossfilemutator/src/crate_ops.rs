/// The crate-root spelling: `crate::till::Till`, written from an ordinary
/// sibling module at the crate root.
impl crate::till::Till {
    pub fn crate_bump(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
