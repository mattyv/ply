//! The other half of the honesty condition TODO.md names outright: not
//! every parameter Ply cannot build is one it can mutate once an example
//! supplies a value. `Opaque` has a private field and no constructor Ply's
//! scan finds -- there is no text, no length, no element, nothing this
//! codegen knows how to vary -- so the one `examples:` entry below is the
//! *only* distinct input this run will ever have. The verdict must say
//! exactly that (`tested`), never a `fuzzed(n)` that implies n independent
//! cases were drawn.

#[derive(Default)]
pub struct Opaque {
    #[allow(dead_code)]
    x: u32,
}

#[ply::ensures(|result| *result)]
pub fn always_true(_o: Opaque) -> bool {
    true
}
