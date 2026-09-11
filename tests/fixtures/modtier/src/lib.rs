//! Two modules in one crate, with a rule between them that only a
//! module-level check can enforce: at the Cargo level this is one package
//! depending on nothing, so package dependencies have nothing to say.

pub mod exec;
pub mod parse;
pub mod shared;
