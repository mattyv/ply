//! Also-fix, smaller (task 2026-08-27, docs/review-strings-receivers.md,
//! "a proof refused on a method blames the u32 return type instead of the
//! receiver"): `Gauge::level` takes only `&self` and returns a plain `u32`
//! -- both perfectly fine for `bounded`. The real reason `bounded` refuses
//! it is that Ply's exhaustive (Kani) tier has no receiver-construction
//! support at all; only the sampling tier does. The refusal must name that,
//! never blame `u32`.

pub struct Gauge {
    pub n: u32,
}

impl Gauge {
    pub fn new(n: u32) -> Self {
        Gauge { n }
    }

    #[ply::ensures(|result| *result == *result)]
    pub fn level(&self) -> u32 {
        self.n
    }
}
