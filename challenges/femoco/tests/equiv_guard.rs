//! Near-threshold adversarial cases for the folding tier's guard band (spec/FAST-EVALUATOR.md
//! sections 6.1 and 10.4): circuits whose lanes put the tracker's `Ad` residual `off` within a few
//! ulps-of-the-computation of `AD_TOL`, so that any engine that evaluates the residual in another
//! floating-point order can only match the reference by falling back to the reference's own
//! code path on those lanes.
//!
//! Construction. A group commutator of two Givens rotations on overlapping mode pairs,
//! `G_01(a) G_12(b) G_01(-a) G_12(-b)` with `beta = 32`, is the identity up to a rotation of size
//! about `theta_a theta_b` (`theta = 2 pi x / 2^32`), so its residual is tunable in steps of
//! `theta_b * 2 pi / 2^32`: about `2e-18` for `b = 1`, far below the residual's own rounding
//! error. The angle `a = a0 + s_low` is lane dependent (`CX` from the low uniform bits into the
//! angle register, `a0` with those bits clear), `-a` is applied as `G(~a) G(1)` (`~a = ~a0 ^
//! s_low`), and `a0` is calibrated so that the threshold falls inside the lane range. Optional
//! drift: a random constant network `N` before the commutator and `N^-1` (negated angles) after
//! it, so the residual also carries the rounding of many rotations, as in a real circuit.
//! Lanes with control 1 and uniform bit 12 set also pick up a `-1` (`CZ`), a sign-flip failure
//! decided by the vacuum overlap after the `Ad` check.
//!
//! Each case asserts:
//! - both engines agree on every lane's verdict and the whole `Outcome` (1 and 4 threads);
//! - the case is adversarial: the reference verdicts include passes and residual failures, and
//!   lanes lie within `1e-14` of `AD_TOL`;
//! - the residual computed here (the reference tracker on the same rotations, in the same order)
//!   predicts every reference verdict, so the lanes really are decided at the threshold;
//! - when the candidate reports a guard-band counter (a counter with `guard` in its name;
//!   `equiv::engine_counters`), it is nonzero: the band triggered. An engine without a folding
//!   tier has no such counter and computes the residual bit-identically, so it needs none.
#![cfg(feature = "walk")]
mod equiv_common;

use equiv_common::Rng;
use femoco_walk::circuit::{Op, OperationType as K, NONE};
use femoco_walk::equiv;
use femoco_walk::fiat_shamir::Lane;
use femoco_walk::sim::compile::{compile_nested, Layout};
use femoco_walk::sim::gaussian::{self, GaussianLane, AD_TOL};
use femoco_walk::sim::tracker::{LaneFrame, LaneTracker};
use femoco_walk::sim::validate::{Context, Engine, VERDICT_PASS};
use femoco_walk::sim::TrackerFactory;
use femoco_walk::spec::{Monomial, SystemOp};
use serde_json::json;

const N: usize = 6;
const U: u32 = 14;
/// Uniform bits that feed the angle `a`.
const LOW: u32 = 12;
/// Uniform bit that selects the sign-flip lanes (with control 1).
const FLIP: u32 = 12;
const BETA: u32 = 32;
const MASK: u64 = (1u64 << BETA) - 1;

fn sys(j: usize) -> u32 {
    1 + j as u32
}
fn uni(i: u32) -> u32 {
    1 + N as u32 + i
}
fn reg(i: u32) -> u32 {
    1 + N as u32 + U + i
}

fn op(kind: K, t: u32, c1: u32) -> Op {
    let mut o = Op::new(kind);
    (o.q_target, o.q_control1, o.q_control2) = (t, c1, NONE);
    o
}

struct Gadget {
    name: &'static str,
    b: u64,
    a0: u64,
    drift: Vec<(usize, usize, u64)>,
}

