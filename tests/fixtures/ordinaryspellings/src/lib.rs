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

// --- The qualified spelling must not be trimmed away -------------------------
//
// The first attempt at the `crate::Beta` fix trimmed every path to its last
// segment before looking the type up. That made a parameter naming *another*
// crate's type resolve to a local type of the same name, build the wrong
// thing, and report a compile failure in Ply's own generated code -- a calm
// refusal turned into an internal error. Here the parameter is
// `std::net::Ipv4Addr` while this crate declares its own `Ipv4Addr`: the two
// share a last segment and are entirely different types.

/// This crate's own `Ipv4Addr`, which is not the one in the signature below.
pub struct Ipv4Addr {
    n: u32,
}

impl Ipv4Addr {
    pub fn new(n: u32) -> Self {
        Ipv4Addr { n: n.max(1) }
    }

    pub fn get(&self) -> u32 {
        self.n
    }
}

/// Refused, and refused naming the path as written. Building the local
/// `Ipv4Addr` here would be building a different type than the one asked for.
#[ply::ensures(|result| *result >= 0)]
pub fn foreign_shaped_name(v: std::net::Ipv4Addr) -> u32 {
    v.octets()[0] as u32
}
