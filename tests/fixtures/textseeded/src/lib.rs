//! `docs/reach-measurement-2.md`'s own probe, close to verbatim: a receiver
//! method (`is_empty`) whose receiver is built by a fallible constructor
//! that parses free-form text (`Prerelease::new`). Random text almost never
//! satisfies it -- most characters are rejected -- so uniform sampling alone
//! earns no fuzz evidence at all. One `examples:` entry (in `ply.yaml`) is
//! enough to seed generation: Ply grows a corpus of known-valid text from it
//! and from every value the constructor accepts during the run, and mutates
//! that corpus instead of guessing text uniformly.

pub struct PrereleaseErr;

pub struct Prerelease {
    pub text: String,
}

impl Prerelease {
    pub fn new(text: &str) -> Result<Self, PrereleaseErr> {
        if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
            Ok(Prerelease {
                text: text.to_string(),
            })
        } else {
            Err(PrereleaseErr)
        }
    }

    #[ply::ensures(|result| *result == self.text.is_empty())]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}
