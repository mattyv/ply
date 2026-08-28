/// Ordinary Rust: from inside `till`'s own submodule, `super::Till` means
/// `till::Till`. But the crate root has its *own*, unrelated `struct Till`
/// too, so the bare name `Till` is ambiguous crate-wide -- Ply cannot
/// confirm this really is `till::Till` rather than the crate root's, and
/// must not guess.
impl super::Till {
    pub fn take(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
