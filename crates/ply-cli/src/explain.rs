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

/// The spec, as of this build. Embedded rather than read from disk so an
/// installed binary explains the rules it actually implements, instead of
/// whatever happens to be in the working tree beside it.
const SPEC: &str = include_str!("../../../The-Ply-Spec.md");

/// Prints one code's entry, one spec section, or — with nothing asked for —
/// the whole code table.
pub fn explain_command(code: Option<&str>, out: &mut impl Write) -> anyhow::Result<()> {
    match code {
        // Two namespaces, and they cannot collide: a code is one letter and
        // four digits, a section is digits and dots. Checked before the code
        // table so a section reference is never reported as a bad code.
        Some(asked) => match section_reference(asked) {
            Some(section) => explain_section(&section, asked, out),
            None => one(asked, out),
        },
        None => list(out),
    }
}

/// The section number in what the reader typed, if that is what it is.
///
/// `§` needs a key most keyboards do not have, so it is never required —
/// only tolerated. `8`, `5.4b`, `§5.4b`, `s5.4b`, `sec 5.4b` and
/// `section 5.4b` are all the same request, because a reader should not have
/// to discover which spelling this command prefers.
fn section_reference(asked: &str) -> Option<String> {
    let mut rest = asked.trim().trim_start_matches('\u{a7}').trim();
    for prefix in ["section", "sec", "s"] {
        if let Some(stripped) = rest
            .strip_prefix(prefix)
            .or_else(|| rest.strip_prefix(&prefix.to_ascii_uppercase()))
        {
            // Only when a number follows: bare `s` is not a section, and
            // `sec` must not eat the `s` of something else.
            if stripped
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit())
            {
                rest = stripped.trim_start();
                break;
            }
        }
    }
    let number = rest.trim();
    // `5`, `5.1`, `5.1a`, `5.4b` — digits and dots, optionally one trailing
    // letter. Never one letter followed by four digits, which is a code.
    let mut chars = number.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut seen_letter = false;
    for c in chars {
        if c.is_ascii_digit() || c == '.' {
            if seen_letter {
                return None;
            }
        } else if c.is_ascii_alphabetic() && !seen_letter {
            seen_letter = true;
        } else {
            return None;
        }
    }
    Some(number.to_ascii_lowercase())
}

