//! The df spec, `df-pair-alias-v1` and the Gaussian tracker end to end through `evaluate`: the
//! naive circuit passes with exact counts, and each df mutant is rejected with a specific
//! message (the DF encoding; no DF spec ships here).
mod df_common;
use df_common::{eval_mutant, lanemap, spec, Mutation, BETA};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::spec::df::Pair;
use femoco_walk::spec::Exact;

const K: usize = 2048;

fn rejects(m: Mutation, want: &str) {
    let e = eval_mutant(m, K).expect_err(&format!("{m:?} must be rejected"));
    println!("{m:?}: {e}");
    assert!(e.contains(want), "{m:?}: got {e}");
}

#[test]
fn naive_df_circuit_passes_with_exact_counts() {
    let ev = eval_mutant(Mutation::None, K).unwrap();
    assert_eq!(ev.rounding_error, Exact::zero());
    assert_eq!(ev.lambda, Exact::from_int(8));
    // 64 uniform values; each runs 4 (one-body) or 8 (two-body) networks of 2 rotations.
    // Pairs 0..3 are one-body (4 of 16 buckets hold 8 of the 16 index values).
    let s = spec();
    let map = lanemap(&s);
    let givens: u64 = (0..64)
        .map(|v| match s.pair(map.decode(v).0).unwrap() {
            Pair::OneBody { .. } => 4,
            Pair::TwoBody { .. } => 8,
        })
        .sum();
    assert_eq!(ev.facts.executed_givens, givens as f64);
    let charge = 2.0 * f64::from(BETA - 2);
    let ccx = 64.0 * 12.0; // AND ladder up and down, 6 CCX each
    assert_eq!(
        ev.toffoli,
        ccx + charge * givens as f64 + 4.0,
        "{}",
        ev.toffoli
    );
    println!(
        "C_step {} Q_peak {} lambda {}",
        ev.toffoli, ev.qubits, ev.lambda
    );
}

#[test]
fn phase_gradient_register_counts_toward_qubits() {
    let ev = eval_mutant(Mutation::None, 256).unwrap();
    // control 1 + system 6 + uniform 6 + angle register 8 + ladder 6 + phase gradient 8.
    assert_eq!(ev.qubits, 1 + 6 + 6 + 8 + 6 + u64::from(BETA));
}

#[test]
fn flipped_sign_is_rejected() {
    rejects(Mutation::FlipSign(5), "sign flipped");
    rejects(Mutation::FlipSign(1), "sign flipped");
}

#[test]
fn wrong_leaf_is_rejected() {
    rejects(Mutation::WrongLeaf(0), "differs from the reference");
}

#[test]
fn wrong_eigenvector_is_rejected() {
    rejects(Mutation::WrongEigenvector(4), "differs from the reference");
}

#[test]
fn angle_one_unit_off_is_rejected() {
    rejects(Mutation::AngleOffByOne(0), "differs from the reference");
    rejects(Mutation::AngleOffByOne(9), "differs from the reference");
}

#[test]
fn missing_rotation_is_rejected() {
    rejects(Mutation::MissingRotation(2), "differs from the reference");
    rejects(Mutation::MissingRotation(7), "differs from the reference");
}

#[test]
fn lane_map_round_trips_and_rejects_bad_tables() {
    let s = spec();
    let map = lanemap(&s);
    let bytes = map.to_bytes();
    let back = femoco_walk::lanemap::parse(&bytes, &s).unwrap();
    assert_eq!(back.to_bytes(), bytes);
    assert_eq!(back.uniform_bits(), 6);
    assert_eq!(back.rounding_error(&s).unwrap(), Exact::zero());
    // An alias target past the last pair.
    let mut bad = map.to_bytes();
    let n = bad.len();
    bad[n - 4..].copy_from_slice(&99u32.to_le_bytes());
    let e = femoco_walk::lanemap::parse(&bad, &s).err().unwrap();
    assert!(e.contains("is not a pair"), "{e}");
}
