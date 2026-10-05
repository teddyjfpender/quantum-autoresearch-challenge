//! Brute-force checks of the Jordan-Wigner conversion and of the simulator's frame algebra
//! against dense matrices on small systems.
use femoco_walk::circuit::{Builder, Op};
use femoco_walk::fiat_shamir::hmr_stream;
use femoco_walk::sim::lanes::Lanes;
use femoco_walk::sim::{compile, monomial_to_frame, Layout};
use femoco_walk::spec::Monomial;

type C = (f64, f64);
type M = Vec<Vec<C>>;

fn cmul(a: C, b: C) -> C {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

fn matmul(a: &M, b: &M) -> M {
    let d = a.len();
    (0..d)
        .map(|i| {
            (0..d)
                .map(|j| {
                    (0..d).fold((0.0, 0.0), |acc, k| {
                        let p = cmul(a[i][k], b[k][j]);
                        (acc.0 + p.0, acc.1 + p.1)
                    })
                })
                .collect()
        })
        .collect()
}

fn ident(d: usize) -> M {
    (0..d)
        .map(|i| {
            (0..d)
                .map(|j| if i == j { (1.0, 0.0) } else { (0.0, 0.0) })
                .collect()
        })
        .collect()
}

/// A single-qubit matrix `g` on qubit `q` of `n` (basis index bit `q` is that qubit).
fn on(q: usize, n: usize, g: [[C; 2]; 2]) -> M {
    let d = 1 << n;
    let mut m = vec![vec![(0.0, 0.0); d]; d];
    for col in 0..d {
        let b = col >> q & 1;
        for (r, row) in g.iter().enumerate() {
            m[(col & !(1 << q)) | r << q][col] = row[b];
        }
    }
    m
}

const O: C = (0.0, 0.0);
const I1: C = (1.0, 0.0);
const X: [[C; 2]; 2] = [[O, I1], [I1, O]];
const Y: [[C; 2]; 2] = [[O, (0.0, -1.0)], [(0.0, 1.0), O]];
const Z: [[C; 2]; 2] = [[I1, O], [O, (-1.0, 0.0)]];
const S: [[C; 2]; 2] = [[I1, O], [O, (0.0, 1.0)]];

fn w_pow(p: u8) -> C {
    let a = std::f64::consts::FRAC_PI_4 * f64::from(p);
    (a.cos(), a.sin())
}

fn scale(m: &M, c: C) -> M {
    m.iter()
        .map(|r| r.iter().map(|&v| cmul(v, c)).collect())
        .collect()
}

fn close(a: &M, b: &M) -> bool {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .all(|(x, y)| (x.0 - y.0).abs() < 1e-9 && (x.1 - y.1).abs() < 1e-9)
}

/// Direct definition: gamma_{2j} = Z_<j X_j, gamma_{2j+1} = Z_<j Y_j (Y as a matrix).
fn gamma(m: usize, n: usize) -> M {
    let j = m / 2;
    let mut g = on(j, n, if m.is_multiple_of(2) { X } else { Y });
    for q in 0..j {
        g = matmul(&on(q, n, Z), &g);
    }
    g
}

/// `w^phase * prod_q S_q^{s_q} * prod_q X_q^{x_q} Z_q^{z_q}`.
fn frame_matrix(n: usize, x: u64, z: u64, s: &[u8], phase: u8) -> M {
    let mut m = ident(1 << n);
    for q in 0..n {
        if z >> q & 1 == 1 {
            m = matmul(&on(q, n, Z), &m);
        }
        if x >> q & 1 == 1 {
            m = matmul(&on(q, n, X), &m);
        }
    }
    for (q, &k) in s.iter().enumerate() {
        for _ in 0..k {
            m = matmul(&on(q, n, S), &m);
        }
    }
    scale(&m, w_pow(phase))
}

#[test]
fn jordan_wigner_matches_dense_matrices() {
    let n = 3;
    let mut checked = 0;
    for mask in 0u32..1 << (2 * n) {
        let maj: Vec<u16> = (0..2 * n as u16).filter(|&i| mask >> i & 1 == 1).collect();
        for phase in 0..4u8 {
            let mono = Monomial {
                phase,
                majoranas: maj.clone(),
            };
            let mut dense = scale(&ident(1 << n), w_pow(2 * phase));
            for &g in &maj {
                dense = matmul(&dense, &gamma(usize::from(g), n));
            }
            let hermitian = close(&dense, &adjoint(&dense));
            match monomial_to_frame(&mono, n) {
                Ok(f) => {
                    assert!(hermitian, "{mono:?} accepted but not Hermitian");
                    let got = frame_matrix(n, f.x[0], f.z[0], &[], f.phase);
                    assert!(close(&got, &dense), "{mono:?} -> {}", f.describe(n));
                    checked += 1;
                }
                Err(e) => assert!(!hermitian, "{mono:?} rejected ({e}) but is Hermitian"),
            }
        }
    }
    // Exactly two of the four phases give a Hermitian product for every Majorana subset.
    assert_eq!(checked, 2 << (2 * n));
}

fn adjoint(m: &M) -> M {
    let d = m.len();
    (0..d)
        .map(|i| (0..d).map(|j| (m[j][i].0, -m[j][i].1)).collect())
        .collect()
}

#[test]
fn rejects_malformed_monomials() {
    let bad = [
        Monomial {
            phase: 0,
            majoranas: vec![3, 1],
        },
        Monomial {
            phase: 0,
            majoranas: vec![1, 1],
        },
        Monomial {
            phase: 4,
            majoranas: vec![],
        },
        Monomial {
            phase: 0,
            majoranas: vec![12],
        },
    ];
    for m in bad {
        assert!(monomial_to_frame(&m, 6).is_err(), "{m:?}");
    }
}

/// Runs `ops` on one lane with the control set and returns that lane's frame as a matrix.
fn simulate(ops: &[Op], n: usize) -> M {
    let layout = Layout {
        system: n,
        uniform: 0,
    };
    let c = compile(ops, &layout, None).unwrap();
    let mut lanes = Lanes::new(&c, n, None);
    lanes.active[0] = 1;
    lanes.q[0] |= 1;
    lanes.run(&c.ops, &mut hmr_stream(&[0; 32], 0)).unwrap();
    let f = lanes.lane_frame(0, true);
    frame_matrix(n, f.x[0], f.z[0], &f.s_pow, f.phase)
}

#[test]
fn frame_algebra_matches_dense_products() {
    let n = 2;
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..300 {
        let mut b = Builder::new(n);
        b.declare_uniform(0);
        let mut dense = ident(1 << n);
        for _ in 0..12 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let q = (seed >> 33) as usize % n;
            let (t, c) = (b.system(q), b.control());
            let g = match (seed >> 40) % 6 {
                0 => {
                    b.x(t);
                    X
                }
                1 => {
                    b.z(t);
                    Z
                }
                2 => {
                    b.s(t);
                    S
                }
                3 => {
                    b.sdg(t);
                    [[I1, O], [O, (0.0, -1.0)]]
                }
                4 => {
                    b.cx(c, t);
                    X
                }
                _ => {
                    b.cz(c, t);
                    Z
                }
            };
            dense = matmul(&on(q, n, g), &dense);
        }
        let ops = b.finish();
        assert!(close(&simulate(&ops, n), &dense), "{ops:?}");
    }
}
