//! The Gaussian tracker on non-orthogonal pairs (the THC encoding; no THC spec ships here), against dense
//! 64 x 64 matrices on 3 spatial orbitals (6 spin orbitals): for products `Z(u1, s1) Z(u2, s2)`
//! of two rotated number-type operators whose vectors overlap arbitrarily (random, nearly parallel,
//! identical), the dense circuit equals the dense reference, the tracker accepts it, rejects the
//! reversed product exactly when the two factors do not commute, and names a flipped sign.
use femoco_walk::sim::gaussian::GaussianLane;
use femoco_walk::sim::{LaneFrame, LaneTracker};
use femoco_walk::spec::{Network, Rotated, RotatedPart, SystemOp};
use std::sync::Arc;

const SPATIAL: usize = 3;
const MODES: usize = 2 * SPATIAL;
const D: usize = 1 << MODES;
type C = (f64, f64);
type M = Vec<C>;

fn cm(a: C, b: C) -> C {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}
fn eye() -> M {
    (0..D * D)
        .map(|i| (f64::from(u8::from(i / D == i % D)), 0.0))
        .collect()
}
fn mul(a: &M, b: &M) -> M {
    let mut o = vec![(0.0, 0.0); D * D];
    for i in 0..D {
        for k in 0..D {
            let x = a[i * D + k];
            if x == (0.0, 0.0) {
                continue;
            }
            for j in 0..D {
                let p = cm(x, b[k * D + j]);
                o[i * D + j].0 += p.0;
                o[i * D + j].1 += p.1;
            }
        }
    }
    o
}
fn lin(a: &M, x: C, b: &M, y: C) -> M {
    a.iter()
        .zip(b)
        .map(|(&p, &q)| (cm(p, x).0 + cm(q, y).0, cm(p, x).1 + cm(q, y).1))
        .collect()
}
fn scale(a: &M, x: C) -> M {
    a.iter().map(|&p| cm(p, x)).collect()
}
fn dagger(a: &M) -> M {
    (0..D * D)
        .map(|i| {
            let v = a[(i % D) * D + i / D];
            (v.0, -v.1)
        })
        .collect()
}
fn dist(a: &M, b: &M) -> f64 {
    a.iter()
        .zip(b)
        .map(|(p, q)| (p.0 - q.0).hypot(p.1 - q.1))
        .fold(0.0, f64::max)
}
/// Single-qubit gate `g` (2x2) on qubit `q` (bit q of the basis index).
fn on(q: usize, g: [C; 4]) -> M {
    let mut o = vec![(0.0, 0.0); D * D];
    for i in 0..D {
        for b in 0..2 {
            let j = (i & !(1 << q)) | (b << q);
            o[j * D + i] = g[b * 2 + (i >> q & 1)];
        }
    }
    o
}
const X: [C; 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 0.0), (0.0, 0.0)];
const Z: [C; 4] = [(1.0, 0.0), (0.0, 0.0), (0.0, 0.0), (-1.0, 0.0)];
const Y: [C; 4] = [(0.0, 0.0), (0.0, -1.0), (0.0, 1.0), (0.0, 0.0)];

fn gamma(m: usize) -> M {
    let j = m / 2;
    let mut o = on(j, if m.is_multiple_of(2) { X } else { Y });
    for q in 0..j {
        o = mul(&on(q, Z), &o);
    }
    o
}
fn ann(p: usize) -> M {
    lin(&gamma(2 * p), (0.5, 0.0), &gamma(2 * p + 1), (0.0, 0.5))
}
fn givens(p: usize, q: usize, a: u32, beta: u8) -> M {
    let th = f64::from(a) * std::f64::consts::TAU / f64::from(1u32 << beta);
    let k = lin(
        &mul(&dagger(&ann(q)), &ann(p)),
        (1.0, 0.0),
        &mul(&dagger(&ann(p)), &ann(q)),
        (-1.0, 0.0),
    );
    let k2 = mul(&k, &k);
    lin(
        &lin(&eye(), (1.0, 0.0), &k, (th.sin(), 0.0)),
        (1.0, 0.0),
        &k2,
        (1.0 - th.cos(), 0.0),
    )
}

/// `G` of a network on spin `s`: the first rotation acts first.
fn network_dense(net: &Network, s: usize) -> M {
    net.rotations.iter().fold(eye(), |g, &(p, q, a)| {
        mul(
            &givens(2 * usize::from(p) + s, 2 * usize::from(q) + s, a, net.beta),
            &g,
        )
    })
}

/// `Z(u, s) = G Z_s G^dagger`.
fn z_dense(net: &Network, s: usize) -> M {
    let g = network_dense(net, s);
    mul(&mul(&g, &on(s, Z)), &dagger(&g))
}

fn part(net: &Arc<Network>, s: u8) -> RotatedPart {
    let m = 2 * u16::from(s);
    RotatedPart {
        network: Arc::clone(net),
        spin: Some(s),
        majoranas: vec![m, m + 1],
    }
}

/// `+ Z(u1, s1) Z(u2, s2)` as a reference (`Z_m = -i gamma gamma`, so the phase is `i^2`).
fn product(a: (&Arc<Network>, u8), b: (&Arc<Network>, u8)) -> Rotated {
    Rotated {
        phase: 2,
        parts: vec![part(a.0, a.1), part(b.0, b.1)],
    }
}

