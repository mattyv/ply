//! The first concurrent milestone: two real Kani checks may overlap without
//! changing evidence or attaching one proof's failure to the other claim.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

use ply_e2e::{build_cargo_ply, copy_fixture};

fn ensure_lock(fixture: &std::path::Path) {
    if fixture.join("Cargo.lock").is_file() {
        return;
    }
    let locked = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(fixture)
        .output()
        .expect("generating the dependency resolution used by parallel workers");
    assert!(
        locked.status.success(),
        "could not prepare Cargo.lock: {}",
        String::from_utf8_lossy(&locked.stderr)
    );
}

fn run(cargo_ply: &std::path::Path, fixture: &std::path::Path, jobs: usize) -> serde_json::Value {
    if jobs > 1 {
        ensure_lock(fixture);
    }
    let output = Command::new(cargo_ply)
        .args([
            "verify",
            fixture.to_str().unwrap(),
            "--json",
            "--engine-timeout",
            "120",
            "--jobs",
            &jobs.to_string(),
        ])
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .expect("spawning cargo-ply verify");
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "verify did not return JSON: {error}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn function<'a>(root: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    root["children"][0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == id)
        .unwrap_or_else(|| panic!("missing {id} in {root}"))
}

fn without_solver_chosen_inputs(mut envelope: serde_json::Value) -> serde_json::Value {
    for diagnostic in envelope["diagnostics"].as_array_mut().unwrap() {
        if let Some(counterexample) = diagnostic.get_mut("counterexample") {
            if !counterexample.is_null() {
                counterexample["inputs"] =
                    serde_json::json!("valid witness omitted for comparison");
            }
        }
    }
    envelope
}

#[test]
fn concurrent_real_proofs_match_serial_results_and_keep_attribution() {
    let cargo_ply = build_cargo_ply();
    let serial_fixture = copy_fixture("parallelbounded");
    let parallel_fixture = copy_fixture("parallelbounded");
    let original_manifest = std::fs::read(parallel_fixture.path().join("Cargo.toml")).unwrap();

    let serial = run(&cargo_ply, serial_fixture.path(), 1);
    let parallel = run(&cargo_ply, parallel_fixture.path(), 2);
    assert_eq!(
        without_solver_chosen_inputs(parallel.clone()),
        without_solver_chosen_inputs(serial),
        "-j 2 changed the verification envelope beyond Kani's choice among valid witnesses"
    );
    assert_eq!(function(&parallel["root"], "good")["verdict"], "bounded(2)");
    assert_eq!(
        function(&parallel["root"], "good_two")["verdict"],
        "bounded(2)"
    );
    assert_eq!(function(&parallel["root"], "bad")["verdict"], "violation");
    assert_eq!(
        function(&parallel["root"], "bad_two")["verdict"],
        "violation"
    );
    let diagnostics = parallel["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "K0502" && diagnostic["node_id"] == "proofs::bad"
    }));
    assert!(!diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "K0502" && diagnostic["node_id"] == "proofs::good"
    }));
    assert!(!diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "K0502" && diagnostic["node_id"] == "proofs::good_two"
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "K0502" && diagnostic["node_id"] == "proofs::bad_two"
    }));

    assert_eq!(
        std::fs::read(parallel_fixture.path().join("Cargo.toml")).unwrap(),
        original_manifest,
        "parallel bounded checks must not change Cargo features or the manifest"
    );
    let src_entries: Vec<String> = std::fs::read_dir(parallel_fixture.path().join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        src_entries
            .iter()
            .all(|name| !name.starts_with("ply_generated_worker_")),
        "worker proof modules survived: {src_entries:?}"
    );
    assert!(
        !src_entries.iter().any(|name| name == "ply_generated.rs"),
        "the concurrent proof was written into the original source tree"
    );
    let cex =
        std::fs::read_to_string(parallel_fixture.path().join("src/ply_generated_cex.rs")).unwrap();
    assert!(cex.contains("ply_cex_bad"), "{cex}");
    assert!(cex.contains("ply_cex_bad_two"), "{cex}");
    assert!(!cex.contains("ply_cex_good"), "{cex}");
    assert!(!cex.contains("ply_cex_good_two"), "{cex}");

    // Only clean evidence is recorded. On a second run the passing claim is
    // cached while both violated claims run fresh, and concurrency must not
    // turn either cache decision into a crate-wide one.
    let mixed = run(&cargo_ply, parallel_fixture.path(), 2);
    assert_eq!(function(&mixed["root"], "good")["reused"], true);
    assert_eq!(function(&mixed["root"], "good_two")["reused"], true);
    assert_ne!(function(&mixed["root"], "bad")["reused"], true);
    assert_ne!(function(&mixed["root"], "bad_two")["reused"], true);
}

