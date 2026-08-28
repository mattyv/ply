pub mod super_ops;

pub struct Till {
    pub(crate) total: u32,
}

impl Till {
    pub fn new() -> Self {
        Till { total: 0 }
    }

    /// FALSE after any one of this file's, or any other file's, mutating
    /// operations runs.
    #[ply::ensures(|result| *result == 0)]
    pub fn total(&self) -> u32 {
        self.total
    }
}

/// The same-module spelling: `self::Till`, written in the very file `Till`
/// is declared in, right alongside its own inherent `impl Till` block
/// above.
impl self::Till {
    pub fn self_bump(&mut self, cents: u32) -> u32 {
        self.total = self.total.saturating_add(cents);
        self.total
    }
}
