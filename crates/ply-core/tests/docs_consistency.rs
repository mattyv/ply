//! `docs/SCHEMA.md` describes the same behaviour from three different
//! sections, and prose that drifts across sections is worse than prose that
//! never existed -- a reader who lands on section 8 first has no way to know
//! sections 2 and 14 disagree with it. There is no doc test harness for
//! `docs/`, so this pins the one sentence that must never come back: section
//! 8 once opened with a blanket "nothing in this section is enforced"
//! warning that sections 2 and 14 already contradicted, because `edges:` and
//! `deny:` ARE checked at crate level against the real `cargo metadata`
//! dependency graph (`A0401`, `A0405`).

use std::path::Path;

fn schema_md() -> String {
    // `crates/ply-core` -> `crates` -> the workspace root, same convention
    // `ply-cli/build.rs` uses to find `schema/ply.schema.json`.
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ply-core sits two directories below the workspace root");
    let path = workspace_root.join("docs/SCHEMA.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The bug this catches: section 8 claimed nothing in it was enforced, while
/// section 2 (~L138) and section 14 (~L1426) already documented `edges:` and
/// `deny:` as checked at crate level. A reader who only opens section 8 has
/// no way to discover the other two sections disagree with it, so the false
/// blanket sentence has to be gone, not merely outvoted.
#[test]
fn section_8_no_longer_claims_none_of_its_rules_are_enforced() {
    let text = schema_md();
    assert!(
        !text.contains("None of the rules in this section is enforced"),
        "docs/SCHEMA.md still carries the blanket warning that sections 2 and 14 \
         already contradict -- edges: and deny: are enforced at crate level"
    );
}

/// The replacement has to actually say what section 2 and section 14 say:
/// that crate-tier `edges:` (`A0401`) and `deny:` (`A0405`) findings are
/// enforced today. Pinning both codes inside section 8 itself (not merely
/// present somewhere else in the file) so a future edit that vagues out the
/// tier matrix without removing the codes still fails this.
#[test]
fn section_8_names_the_enforced_crate_tier_codes() {
    let text = schema_md();
    let section_8 = section(&text, "## 8. ");
    assert!(
        section_8.contains("A0401"),
        "section 8 should name A0401 (enforced crate-tier edges finding)"
    );
    assert!(
        section_8.contains("A0405"),
        "section 8 should name A0405 (enforced crate-tier deny finding)"
    );
    assert!(
        section_8.to_lowercase().contains("enforced"),
        "section 8 should say plainly that these two are enforced, not just declared"
    );
}

/// Section 2 already tells readers where to go when prose and behaviour
/// disagree: "`check` says exactly this at the bottom of its own output,
/// and that paragraph is the authority if this one ever drifts from it
/// again." Section 8 should point at the same authority rather than
/// re-asserting its own claim as if no other section could go stale too.
#[test]
fn section_8_defers_to_checks_own_output_as_the_authority() {
    let text = schema_md();
    let section_8 = section(&text, "## 8. ");
    assert!(
        section_8.to_lowercase().contains("authority"),
        "section 8 should name check's own closing output as the authority, \
         the same convention section 2 already establishes"
    );
}

/// Slices out one `## N. Title` section up to (not including) the next
/// `## ` heading, so assertions about "section 8" cannot accidentally pass
/// by matching text that actually lives in section 2 or section 14.
fn section<'a>(text: &'a str, heading_prefix: &str) -> &'a str {
    let start = text
        .find(heading_prefix)
        .unwrap_or_else(|| panic!("{heading_prefix:?} not found in docs/SCHEMA.md"));
    let after = &text[start..];
    let end = after[heading_prefix.len()..]
        .find("\n## ")
        .map(|i| i + heading_prefix.len())
        .unwrap_or(after.len());
    &after[..end]
}
