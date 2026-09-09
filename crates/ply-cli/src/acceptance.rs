//! Named application acceptance evidence.
//!
//! A claim names one Cargo package, integration-test target, and exact
//! libtest name. The runner first proves that identity from Cargo's artifact
//! stream, then executes only that binary. Its result stays separate from
//! the function-contract tree and is never written to `ply.lock`.

use std::path::{Component as PathComponent, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ply_core::diag::{AcceptanceOutcome, AcceptanceResult, AcceptanceTestIdentity};
use ply_core::model::{AcceptanceClaim, Component, Document};

const DEFAULT_ACCEPTANCE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug)]
struct CargoPackage {
    id: String,
    name: String,
    dir: PathBuf,
    manifest: PathBuf,
    test_targets: Vec<String>,
}

#[derive(Debug)]
struct CargoContext {
    workspace_root: PathBuf,
    packages: Vec<CargoPackage>,
}

enum SetupFailure {
    Timeout(String),
    Tool(String),
    Cancelled,
}

/// Run every acceptance claim in deterministic id order. Ordinary setup,
/// build, and test failures become attributed results; interruption remains
/// a run-level error so callers preserve the existing verification record.
pub fn run(
    crate_dir: &Path,
    document: &Document,
    timeout_secs: Option<u32>,
) -> Result<Vec<AcceptanceResult>> {
    if document.acceptance.is_empty() {
        return Ok(Vec::new());
    }
    let timeout = Duration::from_secs(
        timeout_secs
            .map(u64::from)
            .unwrap_or(DEFAULT_ACCEPTANCE_TIMEOUT_SECS),
    );
    let components = declared_component_paths(document);
    let context = cargo_context(crate_dir, timeout);
    let mut claims = document.acceptance.iter().collect::<Vec<_>>();
    claims.sort_by_key(|(name, _)| *name);

    claims
        .into_iter()
        .map(|(id, claim)| {
            let outcome = match &context {
                Ok(context) => run_one(context, &components, claim, timeout)?,
                Err(SetupFailure::Timeout(detail)) => (AcceptanceOutcome::Timeout, detail.clone()),
                Err(SetupFailure::Tool(detail)) => (AcceptanceOutcome::ToolError, detail.clone()),
                Err(SetupFailure::Cancelled) => {
                    bail!("verification was interrupted while resolving acceptance targets")
                }
            };
            let mut result = to_result(id, claim, outcome);
            if let Ok(context) = &context {
                normalize_result_paths(crate_dir, context, claim, &mut result);
            }
            Ok(result)
        })
        .collect()
}

pub fn not_run(document: &Document, detail: &str) -> Vec<AcceptanceResult> {
    let mut claims = document.acceptance.iter().collect::<Vec<_>>();
    claims.sort_by_key(|(name, _)| *name);
    claims
        .into_iter()
        .map(|(id, claim)| to_result(id, claim, (AcceptanceOutcome::NotRun, detail.to_string())))
        .collect()
}

fn declared_component_paths(document: &Document) -> Vec<String> {
    fn walk(
        prefix: &str,
        components: &indexmap::IndexMap<String, Component>,
        out: &mut Vec<String>,
    ) {
        for (name, component) in components {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            out.push(path.clone());
            walk(&path, &component.components, out);
        }
    }

    let mut paths = Vec::new();
    walk("", &document.components, &mut paths);
    paths
}

fn to_result(
    id: &str,
    claim: &AcceptanceClaim,
    (outcome, detail): (AcceptanceOutcome, String),
) -> AcceptanceResult {
    AcceptanceResult {
        id: format!("ply.yaml::{id}"),
        requirement: claim.requirement.clone(),
        component: claim.component.clone(),
        entry: claim.entry.clone(),
        test: AcceptanceTestIdentity {
            package: claim.test.package.clone(),
            target: claim.test.target.clone(),
            name: claim.test.name.clone(),
        },
        inputs: claim.inputs.clone(),
        expected: claim.expected.clone(),
        required: claim.required,
        outcome,
        detail,
    }
}