/// One section of the spec, from its own heading up to the next heading at
/// the same level or shallower — so asking for one rule does not hand back
/// the rest of the document.
fn explain_section(number: &str, asked: &str, out: &mut impl Write) -> anyhow::Result<()> {
    let lines: Vec<&str> = SPEC.lines().collect();
    let heading_of = |line: &str| -> Option<(usize, String)> {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes == 0 {
            return None;
        }
        let label = line[hashes..]
            .split_whitespace()
            .next()?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        Some((hashes, label))
    };
    let start = lines
        .iter()
        .position(|line| heading_of(line).is_some_and(|(_, label)| label == number));
    let Some(start) = start else {
        writeln!(
            out,
            "There is no section {} in the spec this build of Ply carries.\n\n\
             Sections are numbered like `5.1a` or `8`, and every message Ply prints ends \
             with the one behind it. The `\u{a7}` sign is never needed: `cargo ply explain 8` \
             and `cargo ply explain \u{a7}8` are the same request.",
            asked.trim()
        )?;
        return Ok(());
    };
    let (level, _) = heading_of(lines[start]).expect("the line matched as a heading");
    let end = lines[start + 1..]
        .iter()
        .position(|line| heading_of(line).is_some_and(|(depth, _)| depth <= level))
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());

    writeln!(out, "The-Ply-Spec.md \u{a7}{number}\n")?;
    for line in &lines[start..end] {
        writeln!(out, "{line}")?;
    }
    Ok(())
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

    /// `W0503` fires for two different outcomes at runtime (`verify.rs`):
    /// a high-but-survivable rejection rate, where the run still reaches
    /// the case count it was asked for and keeps its `fuzzed(n)` verdict,
    /// and a run proptest abandons outright, which earns no fuzz evidence
    /// at all. Before this test, `explain W0503` described only the second
    /// -- a reader who saw the warning on a passing `fuzzed(64)` run would
    /// read the explanation and conclude their run found nothing, which is
    /// the opposite of what happened.
    #[test]
    fn explain_w0503_covers_both_outcomes_the_code_actually_emits() {
        // The 88-column wrap can fold a newline into the middle of any of
        // these phrases, so the check runs against the unwrapped text --
        // wrapping is `wrap`'s own concern, not this test's.
        let out = render(Some("W0503")).replace('\n', " ");
        assert!(
            out.contains(
                "keeps drawing new values until it has as many passing cases as it \
                          was asked for, so that count is real"
            ),
            "must describe the survivable case, where the verdict is still `fuzzed(n)`: {out}"
        );
        assert!(
            out.contains(
                "it never recovers: it gives up before reaching that count, so no case was \
                 actually checked"
            ),
            "must still describe the abandoned case, where no evidence was earned at all: {out}"
        );
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
    // A reader who sees "§5.1a" in a diagnostic should be able to read §5.1a
    // without leaving the terminal, and without typing `§` -- which needs a
    // key most keyboards do not have. So the section is addressed by its
    // number, and the glyph is merely tolerated.
    #[test]
    fn a_section_is_explained_when_asked_for_by_bare_number() {
        let out = render(Some("8"));
        assert!(
            out.starts_with("The-Ply-Spec.md \u{a7}8"),
            "the answer must name what it is showing: {out}"
        );
        assert!(
            out.len() > 400,
            "a section's own prose is the body of the answer, not just its title: {out}"
        );
    }

    #[test]
    fn every_spelling_of_a_section_reference_reaches_the_same_section() {
        let canonical = render(Some("5.1a"));
        for typed in [
            "\u{a7}5.1a",
            "5.1A",
            " 5.1a ",
            "s5.1a",
            "sec5.1a",
            "section 5.1a",
        ] {
            assert_eq!(
                render(Some(typed)),
                canonical,
                "{typed:?} is the same request as `5.1a` -- a reader should not have to \
                 discover which spelling this command prefers"
            );
        }
    }

    /// A section stops where the next one starts. Printing past it would
    /// hand a reader who asked for one rule the whole rest of the document.
    #[test]
    fn a_section_stops_before_the_next_one_begins() {
        let out = render(Some("5.1a"));
        assert!(
            out.contains("Strictness"),
            "expected \u{a7}5.1a's own heading in its answer: {out}"
        );
        assert!(
            !out.contains("## 6."),
            "the answer ran past the end of the section into a later one: {out}"
        );
    }

    /// The two namespaces cannot collide -- a code is a letter and four
    /// digits, a section is digits and dots -- but the failure has to be
    /// legible either way, and must not silently answer the wrong question.
    #[test]
    fn a_section_that_does_not_exist_says_so_and_does_not_fall_back_to_a_code() {
        let out = render(Some("99.7"));
        assert!(
            out.contains("no section") && out.contains("99.7"),
            "expected a refusal naming what was asked for: {out}"
        );
        assert!(
            !out.contains("one letter, then four digits"),
            "a section reference must not be reported as a bad diagnostic code: {out}"
        );
    }

    /// Codes still win their own namespace: `W0419` is not a section.
    #[test]
    fn a_code_is_still_a_code_and_not_read_as_a_section() {
        assert!(render(Some("W0419")).starts_with("W0419  ("));
    }
    /// The sweep, rather than a handful of spot-checks: every numbered
    /// heading in the spec this build carries must be reachable by its own
    /// number. A section added later with a heading this parser cannot read
    /// fails here rather than being discovered by a reader who asked for it.
    #[test]
    fn every_numbered_section_in_the_spec_can_be_asked_for_by_its_number() {
        let mut checked = 0usize;
        for line in SPEC.lines() {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            if hashes == 0 {
                continue;
            }
            let Some(label) = line[hashes..].split_whitespace().next() else {
                continue;
            };
            let label = label.trim_end_matches('.');
            if !label.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            let out = render(Some(label));
            assert!(
                !out.contains("There is no section"),
                "the spec has a section {label:?} and this command cannot reach it: {out}"
            );
            checked += 1;
        }
        assert!(
            checked > 20,
            "expected the spec's numbered sections to be found and swept; got {checked}, \
             which means this test is now proving nothing"
        );
    }
}
