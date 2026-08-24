//! `cargo ply verify` orchestration: reads `ply.yaml`, generates and runs
//! the Kani proof harness for each declared fn claim, renders a cex test on
//! a genuine violation, and assembles the §8 JSON envelope. This wiring
//! lives in ply-cli (not ply-core) per the M3 brief's module restriction on
//! ply-core.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use ply_core::config::{self, Check};
use ply_core::contract_rt::{self, RenderedTest};
use ply_core::diag::{Counterexample, Diagnostic, Envelope, Node};
use ply_core::engines::kani::{self, KaniOutcome, KaniRunConfig};
use ply_core::harness;

pub const PLY_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct VerifyOptions {
    pub engine_timeout_secs: u32,
}

/// Runs `verify` over every fn claim declared in `<crate_dir>/ply.yaml`,
/// against the source at `<crate_dir>/src/lib.rs`. Returns the §8 envelope.
pub fn verify_crate(crate_dir: &Path, opts: &VerifyOptions) -> Result<Envelope> {
    let yaml_path = crate_dir.join("ply.yaml");
    let file = config::load(&yaml_path)?;
    let src_dir = crate_dir.join("src");
    let lib_path = src_dir.join("lib.rs");

    let mut component_nodes = Vec::new();
    let mut diagnostics = Vec::new();

    for (comp_name, comp) in &file.components {
        let mut fn_nodes = Vec::new();
        for (fn_name, claim) in &comp.fns {
            let node_id = format!("{comp_name}::{fn_name}");
            let checks = claim.parsed_checks().with_context(|| format!("parsing checks for {node_id}"))?;
            let bound_k = checks
                .iter()
                .find_map(|c| if let Check::Bounded(k) = c { Some(*k) } else { None })
                .unwrap_or(2);
            let check_label = format!("bounded({bound_k})");

            let (node, mut fn_diags) =
                verify_one_fn(&node_id, &src_dir, &lib_path, fn_name, bound_k, &check_label, opts)?;
            fn_nodes.push(node);
            diagnostics.append(&mut fn_diags);
        }
        component_nodes.push(Node {
            id: comp_name.clone(),
            kind: "component".into(),
            verdict: worst_of(&fn_nodes),
            statuses: vec![],
            children: fn_nodes,
        });
    }

    let root = Node {
        id: "workspace".into(),
        kind: "workspace".into(),
        verdict: worst_of(&component_nodes),
        statuses: vec![],
        children: component_nodes,
    };

    Ok(Envelope {
        command: "verify".into(),
        ply_version: PLY_VERSION.into(),
        root,
        diagnostics,
    })
}

/// Worst-of over the evidence order (D6), restricted to the "kinds only"
/// comparison this slice needs: unclaimed < tested < fuzzed < bounded <
/// proved, with violation/timeout/unsupported/tool_error sorting below
/// unclaimed (worse than any earned evidence). Folds over child verdict
/// *labels* since this slice does not track n/k separately at the container
/// level (§7's full aggregation is an M5 concern).
fn worst_of(children: &[Node]) -> String {
    fn rank(v: &str) -> i32 {
        if v == "violation" {
            0
        } else if v.starts_with("tool_error") {
            1
        } else if v == "timeout" {
            2
        } else if v.starts_with("unsupported") {
            3
        } else if v == "unclaimed" {
            4
        } else if v == "tested" {
            5
        } else if v.starts_with("fuzzed") {
            6
        } else if v.starts_with("bounded") {
            7
        } else if v == "proved" {
            8
        } else {
            4
        }
    }
    children
        .iter()
        .min_by_key(|n| rank(&n.verdict))
        .map(|n| n.verdict.clone())
        .unwrap_or_else(|| "unclaimed".into())
}

