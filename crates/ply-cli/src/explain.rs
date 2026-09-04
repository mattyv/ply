//! `cargo ply explain <CODE>` — look up one diagnostic code.
//!
//! Every message Ply prints ends in a short code (`K0502`, `W0413`). The
//! codes exist so a message can be searched for, quoted in a review, and
//! tracked across releases without depending on the exact wording of the
//! sentence. Until this command existed there was no way for a reader to
//! find out what one meant: the table is in Ply's own source, and the
//! meaning of the leading letter was written down nowhere at all.
//!
//! Everything printed here comes from `ply_core::registry`, the same table
//! two invariant tests hold the rest of the tool to — so this cannot drift
//! from what the code actually emits, which is the failure the registry was
//! built to end.

use std::io::Write;

use ply_core::registry::{self, Severity, Status, Tier};

/// Prints one code's entry, or — with no code — the whole table.
pub fn explain_command(code: Option<&str>, out: &mut impl Write) -> anyhow::Result<()> {
    match code {
        Some(code) => one(code, out),
        None => list(out),
    }
}

fn one(code: &str, out: &mut impl Write) -> anyhow::Result<()> {
    let Some(entry) = registry::lookup(code) else {
        // Not an error the user can act on by trying harder, so it says what
        // to do instead of only what went wrong. A near miss is the likely
        // case: a code read off a screenshot, or one digit out.
        writeln!(
            out,
            "There is no diagnostic code `{}` in this build of Ply.\n\n\
             Codes look like `K0502` or `W0413`: one letter, then four digits. Run `cargo ply \
             explain` with nothing after it to see every code this build can produce.",
            code.trim()
        )?;
        let near = near_misses(code);
        if !near.is_empty() {
            writeln!(out, "\nDid you mean: {}?", near.join(", "))?;
        }
        return Ok(());
    };

    let name = format!("{:?}", entry.code);
    writeln!(out, "{name}  ({})", severity_word(entry.severity))?;
    writeln!(out)?;
    writeln!(out, "{}", wrap(entry.gloss, 88, ""))?;
    writeln!(out)?;
    writeln!(out, "Who reports it: {}.", registry::family(entry.code))?;
    writeln!(out, "When: {}.", stage_word(entry.tier))?;
    match entry.status {
        Status::Enforced => {}
        Status::DeclaredOnly => {
            // The one thing a reader most needs and would never guess: this
            // code is described but nothing emits it yet. Saying so is the
            // whole point of the registry computing `status` rather than
            // taking a document's word for it.
            writeln!(
                out,
                "\nThis build never produces this code. It is described and planned, and no part \
                 of Ply emits it yet, so seeing it in a document is not a promise that a run \
                 would ever report it."
            )?;
        }
    }
    writeln!(
        out,
        "\nThe reasoning behind this rule is in The-Ply-Spec.md {}.",
        entry.spec_anchor
    )?;
    Ok(())
}

fn list(out: &mut impl Write) -> anyhow::Result<()> {
    writeln!(
        out,
        "Every diagnostic code this build of Ply knows about. Run `cargo ply explain <CODE>` for \
         any one of them.\n"
    )?;
    writeln!(out, "The first letter says who is reporting it:\n")?;
    for letter in ['E', 'A', 'W', 'V', 'K', 'P', 'R', 'M', 'X'] {
        let Some(entry) = registry::all().into_iter().find(|e| e.letter() == letter) else {
            continue;
        };
        writeln!(out, "  {letter}   {}", registry::family(entry.code))?;
    }
    writeln!(out)?;

    let mut rows = registry::all();
    rows.sort_by_key(|e| format!("{:?}", e.code));
    for entry in rows {
        let name = format!("{:?}", entry.code);
        let planned = match entry.status {
            Status::Enforced => "",
            Status::DeclaredOnly => "  (planned; nothing emits it yet)",
        };
        writeln!(out, "{name}  {}{planned}", first_sentence(entry.gloss))?;
    }
    Ok(())
}

/// Codes within one character of what was typed, so a mistyped digit gets a
/// suggestion rather than a flat refusal.
fn near_misses(typed: &str) -> Vec<String> {
    let wanted = typed.trim().to_ascii_uppercase();
    if wanted.len() != 5 {
        return Vec::new();
    }
    let mut hits: Vec<String> = registry::all()
        .into_iter()
        .map(|e| format!("{:?}", e.code))
        .filter(|name| {
            name.chars()
                .zip(wanted.chars())
                .filter(|(a, b)| a != b)
                .count()
                == 1
        })
        .collect();
    // Same first letter first. One digit out is the common slip, and a
    // reader who typed `K0503` almost certainly meant another code from the
    // exhaustive prover -- not the one that happens to share four digits
    // with it and is reported by something else entirely.
    let head = wanted.chars().next();
    hits.sort_by_key(|name| (name.chars().next() != head, name.clone()));
    hits.truncate(4);
    hits
}

fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Error => "an error — a run reporting this did not pass",
        Severity::Warning => "a warning — worth reading, does not fail a run on its own",
        Severity::Info => "information, not a problem",
    }
}

fn stage_word(t: Tier) -> &'static str {
    match t {
        Tier::Schema => "reading your ply.yaml, before any code is looked at",
        Tier::Anchor => "matching a claim in your document to the real function it names",
        Tier::Crate => {
            "checking which crate may depend on which — exact, so a finding here is real"
        }
        Tier::Item => {
            "checking inside your functions — approximate, so a finding here is a strong hint rather than a certainty"
        }
        Tier::Contract => {
            "running a check against a function or a structure, and reading what came back"
        }
    }
}

fn first_sentence(gloss: &str) -> String {
    let cut = gloss.find(". ").map(|i| i + 1).unwrap_or(gloss.len());
    let s = &gloss[..cut];
    let s = s.split(" -- ").next().unwrap_or(s);
    s.trim_end_matches('.').to_string()
}

/// Wraps at `width`, never mid-word.
fn wrap(text: &str, width: usize, indent: &str) -> String {
    let mut out = String::new();
    let mut line = String::from(indent);
    for word in text.split_whitespace() {
        if line.len() > indent.len() && line.len() + 1 + word.len() > width {
            out.push_str(line.trim_end());
            out.push('\n');
            line = String::from(indent);
        }
        if line.len() > indent.len() {
            line.push(' ');
        }
        line.push_str(word);
    }
    out.push_str(line.trim_end());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(code: Option<&str>) -> String {
        let mut buf: Vec<u8> = Vec::new();
        explain_command(code, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    /// The whole reason this command exists: a reader with `K0502` in their
    /// terminal learns what it means, who said it, and whether their run
    /// passed -- without opening Ply's source.
    #[test]
    fn a_real_code_is_explained_in_words_a_newcomer_can_use() {
        let out = render(Some("K0502"));
        assert!(out.starts_with("K0502  (an error"), "{out}");
        assert!(
            out.contains("exhaustive check searched every possible value"),
            "the plain sentence from the table is the body of the answer: {out}"
        );
        assert!(
            out.contains("Who reports it: the exhaustive prover (Kani)."),
            "the leading letter is the thing nothing else explains: {out}"
        );
        assert!(out.contains("The-Ply-Spec.md §8"), "{out}");
    }

    /// Lowercase, because a code retyped from a screenshot arrives however
    /// the reader typed it and refusing that would be a refusal about
    /// typing rather than about the code.
    #[test]
    fn a_code_typed_in_lower_case_still_resolves() {
        assert_eq!(render(Some("k0502")), render(Some("K0502")));
    }

    /// A code that is described but that nothing emits must say so. Reading
    /// about a rule and believing a run would report it is exactly the gap
    /// the registry computes `status` to close.
    #[test]
    fn a_planned_code_says_no_run_will_ever_report_it() {
        let planned = registry::all()
            .into_iter()
            .find(|e| matches!(e.status, Status::DeclaredOnly))
            .expect("this build has at least one planned-only code");
        let out = render(Some(&format!("{:?}", planned.code)));
        assert!(out.contains("This build never produces this code"), "{out}");
    }

    /// A code that does not exist gets told so plainly, and pointed at the
    /// list -- and a single mistyped character gets a suggestion.
    #[test]
    fn an_unknown_code_says_so_and_offers_the_nearest_real_one() {
        let out = render(Some("K0503"));
        assert!(out.contains("There is no diagnostic code `K0503`"), "{out}");
        assert!(out.contains("cargo ply explain"), "{out}");
        assert!(
            out.contains("Did you mean: K0502"),
            "one digit out is the likely mistake, so it is worth catching: {out}"
        );
    }

    /// With no code, every code in the build, and the letter key first --
    /// the part a reader cannot derive from any single message.
    #[test]
    fn the_listing_covers_every_code_and_leads_with_the_letters() {
        let out = render(None);
        assert!(
            out.contains("The first letter says who is reporting it"),
            "{out}"
        );
        assert!(out.contains("the exhaustive prover (Kani)"), "{out}");
        for entry in registry::all() {
            let name = format!("{:?}", entry.code);
            assert!(
                out.contains(&name),
                "`{name}` is a code this build can produce and the listing has to carry it"
            );
        }
    }
}