fn cargo_context(
    crate_dir: &Path,
    timeout: Duration,
) -> std::result::Result<CargoContext, SetupFailure> {
    let mut command = Command::new("cargo");
    command
        .args(["metadata", "--format-version=1", "--no-deps"])
        .current_dir(crate_dir);
    let output = ply_core::engines::run_with_timeout(&mut command, timeout).map_err(|error| {
        SetupFailure::Tool(format!("could not start Cargo metadata: {error:#}"))
    })?;
    if output.cancelled {
        return Err(SetupFailure::Cancelled);
    }
    if output.timed_out {
        return Err(SetupFailure::Timeout(
            "Cargo metadata timed out before acceptance targets were resolved".into(),
        ));
    }
    if !output.status.success() {
        return Err(SetupFailure::Tool(format!(
            "Cargo metadata failed while resolving acceptance targets: {}",
            concise(&output.stderr_string())
        )));
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        SetupFailure::Tool(format!("Cargo metadata returned invalid JSON: {error}"))
    })?;
    let workspace_root = json_path(&value, "workspace_root")?;
    let packages = value["packages"]
        .as_array()
        .ok_or_else(|| SetupFailure::Tool("Cargo metadata returned no package list".into()))?
        .iter()
        .map(|package| {
            let manifest = json_path(package, "manifest_path")?;
            let dir = manifest
                .parent()
                .ok_or_else(|| {
                    SetupFailure::Tool(format!(
                        "package manifest {} has no parent",
                        manifest.display()
                    ))
                })?
                .to_path_buf();
            let test_targets = package["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|target| {
                    target["kind"]
                        .as_array()
                        .is_some_and(|kinds| kinds.iter().any(|kind| kind == "test"))
                })
                .filter_map(|target| target["name"].as_str().map(str::to_string))
                .collect();
            Ok(CargoPackage {
                id: json_string(package, "id")?,
                name: json_string(package, "name")?,
                dir,
                manifest,
                test_targets,
            })
        })
        .collect::<std::result::Result<Vec<_>, SetupFailure>>()?;

    Ok(CargoContext {
        workspace_root,
        packages,
    })
}

fn json_path(value: &serde_json::Value, key: &str) -> std::result::Result<PathBuf, SetupFailure> {
    value[key]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| SetupFailure::Tool(format!("Cargo metadata omitted `{key}`")))
}

fn json_string(value: &serde_json::Value, key: &str) -> std::result::Result<String, SetupFailure> {
    value[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| SetupFailure::Tool(format!("Cargo metadata omitted `{key}`")))
}