/// Runs the circuit of `factors` (product order, the last acts first) with an optional extra
/// `-1`, and returns the tracker's verdict against `reference`.
fn verdict(
    factors: &[(&Arc<Network>, u8)],
    negate: bool,
    reference: &Rotated,
) -> Result<(), String> {
    let beta = factors[0].0.beta;
    let unit = 1u32 << beta;
    let mut t = GaussianLane::new(MODES, beta);
    let clear = LaneFrame {
        x: vec![0],
        z: vec![0],
        s_pow: vec![0; MODES],
        phase: 0,
    };
    let mut pending = clear.clone();
    for &(net, s) in factors.iter().rev() {
        let s = usize::from(s);
        for &(p, q, a) in net.rotations.iter().rev() {
            let (p, q) = (2 * usize::from(p) + s, 2 * usize::from(q) + s);
            t.givens_modes(&pending, p, q, u64::from((unit - a) % unit))?;
            pending = clear.clone();
        }
        pending.z[0] |= 1 << s;
        for &(p, q, a) in &net.rotations {
            let (p, q) = (2 * usize::from(p) + s, 2 * usize::from(q) + s);
            t.givens_modes(&pending, p, q, u64::from(a))?;
            pending = clear.clone();
        }
    }
    if negate {
        pending.phase = 4;
    }
    t.finish(&pending, Some(&SystemOp::Rotated(reference.clone())))
}

fn net(beta: u8, angles: [u32; 2]) -> Arc<Network> {
    Arc::new(Network {
        beta,
        rotations: vec![(0, 1, angles[0]), (1, 2, angles[1])],
    })
}

fn overlap(a: &Network, b: &Network) -> f64 {
    let v = |n: &Network| {
        let mut v = [1.0, 0.0, 0.0];
        for &(p, q, x) in &n.rotations {
            let (c, s) = femoco_walk::sim::gaussian::cos_sin(u64::from(x), n.beta);
            femoco_walk::sim::gaussian::rotate(&mut v, usize::from(p), usize::from(q), c, s);
        }
        v
    };
    let (x, y) = (v(a), v(b));
    x.iter().zip(&y).map(|(p, q)| p * q).sum()
}

#[test]
fn nonorthogonal_pairs_match_dense_and_order_and_sign_are_exact() {
    let beta = 10u8;
    let base = [217u32, 601];
    let cases: Vec<(&str, [u32; 2], [u32; 2])> = vec![
        ("random", base, [83, 940]),
        ("random 2", [5, 333], [700, 12]),
        (
            "nearly parallel (one angle unit)",
            base,
            [base[0] + 1, base[1]],
        ),
        ("nearly antiparallel", base, [base[0] + 512 + 3, base[1]]),
        ("orthogonal", [0, 0], [256, 0]),
        ("identical", base, base),
    ];
    let mut checked = (0, 0);
    for (name, a1, a2) in cases {
        let (n1, n2) = (net(beta, a1), net(beta, a2));
        let ov = overlap(&n1, &n2);
        for (s1, s2) in [(0u8, 0u8), (0, 1), (1, 0), (1, 1)] {
            let (f1, f2) = ((&n1, s1), (&n2, s2));
            let fwd = product(f1, f2);
            let rev = product(f2, f1);
            let (z1, z2) = (z_dense(&n1, s1.into()), z_dense(&n2, s2.into()));
            let (d12, d21) = (mul(&z1, &z2), mul(&z2, &z1));
            // The circuit's dense operator (V1 Z V1^dagger)(V2 Z V2^dagger) is the reference's.
            let reference = scale(
                &mul(
                    &mul(
                        &mul(
                            &network_dense(&n1, s1.into()),
                            &mul(&gamma(2 * usize::from(s1)), &gamma(2 * usize::from(s1) + 1)),
                        ),
                        &dagger(&network_dense(&n1, s1.into())),
                    ),
                    &mul(
                        &mul(
                            &network_dense(&n2, s2.into()),
                            &mul(&gamma(2 * usize::from(s2)), &gamma(2 * usize::from(s2) + 1)),
                        ),
                        &dagger(&network_dense(&n2, s2.into())),
                    ),
                ),
                (-1.0, 0.0), // i^2
            );
            assert!(
                dist(&reference, &d12) < 1e-10,
                "{name} {s1}{s2}: reference semantics"
            );
            verdict(&[f1, f2], false, &fwd).unwrap_or_else(|e| panic!("{name} {s1}{s2}: {e}"));
            verdict(&[f2, f1], false, &rev).unwrap_or_else(|e| panic!("{name} {s1}{s2}: {e}"));
            let e = verdict(&[f1, f2], true, &fwd).unwrap_err();
            assert!(e.contains("sign flipped"), "{name} {s1}{s2}: {e}");
            let commute = dist(&d12, &d21) < 1e-9;
            match verdict(&[f2, f1], false, &fwd) {
                Ok(()) => assert!(
                    commute,
                    "{name} {s1}{s2}: reversed order accepted, overlap {ov}"
                ),
                Err(e) => {
                    assert!(!commute, "{name} {s1}{s2}: commuting order rejected: {e}");
                    assert!(e.contains("differs from the reference"), "{e}");
                    checked.1 += 1;
                }
            }
            checked.0 += 1;
        }
        println!("{name}: chi1 . chi2 = {ov:.6}");
    }
    // Same-spin pairs that neither coincide nor are orthogonal do not commute: 2 spins x 4 cases.
    assert_eq!(checked, (24, 8));
}