/// The rotations every lane hands to the tracker, in order, for low uniform bits `s_low`.
fn rotations(g: &Gadget, s_low: u64) -> Vec<(usize, usize, u64)> {
    let a = g.a0 ^ s_low;
    let mut r = g.drift.clone();
    r.push((0, 1, a));
    r.push((1, 2, g.b));
    r.push((0, 1, !a & MASK));
    r.push((0, 1, 1));
    r.push((1, 2, g.b.wrapping_neg() & MASK));
    r.extend(
        g.drift
            .iter()
            .rev()
            .map(|&(p, q, v)| (p, q, v.wrapping_neg() & MASK)),
    );
    r
}

fn identity() -> LaneFrame {
    LaneFrame {
        x: vec![0; N.div_ceil(64)],
        z: vec![0; N.div_ceil(64)],
        s_pow: vec![0; N],
        phase: 0,
    }
}

/// The reference tracker's residual for one lane (the same calls, in the same order, as the
/// reference engine makes on that lane: every hand-off frame is the identity).
fn off(g: &Gadget, s_low: u64, c: bool) -> f64 {
    let mut l = GaussianLane::new(N, BETA as u8);
    let id = identity();
    for (p, q, v) in rotations(g, s_low) {
        l.givens_modes(&id, p, q, v).expect("givens");
    }
    let r = c.then(|| {
        SystemOp::Monomial(Monomial {
            phase: 0,
            majoranas: vec![],
        })
    });
    l.measure(&id, r.as_ref()).expect("measure").0
}

/// `a0` (low `LOW` bits clear) with the threshold inside `[a0, a0 + 2^LOW)`: the residual grows
/// with `a` here (`theta_a theta_b` with both small), so a bisection finds the crossing.
fn calibrate(mut g: Gadget) -> Gadget {
    let (mut lo, mut hi) = (1u64, 1u64 << 30);
    let f = |g: &Gadget, a: u64| {
        let t = Gadget {
            name: g.name,
            b: g.b,
            a0: a,
            drift: g.drift.clone(),
        };
        off(&t, 0, false)
    };
    assert!(
        f(&g, lo) < AD_TOL && f(&g, hi) > AD_TOL,
        "{}: no crossing",
        g.name
    );
    while hi - lo > 1 {
        let m = (lo + hi) / 2;
        if f(&g, m) > AD_TOL {
            hi = m;
        } else {
            lo = m;
        }
    }
    g.a0 = lo & !((1u64 << LOW) - 1);
    g
}

fn set_const(ops: &mut Vec<Op>, v: u64) {
    for i in 0..BETA {
        if v >> i & 1 == 1 {
            ops.push(op(K::X, reg(i), NONE));
        }
    }
}

fn feed(ops: &mut Vec<Op>) {
    for i in 0..LOW {
        ops.push(op(K::CX, reg(i), uni(i)));
    }
}

fn givens(ops: &mut Vec<Op>, p: usize, q: usize) {
    let mut o = op(K::Givens, sys(q), sys(p));
    o.r_target = 0;
    ops.push(o);
}

fn circuit(g: &Gadget) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut r = Op::new(K::Register);
    r.r_target = 0;
    ops.push(r);
    for i in 0..BETA {
        let mut a = Op::new(K::AppendToRegister);
        a.q_target = reg(i);
        a.r_target = 0;
        ops.push(a);
    }
    let constant = |ops: &mut Vec<Op>, p, q, v| {
        set_const(ops, v);
        givens(ops, p, q);
        set_const(ops, v);
    };
    for &(p, q, v) in &g.drift {
        constant(&mut ops, p, q, v);
    }
    for v in [g.a0, !g.a0 & MASK] {
        set_const(&mut ops, v);
        feed(&mut ops);
        givens(&mut ops, 0, 1);
        feed(&mut ops);
        set_const(&mut ops, v);
        if v == g.a0 {
            constant(&mut ops, 1, 2, g.b);
        } else {
            constant(&mut ops, 0, 1, 1);
            constant(&mut ops, 1, 2, g.b.wrapping_neg() & MASK);
        }
    }
    for &(p, q, v) in g.drift.iter().rev() {
        constant(&mut ops, p, q, v.wrapping_neg() & MASK);
    }
    ops.push(op(K::CZ, uni(FLIP), 0));
    ops
}

