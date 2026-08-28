pub struct Till {
    pub(crate) total: u32,
}

impl Till {
    pub fn new() -> Self {
        Till { total: 0 }
    }

    /// FALSE after a single `take` call -- and `take` lives in `more.rs`,
    /// not here.
    #[ply::ensures(|result| *result == 0)]
    pub fn total(&self) -> u32 {
        self.total
    }
}
