//! A build script compiles this same source against different values after
//! an input file or environment value changes.

pub fn answer(x: u32) -> u32 {
    x.min(env!("PLY_FIXTURE_ANSWER").parse::<u32>().unwrap_or(0))
}
