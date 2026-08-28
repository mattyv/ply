/// The parent-module spelling: `super::Till`, written from `till`'s own
/// submodule -- exactly what a real crate writes when it splits a type's
/// operations into a submodule of the type's own module. This is the
/// spelling the coordinator's own review reproduced as still green after
/// the plain cross-file fix: this scan used to resolve only a bare,
/// single-segment `impl Till`, and `super::Till` (two segments) fell
/// straight through as "not this type" without ever being disclosed.
impl super::Till {
    pub fn super_bump(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
