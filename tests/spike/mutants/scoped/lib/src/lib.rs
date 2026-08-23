//! The "mutated crate" for the item-3 experiment: no local tests at all,
//! standing in for a real Ply target where the checks live in a
//! Ply-generated harness crate (§5.4c: "generated harness crate under
//! `target/ply/fuzz/`"), not in the crate's own `#[cfg(test)]` modules.

pub fn strong_target(x: i32, y: i32) -> i32 {
    if x > 0 && y > 0 {
        x + y
    } else {
        x - y
    }
}

pub fn weak_target(x: i32, y: i32) -> i32 {
    if x > 0 && y > 0 {
        x + y
    } else {
        x - y
    }
}
