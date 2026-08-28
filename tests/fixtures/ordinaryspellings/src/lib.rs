//! Two more ordinary ways to write Rust that Ply used to refuse, each with
//! the same false sentence: "it has no constructor Ply can call", about a
//! type whose public `new` is a few lines away.
//!
//! `Alpha`'s `impl` block is written inside an inline `mod` in this file.
//! The scan walked only the file's top-level items, so the block was never
//! looked at. The module path has to travel with it: `super::Alpha` inside
//! `inner` means this file's `Alpha`, and resolving it against the file's
//! own module instead would land somewhere else.
//!
//! `beta_qualified`'s parameter is spelled `crate::Beta` rather than
//! `Beta`. Ply looks a user type up by its bare name, and the qualified
//! spelling was being carried around as the rendering of a token stream --
//! `crate :: Beta`, spaces and all -- which no lookup could match and no
//! sentence should quote at a reader.

pub struct Alpha {
    n: u32,
}

pub struct Beta {
    n: u32,
}

pub mod inner {
    impl super::Alpha {
        pub fn new(n: u32) -> Self {
            super::Alpha { n: n.max(1) }
        }
    }
}

impl Beta {
    pub fn new(n: u32) -> Self {
        Beta { n: n.max(1) }
    }
}

#[ply::ensures(|result| *result >= 1)]
pub fn alpha_inline_mod(v: Alpha) -> u32 {
    v.n
}

#[ply::ensures(|result| *result >= 1)]
pub fn beta_qualified(v: crate::Beta) -> u32 {
    v.n
}
