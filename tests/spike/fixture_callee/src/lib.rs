//! Sibling crate for item 5 (cross-crate stub_verified). Throwaway spike fixture.

#[cfg_attr(kani, kani::requires(x < 1000))]
#[cfg_attr(kani, kani::ensures(|result| *result == x + 1))]
pub fn g_remote(x: u32) -> u32 {
    x + 1
}

#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof_for_contract(g_remote)]
    fn check_g_remote() {
        let x: u32 = kani::any();
        g_remote(x);
    }
}
