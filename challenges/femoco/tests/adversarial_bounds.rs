//! Adversarial tests: what `score.json`'s sampling bounds certify.
//!
//! **Regression (the implied error bound was about 2x too small).** `f_bound = ln(1/delta) / K`
//! bounds the fraction of all `(c, s)` lanes that are wrong. Only control-1 lanes test the
//! encoded operator `A = lambda E_s[M_s]`, and about half the lanes have control 0. A circuit
//! wrong on every control-1 lane of a fraction `g` of uniform values has `f = g / 2` of its
//! lanes wrong and is `2 lambda g = 4 lambda f` from the declared operator, so the reported
//! `2 lambda f_bound` was not a bound. For example the harness fixture's `SignAt(s)` mutant has
//! 1 of 128 lanes wrong and `||A' - A|| = lambda / 32 = 4 lambda / 128`. The bound now counts
//! the sampled control-1 lanes (`K_1`, and `K_p1`, `K_d1` for nested runs):
//! `2 lambda ln(1/delta) / K_1`. `f_bound` itself is unchanged, as a statement about all lanes.
//!
//! **Measurement (a documented limit, not changed).** `sampling_lets_through_what_f_bound_says`
//! runs the harness fixture with a wrong-lane fraction `f` set by measurement outcomes and
//! counts how often a whole sample passes; the pass rate follows `(1 - f)^K`. The ignored
//! `default_k_pass_rates` does the same at the default `K`.
mod harness_common;
mod nested_common;

use femoco_walk::circuit::{Op, OperationType, NONE};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::fiat_shamir::{sample, sample_nested};
use femoco_walk::lanemap::df_nested::OneBody;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{FamilyOut, Inputs, DELTA, GRINDING_LOG2};
use femoco_walk::spec::EncodingSpec;
use femoco_walk::taxonomy::Family;
use harness_common::{alias, build, declared_only, ops_file, Mutation, TestSpec};

fn log_term(draws_log2: u32) -> f64 {
    (1.0 / DELTA).ln() + f64::from(draws_log2) * std::f64::consts::LN_2
}

#[test]
fn flat_implied_error_bound_counts_only_control_one_lanes() {
    let k = 4096;
    let ev = harness_common::eval_mutant(Mutation::None, k).unwrap();
    let k1 = sample(&ev.digests, 6, k)
        .lanes
        .iter()
        .filter(|l| l.c)
        .count();
    assert!(k1 < k && k1 > k / 4, "{k1}");
    let lambda = ev.lambda.to_f64();
    for draws in [0, GRINDING_LOG2] {
        let need = 2.0 * lambda * log_term(draws) / k1 as f64;
        let got = ev.implied_error_bound_at(draws);
        assert!(
            got >= need * (1.0 - 1e-12),
            "implied_error_bound {got} is below 2 lambda ln(T/delta) / K_1 = {need}"
        );
    }
    let json = femoco_walk::score::score_json(&ev);
    assert_eq!(json["metrics"]["control_one_samples"], k1);
}

#[test]
fn nested_implied_error_bound_counts_only_control_one_lanes() {
    let k = 2048;
    let ev = nested_common::eval_mutant(OneBody::Direct, nested_common::Mutation::None, k).unwrap();
    let spec = nested_common::spec();
    let map = nested_common::exact_map(&spec, OneBody::Direct);
    let (lo, w) = map.inner_bits().unwrap();
    let smp = sample_nested(&ev.digests, map.uniform_bits(), lo, w, k);
    let ones = |diag: bool| {
        smp.sample
            .lanes
            .iter()
            .zip(&smp.diagonal)
            .filter(|(l, &d)| l.c && d == diag)
            .count() as f64
    };
    let (kp1, kd1) = (ones(false), ones(true));
    let lambda = ev.lambda.to_f64();
    for draws in [0, GRINDING_LOG2] {
        let l = log_term(draws);
        let need = 2.0 * lambda * (2.0 * l / kp1 + l / kd1);
        let got = ev.implied_error_bound_at(draws);
        assert!(got >= need * (1.0 - 1e-12), "{got} < {need}");
    }
}

