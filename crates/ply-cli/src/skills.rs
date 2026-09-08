//! The skills, shipped inside the binary and written out on request.
//!
//! Ply is installed with `cargo install`, which copies one executable and
//! nothing else. The skills are what tell an author how to write code Ply
//! can check -- `ply-checkable-code`'s rule 9 is the only written
//! explanation of how to promise something about a method that changes
//! state -- and until now they lived only in this repository, where only
//! someone reading the source would find them. Installing the tool got you
//! the tool without the guidance that makes it usable.
//!
//! They are embedded rather than fetched, so `cargo ply skills` works
//! offline and always writes the guidance that matches the binary you are
//! running. A skill written for a newer Ply, describing a refusal this build
//! does not make, would be worse than none.
//!
//! Nothing is written at install time. A tool that scatters files into a
//! user's configuration the moment it is installed is a tool people
//! uninstall; this is a command they run when they want it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One file belonging to one skill: where it goes under the destination,
/// and what it contains.
struct SkillFile {
    /// Destination-relative, always with `/` separators.
    rel: &'static str,
    body: &'static str,
}

/// Every file `cargo ply skills` writes.
///
/// Listed by hand rather than walked at build time, and
/// `every_skill_in_the_repository_is_shipped` is what keeps that honest:
/// it walks the real directory and fails naming any file this list has
/// missed. A skill added later cannot quietly fail to ship.
const SKILL_FILES: &[SkillFile] = &[
    SkillFile {
        rel: "ply-audit/SKILL.md",
        body: include_str!("../../../skills/ply-audit/SKILL.md"),
    },
    SkillFile {
        rel: "ply-audit/agents/openai.yaml",
        body: include_str!("../../../skills/ply-audit/agents/openai.yaml"),
    },
    SkillFile {
        rel: "ply-author/SKILL.md",
        body: include_str!("../../../skills/ply-author/SKILL.md"),
    },
    SkillFile {
        rel: "ply-author/agents/openai.yaml",
        body: include_str!("../../../skills/ply-author/agents/openai.yaml"),
    },
    SkillFile {
        rel: "ply-checkable-code/SKILL.md",
        body: include_str!("../../../skills/ply-checkable-code/SKILL.md"),
    },
    SkillFile {
        rel: "ply-checkable-code/agents/openai.yaml",
        body: include_str!("../../../skills/ply-checkable-code/agents/openai.yaml"),
    },
    SkillFile {
        rel: "ply-review/SKILL.md",
        body: include_str!("../../../skills/ply-review/SKILL.md"),
    },
    SkillFile {
        rel: "ply-review/agents/openai.yaml",
        body: include_str!("../../../skills/ply-review/agents/openai.yaml"),
    },
    SkillFile {
        rel: "ply-verify/SKILL.md",
        body: include_str!("../../../skills/ply-verify/SKILL.md"),
    },
    SkillFile {
        rel: "ply-verify/agents/openai.yaml",
        body: include_str!("../../../skills/ply-verify/agents/openai.yaml"),
    },
];

/// What one run wrote, and what it left alone.
pub struct Written {
    pub created: Vec<String>,
    /// Files that were already there and already identical -- nothing to do,
    /// and reported separately from a real refusal so a second run of the
    /// same command reads as the no-op it is.
    pub unchanged: Vec<String>,
    /// Files that were already there and say something different. Never
    /// overwritten without being asked: a skill a user has edited is their
    /// edit, not a stale copy to reclaim.
    pub differing: Vec<String>,
}

