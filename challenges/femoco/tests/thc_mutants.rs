//! The thc spec, `thc-pair-alias-v1` and the Gaussian tracker end to end through `evaluate`: the
//! naive circuit passes with exact counts, and each thc mutant is rejected with a specific message
//! (the THC encoding; no THC spec ships here). The synthetic factors are random 8-bit networks, so they
//! overlap strongly (non-orthogonal pairs are the normal case here, not an edge case).
mod thc_common;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::spec::thc::Pair;
use femoco_walk::spec::{Exact, SystemOp};
use thc_common::{eval_mutant, lanemap, spec, Mutation, BETA};

const K: usize = 4096;

fn rejects(m: Mutation, want: &str) {
    let e = eval_mutant(m, K).expect_err(&format!("{m:?} must be rejected"));
    println!("{m:?}: {e}");
    assert!(e.contains(want), "{m:?}: got {e}");
}

#[test]
fn naive_thc_circuit_passes_with_exact_counts() {
    let ev = eval_mutant(Mutation::None, K).unwrap();
    assert_eq!(ev.rounding_error, Exact::zero());
    assert_eq!(ev.lambda, Exact::from_int(16));
    // 128 uniform values; a factor is 2 x 2 rotations (V^dagger, V) on 3 orbitals.
    let s = spec();
    let map = lanemap(&s);
    let givens: u64 = (0..128)
        .map(|v| {
            let l = map.decode(v);
            match s.pair(l.pair).unwrap() {
                Pair::OneBody { .. } => 4,
                Pair::TwoBody { mu, nu } if mu == nu && l.s1 == l.s2 => 0,
                Pair::TwoBody { .. } => 8,
            }
        })
        .sum();
    assert_eq!(ev.facts.executed_givens, givens as f64);
    let charge = 2.0 * f64::from(BETA - 2);
    let ccx = 128.0 * 14.0; // AND ladder up and down, 7 CCX each
                            // Reflection: u - 2 = 5.
    assert_eq!(
        ev.toffoli,
        ccx + charge * givens as f64 + 5.0,
        "{}",
        ev.toffoli
    );
    // control 1 + system 6 + uniform 7 + angle register 8 + ladder 7 + phase gradient 8.
    assert_eq!(ev.qubits, 1 + 6 + 7 + 8 + 7 + u64::from(BETA));
}

#[test]
fn flipped_sign_is_rejected() {
    rejects(Mutation::FlipSign(4), "sign flipped"); // two-body mu < nu
    rejects(Mutation::FlipSign(1), "sign flipped"); // one-body
    rejects(Mutation::FlipSign(3), "sign flipped"); // mu = nu, including its identity lanes
}

#[test]
fn wrong_mu_is_rejected() {
    rejects(Mutation::WrongMu(4), "differs from the reference");
    rejects(Mutation::WrongMu(8), "differs from the reference");
}

#[test]
fn wrong_nu_is_rejected() {
    rejects(Mutation::WrongNu(6), "differs from the reference");
    rejects(Mutation::WrongNu(7), "differs from the reference");
}

#[test]
fn angle_one_unit_off_is_rejected() {
    rejects(Mutation::AngleOffByOne(0), "differs from the reference");
    rejects(Mutation::AngleOffByOne(7), "differs from the reference");
}

#[test]
fn missing_rotation_is_rejected() {
    rejects(Mutation::MissingRotation(2), "differs from the reference");
    rejects(Mutation::MissingRotation(4), "differs from the reference");
}

#[test]
fn a_select_that_is_not_a_reflection_is_rejected() {
    // Without the swap-bit flip the uniform register comes back unchanged on control-1 lanes.
    rejects(Mutation::NoSwapFlip, "control/uniform end as");
    // Flipping it on control-0 lanes too breaks the controlled identity.
    rejects(Mutation::SwapFlipUncontrolled, "control/uniform end as");
    // Flipping it but applying Z(chi_mu) Z(chi_nu) on both halves: for overlapping same-spin
    // factors the b = 1 lanes then apply M_s, not M_s^dagger.
    rejects(Mutation::IgnoreOrder, "differs from the reference");
}

#[test]
fn the_swapped_lane_applies_the_adjoint() {
    // M_{pi(s)} = M_s^dagger structurally: the same phase (real: 0 or 2) and the parts reversed.
    let s = spec();
    let map = lanemap(&s);
    for v in 0..1u64 << map.uniform_bits() {
        let w = map.uniform_after(v);
        assert_eq!(map.uniform_after(w), v);
        assert_eq!(w ^ v, 1 << map.swap_bit());
        match (map.reference_op(&s, v), map.reference_op(&s, w)) {
            (SystemOp::Rotated(a), SystemOp::Rotated(b)) => {
                assert_eq!(a.phase, b.phase);
                assert!(a.phase % 2 == 0 || a.parts.len() == 1, "phase {}", a.phase);
                let mut rev = b.parts.clone();
                rev.reverse();
                assert_eq!(a.parts, rev);
            }
            (SystemOp::Monomial(a), SystemOp::Monomial(b)) => assert_eq!(a, b),
            (a, b) => panic!("lane {v}: {a:?} vs {b:?}"),
        }
    }
}

#[test]
fn lane_map_round_trips_and_rejects_bad_tables() {
    let s = spec();
    let map = lanemap(&s);
    let bytes = map.to_bytes();
    let back = femoco_walk::lanemap::parse(&bytes, &s).unwrap();
    assert_eq!(back.to_bytes(), bytes);
    assert_eq!(back.uniform_bits(), 7);
    assert_eq!(back.rounding_error(&s).unwrap(), Exact::zero());
    assert_eq!(back.uniform_after(5), 5 ^ 64);
    // An alias target past the last pair.
    let mut bad = map.to_bytes();
    let n = bad.len();
    bad[n - 4..].copy_from_slice(&99u32.to_le_bytes());
    let e = femoco_walk::lanemap::parse(&bad, &s).err().unwrap();
    assert!(e.contains("is not a pair"), "{e}");
    // A df lane map cannot be declared against a thc spec.
    let mut df = bytes.clone();
    df[10..27].copy_from_slice(b"df-pair-alias-v1\0"[..17].as_ref());
    assert!(femoco_walk::lanemap::parse(&df, &s).is_err());
}