/// Appends to the fixture circuit a sign error on control-1 lanes where `j` fresh measurement
/// outcomes are all 1: each lane is wrong with probability `2^-(j+1)`, independently.
fn wrong_with_probability(j: usize, nonce: u32) -> Vec<Op> {
    let spec = TestSpec::new();
    let map = alias();
    let mut ops = build(&spec, &map, Mutation::None);
    let spare = ops.iter().flat_map(Op::qubits).max().unwrap() + 1;
    let first_bit = ops
        .iter()
        .flat_map(|op| [op.c_target, op.c_condition])
        .filter(|&b| b != NONE)
        .max()
        .map_or(0, |b| b + 1);
    for i in 0..j as u32 {
        let mut h = Op::new(OperationType::Hmr);
        (h.q_target, h.c_target) = (spare, first_bit + i);
        ops.push(h);
    }
    for i in 0..j as u32 {
        let mut p = Op::new(OperationType::PushCondition);
        p.c_condition = first_bit + i;
        ops.push(p);
    }
    let mut z = Op::new(OperationType::Z);
    z.q_target = 0; // The control: -1 on control-1 lanes only.
    ops.push(z);
    ops.extend((0..j).map(|_| Op::new(OperationType::PopCondition)));
    // A nonce: a DebugPrint naming a different register redraws every lane.
    let mut d = Op::new(OperationType::DebugPrint);
    d.r_target = nonce;
    ops.push(d);
    ops
}

fn passes(ops: &[Op], k: usize) -> bool {
    let spec = TestSpec::new();
    let map = alias();
    let file = ops_file(ops);
    let fam = serde_json::to_vec(&FamilyOut {
        family: Family {
            taxonomy_version: "1.0.0".into(),
            name: "sampling".into(),
            parent: None,
            axes: Default::default(),
        },
        spec: spec.id().into(),
    })
    .unwrap();
    evaluate(&Inputs {
        spec: &spec,
        lanemap: &map.to_bytes(),
        family: &fam,
        ops: &file,
        samples: k,
        tracker: None,
        check: &declared_only,
    })
    .is_ok()
}

/// Pass counts over `trials` nonces for per-lane failure probability `2^-(j+1)` at `K = k`.
fn pass_rate(j: usize, k: usize, trials: u32) -> (u32, f64) {
    let hits = (0..trials)
        .filter(|&n| passes(&wrong_with_probability(j, n), k))
        .count() as u32;
    let f = 0.5f64.powi(j as i32 + 1);
    (hits, (1.0 - f).powf(k as f64))
}

#[test]
fn sampling_lets_through_what_f_bound_says() {
    let k = 1 << 12;
    // f = 2^-13: K f = 0.5, pass probability 0.61. f = 2^-9: K f = 8, pass probability 3.4e-4.
    let (hits, p) = pass_rate(12, k, 40);
    let expect = p * 40.0;
    assert!((f64::from(hits) - expect).abs() < 4.0 * (expect * (1.0 - p)).sqrt() + 1.0);
    let (hits, _) = pass_rate(8, k, 40);
    assert!(hits <= 1, "{hits} of 40 passed at K f = 8");
}

/// At the default `K = 2^19`: prints the empirical pass rate next to `(1 - f)^K` for
/// `f = 2^-16, 2^-18, 2^-20` (single draw; `f_bound = 2.6e-5 = 2^-15.2`).
#[test]
#[ignore = "measurement at the default K; about 20 s in release"]
fn default_k_pass_rates() {
    let k = femoco_walk::score::DEFAULT_SAMPLES;
    for j in [15, 17, 19] {
        let (hits, p) = pass_rate(j, k, 100);
        println!(
            "f = 2^-{}: {hits} of 100 samples passed; (1 - f)^K = {p:.4}",
            j + 1
        );
    }
}
