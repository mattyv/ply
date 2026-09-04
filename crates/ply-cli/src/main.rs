//! The binary, and deliberately nothing else -- every line of decision
//! lives in the library beside it, where Ply can be pointed at it. See
//! `lib.rs`'s own doc comment for why.

fn main() -> anyhow::Result<()> {
    ply_cli::run()
}
