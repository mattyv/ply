//! The invariant test CLAUDE.md asks for (see `tools/render/tests/render.rs`'s
//! `every_painted_element_resolves_a_style_rule`): walk the shapes the
//! composition grammar (TODO.md, "make the sampling engine's decision
//! recursive", 2026-09-02) admits, to a small bound, and require every one
//! to yield something that actually compiles and runs -- never just a
//! plausible-looking generated file. `compositiongrammar_fixture.rs` is the
//! walk; every function below is one shape, depth <= 2, mixing `Option`,
//! `Result`, a list, a set, a map, a fixed array, a slice, a tuple, `Box`,
//! and a nested user struct over the two base leaves (`u32`, `String`) this
//! grammar already builds alone -- so a construct added to the grammar
//! later that quietly breaks one combination cannot pass silently: the walk
//! fails on the first function that does not earn a real verdict.
//!
//! Every contract here is a trivial, always-true `*result >= 0` -- this
//! fixture is about *compiling and running*, not about finding a bug (see
//! `compositionbites_fixture.rs`/`paramseeded_fixture.rs` for that half).

pub struct Leaf {
    n: u32,
}

impl Leaf {
    pub fn new(n: u32) -> Self {
        Leaf { n }
    }
}

#[ply::ensures(|result| *result >= 0)]
pub fn option_u32(x: Option<u32>) -> i64 {
    x.map(|v| v as i64).unwrap_or(0)
}

#[ply::ensures(|result| *result >= 0)]
pub fn option_string(x: Option<String>) -> i64 {
    x.map(|s| s.len() as i64).unwrap_or(0)
}

#[ply::ensures(|result| *result >= 0)]
pub fn option_option_u32(x: Option<Option<u32>>) -> i64 {
    x.flatten().map(|v| v as i64).unwrap_or(0)
}

#[ply::ensures(|result| *result >= 0)]
pub fn option_vec_u32(x: Option<Vec<u32>>) -> i64 {
    x.map(|v| v.len() as i64).unwrap_or(0)
}

#[ply::ensures(|result| *result >= 0)]
pub fn option_leaf(x: Option<Leaf>) -> i64 {
    x.map(|l| l.n as i64).unwrap_or(0)
}

#[ply::ensures(|result| *result >= 0)]
pub fn result_u32_string(x: Result<u32, String>) -> i64 {
    match x {
        Ok(v) => v as i64,
        Err(s) => s.len() as i64,
    }
}

#[ply::ensures(|result| *result >= 0)]
pub fn result_string_u32(x: Result<String, u32>) -> i64 {
    match x {
        Ok(s) => s.len() as i64,
        Err(v) => v as i64,
    }
}

#[ply::ensures(|result| *result >= 0)]
pub fn vec_u32(x: Vec<u32>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn vec_string(x: Vec<String>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn vec_option_u32(x: Vec<Option<u32>>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn vec_vec_u32(x: Vec<Vec<u32>>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn vec_leaf(x: Vec<Leaf>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn btreeset_u32(x: std::collections::BTreeSet<u32>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn btreeset_string(x: std::collections::BTreeSet<String>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn btreemap_u32_string(x: std::collections::BTreeMap<u32, String>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn btreemap_string_u32(x: std::collections::BTreeMap<String, u32>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn array_u32(x: [u32; 3]) -> i64 {
    x.iter().sum::<u32>() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn array_string(x: [String; 2]) -> i64 {
    x.iter().map(|s| s.len()).sum::<usize>() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn array_option_u32(x: [Option<u32>; 2]) -> i64 {
    x.iter().filter(|v| v.is_some()).count() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn slice_u32(x: &[u32]) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn slice_string(x: &[String]) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn slice_option_u32(x: &[Option<u32>]) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn tuple_u32_string(x: (u32, String)) -> i64 {
    x.0 as i64 + x.1.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn tuple_u32_string_option(x: (u32, String, Option<u32>)) -> i64 {
    x.0 as i64 + x.1.len() as i64 + x.2.map(|v| v as i64).unwrap_or(0)
}

#[ply::ensures(|result| *result >= 0)]
pub fn box_u32(x: Box<u32>) -> i64 {
    *x as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn box_string(x: Box<String>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn box_vec_u32(x: Box<Vec<u32>>) -> i64 {
    x.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn box_leaf(x: Box<Leaf>) -> i64 {
    x.n as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn tuple_leaf_u32(x: (Leaf, u32)) -> i64 {
    x.0.n as i64 + x.1 as i64
}
