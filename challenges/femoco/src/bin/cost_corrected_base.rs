//! Research-only rebuild at certified rotation precision. Not a judge entry.
use femoco_walk::circuit::{Builder, OperationType as K};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::sim;
use femoco_walk::spec::{sa::parse_payload, EncodingSpec};
use femoco_walk::walk::sa_low::{self, Params, Tweaks};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: cost_corrected_base PAYLOAD INSTANCE OUT_JSON".into());
    }
    if !matches!(args[1].as_str(), "reiher" | "li") {
        return Err("unknown instance".into());
    }
    if let Err(e) = std::fs::remove_file(&args[2]) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e.to_string());
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&args[0]).map_err(|e| e.to_string())?;
    let sa = parse_payload(&format!("{}-corrected-research", args[1]), &bytes)?;
    let catalogue: Value = serde_json::from_slice(
        &std::fs::read(root.join("rigorous/promotions.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for c in catalogue["candidates"].as_array().ok_or("no catalogue")? {
        let id = c["id"].as_str().ok_or("no id")?;
        if !id.starts_with(&format!("{}-", args[1])) {
            continue;
        }
        let build = &c["build"];
        let string = |key: &str| build[key].as_str().ok_or_else(|| format!("missing {key}"));
        let number = |key: &str| string(key)?.parse::<u32>().map_err(|e| e.to_string());
        let mut p = Params::for_spec(&sa);
        p.outer.1 = number("FEMOCO_SA_MU_O")?;
        p.inner.1 = number("FEMOCO_SA_MU_I")?;
        p.outer_a = number("FEMOCO_SA_OUTER_A")? as usize;
        p.inner_a = number("FEMOCO_SA_INNER_A")? as usize;
        p.tw = if string("FEMOCO_WALK_ARCH")? == "sa-toff" {
            Tweaks::parse(string("FEMOCO_SA_TWEAKS")?)
        } else {
            Tweaks::OFF
        };
        // The old Reiher cache budgets cannot hold even one 26-bit rotation.
        // Recalibrate only those too-small caches; this is research configuration,
        // not a change to any shipped knob or historical circuit.
        if p.tw.rank_hold > 0 && p.tw.rank_hold <= u16::from(sa.beta) {
            let old_beta = if args[1] == "reiher" { 16 } else { 15 };
            p.tw.rank_hold = u16::from(sa.beta) + 1;
            if p.tw.rank_park > 0 {
                p.tw.rank_park =
                    (p.tw.rank_park + u16::from(sa.beta) - old_beta).min(p.tw.rank_hold);
            }
        }
        let map = sa_low::lane_map(&sa, p)?;
        let rounding = map.rounding_error(&sa)?;
        if rounding > femoco_walk::score::max_rounding_error() {
            return Err(format!(
                "{id} fails exact 0.1 mHa coefficient bound: {rounding}"
            ));
        }
        let mut b = Builder::new(sa.system_qubits());
        b.declare_uniform(map.uniform_bits());
        let ledger = sa_low::emit(&sa, &map, &mut b, p);
        let ops = b.finish();
        let layout = sim::Layout {
            system: sa.system_qubits(),
            uniform: map.uniform_bits(),
        };
        let inner = map.inner_bits().map(|(lo, width)| sim::Inner { lo, width });
        let tracker = sim::givens_tracker(&sa);
        let compiled = sim::compile_sa(&ops, &layout, tracker, inner)?;
        let (to, go) = ledger.expected_sum("outer");
        let (tc, gc) = ledger.expected_sum("copy");
        let givens = go + 2 * gc;
        let reflection = f64::from(map.uniform_bits() - 2) + f64::from(map.inner_width() - 2);
        let swaps = if p.swap { 4.0 * (sa.n + 1) as f64 } else { 0.0 };
        let expected = to + 2.0 * tc + reflection + swaps;
        let static_t = ops
            .iter()
            .filter(|op| matches!(op.kind, K::CCX | K::CCZ))
            .count();
        let q = compiled.q_peak + tracker.map_or(0, |t| t.phase_gradient_qubits());
        results.push(json!({"source_candidate": id, "role": c["role"], "beta": sa.beta,
            "runtime_rank_hold": p.tw.rank_hold, "runtime_rank_park": p.tw.rank_park,
            "exact_coefficient_1norm_Ha": rounding.to_string(),
            "lambda_Ha": sa.lambda.to_string(), "emitted_ops": ops.len(),
            "static_CCX_CCZ": static_t, "logical_Givens": givens,
            "expected_toffoli_harness_givens_charge": expected+givens as f64*2.0*f64::from(sa.beta-2),
            "expected_toffoli_low_2beta_givens_charge": expected+givens as f64*2.0*f64::from(sa.beta),
            "logical_qubits_peak": q,
            "lane_map_sha256": hex::encode(Sha256::digest(map.to_bytes())),
            "scope": "emitted and compiled DFTHC base only; new full symbolic equivalence not run"}));
        eprintln!("{id}: beta {} Q {q}", sa.beta);
    }
    if results.len() != 4 {
        return Err("expected four catalogue circuits".into());
    }
    let report = json!({"schema": "femoco-corrected-base-resources-v1", "instance": args[1],
        "payload_sha256": hex::encode(Sha256::digest(&bytes)), "circuits": results,
        "source_sha256": hex::encode(Sha256::digest(include_bytes!("cost_corrected_base.rs"))),
        "does_not_include_residual_correction": true});
    std::fs::write(
        &args[2],
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("cost_corrected_base: {e}");
        std::process::exit(2);
    }
}
