pub mod fuzz;
pub mod kani;
pub mod mutants;

/// Every engine Ply drives is a program whose output Ply then *reads* --
/// which compiler error, which counterexample, which mutant survived. That
/// reading is line-oriented and matches on what a line starts with, so it
/// silently stops working the moment the tool decides to colour its output.
///
/// It is not hypothetical. This project's own CI sets
/// `CARGO_TERM_COLOR=always`, so every `error:` line reached Ply as
/// `\x1b[1m\x1b[91merror\x1b[0m: ...`, matched nothing, and a build failure
/// with a perfectly clear cause was reported as "the compiler gave no
/// specific error line" -- Ply's honest fallback for a failure it genuinely
/// cannot attribute, taken for one it could have attributed exactly. Worse
/// than a wrong answer: a true sentence used in the wrong place, which
/// looks like the tool working.
///
/// So output is stripped of ANSI escape sequences before anything parses
/// it. Passing `--color=never` would fix cargo alone; every engine's output
/// goes through here instead, because the next tool to add colour should
/// not cost another silent regression.
///
/// Handles the two forms that matter for terminal output: CSI sequences
/// (`ESC [ ... final-byte`, which covers every colour and style code) and
/// OSC sequences (`ESC ] ... BEL` or `ESC ] ... ESC \`, which is how
/// hyperlinks arrive). A bare `ESC` followed by anything else drops the
/// escape and keeps the character, so nothing is ever swallowed wholesale.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // CSI: parameter and intermediate bytes, then one final
                // byte in 0x40..=0x7e ends it.
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                // OSC: runs until BEL, or until the ST pair `ESC \`.
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod strip_ansi_tests {
    use super::strip_ansi;

    /// The exact shape that made this repository's own CI report a
    /// perfectly attributable build failure as unattributable.
    #[test]
    fn a_coloured_compiler_error_still_begins_with_the_word_error() {
        let coloured = "\u{1b}[1m\u{1b}[91merror\u{1b}[0m\u{1b}[1m: expected `;`\u{1b}[0m";
        assert_eq!(strip_ansi(coloured), "error: expected `;`");
        assert!(strip_ansi(coloured).trim().starts_with("error"));
    }

    /// The span line is what attributes an error to the function that
    /// caused it, so it has to survive too.
    #[test]
    fn a_coloured_span_line_still_points_at_a_file_and_line() {
        let coloured = "  \u{1b}[1m\u{1b}[94m-->\u{1b}[0m src/lib.rs:42:9";
        assert_eq!(strip_ansi(coloured), "  --> src/lib.rs:42:9");
    }

    #[test]
    fn plain_output_is_returned_unchanged() {
        let plain = "error[E0308]: mismatched types\n  --> src/lib.rs:1:1\n";
        assert_eq!(strip_ansi(plain), plain);
    }

    /// A terminal hyperlink is an OSC sequence, not a CSI one, and it
    /// carries a URL that must not be left behind as text.
    #[test]
    fn an_osc_hyperlink_is_removed_along_with_its_url() {
        let linked = "see \u{1b}]8;;https://example.com\u{7}the docs\u{1b}]8;;\u{7}";
        assert_eq!(strip_ansi(linked), "see the docs");
    }

    /// Nothing is swallowed wholesale: an escape Ply does not recognise
    /// costs one byte, not the rest of the line.
    #[test]
    fn an_unrecognised_escape_drops_only_itself() {
        assert_eq!(strip_ansi("a\u{1b}Zb"), "aZb");
    }
}
