//! The Gaussian tracker against dense 2^n x 2^n matrices on n = 3 modes: the Givens semantics
//! (spec/DESIGN.md section 15), frame -> Majorana conversion, and the tracker's accept/reject verdict on
//! random references, their circuits, sign flips, angle errors and unrelated sequences.
use femoco_walk::sim::gaussian::{frame_to_majoranas, GaussianLane};
use femoco_walk::sim::{Frame, LaneFrame, LaneTracker};
use femoco_walk::spec::{Network, Rotated, RotatedPart, SystemOp};
use std::sync::Arc;

const N: usize = 3;
const D: usize = 1 << N;
const BETA: u8 = 6;
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
fn w(k: u8) -> C {
    let t = f64::from(k % 8) * std::f64::consts::FRAC_PI_4;
    (t.cos(), t.sin())
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
const S: [C; 4] = [(1.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 1.0)];

fn gamma(m: usize) -> M {
    let j = m / 2;
    let mut o = on(j, if m.is_multiple_of(2) { X } else { Y });
    for q in 0..j {
        o = mul(&on(q, Z), &o);
    }
    o
}
/// `a_p = (gamma_{2p} + i gamma_{2p+1}) / 2`.
fn ann(p: usize) -> M {
    lin(&gamma(2 * p), (0.5, 0.0), &gamma(2 * p + 1), (0.0, 0.5))
}
/// `exp(theta (a+_q a_p - a+_p a_q)) = I + sin K + (1 - cos) K^2` (K has eigenvalues 0, +-i).
fn givens(p: usize, q: usize, a: u64) -> M {
    let th = a as f64 * std::f64::consts::TAU / f64::from(1u32 << BETA);
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
/// `w^phase D X^x Z^z` as the simulator defines a lane frame.
fn dense_frame(f: &LaneFrame) -> M {
    let mut o = scale(&eye(), w(f.phase));
    for q in 0..N {
        for _ in 0..f.s_pow[q] {
            o = mul(&o, &on(q, S));
        }
    }
    for q in (0..N).filter(|&q| f.x[0] >> q & 1 == 1) {
        o = mul(&o, &on(q, X));
    }
    for q in (0..N).filter(|&q| f.z[0] >> q & 1 == 1) {
        o = mul(&o, &on(q, Z));
    }
    o
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn frame(x: u64, z: u64) -> LaneFrame {
    LaneFrame {
        x: vec![x],
        z: vec![z],
        s_pow: vec![0; N],
        phase: 0,
    }
}

#[test]
fn givens_semantics_rotates_creators_and_fixes_vacuum() {
    let (p, q, a) = (0, 2, 5u64);
    let g = givens(p, q, a);
    let th = a as f64 * std::f64::consts::TAU / 64.0;
    let lhs = mul(&mul(&g, &dagger(&ann(p))), &dagger(&g));
    let rhs = lin(
        &dagger(&ann(p)),
        (th.cos(), 0.0),
        &dagger(&ann(q)),
        (th.sin(), 0.0),
    );
    assert!(dist(&lhs, &rhs) < 1e-12);
    assert!((g[0].0 - 1.0).abs() < 1e-12 && (1..D).all(|i| g[i * D].0.abs() < 1e-12));
}

#[test]
fn frames_convert_to_majorana_products_exactly() {
    let mut r = Rng(7);
    for _ in 0..300 {
        let mut f = frame(r.below(8), r.below(8));
        f.phase = r.below(8) as u8;
        f.s_pow = (0..N).map(|_| 2 * r.below(2) as u8).collect();
        let (k, ms) = frame_to_majoranas(&f, N).unwrap();
        let prod = ms.iter().fold(scale(&eye(), w(k)), |acc, &m| {
            mul(&acc, &gamma(usize::from(m)))
        });
        assert!(dist(&prod, &dense_frame(&f)) < 1e-12, "{f:?} -> {k} {ms:?}");
    }
    let odd = LaneFrame {
        s_pow: vec![1, 0, 0],
        ..frame(0, 0)
    };
    assert!(frame_to_majoranas(&odd, N).unwrap_err().contains("S^1"));
}

/// A lane: frames and Givens in time order, then a final frame.
struct Seq {
    steps: Vec<(LaneFrame, usize, usize, u64)>,
    after: LaneFrame,
}

impl Seq {
    fn dense(&self) -> M {
        let mut o = eye();
        for (f, p, q, a) in &self.steps {
            o = mul(&givens(*p, *q, *a), &mul(&dense_frame(f), &o));
        }
        mul(&dense_frame(&self.after), &o)
    }
    fn verdict(&self, r: Option<&SystemOp>) -> Result<(), String> {
        let mut t = GaussianLane::new(N, BETA);
        for (f, p, q, a) in &self.steps {
            t.givens_modes(f, *p, *q, *a)?;
        }
        t.finish(&self.after, r)
    }
}

fn gamma_frame(ms: &[u16]) -> (LaneFrame, u8) {
    let mut f = Frame::identity(N);
    for &m in ms {
        let j = usize::from(m) / 2;
        let mut g = Frame::identity(N);
        g.x[0] |= 1 << j;
        g.z[0] |= (1 << (j + usize::from(m) % 2)) - 1;
        g.phase = 2 * (m % 2) as u8;
        f = f.mul(&g);
    }
    (frame(f.x[0], f.z[0]), f.phase)
}

fn random_reference(r: &mut Rng) -> Rotated {
    let parts = (0..1 + r.below(2))
        .map(|_| {
            let rotations = (0..1 + r.below(4))
                .map(|_| {
                    let p = r.below(N as u64 - 1) as u16;
                    let q = p + 1 + r.below(N as u64 - 1 - u64::from(p)) as u16;
                    (p, q, r.below(64) as u32)
                })
                .collect();
            let majoranas = (0..2 * N as u16).filter(|_| r.below(3) == 0).collect();
            RotatedPart {
                network: Arc::new(Network {
                    beta: BETA,
                    rotations,
                }),
                spin: None,
                majoranas,
            }
        })
        .collect();
    Rotated {
        phase: r.below(4) as u8,
        parts,
    }
}

fn dense_reference(r: &Rotated) -> M {
    let mut o = scale(&eye(), w(2 * r.phase));
    for part in &r.parts {
        let g = part
            .network
            .rotations
            .iter()
            .fold(eye(), |acc, &(p, q, a)| {
                mul(&givens(usize::from(p), usize::from(q), u64::from(a)), &acc)
            });
        let m = part
            .majoranas
            .iter()
            .fold(eye(), |acc, &m| mul(&acc, &gamma(usize::from(m))));
        o = mul(&o, &mul(&mul(&g, &m), &dagger(&g)));
    }
    o
}

/// The circuit for a reference: per part (last first) V^dagger, the monomial, V.
fn circuit_for(r: &Rotated) -> Seq {
    let (mut steps, mut pending, mut phase) = (Vec::new(), frame(0, 0), 2 * r.phase);
    for part in r.parts.iter().rev() {
        let rots = &part.network.rotations;
        for &(p, q, a) in rots.iter().rev() {
            steps.push((
                pending,
                usize::from(p),
                usize::from(q),
                (64 - u64::from(a)) % 64,
            ));
            pending = frame(0, 0);
        }
        let (f, ph) = gamma_frame(&part.majoranas);
        let j = Frame {
            x: f.x.clone(),
            z: f.z.clone(),
            phase: 0,
        }
        .mul(&Frame {
            x: pending.x.clone(),
            z: pending.z.clone(),
            phase: 0,
        });
        phase = (phase + ph + j.phase) % 8;
        pending = frame(j.x[0], j.z[0]);
        for &(p, q, a) in rots {
            steps.push((pending, usize::from(p), usize::from(q), u64::from(a)));
            pending = frame(0, 0);
        }
    }
    Seq {
        steps,
        after: LaneFrame { phase, ..pending },
    }
}

#[test]
fn tracker_agrees_with_dense_matrices() {
    let mut r = Rng(0x5eed);
    let (mut accepted, mut rejected) = (0, 0);
    for case in 0..200 {
        let rf = random_reference(&mut r);
        let want = dense_reference(&rf);
        let op = SystemOp::Rotated(rf.clone());
        let mut seq = circuit_for(&rf);
        assert!(
            dist(&seq.dense(), &want) < 1e-10,
            "case {case}: the test's own circuit is wrong"
        );
        seq.verdict(Some(&op))
            .unwrap_or_else(|e| panic!("case {case}: {e}"));
        accepted += 1;
        // -1 times the reference.
        seq.after.phase = (seq.after.phase + 4) % 8;
        assert!(seq.verdict(Some(&op)).unwrap_err().contains("sign flipped"));
        seq.after.phase = (seq.after.phase + 4) % 8;
        // One angle unit off, or a random unrelated sequence: agree with the dense verdict.
        let mut bad = circuit_for(&rf);
        let i = r.below(bad.steps.len() as u64) as usize;
        bad.steps[i].3 = (bad.steps[i].3 + 1 + r.below(63)) % 64;
        let equal = dist(&bad.dense(), &want) < 1e-9;
        assert_eq!(bad.verdict(Some(&op)).is_ok(), equal, "case {case}");
        rejected += usize::from(!equal);
    }
    assert!(accepted == 200 && rejected > 150, "{accepted} {rejected}");
}

#[test]
fn control_zero_lane_must_be_plus_identity() {
    // (G gamma_0 gamma_1 G^dagger)^2 = -1: the circuit of that "reference" is -I.
    let part = RotatedPart {
        network: Arc::new(Network {
            beta: BETA,
            rotations: vec![(0, 1, 9), (1, 2, 40)],
        }),
        spin: None,
        majoranas: vec![0, 1],
    };
    let rf = Rotated {
        phase: 0,
        parts: vec![part.clone(), part],
    };
    let mut seq = circuit_for(&rf);
    assert!(dist(&seq.dense(), &scale(&eye(), (-1.0, 0.0))) < 1e-10);
    assert!(seq.verdict(None).unwrap_err().contains("sign flipped"));
    seq.after.phase = (seq.after.phase + 4) % 8;
    assert!(dist(&seq.dense(), &eye()) < 1e-10);
    seq.verdict(None).unwrap();
    // A plain monomial reference with Givens in between is compared the same way.
    let mono = SystemOp::Monomial(femoco_walk::spec::Monomial {
        phase: 0,
        majoranas: vec![],
    });
    seq.verdict(Some(&mono)).unwrap();
}
