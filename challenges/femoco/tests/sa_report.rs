//! The spectrum-amplified scoring view (spec/SPEC-SA.md section 6): for an sos-sa run the score
//! uses `lambda_eff_used = max(ours, published)`, the `spectral_amplification` block of
//! `score.json` follows its formulas (re-implemented here), and the certificates are bound to their
//! specs.
mod harness_common;

use femoco_walk::score::{score_json, Evaluation};
use femoco_walk::spec::sa::SaSpec;
use femoco_walk::spec::{self, Exact};
use harness_common::Mutation;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::f64::consts::PI;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(rel: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root().join(rel)).unwrap()).unwrap()
}

fn eval_as(spec_id: &str, lambda: Exact, toffoli: f64, qubits: u64, givens: f64) -> Evaluation {
    let mut ev = harness_common::eval_mutant(Mutation::None, 256).unwrap();
    ev.facts.spec_id = spec_id.into();
    ev.facts.encoding = "sos-sa".into();
    ev.facts.lane_map_family = "sa-nested-alias-v1".into();
    ev.facts.executed_givens = givens;
    ev.lambda = lambda;
    ev.rounding_error = Exact::zero();
    ev.toffoli = toffoli;
    ev.qubits = qubits;
    ev
}

#[test]
fn sa_score_uses_the_larger_lambda_eff() {
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let s = spec::load(root(), id).unwrap();
        let d = s.as_any().downcast_ref::<SaSpec>().unwrap();
        let cert = read_json(&format!("specs/{id}/certificate.json"));
        let lam = s.lambda();
        let ev = eval_as(id, lam.clone(), 10_000.0, 1_000, 100.0);
        let j = score_json(&ev);
        let sa = &j["metrics"]["spectral_amplification"];
        assert_eq!(sa["available"], true, "{id}");
        // Ours, independently: E0' = E_up - identity + (lambda - lambda_flat) + (lambda_decl - lambda)
        // with lambda_decl = lambda and no rounding error: E_gap - lambda.
        let e_up = cert["ground_energy_upper_bound"]["E_up"].as_f64().unwrap();
        let gap = e_up - d.e_sos.to_f64();
        let l = lam.to_f64();
        let ours = (gap * (2.0 * l - gap)).sqrt();
        let got = sa["lambda_eff_ours"].as_f64().unwrap();
        assert!((got - ours).abs() < 1e-9 * ours, "{id}: {got} vs {ours}");
        let cert_ours = cert["spectral_amplification"]["lambda_eff_upper"]
            .as_f64()
            .unwrap();
        assert!((got - cert_ours).abs() < 1e-9 * ours);
        let theirs = sa["lambda_eff_published"].as_f64().unwrap();
        assert_eq!(theirs, d.published_lambda_eff);
        let used = ours.max(theirs);
        assert!((sa["lambda_eff_used"].as_f64().unwrap() - used).abs() < 1e-9 * used);
        assert!((ev.score() - used * 10_000.0 * 1_000.0).abs() < 1e-6 * ev.score());
        assert_eq!(j["score"].as_f64().unwrap(), ev.score());
        // Low et al.'s convention: ceil(pi lambda_eff / 0.002) steps; ours: / 0.0032.
        for (key, eps) in [("sigma_pea_1_0_mHa", 0.001), ("controlled_1_6_mHa", 0.0016)] {
            let v = &sa["used"][key];
            let steps = (PI * used / (2.0 * eps)).ceil();
            assert_eq!(v["steps"].as_f64().unwrap(), steps, "{id} {key}");
            assert_eq!(v["total_toffoli"].as_f64().unwrap(), steps * 10_000.0);
            let reg = 2.0 * (steps + 1.0).log2().ceil() - 1.0;
            assert_eq!(v["qubits_lee"].as_f64().unwrap(), 1_000.0 + reg);
        }
        assert_eq!(sa["qubits_without_pe_register"].as_u64().unwrap(), 1_000);
        assert_eq!(
            sa["low2025_givens_charge"]["toffoli_per_step"]
                .as_f64()
                .unwrap(),
            10_000.0 + 4.0 * 100.0
        );
        // The conventions block's lambda_eff view is the same bound.
        let conv = &j["metrics"]["conventions"]["lambda_eff"];
        assert!((conv["lambda_eff"].as_f64().unwrap() - got).abs() < 1e-12 * got);
        println!("{id}: lambda_eff ours {ours:.4}, published {theirs}, used {used:.4}");
    }
}

#[test]
fn a_rounding_error_raises_our_bound() {
    let id = "reiher-sa-v1";
    let s = spec::load(root(), id).unwrap();
    let mut ev = eval_as(id, s.lambda(), 1.0, 1, 0.0);
    let base = score_json(&ev)["metrics"]["spectral_amplification"]["lambda_eff_ours"]
        .as_f64()
        .unwrap();
    ev.rounding_error = Exact::new(1.into(), 10_000u32.into());
    let more = score_json(&ev)["metrics"]["spectral_amplification"]["lambda_eff_ours"]
        .as_f64()
        .unwrap();
    assert!(more > base, "{more} vs {base}");
}

#[test]
fn sa_certificates_are_bound_to_their_specs() {
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let cert = read_json(&format!("specs/{id}/certificate.json"));
        let meta_bytes = std::fs::read(root().join(format!("specs/{id}/spec.json"))).unwrap();
        let meta: Value = serde_json::from_slice(&meta_bytes).unwrap();
        assert_eq!(
            cert["spec_json_sha256"].as_str().unwrap(),
            hex::encode(Sha256::digest(&meta_bytes))
        );
        assert_eq!(cert["identity"]["exact"], meta["identity"]["exact"]);
        assert_eq!(cert["lambda"]["exact"], meta["lambda"]["exact"]);
        let s = spec::load(root(), id).unwrap();
        let d = s.as_any().downcast_ref::<SaSpec>().unwrap();
        // Walk offset lambda - lambda_flat, exactly.
        let off = &cert["walk_offsets"]["sa-nested-alias-v1"];
        let (n, den) = off["exact"].as_str().unwrap().split_once('/').unwrap();
        let got = Exact::new(n.parse().unwrap(), den.parse().unwrap());
        assert_eq!(got, d.lambda.sub(&d.lambda_flat));
        assert_eq!(off["error_factor"], 2);
        assert_eq!(off["plus_lambda_decl_minus_lambda"], true);
    }
}
