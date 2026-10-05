//! Dense check of the sos-sa lane semantics (spec/SPEC-SA.md section 5) on the 3-orbital spec of
//! tests/sa_nested.rs: the operator the lane map encodes, `A = lambda_decl E_s [2 E_b R(s, b) -
//! R(s, a)]` with `R(s, b) = M(b)^dagger M(a)` as `reference_nested` returns it, equals
//! `H_spec - identity + (lambda_decl - lambda_flat)` built densely from the spec's definition
//! `sos + sum e Et(u) + sum (1/2)(wB + sum w Et(u_b))^2` (positive semidefiniteness of
//! `H_spec - E_SOS` was checked densely when the specs were generated). Rotated Majoranas use the
//! Givens action of spec/DESIGN.md section 15 (checked against matrix exponentials in
//! tests/df_gaussian.rs).
use femoco_walk::lanemap::df_nested::Table;
use femoco_walk::lanemap::sa_nested::{self, SaNestedMap};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::sim::gaussian::cos_sin;
use femoco_walk::spec::sa::{parse_payload, SaSpec};
use femoco_walk::spec::{Exact, Network, SystemOp};

const N: usize = 3;
const MODES: usize = 2 * N;
const DIM: usize = 1 << MODES;
const BETA: u32 = 8;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct C(f64, f64);
impl C {
    fn mul(self, o: C) -> C {
        C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn add(self, o: C) -> C {
        C(self.0 + o.0, self.1 + o.1)
    }
    fn scale(self, s: f64) -> C {
        C(self.0 * s, self.1 * s)
    }
}

#[derive(Clone)]
struct M(Vec<C>);
impl M {
    fn zero() -> M {
        M(vec![C::default(); DIM * DIM])
    }
    fn eye() -> M {
        let mut m = M::zero();
        (0..DIM).for_each(|i| m.0[i * DIM + i] = C(1.0, 0.0));
        m
    }
    fn mul(&self, o: &M) -> M {
        let mut out = M::zero();
        for i in 0..DIM {
            for k in 0..DIM {
                let a = self.0[i * DIM + k];
                if a == C::default() {
                    continue;
                }
                for j in 0..DIM {
                    out.0[i * DIM + j] = out.0[i * DIM + j].add(a.mul(o.0[k * DIM + j]));
                }
            }
        }
        out
    }
    fn add(&self, o: &M, s: C) -> M {
        M(self
            .0
            .iter()
            .zip(&o.0)
            .map(|(a, b)| a.add(b.mul(s)))
            .collect())
    }
    fn max_diff(&self, o: &M) -> f64 {
        self.0
            .iter()
            .zip(&o.0)
            .map(|(a, b)| (a.0 - b.0).abs().max((a.1 - b.1).abs()))
            .fold(0.0, f64::max)
    }
}

/// Jordan-Wigner Majorana `gamma_k` (spec/DESIGN.md section 3): qubit `j` is bit `j` of the basis
/// index; `gamma_{2j} = Z_0..Z_{j-1} X_j`, `gamma_{2j+1} = Z_0..Z_{j-1} Y_j`.
fn gamma(k: usize) -> M {
    let (j, y) = (k / 2, k % 2 == 1);
    let mut m = M::zero();
    for col in 0..DIM {
        let row = col ^ (1 << j);
        let parity = (col & ((1 << j) - 1)).count_ones() % 2;
        let mut v = C(if parity == 1 { -1.0 } else { 1.0 }, 0.0);
        if y {
            // Y|0> = i|1>, Y|1> = -i|0>
            v = v.mul(if col >> j & 1 == 0 {
                C(0.0, 1.0)
            } else {
                C(0.0, -1.0)
            });
        }
        m.0[row * DIM + col] = v;
    }
    m
}

/// The unit vector `V e_0` of a network (G_{p,q}: e_p -> c e_p + s e_q).
fn vector(net: &Network) -> Vec<f64> {
    let mut x = vec![0.0; N];
    x[0] = 1.0;
    for &(p, q, a) in &net.rotations {
        let (c, s) = cos_sin(u64::from(a), net.beta);
        let (xp, xq) = (x[usize::from(p)], x[usize::from(q)]);
        x[usize::from(p)] = c * xp - s * xq;
        x[usize::from(q)] = s * xp + c * xq;
    }
    x
}

/// `SystemOp::Rotated` densely: `i^phase prod_parts prod_m gamma(u, spin, m)` with
/// `gamma(u, s, x) = sum_p u_p gamma_{2(2p + s) + x}`.
fn dense(op: &SystemOp, g: &[M]) -> M {
    let SystemOp::Rotated(r) = op else {
        panic!("sa operators are rotated")
    };
    let mut out = M::eye();
    for part in &r.parts {
        let u = vector(&part.network);
        let s = usize::from(part.spin.unwrap());
        for &m in &part.majoranas {
            let x = usize::from(m) - 2 * s;
            let mut v = M::zero();
            for (p, &up) in u.iter().enumerate() {
                v = v.add(&g[2 * (2 * p + s) + x], C(up, 0.0));
            }
            out = out.mul(&v);
        }
    }
    let ph = [C(1.0, 0.0), C(0.0, 1.0), C(-1.0, 0.0), C(0.0, -1.0)][usize::from(r.phase % 4)];
    M(out.0.iter().map(|z| z.mul(ph)).collect())
}

fn payload(e: &[f64], wb: &[f64], w: &[f64]) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15u64 | 1;
    let mut ang = |count: usize| -> Vec<u32> {
        (0..count)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % (1 << BETA)) as u32
            })
            .collect()
    };
    let mut out = b"FEMOSAS1".to_vec();
    for v in [1u32, N as u32, 1, 2, 2, BETA, 3] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(-2.5f64).to_le_bytes());
    e.iter()
        .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    ang(N * (N - 1))
        .iter()
        .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    wb.iter()
        .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    w.iter()
        .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    ang(2 * (N - 1))
        .iter()
        .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    out
}