fn drift(rng: &mut Rng, len: usize) -> Vec<(usize, usize, u64)> {
    (0..len)
        .map(|_| {
            let p = rng.idx(N - 1);
            let q = p + 1 + rng.idx(N - 1 - p);
            (p, q, rng.next() & MASK)
        })
        .collect()
}

/// Which lanes a case samples.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pick {
    /// The 512 low values nearest the threshold, then 1,536 uniformly random lanes (flip lanes
    /// included): every batch has failing lanes, so batch fallback is exercised.
    Mixed,
    /// Only lanes the reference passes, nearest the threshold first (no flip lanes): every batch
    /// passes, so the fast path's own check decides every lane.
    Below,
    /// Per 512-lane batch, 511 passing lanes nearest the threshold and one lane just above it at
    /// a random position: the batch is rejected only if that one lane's check is right. The
    /// dangerous direction for a folded check is a false accept, and this is it.
    OneAbove,
}

/// Runs one case against `cand`; returns the differences found (the reference-side assertions
/// hold for any candidate).
fn run(g: Gadget, pick: Pick, rng: &mut Rng, cand: &(String, Engine)) -> Vec<String> {
    let g = calibrate(g);
    let threads = equiv::env_threads();
    let ops = circuit(&g);
    let layout = Layout {
        system: N,
        uniform: U,
    };
    let factory = gaussian::factory(BETA).expect("beta 32");
    let tracker: Option<&dyn TrackerFactory> = Some(factory);
    let compiled = compile_nested(&ops, &layout, tracker, None).expect("compiles");
    // Residual per low value (control 0 and 1 give the same rotation; both are computed).
    let span = 1usize << LOW;
    let offs: Vec<[f64; 2]> = (0..span as u64)
        .map(|s| [off(&g, s, false), off(&g, s, true)])
        .collect();
    // Lanes: the 512 low values nearest the threshold (both controls, random high bits), then
    // uniformly random lanes, over several batches.
    let mut near: Vec<usize> = (0..span).collect();
    near.sort_by(|&x, &y| {
        (offs[x][0] - AD_TOL)
            .abs()
            .total_cmp(&(offs[y][0] - AD_TOL).abs())
    });
    let high = |rng: &mut Rng| rng.next() & !((1u64 << LOW) - 1) & ((1u64 << U) - 1);
    let below: Vec<usize> = near
        .iter()
        .copied()
        .filter(|&x| offs[x][0] <= AD_TOL && offs[x][1] <= AD_TOL)
        .collect();
    let above: Vec<usize> = near
        .iter()
        .copied()
        .filter(|&x| offs[x][0] > AD_TOL && offs[x][1] > AD_TOL)
        .collect();
    // A passing lane: control 0, or control 1 with the flip bit clear.
    let passing = |rng: &mut Rng, s: usize| {
        let c = rng.chance(0.5);
        let mut s = s as u64 | high(rng);
        if c {
            s &= !(1u64 << FLIP);
        }
        Lane { c, s }
    };
    let lanes: Vec<Lane> = match pick {
        Pick::Mixed => {
            let mut v: Vec<Lane> = near[..512]
                .iter()
                .map(|&s| Lane {
                    c: rng.chance(0.5),
                    s: s as u64 | high(rng),
                })
                .collect();
            v.extend((0..1536).map(|_| Lane {
                c: rng.chance(0.5),
                s: rng.next() & ((1u64 << U) - 1),
            }));
            v
        }
        Pick::Below => (0..2048)
            .map(|i| passing(rng, below[i % below.len().min(512)]))
            .collect(),
        Pick::OneAbove => {
            let mut v = Vec::new();
            for b in 0..4 {
                let at = rng.idx(512);
                for i in 0..512 {
                    if i == at {
                        let s = above[b % above.len().min(8)];
                        v.push(passing(rng, s));
                    } else {
                        v.push(passing(rng, below[(b * 512 + i) % below.len().min(512)]));
                    }
                }
            }
            v
        }
    };
    let mut key = [0u8; 32];
    key[0] = 7;
    let reference = |_s: u64| {
        SystemOp::Monomial(Monomial {
            phase: 0,
            majoranas: vec![],
        })
    };
    let uniform_after = |s: u64| s;
    let ctx = Context {
        compiled: &compiled,
        layout,
        hmr_key: key,
        reference: &reference,
        uniform_after: &uniform_after,
        tracker,
    };
    let (_, p, diffs) = equiv::compare_engines(cand.1, &threads, &ctx, None, &lanes);
    let counters = equiv::engine_counters(&cand.0);
    // The residual predicts every reference verdict.
    let mut band = [0usize; 3]; // within 1e-13, 1e-14, 1e-15 of AD_TOL
    let mut d_min = f64::INFINITY;
    let (mut pass, mut ad_fail, mut flip_fail) = (0usize, 0usize, 0usize);
    let mut mispredicted = Vec::new();
    for (i, (l, &v)) in lanes.iter().zip(&p.verdicts).enumerate() {
        let o = offs[(l.s & ((1u64 << LOW) - 1)) as usize][usize::from(l.c)];
        let d = (o - AD_TOL).abs();
        d_min = d_min.min(d);
        for (k, t) in [1e-13, 1e-14, 1e-15].iter().enumerate() {
            band[k] += usize::from(d <= *t);
        }
        let flip = l.c && l.s >> FLIP & 1 == 1;
        let want = if o > AD_TOL || flip { 1 } else { VERDICT_PASS };
        match (o > AD_TOL, flip, v) {
            (_, _, VERDICT_PASS) => pass += 1,
            (true, _, _) => ad_fail += 1,
            (false, true, _) => flip_fail += 1,
            _ => {}
        }
        if v != want {
            mispredicted.push(format!(
                "lane {i} {l:?}: off {o:e}, verdict {v}, predicted {want}"
            ));
        }
    }
    let guard: Vec<_> = counters
        .iter()
        .filter(|(k, _)| k.contains("guard"))
        .collect();
    let summary = json!({
        "kind": "guard-band",
        "case": g.name,
        "pick": format!("{pick:?}"),
        "engine": cand.0,
        "threads": threads,
        "a0": g.a0, "b": g.b, "drift": g.drift.len(),
        "lanes": lanes.len(),
        "within_1e-13": band[0], "within_1e-14": band[1], "within_1e-15": band[2],
        "pass": pass, "ad_fail": ad_fail, "flip_fail": flip_fail,
        "verdicts": equiv::verdict_counts(&p.verdicts),
        "engine_counters": counters.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "equal": diffs.iter().all(|(_, d)| d.is_empty()),
    });
    equiv::log(&summary);
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    assert!(
        mispredicted.is_empty(),
        "{}: {}",
        g.name,
        mispredicted[..mispredicted.len().min(8)].join("\n")
    );
    match pick {
        Pick::Mixed => assert!(
            pass > 0 && ad_fail > 0 && flip_fail > 0,
            "{}: not adversarial ({pass}/{ad_fail}/{flip_fail})",
            g.name
        ),
        Pick::Below => assert_eq!(pass, lanes.len(), "{}: a lane failed", g.name),
        Pick::OneAbove => assert_eq!(ad_fail, 4, "{}: one failing lane per batch", g.name),
    }
    assert!(band[1] > 0, "{}: no lane within 1e-14 of AD_TOL", g.name);
    if pick == Pick::Mixed {
        // Every batch has failing lanes; a batch fallback may pre-empt the guard band.
    } else if guard.is_empty() {
        println!(
            "{}: engine {} reports no guard-band counter (no folding tier)",
            g.name, cand.0
        );
    } else {
        // An engine that reports its band's width (`fold_max_g_attos`, the largest bound g in
        // units of 1e-18) may certify every lane when no lane lies inside the band: then
        // `off_fold + g <= AD_TOL` proves the reference passes, and no fallback is due. Any lane
        // within the band requires a fallback.
        let width = counters
            .iter()
            .find(|(k, _)| *k == "fold_max_g_attos")
            .map(|(_, v)| *v as f64 * 1e-18);
        let due = width.is_none_or(|w| d_min <= w);
        println!(
            "{}: closest lane {d_min:e} from AD_TOL, band width {width:?}, fallback due {due}",
            g.name
        );
        assert!(
            !due || guard.iter().any(|(_, v)| *v > 0),
            "{}: lanes within 1e-14 of AD_TOL, but the guard band never fell back ({guard:?})",
            g.name
        );
    }
    diffs
        .iter()
        .flat_map(|(t, d)| {
            d.iter()
                .map(move |x| format!("{} {pick:?} @{t}: {x}", g.name))
        })
        .collect()
}