/// Writes every skill under `dest`, creating directories as needed.
///
/// An existing file whose content already matches is left alone and counted
/// as unchanged. An existing file that differs is left alone too and named
/// in `differing`, unless `force` -- because the likely reason a shipped
/// skill differs on disk is that someone edited it deliberately, and
/// silently replacing that is the kind of thing a tool gets distrusted for.
pub fn write_skills(dest: &Path, force: bool) -> Result<Written> {
    let mut out = Written {
        created: Vec::new(),
        unchanged: Vec::new(),
        differing: Vec::new(),
    };
    for file in SKILL_FILES {
        let path = dest.join(file.rel);
        if let Ok(existing) = std::fs::read_to_string(&path) {
            if existing == file.body {
                out.unchanged.push(file.rel.to_string());
                continue;
            }
            if !force {
                out.differing.push(file.rel.to_string());
                continue;
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, file.body).with_context(|| format!("writing {}", path.display()))?;
        out.created.push(file.rel.to_string());
    }
    Ok(out)
}

/// Where the skills go when the caller names nowhere: the directory Claude
/// Code reads them from, relative to wherever the command was run.
pub fn default_destination() -> PathBuf {
    PathBuf::from(".claude/skills")
}

/// The plain report a person reads. Written for someone who has just
/// installed Ply and does not yet know what a skill is for.
pub fn report(dest: &Path, written: &Written) -> String {
    let mut out = String::new();
    if !written.created.is_empty() {
        out.push_str(&format!(
            "Wrote {} skill file(s) to {}. These are the guides for writing code Ply can \
             check and for reading what it reports -- an assistant working in this directory \
             will pick them up from here.\n",
            written.created.len(),
            dest.display()
        ));
    }
    if !written.unchanged.is_empty() {
        out.push_str(&format!(
            "{} file(s) were already there and already identical, so nothing was rewritten.\n",
            written.unchanged.len()
        ));
    }
    if !written.differing.is_empty() {
        out.push_str(&format!(
            "Left alone because what is on disk differs from what this build ships -- most \
             likely you edited it, and that is yours to keep. Pass `--force` to replace: {}\n",
            written.differing.join(", ")
        ));
    }
    if out.is_empty() {
        out.push_str("Nothing to write.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// The invariant, rather than one assertion per skill: walk the real
    /// `skills/` directory and fail naming anything the shipped list does
    /// not carry.
    ///
    /// A skill added later is exactly the thing that would otherwise ship
    /// silently missing -- the tool would install four guides out of five
    /// and say nothing, which is the shape of every absence bug this
    /// project exists to refuse.
    #[test]
    fn every_skill_in_the_repository_is_shipped() {
        let root = repo_root().join("skills");
        let mut on_disk: Vec<String> = Vec::new();
        for skill in std::fs::read_dir(&root).expect("the skills directory exists") {
            let skill = skill.unwrap().path();
            if !skill.is_dir() {
                continue;
            }
            let name = skill.file_name().unwrap().to_string_lossy().to_string();
            for entry in walk(&skill) {
                let rel = entry.strip_prefix(&skill).unwrap().to_string_lossy();
                on_disk.push(format!("{name}/{rel}"));
            }
        }
        on_disk.sort();

        let mut shipped: Vec<String> = SKILL_FILES.iter().map(|f| f.rel.to_string()).collect();
        shipped.sort();

        assert_eq!(
            on_disk, shipped,
            "every file under `skills/` has to be embedded in the binary, or `cargo ply \
             skills` installs a subset and says nothing about the rest. Add the missing \
             entries to `SKILL_FILES`."
        );
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    /// What a person actually gets: real files, with the real content, in
    /// the directory an assistant reads them from.
    #[test]
    fn writing_the_skills_produces_files_an_assistant_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(".claude/skills");
        let written = write_skills(&dest, false).unwrap();

        assert_eq!(
            written.created.len(),
            SKILL_FILES.len(),
            "a first run into an empty directory writes everything: {written:?}",
            written = written.created
        );
        let rule_nine = std::fs::read_to_string(dest.join("ply-checkable-code/SKILL.md"))
            .expect("the guide to writing checkable code is the point of shipping these");
        assert!(
            rule_nine.contains("checkable too"),
            "the guidance that actually shipped has to be this build's -- a guide describing a \
             Ply that does not match the binary beside it is worse than none"
        );
    }

    /// Running it twice changes nothing and says so, rather than reporting
    /// ten writes that did not happen.
    #[test]
    fn a_second_run_writes_nothing_and_reports_that_plainly() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("skills");
        write_skills(&dest, false).unwrap();
        let again = write_skills(&dest, false).unwrap();

        assert!(again.created.is_empty(), "nothing had changed");
        assert_eq!(again.unchanged.len(), SKILL_FILES.len());
        assert!(
            report(&dest, &again).contains("already identical, so nothing was rewritten"),
            "a no-op has to read as a no-op: {}",
            report(&dest, &again)
        );
    }

    /// An edited skill is the user's, and is not reclaimed silently.
    #[test]
    fn an_edited_skill_is_left_alone_and_named_rather_than_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("skills");
        write_skills(&dest, false).unwrap();

        let edited = dest.join("ply-author/SKILL.md");
        std::fs::write(&edited, "my own notes\n").unwrap();

        let second = write_skills(&dest, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(&edited).unwrap(),
            "my own notes\n",
            "an edit someone made deliberately must survive a second run"
        );
        assert!(
            second
                .differing
                .contains(&"ply-author/SKILL.md".to_string()),
            "and be named, or the file is quietly out of date with no way to tell: {:?}",
            second.differing
        );
        let text = report(&dest, &second);
        assert!(
            text.contains("Pass `--force` to replace"),
            "with the one thing a reader needs next: {text}"
        );

        let forced = write_skills(&dest, true).unwrap();
        assert!(
            forced.created.contains(&"ply-author/SKILL.md".to_string()),
            "and `--force` really replaces it: {:?}",
            forced.created
        );
    }
}
