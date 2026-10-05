//! Adversarial tests: evaluator cost that a small submission can force (low: bounded amplifications).
//!
//! **Fixed: the end-of-lane ancilla scan.** Every lane checked every ancilla id up to the
//! highest one the circuit names, one bit at a time. One `X` pair on qubit `2^20 - 1` (the
//! simulator's cap) made the harness fixture evaluate 21x slower (3.9 s against 0.18 s at
//! `K = 2^13`, two threads; about 4 minutes at the default `K`). `validate` now ORs the
//! ancilla words of a batch once and scans only lanes that have a set ancilla.
//!
//! **Measured, not fixed (needs a design decision): Majorana factors per lane.** The Givens
//! tracker keeps up to `MAX_FACTORS = 512` Majorana vectors per lane and finishes each lane with
//! a Pfaffian and an `Ad` residual over all of them. A circuit that puts a full system Pauli
//! frame between zero-angle `Givens` fills that on every control-1 lane and still passes.
//! `many_factor_lane_cost` times one lane at FeMoco's width: 3.7 ms with two full frames and
//! 17 ms with four (432 factors; Apple M-series, one thread), against about 0.13 ms for an
//! honest reiher lane (spec/DESIGN.md section 15). Four controlled `X` layers and four `Givens`
//! (about 440 ops) would cost about 2^18 x 17 ms = 75 core-minutes at the default `K`. Options:
//! lower `MAX_FACTORS` to the honest maximum plus margin (needs that maximum measured on the
//! baselines); compress a lane's vectors whenever they exceed `2n` (a product of Majorana
//! vectors is a Pin element, which `2n` reflections express); or a per-run factor budget.
mod harness_common;

use femoco_walk::circuit::{Op, OperationType as K};
use harness_common::{alias, build, eval_with, Mutation, TestSpec};
use std::time::Instant;

fn timed(ops: &[Op], k: usize) -> (f64, Result<femoco_walk::score::Evaluation, String>) {
    let t = Instant::now();
    let r = eval_with(ops, &alias(), k, None);
    (t.elapsed().as_secs_f64(), r)
}

fn far_ids(dirty: bool) -> Vec<Op> {
    let mut ops = build(&TestSpec::new(), &alias(), Mutation::None);
    let q = (1u32 << 20) - 1;
    for _ in 0..if dirty { 1 } else { 2 } {
        let mut x = Op::new(K::X);
        x.q_target = q;
        ops.push(x);
    }
    let mut bit = Op::new(K::BitStore0);
    bit.c_target = (1 << 22) - 1;
    ops.push(bit);
    ops
}

#[test]
fn far_qubit_ids_do_not_multiply_the_per_lane_cost() {
    let k = 1 << 13;
    let honest = build(&TestSpec::new(), &alias(), Mutation::None);
    let (t0, r0) = timed(&honest, k);
    assert!(r0.is_ok());
    let (t1, r1) = timed(&far_ids(false), k);
    assert!(r1.is_ok(), "{:?}", r1.err());
    // Before the fix: about 20x. The margin absorbs a shared machine.
    assert!(
        t1 < 6.0 * t0 + 1.0,
        "one op on qubit 2^20 - 1 took {t1:.2} s against {t0:.2} s for the honest circuit"
    );
}

#[test]
fn a_dirty_far_ancilla_is_still_reported() {
    let err = timed(&far_ids(true), 1024).1.expect_err("dirty ancilla");
    assert!(
        err.starts_with("dirty-ancilla-at-end") && err.contains("q1048575"),
        "{err}"
    );
}

/// One control-1 lane at FeMoco's width (108 system qubits) with two and with four full `X`
/// frames (up to 432 Majorana factors). Prints the per-lane time.
#[test]
#[ignore = "timing measurement"]
fn many_factor_lane_cost() {
    use femoco_walk::sim::gaussian::GaussianLane;
    use femoco_walk::sim::{LaneFrame, LaneTracker};
    let n: usize = 108;
    let frame = |x: bool| LaneFrame {
        x: (0..n.div_ceil(64))
            .map(|w| {
                if x {
                    u64::MAX >> (64 * (w + 1)).saturating_sub(n).min(63)
                } else {
                    0
                }
            })
            .collect(),
        z: vec![0; n.div_ceil(64)],
        s_pow: vec![0; n],
        phase: 0,
    };
    let (full, empty) = (frame(true), frame(false));
    for frames in [2usize, 4] {
        // A full X frame; an even count multiplies to the identity, so the lane passes.
        let t = Instant::now();
        let reps = 20;
        for _ in 0..reps {
            let mut lane = GaussianLane::new(n, 16);
            for _ in 0..frames {
                lane.givens_modes(&full, 0, 1, 0).unwrap();
            }
            let _ = lane.finish(&empty, None);
        }
        println!(
            "{frames} full frames: {:.3} ms per lane",
            t.elapsed().as_secs_f64() / f64::from(reps) * 1e3
        );
    }
}