fn run_one(
    context: &CargoContext,
    components: &[String],
    claim: &AcceptanceClaim,
    timeout: Duration,
) -> Result<(AcceptanceOutcome, String)> {
    if !components.contains(&claim.component) {
        return Ok((
            AcceptanceOutcome::ToolError,
            format!(
                "acceptance component `{}` is not declared in this document",
                claim.component
            ),
        ));
    }
    let matching = context
        .packages
        .iter()
        .filter(|package| package.name == claim.test.package)
        .collect::<Vec<_>>();
    let package = match matching.as_slice() {
        [package] => *package,
        [] => {
            return Ok((
                AcceptanceOutcome::ToolError,
                format!(
                    "Cargo workspace has no package named `{}`",
                    claim.test.package
                ),
            ));
        }
        packages => {
            return Ok((
                AcceptanceOutcome::ToolError,
                format!(
                    "Cargo workspace has {} packages named `{}`; the target is ambiguous",
                    packages.len(),
                    claim.test.package
                ),
            ));
        }
    };
    if !package.test_targets.contains(&claim.test.target) {
        return Ok((
            AcceptanceOutcome::ToolError,
            format!(
                "package `{}` has no integration-test target named `{}`",
                claim.test.package, claim.test.target
            ),
        ));
    }
    for path in claim.inputs.iter().chain(&claim.expected) {
        let Err(reason) = validate_input_path(&package.dir, path) else {
            continue;
        };
        return Ok((AcceptanceOutcome::ToolError, reason));
    }
    if manifest_disables_harness(&package.manifest, &claim.test.target)? {
        return Ok((
            AcceptanceOutcome::ToolError,
            format!(
                "integration-test target `{}` sets `harness = false`; acceptance requires libtest so exact execution can be established",
                claim.test.target
            ),
        ));
    }

    let mut build = Command::new("cargo");
    build
        .args([
            "test",
            "--no-run",
            "--message-format=json-render-diagnostics",
            "-p",
            &claim.test.package,
            "--test",
            &claim.test.target,
        ])
        // Cargo discovers `.cargo/config.toml` from the invocation directory
        // upward. Starting at the workspace root silently drops a member's
        // own configuration and can build different code than native Cargo.
        .current_dir(&package.dir);
    let built = ply_core::engines::run_with_timeout(&mut build, timeout)
        .context("starting the Cargo acceptance build")?;
    if built.cancelled {
        bail!("verification was interrupted while building an acceptance test");
    }
    if built.timed_out {
        return Ok((
            AcceptanceOutcome::Timeout,
            format!(
                "building integration-test target `{}` exceeded {} seconds",
                claim.test.target,
                timeout.as_secs()
            ),
        ));
    }
    if !built.status.success() {
        return Ok((
            AcceptanceOutcome::ToolError,
            format!(
                "integration-test target `{}` did not build: {}",
                claim.test.target,
                concise(&built.stderr_string())
            ),
        ));
    }
    let executable = match select_artifact(&built.stdout_string(), &package.id, &claim.test.target)
    {
        Ok(path) => path,
        Err(error) => return Ok((AcceptanceOutcome::ToolError, error.to_string())),
    };
    if !executable.is_file() {
        return Ok((
            AcceptanceOutcome::ToolError,
            format!(
                "Cargo named {}, but that acceptance executable does not exist",
                executable.display()
            ),
        ));
    }

    let mut test = Command::new("cargo");
    // Cargo owns the runtime contract: build-script link-search paths,
    // package environment, toolchain selection, and the package cwd. The
    // JSON build above established the exact artifact identity; this second
    // invocation is normally a cache hit and asks Cargo to launch only that
    // package/target with one exact libtest filter.
    test.args([
        "test",
        "-p",
        &claim.test.package,
        "--test",
        &claim.test.target,
        "--",
        "--exact",
        &claim.test.name,
    ])
    .current_dir(&package.dir)
    .env_remove("RUST_TEST_NOCAPTURE");
    let ran = ply_core::engines::run_with_timeout(&mut test, timeout)
        .context("starting the exact acceptance test")?;
    if ran.cancelled {
        bail!("verification was interrupted while running an acceptance test");
    }
    let stdout = ran.stdout_string();
    let outcome = classify_test_run(
        ran.status.success(),
        ran.timed_out,
        &stdout,
        &claim.test.name,
    );
    let detail = match outcome {
        AcceptanceOutcome::Passed => format!("exact integration test `{}` passed", claim.test.name),
        AcceptanceOutcome::Failed => format!(
            "exact integration test `{}` failed: {}",
            claim.test.name,
            concise(&format!("{stdout}\n{}", ran.stderr_string()))
        ),
        AcceptanceOutcome::Timeout => format!(
            "exact integration test `{}` exceeded {} seconds",
            claim.test.name,
            timeout.as_secs()
        ),
        AcceptanceOutcome::NotRun => format!(
            "integration-test target `{}` did not execute one non-ignored test named exactly `{}`",
            claim.test.target, claim.test.name
        ),
        AcceptanceOutcome::ToolError => format!(
            "integration-test executable did not produce a valid libtest result for `{}`: {}",
            claim.test.name,
            concise(&format!("{stdout}\n{}", ran.stderr_string()))
        ),
    };
    Ok((outcome, detail))
}

fn validate_input_path(package_dir: &Path, declared: &str) -> std::result::Result<(), String> {
    let path = Path::new(declared);
    if declared.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, PathComponent::Normal(_)))
    {
        return Err(format!(
            "acceptance path `{declared}` must be a portable package-relative path with no `.` or `..` segment"
        ));
    }
    let full = package_dir.join(path);
    if !full.is_file() {
        return Err(format!(
            "acceptance path `{declared}` does not name a regular file under package directory {}",
            package_dir.display()
        ));
    }
    Ok(())
}

