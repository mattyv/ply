//! Real-world reproduction (2026-09-01, verified by hand against `semver`'s
//! `Version::cmp_precedence`): Ply's own refusal for a shape `fuzz`/
//! `bounded` cannot build tells the user, verbatim, to "declare `test`
//! instead, with an `examples:` entry, to run the concrete case directly".
//! Doing exactly that on a *method* -- `Weight::matches` below, taking
//! `&Self` the same way `cmp_precedence` does -- used to fail to compile
//! at all: the generated test's own name spliced the checked function's
//! `::`-qualified path in verbatim, which is not a legal Rust identifier.
//! Every fixture exercising this codegen before this one used a free
//! function, whose path has no `::` to go wrong -- and nearly everything in
//! a real library is a method.

pub struct Weight(pub u32);

impl Weight {
    pub fn new(v: u32) -> Self {
        Weight(v)
    }

    /// Real body: two weights match exactly when their inner values are
    /// equal.
    #[ply::ensures(|result| *result == (self.0 == other.0))]
    pub fn matches(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
