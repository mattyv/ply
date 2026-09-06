//! The contract text every fixture produces, pinned byte for byte.
//!
//! A contract's rendered text is not cosmetic. It is a hashed fingerprint
//! input *and* it seeds case generation, so re-rendering an unchanged
//! contract differently invalidates every recorded result in the world and
//! makes every function draw different inputs.
//!
//! That happened on 2026-09-05. A fix for repeated attributes re-rendered
//! even a single clause, and `|result| *result >= 0` became
//! `|result|(*result >= 0)` -- one pair of brackets. Every fingerprint
//! stopped matching, every function reseeded, and the new inputs found a
//! real overflow in a fixture the old inputs happened never to reach. What
//! caught it was three end-to-end tests failing for reasons that looked
//! unrelated, hours later, in CI. Nothing was watching the text itself.
//!
//! Now something is. This walks every fixture, renders the contracts the
//! way the real pipeline does -- through `discover_fn`, not a copy of it --
//! and compares the result against a committed list. A change to how
//! contracts are spelled now arrives as a reviewable diff on this file,
//! before it arrives as a reseed.
//!
//! **A diff here is not automatically a bug.** It means every recorded
//! result for every affected function is about to stop matching. Read it,
//! decide whether the new spelling is worth that, and update the golden in
//! the same commit as the change that caused it -- never separately, and
//! never by regenerating without looking.
//!
//! Regenerate with `PLY_UPDATE_GOLDEN=1 cargo test -p ply-core
//! --test contract_text_is_stable`.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/ply-core`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("ply-core sits two directories below the repository root")
        .to_path_buf()
}

/// Every function in one file carrying a `#[ply::requires]` or
/// `#[ply::ensures]`, by name, in source order.
fn contracted_fn_names(src: &str) -> Vec<String> {
    let Ok(file) = syn::parse_file(src) else {
        return Vec::new();
    };
    fn walk(items: &[syn::Item], out: &mut Vec<String>) {
        for item in items {
            match item {
                syn::Item::Fn(f) => {
                    let has = f.attrs.iter().any(|a| {
                        let p = &a.path();
                        let segs: Vec<String> =
                            p.segments.iter().map(|s| s.ident.to_string()).collect();
                        segs.first().map(String::as_str) == Some("ply")
                            && matches!(
                                segs.get(1).map(String::as_str),
                                Some("requires") | Some("ensures")
                            )
                    });
                    if has {
                        out.push(f.sig.ident.to_string());
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, items)) = &m.content {
                        walk(items, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&file.items, &mut out);
    out
}

#[test]
fn the_text_every_contract_renders_to_is_the_text_that_was_recorded() {
    let root = repo_root();
    let fixtures = root.join("tests/fixtures");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&fixtures)
        .expect("the fixture directory is part of the repository")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let mut lines: Vec<String> = Vec::new();
    for dir in &dirs {
        let lib = dir.join("src/lib.rs");
        let Ok(src) = std::fs::read_to_string(&lib) else {
            continue;
        };
        let fixture = dir.file_name().unwrap().to_string_lossy().to_string();
        for name in contracted_fn_names(&src) {
            // The real path, so this cannot drift from what a run records.
            let Ok(cf) = ply_core::harness::discover_fn(&lib, &name) else {
                // A fixture Ply deliberately refuses to read is not a
                // contract-text question; the suites that own those cases
                // check them.
                continue;
            };
            if let Some((_, text)) = &cf.requires {
                lines.push(format!("{fixture}::{name} requires {text}"));
            }
            if let Some((_, text)) = &cf.ensures {
                lines.push(format!("{fixture}::{name} ensures {text}"));
            }
        }
    }
    lines.sort();
    let rendered = format!("{}\n", lines.join("\n"));

    assert!(
        lines.len() > 100,
        "only {} contracts were found across {} fixtures, which means the walk broke rather \
         than that the contracts went away -- a golden that silently covers nothing passes \
         forever",
        lines.len(),
        dirs.len()
    );

    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contract-text.golden");
    if std::env::var("PLY_UPDATE_GOLDEN").is_ok() {
        std::fs::write(&golden, &rendered).expect("could not write the golden");
        return;
    }
    let recorded = std::fs::read_to_string(&golden).unwrap_or_default();
    if recorded != rendered {
        let mut first_diff = String::new();
        for (a, b) in recorded.lines().zip(rendered.lines()) {
            if a != b {
                first_diff = format!("\n  recorded: {a}\n  now:      {b}");
                break;
            }
        }
        panic!(
            "a contract renders to different text than it did when this was recorded.\n\n\
             That text is a hashed fingerprint input and it seeds case generation, so this \
             change invalidates every recorded result for every affected function and makes \
             them draw different inputs. It is not a formatting question.\n\n\
             {} lines recorded, {} now.{first_diff}\n\n\
             If the new spelling is right, regenerate with PLY_UPDATE_GOLDEN=1 and commit the \
             golden alongside the change that caused it -- having read the diff.",
            recorded.lines().count(),
            rendered.lines().count(),
        );
    }
}