fn normalize_result_paths(
    verification_root: &Path,
    context: &CargoContext,
    claim: &AcceptanceClaim,
    result: &mut AcceptanceResult,
) {
    let Some(package) = context
        .packages
        .iter()
        .find(|package| package.name == claim.test.package)
    else {
        return;
    };
    let verification_root = verification_root
        .canonicalize()
        .unwrap_or_else(|_| verification_root.to_path_buf());
    let package_prefix = relative_path(&verification_root, &package.dir).unwrap_or_else(|| {
        package
            .dir
            .strip_prefix(&context.workspace_root)
            .unwrap_or(Path::new(""))
            .to_path_buf()
    });
    let normalize = |path: &str| {
        package_prefix
            .join(path)
            .to_string_lossy()
            .replace('\\', "/")
    };
    result.inputs = claim.inputs.iter().map(|path| normalize(path)).collect();
    result.expected = claim.expected.iter().map(|path| normalize(path)).collect();
}

fn manifest_disables_harness(manifest: &Path, target: &str) -> Result<bool> {
    let text = std::fs::read_to_string(manifest)
        .with_context(|| format!("reading package manifest {}", manifest.display()))?;
    let manifest: toml::Value = toml::from_str(&text)
        .with_context(|| format!("parsing package manifest {}", manifest.display()))?;
    Ok(manifest
        .get("test")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .any(|test| {
            test.get("name").and_then(toml::Value::as_str) == Some(target)
                && test.get("harness").and_then(toml::Value::as_bool) == Some(false)
        }))
}

fn relative_path(from: &Path, to: &Path) -> Option<PathBuf> {
    let from = from.components().collect::<Vec<_>>();
    let to = to.components().collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        return None;
    }
    let mut path = PathBuf::new();
    for component in &from[common..] {
        if matches!(component, PathComponent::Normal(_)) {
            path.push("..");
        }
    }
    for component in &to[common..] {
        path.push(component.as_os_str());
    }
    Some(path)
}

fn concise(text: &str) -> String {
    const LIMIT: usize = 4_000;
    let trimmed = text.trim();
    if trimmed.len() <= LIMIT {
        return trimmed.to_string();
    }
    let mut start = trimmed.len() - LIMIT;
    while !trimmed.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &trimmed[start..])
}

fn select_artifact(stream: &str, package_id: &str, target: &str) -> Result<PathBuf> {
    let mut matches = Vec::new();
    for line in stream.lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let is_test = message["target"]["kind"]
            .as_array()
            .is_some_and(|kinds| kinds.iter().any(|kind| kind == "test"));
        if message["reason"] != "compiler-artifact"
            || message["package_id"] != package_id
            || message["target"]["name"] != target
            || !is_test
        {
            continue;
        }
        if let Some(executable) = message["executable"].as_str() {
            let executable = PathBuf::from(executable);
            if !matches.contains(&executable) {
                matches.push(executable);
            }
        }
    }

    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => bail!(
            "Cargo produced no executable for integration-test target `{target}` in package `{package_id}`"
        ),
        many => bail!(
            "Cargo produced {} executables for integration-test target `{target}` in package `{package_id}`; the acceptance test is ambiguous",
            many.len()
        ),
    }
}

