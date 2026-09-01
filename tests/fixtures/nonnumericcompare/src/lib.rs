//! Regression fixture for the widening defect found pointing Ply at
//! `semver` (2026-09-01): a promise comparing two non-numeric values with
//! `==`/`!=` used to make the check's own generated file fail to compile,
//! quoting a compiler error about the promise's own comparison rather than
//! ever reporting a verdict on it. Every check in this crate shares one
//! generated harness (The-Ply-Spec.md §5.4c), so one such comparison used
//! to turn every *other* function's evidence into a tool error too --
//! `good_fn` below is unrelated to every other promise in this file and
//! proves that contagion is gone.
//!
//! `Wrapper::new` is the exact shape of the reproduction: `semver`'s own
//! `Prerelease::new(text: &str) -> Result<Prerelease, Error>` declared
//! `result.is_err() || result.as_ref().unwrap().as_str() == text`, and
//! that comparison alone -- nothing else in the file -- used to fail with
//! `error[E0606]: casting &str as i128 is invalid`.

/// A minimal stand-in for `semver::Prerelease`: built from text, and able
/// to read that text back out.
pub struct Wrapper {
    text: String,
}

impl Wrapper {
    /// `semver::Prerelease::new`'s own postcondition, verbatim in shape:
    /// compares the text the value was built from back out through the
    /// type. True for every input -- the constructor never drops or
    /// changes the text it was given.
    #[ply::ensures(|result| result.is_err() || result.as_ref().unwrap().as_str() == text)]
    pub fn new(text: &str) -> Result<Wrapper, String> {
        if text.is_empty() {
            Err("text must not be empty".to_string())
        } else {
            Ok(Wrapper {
                text: text.to_string(),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// The identical promise, made false on purpose: this constructor drops
/// the text it was given. Once the comparison above is actually checkable,
/// this one must earn a real `violation` with a real failing input --
/// proof the check bites, not merely compiles.
pub struct BrokenWrapper {
    text: String,
}

impl BrokenWrapper {
    #[ply::ensures(|result| result.is_err() || result.as_ref().unwrap().as_str() == text)]
    pub fn new(text: &str) -> Result<BrokenWrapper, String> {
        if text.is_empty() {
            Err("text must not be empty".to_string())
        } else {
            Ok(BrokenWrapper {
                text: String::new(),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// Comparing an `Option` value directly with `==` -- `Option<u32>` cannot
/// be cast `as i128` either (`error[E0605]: non-primitive cast`). True for
/// every input: this function returns exactly what it was given.
#[ply::ensures(|result| *result == v)]
pub fn identity_opt(v: Option<u32>) -> Option<u32> {
    v
}

/// The identical promise, made false on purpose: this function drops
/// whatever it was given and always answers `None`.
#[ply::ensures(|result| *result == v)]
pub fn always_none(v: Option<u32>) -> Option<u32> {
    let _ = v;
    None
}

/// A fieldless enum, compared directly with `==`. Carries a (trivial)
/// `Drop` impl: a bare fieldless enum with no `Drop` impl is one of the few
/// non-scalar shapes Rust *does* let `as i128` reach through, so without
/// `Drop` this comparison would not actually demonstrate the compile
/// failure the conservative rule exists to avoid (confirmed directly
/// against `rustc`, not assumed) -- with it, casting either side is a hard
/// compiler error (`error[E0320]: cannot cast enum ... because it
/// implements Drop`).
#[derive(PartialEq, Eq)]
pub enum Sign {
    Pos,
    Neg,
}

impl Drop for Sign {
    fn drop(&mut self) {}
}

/// True for every input: always answers the variant the promise names.
#[ply::ensures(|result| *result == Sign::Pos)]
pub fn always_pos(x: i32) -> Sign {
    let _ = x;
    Sign::Pos
}

/// The identical promise, made false on purpose: answers `Sign::Neg` for a
/// negative input.
#[ply::ensures(|result| *result == Sign::Pos)]
pub fn maybe_pos(x: i32) -> Sign {
    if x < 0 { Sign::Neg } else { Sign::Pos }
}

/// The overflow trap the widening this fixture exercises exists to guard
/// against must still hold: `x + 1` at `u8::MAX` must report the broken
/// promise (`saturating_add` gives 255, plain `+ 1` would give 0), never
/// panic while checking it.
#[ply::ensures(|result| *result == x + 1)]
pub fn saturating_bump(x: u8) -> u8 {
    x.saturating_add(1)
}

/// Entirely unrelated to every promise above -- shares this crate's one
/// generated harness with all of them (The-Ply-Spec.md §5.4c). Before the
/// fix, this function reported `tool_error` purely because another
/// function's comparison could not compile, despite its own contract being
/// perfectly ordinary.
#[ply::requires(x <= 1_000)]
#[ply::ensures(|result| *result >= x)]
pub fn good_fn(x: u32) -> u32 {
    x + 1
}
