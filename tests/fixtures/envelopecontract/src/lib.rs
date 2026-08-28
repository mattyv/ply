//! A claim carrying both halves of a contract -- one clause declared in
//! `ply.yaml`, one written inline -- plus an attested claim no engine
//! checks. All three have to reach the machine-readable envelope, which is
//! the channel an agent reads.

#[ply::ensures(|result| *result >= a)]
pub fn add(a: u32, b: u32) -> u32 {
    a.saturating_add(b)
}