fn classify_test_run(
    success: bool,
    timed_out: bool,
    stdout: &str,
    exact_name: &str,
) -> AcceptanceOutcome {
    if timed_out {
        return AcceptanceOutcome::Timeout;
    }
    // Test and subprocess output can splice itself into libtest's per-test
    // line even with Rust capture enabled. The final summary is emitted
    // after all test output, so use the last summary and the process status.
    // `--exact` guarantees that a one-test summary can belong only to the
    // declared name.
    let lines = stdout.lines().collect::<Vec<_>>();
    let summary_count = lines
        .iter()
        .filter(|line| line.trim_start().starts_with("test result:"))
        .count();
    let one_test_run_count = lines
        .iter()
        .filter(|line| line.trim() == "running 1 test")
        .count();
    let Some(summary_index) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("test result:"))
    else {
        return AcceptanceOutcome::ToolError;
    };
    let summary = lines[summary_index].trim();
    let marker = format!("test {exact_name} ...");
    let named_completion = lines[..summary_index]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, line)| line.find(&marker).map(|start| (index, &line[start..])))
        .is_some_and(|(index, line)| {
            line.trim_end().ends_with("ok")
                || line.trim_end().ends_with("FAILED")
                || lines[index + 1..summary_index]
                    .iter()
                    .any(|line| matches!(line.trim(), "ok" | "FAILED"))
        });

    match (success, named_completion, summary) {
        (true, true, line)
            if line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;") =>
        {
            if summary_count == 1 && one_test_run_count == 1 {
                AcceptanceOutcome::Passed
            } else {
                AcceptanceOutcome::ToolError
            }
        }
        (false, true, line)
            if line.starts_with("test result: FAILED. 0 passed; 1 failed; 0 ignored;") =>
        {
            if summary_count == 1 && one_test_run_count == 1 {
                AcceptanceOutcome::Failed
            } else {
                AcceptanceOutcome::ToolError
            }
        }
        (true, _, line) if line.starts_with("test result: ok. 0 passed; 0 failed; 1 ignored;") => {
            AcceptanceOutcome::NotRun
        }
        (true, _, line) if line.starts_with("test result: ok. 0 passed; 0 failed; 0 ignored;") => {
            AcceptanceOutcome::NotRun
        }
        _ => AcceptanceOutcome::ToolError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_selection_requires_exact_package_target_and_one_executable() {
        let stream = concat!(
            r#"{"reason":"compiler-artifact","package_id":"path+file:///app#app@0.1.0","target":{"name":"helper","kind":["lib"]},"executable":null}"#,
            "\n",
            r#"{"reason":"compiler-artifact","package_id":"path+file:///app#app@0.1.0","target":{"name":"venue_response","kind":["test"]},"executable":"/tmp/venue-a"}"#,
            "\n",
            r#"{"reason":"compiler-artifact","package_id":"path+file:///other#other@0.1.0","target":{"name":"venue_response","kind":["test"]},"executable":"/tmp/venue-other"}"#,
        );

        assert_eq!(
            select_artifact(stream, "path+file:///app#app@0.1.0", "venue_response").unwrap(),
            std::path::PathBuf::from("/tmp/venue-a")
        );
        assert!(select_artifact("", "app", "venue_response").is_err());

        let ambiguous = format!(
            "{stream}\n{}",
            r#"{"reason":"compiler-artifact","package_id":"path+file:///app#app@0.1.0","target":{"name":"venue_response","kind":["test"]},"executable":"/tmp/venue-b"}"#
        );
        assert!(
            select_artifact(&ambiguous, "path+file:///app#app@0.1.0", "venue_response").is_err()
        );
    }

    #[test]
    fn exact_libtest_output_never_turns_zero_or_ignored_into_a_pass() {
        use ply_core::diag::AcceptanceOutcome;

        assert_eq!(
            classify_test_run(false, true, "", "decimal_strings_map"),
            AcceptanceOutcome::Timeout
        );
        assert_eq!(
            classify_test_run(
                true,
                false,
                "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured;\n",
                "decimal_strings_map"
            ),
            AcceptanceOutcome::NotRun
        );
        assert_eq!(
            classify_test_run(
                true,
                false,
                "running 1 test\ntest decimal_strings_map ... ignored\ntest result: ok. 0 passed; 0 failed; 1 ignored; 0 measured;\n",
                "decimal_strings_map"
            ),
            AcceptanceOutcome::NotRun
        );
        assert_eq!(
            classify_test_run(
                true,
                false,
                "running 1 test\nproduction progress:test decimal_strings_map ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n",
                "decimal_strings_map"
            ),
            AcceptanceOutcome::Passed
        );
        assert_eq!(
            classify_test_run(
                false,
                false,
                "running 1 test\ntest decimal_strings_map ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured;\n",
                "decimal_strings_map"
            ),
            AcceptanceOutcome::Failed
        );
        assert_eq!(
            classify_test_run(false, false, "process aborted", "decimal_strings_map"),
            AcceptanceOutcome::ToolError
        );
        assert_eq!(
            classify_test_run(
                true,
                false,
                "running 1 test\ntest decimal_strings_map ... \nrunning 1 test\ntest unrelated ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n",
                "decimal_strings_map"
            ),
            AcceptanceOutcome::ToolError,
            "a nested passing test must not complete the selected outer test"
        );
    }
}
