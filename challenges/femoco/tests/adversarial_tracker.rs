//! An attack that the harness withstands: hiding rotation errors under the tracker's tolerance.
//!
//! A single `Givens` is quantized to `2 pi / 2^beta`, and one unit off is caught (residual at
//! least 1.5e-6 on the pinned specs). The only way below `AD_TOL = 1e-11` is to combine
//! correct-looking units into commutators, whose net rotation shrinks as a power of the unit.
//! The check is on the lane's whole product, not per `Givens`, so errors cannot accumulate past
//! the tolerance: what passes has `|Ad(Omega) - I| <= 1e-11` however it was built, which
//! spec/DESIGN.md section 15 bounds by 1.3e-8 per lane (reiher). This test builds the smallest
//! such products at `beta = 16` and shows the group commutator (net angle about unit^2 = 9e-9)
//! is rejected and only the nested commutator (about unit^3 = 9e-13) passes.
use femoco_walk::sim::gaussian::{GaussianLane, AD_TOL};
use femoco_walk::sim::{LaneFrame, LaneTracker};

const N: usize = 6;
const BETA: u8 = 16;

type Word = Vec<(usize, usize, i64)>;

fn inverse(w: &Word) -> Word {
    w.iter().rev().map(|&(p, q, a)| (p, q, -a)).collect()
}

fn commutator(a: &Word, b: &Word) -> Word {
    [a.clone(), b.clone(), inverse(a), inverse(b)].concat()
}

fn residual(w: &Word) -> (f64, Result<(), String>) {
    let empty = LaneFrame {
        x: vec![0],
        z: vec![0],
        s_pow: vec![0; N],
        phase: 0,
    };
    let unit = 1i64 << BETA;
    let run = || {
        let mut lane = GaussianLane::new(N, BETA);
        for &(p, q, a) in w {
            lane.givens_modes(&empty, p, q, a.rem_euclid(unit) as u64)
                .unwrap();
        }
        lane
    };
    let off = run().measure(&empty, None).unwrap().0;
    (off, run().finish(&empty, None))
}

#[test]
fn only_sub_tolerance_commutators_pass_and_they_are_tiny() {
    let (a, b, c) = (vec![(0, 1, 1)], vec![(1, 2, 1)], vec![(2, 3, 1)]);
    let single = residual(&a);
    assert!(single.1.is_err() && single.0 > 1e-5, "{single:?}");
    let group = commutator(&a, &b);
    let (off, verdict) = residual(&group);
    assert!(verdict.is_err() && off > 1e-9, "group commutator: {off:e}");
    println!("single {:e}, group {off:e}", single.0);
    let nested = commutator(&group, &c);
    let (off, verdict) = residual(&nested);
    assert!(
        verdict.is_ok() && off < AD_TOL,
        "nested commutator: {off:e} {verdict:?}"
    );
    println!("group residual > 1e-9, nested residual {off:e}");
}