/// Runs every pick of one gadget against the candidate; asserts no difference.
fn all_picks(g: impl Fn() -> Gadget, seed: u64) {
    let cand = equiv_common::candidate();
    let mut rng = Rng::new(seed);
    for pick in [Pick::Mixed, Pick::Below, Pick::OneAbove] {
        let d = run(g(), pick, &mut rng, &cand);
        assert!(
            d.is_empty(),
            "engine {} differs:\n{}",
            cand.0,
            d[..d.len().min(10)].join("\n")
        );
    }
}

fn gadget(name: &'static str, b: u64, drift: Vec<(usize, usize, u64)>) -> Gadget {
    Gadget {
        name,
        b,
        a0: 0,
        drift,
    }
}

/// `b = 1`: residual steps of about `2e-18`, so every near lane is decided by the rounding of
/// the reference's own computation.
#[test]
fn guard_band_fine_commutator() {
    all_picks(|| gadget("fine", 1, vec![]), 11);
}

/// `b = 64`: steps of about `1.4e-16`, about the residual's own rounding; lanes on both sides.
#[test]
fn guard_band_medium_commutator() {
    all_picks(|| gadget("medium", 64, vec![]), 12);
}

/// `b = 64` inside a random constant network of 60 rotations and its inverse: the residual also
/// carries the rounding of the drift, as in a real circuit.
#[test]
fn guard_band_drifted_commutator() {
    let d = drift(&mut Rng::new(99), 60);
    all_picks(|| gadget("drifted", 64, d.clone()), 13);
}

