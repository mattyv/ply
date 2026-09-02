//! §5.4b's generator hook (this task, 2026-09-02): "a type is buildable if
//! there is a public way to get one from parts Ply can already build".
//! `Handle` is the routeprobe's own case in miniature -- a struct with a
//! private field, made only by a *free* function, which rule 1's own
//! constructor scan cannot see at all (it only ever looks inside `impl`
//! blocks). `Token` is the contrast: an associated function with a name
//! Ply's constructor scan does not filter by (`parse_unchecked`, not
//! `new`), which already worked before this task and must keep working
//! unchanged.
//!
//! `Stuck` is the one failure a stale-route compile error cannot catch on
//! its own (TODO.md, "the guard this cannot ship without"): its route
//! ignores the value it is handed and returns the same value every time.

/// Private field, and no inherent constructor -- but a free function makes
/// one. `Debug`-derived so the degenerate-route guard's own distinct-value
/// count is a real number, not "could not tell" -- the non-`Debug` case is
/// pinned directly by `crates/ply-core/src/fuzz_gen.rs`'s own unit tests
/// rather than repeated here.
#[derive(Debug)]
pub struct Handle {
    id: u32,
}
pub fn open_handle(id: u32) -> Handle {
    Handle { id }
}

/// Private field; the only way in is an associated function with a name
/// Ply's constructor scan does not filter by -- already supported before
/// this task, and unchanged by it.
pub struct Token {
    text: String,
}
impl Token {
    pub fn parse_unchecked(text: String) -> Token {
        Token { text }
    }
}

/// A route that ignores its own input and returns the same value every
/// time -- the guard's own required proof. A real author's mistake would
/// look exactly like this: the compiler accepts it, the harness runs and
/// passes, and nothing but counting distinct values catches it.
#[derive(Debug)]
pub struct Stuck {
    n: u32,
}
pub fn make_stuck(_n: u32) -> Stuck {
    Stuck { n: 7 }
}

/// A false promise on a route-built parameter: `id` is Ply's own generator
/// output, drawn from the whole `u32` range, so `< 100` is false on almost
/// every case -- proving the check actually bites on a route-built value,
/// not merely that it runs.
#[ply::ensures(|result| *result < 100)]
pub fn use_handle(h: &Handle) -> i64 {
    h.id as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn use_token(t: &Token) -> i64 {
    t.text.len() as i64
}

/// The declared route composing inside a list -- the same composition
/// grammar (2026-09-02) that already closes over a constructor-built user
/// type must close over a route-built one too, since both are the same
/// `RustType::UserTypeCtor` underneath.
#[ply::ensures(|result| *result >= 0)]
pub fn use_many_handles(hs: Vec<Handle>) -> i64 {
    hs.len() as i64
}

#[ply::ensures(|result| *result >= 0)]
pub fn use_stuck(s: Stuck) -> i64 {
    s.n as i64
}
