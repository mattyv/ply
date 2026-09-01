//! The other honesty condition `docs/reach-measurement-2.md` demands:
//! when there are no seeds at all and the constructor never accepts even
//! one generated draw, the refusal must name the action that would fix it
//! rather than repeating the same generic advice every high-rejection
//! abort already gets.
//!
//! `Strict::new` accepts exactly one 25-character string out of the entire
//! space uniform sampling could draw (up to 32 characters, ASCII-biased) --
//! close enough to zero that no run will ever accept one by chance, and no
//! `examples:` entry names a valid call either. There is nothing to grow
//! inputs from.

pub struct StrictErr;

pub struct Strict {
    pub text: String,
}

impl Strict {
    pub fn new(text: &str) -> Result<Self, StrictErr> {
        if text.len() == 25 && text.chars().all(|c| c == 'z') {
            Ok(Strict {
                text: text.to_string(),
            })
        } else {
            Err(StrictErr)
        }
    }

    #[ply::ensures(|result| *result == self.text.len() as u32)]
    pub fn length(&self) -> u32 {
        self.text.len() as u32
    }
}
