//! Two `impl` blocks for the same type, each defining a method of the same
//! name -- real, ordinary Rust: `Wrapper` is generic, and each block targets
//! a different concrete instantiation, so nothing here fails to compile.
//! Ply's syntactic reader cannot tell which `describe` a bare
//! `Wrapper::describe` claim means, and must refuse rather than guess
//! (The-Ply-Spec.md §5.2's own standing rule for a call classification that
//! could lie, applied here to anchor resolution).

pub struct Wrapper<T>(pub T);

impl Wrapper<u8> {
    pub fn describe(&self) -> u32 {
        1
    }
}

impl Wrapper<u16> {
    pub fn describe(&self) -> u32 {
        2
    }
}
