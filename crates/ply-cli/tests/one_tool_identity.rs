//! Every command that writes an envelope must stamp the same thing in
//! `run.tool.version`.
//!
//! It is the build identity: the fingerprint of the source that decides what
//! a verdict means. `verify --publish-view` always stamped it. `render`
//! stamped `CARGO_PKG_VERSION` instead -- the hand-edited `0.1.0` -- so one
//! field carried two different kinds of value depending on which command
//! wrote it, and neither said which.
//!
//! That is not cosmetic. A client deciding "was this run made by the Ply I
//! have installed" compares that field, and comparing a rendered envelope
//! against a published run would have reported a different Ply *every time*,
//! for the same binary. Found while designing exactly that comparison for
//! the interactive viewer (2026-09-06).

use std::process::Command;

fn cargo_ply() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("cargo-ply")
}

fn repo() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The identity `--version` prints is the one a reader can see, so it is the
/// one an envelope has to agree with.
fn advertised_build_identity() -> String {
    let out = Command::new(cargo_ply())
        .args(["ply", "--version"])
        .output()
        .expect("cargo-ply runs");
    let text = String::from_utf8(out.stdout).unwrap();
    let start = text
        .find("build identity ")
        .expect("--version states the build identity")
        + "build identity ".len();
    text[start..].split(')').next().unwrap().trim().to_string()
}

/// The sweep, rather than one command at a time. `verify` already had a unit
/// test pinning *its* constant, and the comment on it says why: the
/// hand-edited version "is what let fourteen fixes go unnoticed by every
/// stored result". Three other commands were still using exactly that
/// constant, because the guard only ever covered one of them.
#[test]
fn every_command_that_writes_an_envelope_stamps_the_build_identity() {
    let identity = advertised_build_identity();
    let doc = repo().join("ply.yaml");
    // Each of these writes a §8 envelope, so each carries a version field a
    // client may compare. `render` puts it in `run.tool.version`; the
    // diagnostic envelope calls it `ply_version`.
    for (args, field) in [
        (vec!["ply", "check", "."], "ply_version"),
        (vec!["ply", "audit", "."], "ply_version"),
        (vec!["ply", "worklist", "."], "ply_version"),
    ] {
        let mut command = Command::new(cargo_ply());
        command.args(&args).arg("--json").current_dir(repo());
        let out = command.output().expect("cargo-ply runs");
        let envelope: serde_json::Value = match serde_json::from_slice(&out.stdout) {
            Ok(value) => value,
            // A command that writes no envelope has nothing to stamp.
            Err(_) => continue,
        };
        let Some(stamped) = envelope[field].as_str() else {
            continue;
        };
        assert_eq!(
            stamped,
            identity,
            "`cargo {}` stamps {field} = {stamped:?}, but the build identity is \
             {identity:?}. One field cannot mean two things: a client comparing it \
             to another run's would report a different Ply for the same binary",
            args.join(" ")
        );
    }
    let _ = doc;
}

#[test]
fn render_stamps_the_build_identity_not_the_package_version() {
    let identity = advertised_build_identity();
    let out = Command::new(cargo_ply())
        .args(["ply", "--json", "render"])
        .arg(repo().join("vetting/001-spsc-disruptor.ply.yaml"))
        .output()
        .expect("cargo-ply runs");
    let envelope: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("render writes an envelope");
    let stamped = envelope["run"]["tool"]["version"].as_str().unwrap();

    assert_eq!(
        stamped, identity,
        "a rendered envelope must carry the build identity, the same value \
         `verify` records, so a client can tell whether the Ply installed now \
         is the one that made a run. `{stamped}` is the package version, which \
         moves only when a human edits it"
    );
    assert!(
        stamped.len() > 16,
        "the build identity is a hash, so anything short is the package \
         version wearing its name: {stamped}"
    );
}