/// `b = 2^12`: steps of about `9e-15`; most lanes are far from the threshold, a few are near.
#[test]
fn guard_band_coarse_commutator() {
    all_picks(|| gadget("coarse", 1 << 12, vec![]), 14);
}

/// The cases can see an unguarded fold: `selftest-unguarded` (the reference tracker with its
/// residual read `equiv::UNGUARDED_ERROR` low, the size of the prototype's folding error) must
/// differ from the reference on the near-threshold cases where the fast path decides.
#[test]
fn guard_cases_catch_an_unguarded_fold() {
    let cand = (
        "selftest-unguarded".to_string(),
        equiv::harness_engine("selftest-unguarded").expect("self-test engine"),
    );
    let mut rng = Rng::new(21);
    let mut caught = Vec::new();
    for (g, pick) in [
        (gadget("fine", 1, vec![]), Pick::OneAbove),
        (gadget("medium", 64, vec![]), Pick::OneAbove),
        (gadget("fine", 1, vec![]), Pick::Mixed),
    ] {
        let name = format!("{} {pick:?}", g.name);
        let d = run(g, pick, &mut rng, &cand);
        assert!(!d.is_empty(), "{name}: the unguarded fold was not caught");
        caught.push(json!({"case": name, "differences": d.len(), "first": d[0]}));
    }
    equiv::log(
        &json!({"kind": "guard-band-selftest", "engine": cand.0, "caught": caught, "equal": false}),
    );
}
