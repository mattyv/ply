//! Golden diagnostics (§15). Each `tests/ui/*.ply` is checked and its JSON diagnostics are
//! snapshotted: the JSON is a product surface, so these diffs are reviewed like API changes.
//!
//! Update with `cargo insta review` (or `INSTA_UPDATE=always cargo test -p ply-driver`).

use ply_driver::Analysis;
use std::path::{Path, PathBuf};

fn ui_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ui");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ply"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "tests/ui is empty");
    files
}

#[test]
fn ui_diagnostics_match_their_goldens() {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ui/snapshots"),
    );
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);

    for path in ui_files() {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&path).unwrap();
        let a = Analysis::of_sources(vec![(format!("{name}.ply"), src)]);
        assert!(
            a.has_errors() || !a.diagnostics().is_empty(),
            "{}: a ui test must produce at least one diagnostic",
            path.display()
        );
        settings.bind(|| {
            insta::assert_snapshot!(format!("{name}.json"), a.json());
            insta::assert_snapshot!(format!("{name}.txt"), a.human(ply_diag::Color::Never));
        });
    }
}
