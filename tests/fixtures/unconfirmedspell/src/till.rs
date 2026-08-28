pub mod ops;

pub struct Till {
    pub(crate) total: u32,
}

impl Till {
    pub fn new() -> Self {
        Till { total: 0 }
    }

    #[ply::ensures(|result| *result == 0)]
    pub fn total(&self) -> u32 {
        self.total
    }
}