#[test]
fn one_invalid_proof_cannot_poison_another_workers_compilation() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("parallelisolation");
    let original_lib = std::fs::read(fixture.path().join("src/lib.rs")).unwrap();

    let result = run(&cargo_ply, fixture.path(), 2);
    assert_eq!(
        function(&result["root"], "a_good")["verdict"],
        "bounded(2)",
        "{result}"
    );
    assert_eq!(
        function(&result["root"], "z_bad")["verdict"],
        "tool_error",
        "{result}"
    );
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diag| { diag["code"] == "X0901" && diag["node_id"] == "proofs::z_bad" }));
    assert!(!result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diag| { diag["code"] == "X0901" && diag["node_id"] == "proofs::a_good" }));
    assert_eq!(
        std::fs::read(fixture.path().join("src/lib.rs")).unwrap(),
        original_lib
    );
    assert!(!fixture.path().join("src/ply_generated.rs").exists());
}

#[test]
fn build_script_closures_stay_serial_even_when_jobs_is_two() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("parallelbuildscript");

    let result = run(&cargo_ply, fixture.path(), 2);
    assert_eq!(
        function(&result["root"], "first")["verdict"],
        "bounded(2)",
        "{result}"
    );
    assert_eq!(
        function(&result["root"], "second")["verdict"],
        "bounded(2)",
        "{result}"
    );
    assert!(!fixture.path().join(".generator-running").exists());
}

#[test]
fn a_registered_shared_harness_keeps_bounded_work_on_the_serial_path() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("parallelmixed");
    let original_manifest = std::fs::read(fixture.path().join("Cargo.toml")).unwrap();

    let result = run(&cargo_ply, fixture.path(), 2);
    assert_eq!(
        function(&result["root"], "bounded_claim")["verdict"],
        "bounded(2)",
        "{result}"
    );
    assert_eq!(
        function(&result["root"], "sampled_claim")["verdict"],
        "fuzzed(4)",
        "{result}"
    );
    assert_eq!(
        std::fs::read(fixture.path().join("Cargo.toml")).unwrap(),
        original_manifest,
        "temporary harness membership was not restored"
    );
}

#[test]
fn compile_time_manifest_paths_keep_the_closure_on_the_serial_path() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("parallelrelocation");

    let result = run(&cargo_ply, fixture.path(), 2);
    assert_eq!(
        function(&result["root"], "ordinary")["verdict"],
        "bounded(2)",
        "{result}"
    );
    assert_eq!(
        function(&result["root"], "path_sensitive")["verdict"],
        "violation",
        "a relocated manifest changed the compiled function: {result}"
    );
}

#[test]
fn yaml_contract_paths_keep_the_generated_proof_on_the_serial_path() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("parallelyamlrelocation");

    let result = run(&cargo_ply, fixture.path(), 2);
    assert_eq!(
        function(&result["root"], "ordinary")["verdict"],
        "bounded(2)",
        "{result}"
    );
    assert_eq!(
        function(&result["root"], "yaml_sensitive")["verdict"],
        "violation",
        "a YAML-only path macro changed after source relocation: {result}"
    );
}