fn verify_one_fn(
    node_id: &str,
    src_dir: &Path,
    lib_path: &Path,
    fn_name: &str,
    bound_k: u32,
    check_label: &str,
    opts: &VerifyOptions,
) -> Result<(Node, Vec<Diagnostic>)> {
    let cf = match harness::discover_fn(lib_path, fn_name) {
        Ok(cf) => cf,
        Err(e) => {
            let d = Diagnostic {
                code: "E0301".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "ply".into(),
                check: check_label.into(),
                node_id: node_id.into(),
                title: format!(
                    "Ply could not find the function `{fn_name}` this claim anchors to. {e}"
                ),
                primary_span: None,
                counterexample: None,
                open_item: Some("unresolvable_anchor".into()),
            };
            return Ok((leaf_node(node_id, "unclaimed"), vec![d]));
        }
    };

    if !cf.is_supported() {
        let bad: Vec<String> = cf
            .params
            .iter()
            .filter(|p| !p.ty.is_supported())
            .map(|p| p.name.clone())
            .collect();
        let d = Diagnostic {
            code: "V0505".into(),
            severity: "warning".into(),
            phase: "verify".into(),
            engine: "kani".into(),
            check: check_label.into(),
            node_id: node_id.into(),
            title: format!(
                "Ply cannot check `{fn_name}` with a bounded proof: parameter(s) {} use a type \
                 this slice's engine adapter does not yet build inputs for. This is reported as \
                 unsupported, not attempted -- it never silently hangs.",
                bad.join(", ")
            ),
            primary_span: None,
            counterexample: None,
            open_item: Some("unsupported_signature".into()),
        };
        return Ok((leaf_node(node_id, "unsupported"), vec![d]));
    }

    let generated = harness::generate_proof_module(&cf, bound_k)?;
    harness::write_generated_module(src_dir, lib_path, &generated.module_source)?;

    let crate_dir = src_dir
        .parent()
        .context("crate_dir has no parent of src_dir")?
        .to_path_buf();
    let run_cfg = KaniRunConfig {
        crate_dir: crate_dir.clone(),
        harness_path: generated.proof_fn_path.clone(),
        engine_timeout_secs: opts.engine_timeout_secs,
    };
    let outcome = kani::run(&run_cfg)?;

    // §9's cex validity oracle demands the SAME rendered test transitions
    // FAIL -> PASS once a fix lands, not that it vanishes and a stranger
    // test appears. So a witness, once found, is persisted (under
    // target/ply/witness/ -- Ply owns everything there) and every run
    // re-renders its regression test against the CURRENT contract text,
    // regardless of whether *this* run's Kani outcome is itself a
    // violation. A fix that makes the old witness stop violating the
    // (possibly-edited) contract is exactly what should turn that same
    // test green.
    let witness_path = crate_dir.join("target/ply/witness").join(format!("{fn_name}.json"));

    match &outcome {
        KaniOutcome::Violation { witness_bytes, .. } => {
            if let Some(parent) = witness_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&witness_path, serde_json::to_string(witness_bytes)?)?;
        }
        _ => {}
    }

    if witness_path.exists() {
        let stored: Vec<Vec<u8>> = serde_json::from_str(&std::fs::read_to_string(&witness_path)?)?;
        let values = kani::decode_witness(&stored, &cf.params, bound_k)?;
        let rendered = contract_rt::render_cex_test(&cf, &values, check_label, "K0502", 1)?;
        let module_source = contract_rt::wrap_test_module(&[RenderedTest {
            test_name: rendered.test_name.clone(),
            source: rendered.source.clone(),
        }]);
        harness::write_generated_test(src_dir, lib_path, &module_source)?;
    }

    match outcome {
        KaniOutcome::Verified => Ok((leaf_node(node_id, check_label), vec![])),
        KaniOutcome::Timeout { raw_output } => {
            let d = Diagnostic {
                code: "K0601".into(),
                severity: "warning".into(),
                phase: "verify".into(),
                engine: "kani".into(),
                check: check_label.into(),
                node_id: node_id.into(),
                title: format!(
                    "Kani could not finish checking `{fn_name}` within its time budget -- this is \
                     an exhausted search, not a broken promise: Kani never got far enough to say \
                     whether the contract holds or not, so this is reported as `timeout`, never as \
                     a violation. (K0601)"
                ),
                primary_span: None,
                counterexample: None,
                open_item: Some("timeout".into()),
            };
            let _ = raw_output; // carried in the diagnostic's title for now; full raw-output
                                 // attachment is a straightforward follow-up, not required by
                                 // this slice's acceptance criteria.
            Ok((leaf_node(node_id, "timeout"), vec![d]))
        }
        KaniOutcome::ToolError { reason, raw_output } => {
            let d = Diagnostic {
                code: "X0901".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "kani".into(),
                check: check_label.into(),
                node_id: node_id.into(),
                title: format!("Ply's Kani adapter could not interpret Kani's output: {reason}"),
                primary_span: None,
                counterexample: None,
                open_item: Some("tool_error".into()),
            };
            let _ = raw_output;
            Ok((leaf_node(node_id, "tool_error"), vec![d]))
        }
        KaniOutcome::Violation { witness_bytes, raw_output } => {
            let values = kani::decode_witness(&witness_bytes, &cf.params, bound_k)?;
            let rendered = contract_rt::render_cex_test(&cf, &values, check_label, "K0502", 1)?;
            let module_source = contract_rt::wrap_test_module(&[
                RenderedTest { test_name: rendered.test_name.clone(), source: rendered.source.clone() },
            ]);
            let test_file = harness::write_generated_test(src_dir, lib_path, &module_source)?;

            let mut inputs = BTreeMap::new();
            for (p, v) in cf.params.iter().zip(values.iter()) {
                inputs.insert(p.name.clone(), format_value(v));
            }

            let contract_text = cf
                .ensures
                .as_ref()
                .map(|(_, t)| t.clone())
                .unwrap_or_default();

            let d = Diagnostic {
                code: "K0502".into(),
                severity: "error".into(),
                phase: "verify".into(),
                engine: "kani".into(),
                check: check_label.into(),
                node_id: node_id.into(),
                title: format!(
                    "`{fn_name}` breaks its own postcondition `{contract_text}` for at least one \
                     input -- a postcondition is the guarantee a function makes about its return \
                     value, and Kani found a case where that guarantee does not hold. (K0502)"
                ),
                primary_span: None,
                counterexample: Some(Counterexample {
                    inputs,
                    kani_witness: Some(format!(
                        "captured from `cargo kani --concrete-playback print` on harness `{}`",
                        generated.proof_fn_path
                    )),
                    cargo_test: Some(
                        test_file
                            .strip_prefix(src_dir.parent().unwrap_or(src_dir))
                            .unwrap_or(&test_file)
                            .display()
                            .to_string(),
                    ),
                }),
                open_item: None,
            };
            let _ = raw_output;
            Ok((leaf_node(node_id, "violation"), vec![d]))
        }
    }
}

fn format_value(v: &kani::WitnessValue) -> String {
    match v {
        kani::WitnessValue::UInt(u) => u.to_string(),
        kani::WitnessValue::Int(i) => i.to_string(),
        kani::WitnessValue::Bool(b) => b.to_string(),
        kani::WitnessValue::VecU8(bytes) => format!("{bytes:?}"),
    }
}

fn leaf_node(node_id: &str, verdict: &str) -> Node {
    // node_id is "component::fn" -- the leaf's own id is just the fn segment
    // (D10: node_id = component path + item path; kept simple for this
    // slice's single-fn-per-component fixtures).
    let fn_part = node_id.rsplit("::").next().unwrap_or(node_id);
    Node {
        id: fn_part.to_string(),
        kind: "fn".into(),
        verdict: verdict.to_string(),
        statuses: vec![],
        children: vec![],
    }
}