/// `sos + sum_r e_r Et(u_r) + sum_q (1/2)(wB_q + sum_b w_qb Et(u_rb))^2`, `Et(u) = sum_s n(u, s) - 1`.
fn h_spec(s: &SaSpec, g: &[M]) -> M {
    let et = |net: &Network| -> M {
        // n(u, s) = b^dagger b with b = sum_p u_p a_{2p+s}, a_m = (gamma_2m + i gamma_2m+1) / 2.
        let u = vector(net);
        let mut acc = M::eye().add(&M::eye(), C(-2.0, 0.0)); // -1
        for sp in 0..2 {
            let mut bm = M::zero();
            for (p, &up) in u.iter().enumerate() {
                let m = 2 * p + sp;
                bm = bm.add(&g[2 * m], C(up / 2.0, 0.0));
                bm = bm.add(&g[2 * m + 1], C(0.0, up / 2.0));
            }
            let bd = M((0..DIM * DIM)
                .map(|k| {
                    let (i, j) = (k / DIM, k % DIM);
                    let z = bm.0[j * DIM + i];
                    C(z.0, -z.1)
                })
                .collect());
            acc = acc.add(&bd.mul(&bm), C(1.0, 0.0));
        }
        acc
    };
    let mut h = M::eye();
    h = M(h.0.iter().map(|z| z.scale(s.sos_const)).collect());
    for r in 0..N {
        h = h.add(&et(&s.e_nets[r]), C(s.e[r], 0.0));
    }
    for q in 0..s.r * s.c {
        let r = q / s.c;
        let mut o = M(M::eye().0.iter().map(|z| z.scale(s.wb[q])).collect());
        for b in 0..s.b {
            o = o.add(&et(&s.nets[r * s.b + b]), C(s.w[q * s.b + b], 0.0));
        }
        h = h.add(&o.mul(&o), C(0.5, 0.0));
    }
    h
}

/// `lambda_decl E_s [2 E_b R(s, b) - R(s, a)]` over every lane of the map.
fn encoded(s: &SaSpec, map: &SaNestedMap, g: &[M]) -> M {
    let (u_o, w) = (map.outer_bits(), map.inner_width());
    let lam = map.lambda_decl.to_f64();
    let mut a = M::zero();
    let mut cache: std::collections::HashMap<String, M> = std::collections::HashMap::new();
    let mut get = |op: SystemOp| -> M {
        let key = format!("{op:?}");
        cache.entry(key).or_insert_with(|| dense(&op, g)).clone()
    };
    for so in 0..1u64 << u_o {
        for ai in 0..1u64 << w {
            let sv = so | ai << u_o;
            let diag = get(map.reference_nested(s, sv, sv));
            a = a.add(&diag, C(-lam / f64::from(1u32 << (u_o + w)), 0.0));
            for bi in 0..1u64 << w {
                let after = so | bi << u_o;
                let r = get(map.reference_nested(s, sv, after));
                a = a.add(&r, C(2.0 * lam / f64::from(1u32 << (u_o + 2 * w)), 0.0));
            }
        }
    }
    a
}

fn check(s: &SaSpec, map: &SaNestedMap) -> (f64, f64) {
    let g: Vec<M> = (0..2 * MODES).map(gamma).collect();
    let h = h_spec(s, &g);
    let lam_decl = map.lambda_decl.to_f64();
    let shift = -s.identity.to_f64() + lam_decl - s.lambda_flat.to_f64();
    let want = h.add(&M::eye(), C(shift, 0.0));
    let got = encoded(s, map, &g);
    let rounding = map.rounding_error(s).unwrap().to_f64();
    (got.max_diff(&want), rounding)
}

#[test]
fn encoded_operator_is_h_spec_shifted() {
    let s = parse_payload(
        "test-sa-v1",
        &payload(&[3.0, -1.0, 4.0], &[2.0, -1.0], &[1.0, 1.0, 2.0, -1.0]),
    )
    .unwrap();
    let outer = Table::from_counts(3, 2, &[6, 2, 8, 8, 8]).unwrap();
    let mut inner: Vec<Table> = (0..N)
        .map(|_| Table::from_counts(2, 0, &[2, 2]).unwrap())
        .collect();
    inner.push(Table::from_counts(2, 0, &[1, 1, 2]).unwrap());
    inner.push(Table::from_counts(2, 0, &[2, 1, 1]).unwrap());
    let map = SaNestedMap::new(Exact::from_int(16), outer, inner, &s).unwrap();
    let (diff, rounding) = check(&s, &map);
    println!("exact map: max |A - (H_spec - identity + lambda_decl - lambda_flat)| = {diff:.3e}");
    assert_eq!(rounding, 0.0);
    assert!(diff < 1e-12, "{diff}");
    // A rounded map: the entrywise distance is at most 2 x the exact rounding error (the certificate's
    // error factor).
    let odd = parse_payload(
        "test-sa-odd",
        &payload(&[0.3, -1.7, 2.9], &[0.45, -1.1], &[0.7, 1.3, -2.2, 0.35]),
    )
    .unwrap();
    let map = sa_nested::build(&odd, (3, 3), (2, 2)).unwrap();
    let (diff, rounding) = check(&odd, &map);
    println!("rounded map: max entry diff {diff:.3e}, rounding error {rounding:.3e}");
    assert!(
        rounding > 0.0 && diff <= 2.0 * rounding + 1e-12,
        "{diff} vs {rounding}"
    );
}