#[test]
fn a_caller_waits_for_the_callees_real_bound_in_parallel_mode() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("stubverified");

    let result = run(&cargo_ply, fixture.path(), 2);
    let caller = function(&result["root"], "f");
    let callee = function(&result["root"], "g");
    assert_eq!(callee["verdict"], "bounded(2)", "{result}");
    assert_eq!(caller["verdict"], "bounded(2)", "{result}");
    assert!(!caller["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|status| status == "conditional" || status == "owed-evidence"));
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diag| {
            diag["code"] == "W0517"
                && diag["node_id"] == "stubverified::f"
                && diag["title"]
                    .as_str()
                    .is_some_and(|title| title.contains('g'))
        }));
}

#[test]
fn dependency_waves_do_not_reorder_displaced_cache_entries() {
    let cargo_ply = build_cargo_ply();
    let serial_fixture = copy_fixture("stubverified");
    let parallel_fixture = copy_fixture("stubverified");
    let fake_bin = tempfile::tempdir().unwrap();
    let real_cargo = Command::new("/usr/bin/which")
        .arg("cargo")
        .output()
        .unwrap();
    let real_cargo = String::from_utf8(real_cargo.stdout).unwrap();
    let fake_cargo = fake_bin.path().join("cargo");
    std::fs::write(
        &fake_cargo,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"kani\" ]; then\n  case \"$*\" in\n    *--version*) echo 'cargo-kani 0.67.0' ;;\n    *) {cargo} generate-lockfile --offline >/dev/null 2>&1; echo 'VERIFICATION:- SUCCESSFUL' ;;\n  esac\n  exit 0\nfi\nexec {cargo} \"$@\"\n",
            cargo = real_cargo.trim(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        fake_bin.path().display(),
        std::env::var("PATH").unwrap()
    );

    let run_fake = |fixture: &std::path::Path, jobs: usize| {
        let output = Command::new(&cargo_ply)
            .args([
                "verify",
                fixture.to_str().unwrap(),
                "--json",
                "--jobs",
                &jobs.to_string(),
            ])
            .env("PATH", &path)
            .env("CARGO_NET_OFFLINE", "true")
            .output()
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "verify did not return JSON: {error}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    };

    let fresh_fixture = copy_fixture("stubverified");
    assert!(!fresh_fixture.path().join("Cargo.lock").is_file());
    let first = run_fake(fresh_fixture.path(), 1);
    assert_ne!(function(&first["root"], "g")["reused"], true);
    assert!(fresh_fixture.path().join("Cargo.lock").is_file());
    assert!(fresh_fixture.path().join("ply.lock").is_file());
    let second = run_fake(fresh_fixture.path(), 1);
    assert_eq!(function(&second["root"], "g")["reused"], true);
    assert_eq!(function(&second["root"], "f")["reused"], true);

    for fixture in [&serial_fixture, &parallel_fixture] {
        ensure_lock(fixture.path());
        let yaml_path = fixture.path().join("ply.yaml");
        let mut yaml = std::fs::read_to_string(&yaml_path).unwrap();
        yaml.push_str("      z:\n        checks: [bounded(2)]\n");
        std::fs::write(yaml_path, yaml).unwrap();
        let mut source = fixture.read_lib_rs();
        source.push_str(
            "\n#[ply::ensures(|result| *result == x)]\npub fn z(x: u32) -> u32 {\n    x\n}\n",
        );
        fixture.write_lib_rs(&source);
        let first = run_fake(fixture.path(), 1);
        assert_eq!(function(&first["root"], "f")["verdict"], "bounded(2)");

        let changed = fixture
            .read_lib_rs()
            .replace(
                "pub fn g(x: u32) -> u32 {\n    x + 1\n}",
                "pub fn g(x: u32) -> u32 {\n    let x = x;\n    x + 1\n}",
            )
            .replace(
                "pub fn f(x: u32) -> u32 {\n    g(g(x))\n}",
                "pub fn f(x: u32) -> u32 {\n    let x = x;\n    g(g(x))\n}",
            )
            .replace(
                "pub fn z(x: u32) -> u32 {\n    x\n}",
                "pub fn z(x: u32) -> u32 {\n    let x = x;\n    x\n}",
            );
        fixture.write_lib_rs(&changed);
    }

    let serial = run_fake(serial_fixture.path(), 1);
    let parallel = run_fake(parallel_fixture.path(), 2);
    assert_eq!(
        parallel["not_carried_forward"],
        serial["not_carried_forward"]
    );
    let ids: Vec<&str> = parallel["not_carried_forward"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["node_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["stubverified::g", "stubverified::f", "stubverified::z"]
    );
}

#[test]
fn one_worker_timeout_keeps_the_other_results_attributed_and_cleans_up() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("parallelbounded");
    ensure_lock(fixture.path());
    let original_lib = std::fs::read(fixture.path().join("src/lib.rs")).unwrap();
    let fake_bin = tempfile::tempdir().unwrap();
    let real_cargo = Command::new("/usr/bin/which")
        .arg("cargo")
        .output()
        .unwrap();
    let real_cargo = String::from_utf8(real_cargo.stdout).unwrap();
    let fake_cargo = fake_bin.path().join("cargo");
    std::fs::write(
        &fake_cargo,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"kani\" ]; then\n  case \"$*\" in\n    *--version*) echo 'cargo-kani 0.67.0'; exit 0 ;;\n    *ply_proof_bad_two*) printf 'VERIFICATION:- FAILED\\nCBMC timed out.\\n'; exit 1 ;;\n    *) printf 'VERIFICATION:- SUCCESSFUL\\n'; exit 0 ;;\n  esac\nfi\nexec {cargo} \"$@\"\n",
            cargo = real_cargo.trim(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new(&cargo_ply)
        .args([
            "verify",
            fixture.path().to_str().unwrap(),
            "--json",
            "--jobs",
            "2",
        ])
        .env(
            "PATH",
            format!(
                "{}:{}",
                fake_bin.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "verify did not return JSON: {error}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });

    assert_eq!(function(&result["root"], "bad_two")["verdict"], "timeout");
    for id in ["good", "good_two", "bad"] {
        assert_eq!(
            function(&result["root"], id)["verdict"],
            "bounded(2)",
            "{result}"
        );
    }
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diag| { diag["code"] == "K0601" && diag["node_id"] == "proofs::bad_two" }));
    assert!(!result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diag| { diag["code"] == "K0601" && diag["node_id"] != "proofs::bad_two" }));
    assert_eq!(
        std::fs::read(fixture.path().join("src/lib.rs")).unwrap(),
        original_lib
    );
    assert!(std::fs::read_dir(fixture.path().join("src"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("ply_generated_worker_")));
}

#[test]
fn sigint_and_sigterm_cancel_a_blocked_planning_probe() {
    let cargo_ply = build_cargo_ply();
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let fixture = copy_fixture("parallelbounded");
        let original_lib = std::fs::read(fixture.path().join("src/lib.rs")).unwrap();
        let fake_bin = tempfile::tempdir().unwrap();
        let markers = tempfile::tempdir().unwrap();
        let real_cargo = Command::new("/usr/bin/which")
            .arg("cargo")
            .output()
            .unwrap();
        let real_cargo = String::from_utf8(real_cargo.stdout).unwrap();
        let fake_cargo = fake_bin.path().join("cargo");
        std::fs::write(
            &fake_cargo,
            format!(
                r#"#!/bin/sh
if [ "$1" = "kani" ] && [ "$2" = "--version" ]; then
  /bin/sleep 300 &
  child=$!
  echo $$ > {markers}/engine.pid
  echo $child > {markers}/child.pid
  wait
  exit $?
fi
exec {cargo} "$@"
"#,
                markers = markers.path().display(),
                cargo = real_cargo.trim(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake_cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

        let original_path = std::env::var("PATH").unwrap();
        let child = Command::new(&cargo_ply)
            .args([
                "verify",
                fixture.path().to_str().unwrap(),
                "--json",
                "--jobs",
                "2",
            ])
            .env(
                "PATH",
                format!("{}:{original_path}", fake_bin.path().display()),
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        while !markers.path().join("engine.pid").exists() {
            assert!(
                Instant::now() < deadline,
                "the Kani planning probe never started"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(unsafe { libc::kill(child.id() as libc::pid_t, signal) }, 0);
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("interrupted"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for name in ["engine.pid", "child.pid"] {
            let pid: libc::pid_t = std::fs::read_to_string(markers.path().join(name))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let gone_deadline = Instant::now() + Duration::from_secs(3);
            while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < gone_deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_ne!(
                unsafe { libc::kill(pid, 0) },
                0,
                "planning subprocess {pid} survived signal {signal}"
            );
        }
        assert_eq!(
            std::fs::read(fixture.path().join("src/lib.rs")).unwrap(),
            original_lib
        );
        assert!(!fixture.path().join("ply.lock").exists());
    }
}

#[test]
fn interruption_stops_both_active_engines_and_restores_the_workspace() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("parallelbounded");
    ensure_lock(fixture.path());
    let original_manifest = std::fs::read(fixture.path().join("Cargo.toml")).unwrap();
    let original_lib = std::fs::read(fixture.path().join("src/lib.rs")).unwrap();
    let fake_bin = tempfile::tempdir().unwrap();
    let markers = tempfile::tempdir().unwrap();
    let real_cargo = Command::new("/usr/bin/which")
        .arg("cargo")
        .output()
        .unwrap();
    let real_cargo = String::from_utf8(real_cargo.stdout).unwrap();
    let fake_cargo = fake_bin.path().join("cargo");
    std::fs::write(
        &fake_cargo,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"kani\" ]; then\n  case \" $* \" in\n    *\" --version \"*) echo 'cargo-kani 0.67.0'; exit 0 ;;\n  esac\n  /bin/sleep 300 &\n  child=$!\n  echo $$ > {markers}/engine-$$.pid\n  echo $child > {markers}/child-$$.pid\n  wait\n  exit $?\nfi\nexec {cargo} \"$@\"\n",
            markers = markers.path().display(),
            cargo = real_cargo.trim(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

    let original_path = std::env::var("PATH").unwrap();
    let child = Command::new(&cargo_ply)
        .args([
            "verify",
            fixture.path().to_str().unwrap(),
            "--json",
            "--jobs",
            "2",
        ])
        .env(
            "PATH",
            format!("{}:{original_path}", fake_bin.path().display()),
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let active = std::fs::read_dir(markers.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("engine-"))
            .count();
        if active == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "two engine workers never overlapped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // SAFETY: this targets the child process just spawned by this test and
    // sends the ordinary terminal interrupt signal. Ply's handler converts
    // it into coordinated cancellation.
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no partial result was published"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for entry in std::fs::read_dir(markers.path()).unwrap() {
        let entry = entry.unwrap();
        let pid: libc::pid_t = std::fs::read_to_string(entry.path())
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let gone_deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < gone_deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "pid {pid} survived cancellation"
        );
    }
    assert_eq!(
        std::fs::read(fixture.path().join("Cargo.toml")).unwrap(),
        original_manifest
    );
    assert_eq!(
        std::fs::read(fixture.path().join("src/lib.rs")).unwrap(),
        original_lib
    );
    assert!(!fixture.path().join("ply.lock").exists());
    assert!(!fixture.path().join("src/ply_generated.rs").exists());
    assert!(std::fs::read_dir(fixture.path().join("src"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("ply_generated_worker_")));
}
