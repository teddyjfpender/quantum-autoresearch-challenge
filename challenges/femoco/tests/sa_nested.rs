//! The spectrum-amplified lane map end to end (spec/SPEC-SA.md sections 5 and 8): a hand-built
//! circuit on a 3-orbital sos-sa spec passes with exact counts and a verified `givens-sa-nested`;
//! the lane map's exact rounding error equals a brute-force enumeration of its lanes; and wrong
//! signs, a missing adjoint, a wrong inner term, a dirty inner register and a mislabelled spin are
//! each rejected.
use femoco_walk::circuit::{read_ops, write_ops, Builder, Op, OpsFile, Qubit, Reg};
use femoco_walk::circuit::{SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::lanemap::df_nested::Table;
use femoco_walk::lanemap::sa_nested::{self, SaNestedMap};
use femoco_walk::lanemap::{self, LaneMap};
use femoco_walk::score::{score_json, Evaluation, FamilyOut, Inputs};
use femoco_walk::sim::givens_tracker;
use femoco_walk::spec::sa::{parse_payload, Generator, SaSpec};
use femoco_walk::spec::{EncodingSpec, Exact, Network};
use femoco_walk::taxonomy::{self, AxisStatus, AxisVerdict, Family};
use num_bigint::BigInt;
use num_traits::Signed;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

const N: usize = 3;
const BETA: u32 = 8;
const K: usize = 4096;

fn angles(seed: u64, count: usize) -> Vec<u32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..count)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % (1 << BETA)) as u32
        })
        .collect()
}

/// `FEMOSAS1` for (R, B, C) = (1, 2, 2): e = (3, -1, 4) (one-body weights 2|e| = 6, 2, 8), squares
/// w = (1, 1), wB = 2 and w = (2, -1), wB = -1 (S = 4, S^2/2 = 8 each). Outer total 32, Lambda =
/// 8 + 4 + 4 = 16, and every weight is dyadic, so exact lane maps exist.
fn payload(e: &[f64], wb: &[f64], w: &[f64]) -> Vec<u8> {
    let mut out = b"FEMOSAS1".to_vec();
    for v in [1u32, N as u32, 1, 2, 2, BETA, 3] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(-2.5f64).to_le_bytes());
    e.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    angles(30, N * (N - 1))
        .iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    wb.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    w.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    angles(31, 2 * (N - 1))
        .iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    out
}

fn spec() -> SaSpec {
    let bytes = payload(&[3.0, -1.0, 4.0], &[2.0, -1.0], &[1.0, 1.0, 2.0, -1.0]);
    parse_payload("test-sa-v1", &bytes).unwrap()
}

/// Outer `k_o = 3, mu_o = 2` (32 lanes: 6, 2, 8, 8, 8); inner `k_i = 2, mu_i = 0` (4 lanes: one-body
/// (2, 2), squares (1, 1, 2) and (2, 1, 1)).
fn exact_map(s: &SaSpec) -> SaNestedMap {
    let outer = Table::from_counts(3, 2, &[6, 2, 8, 8, 8]).unwrap();
    let mut inner: Vec<Table> = (0..N)
        .map(|_| Table::from_counts(2, 0, &[2, 2]).unwrap())
        .collect();
    inner.push(Table::from_counts(2, 0, &[1, 1, 2]).unwrap());
    inner.push(Table::from_counts(2, 0, &[2, 1, 1]).unwrap());
    SaNestedMap::new(Exact::from_int(16), outer, inner, s).unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mutation {
    None,
    /// The second copy applies `M` instead of `M^dagger` on the one-body `i gamma` term.
    NoAdjoint,
    /// Square inner term `(j = 0)` of square 0 gets the wrong sign.
    FlipSquareSign,
    /// One-body item 0 applies its `x = 1` term for `x = 0` as well.
    WrongInnerTerm,
    /// The inner copy leaves inner uniform bit 0 flipped.
    InnerNotRestored,
    /// The one-body generator uses the other spin.
    WrongSpin,
    /// The identity item of square 1 is applied with the wrong sign.
    FlipIdentitySign,
}

struct Ladder {
    sel: Qubit,
    ands: Vec<(Qubit, Qubit, Qubit)>,
    flips: Vec<Qubit>,
}

fn ladder_up(b: &mut Builder, ctrl: Qubit, bits: &[Qubit], v: u64) -> Ladder {
    let flips: Vec<Qubit> = bits
        .iter()
        .enumerate()
        .filter(|(i, _)| v >> i & 1 == 0)
        .map(|(_, &q)| q)
        .collect();
    flips.iter().for_each(|&q| b.x(q));
    let mut ands = Vec::new();
    let mut prev = ctrl;
    for &q in bits {
        let a = b.alloc();
        b.ccx(prev, q, a);
        ands.push((prev, q, a));
        prev = a;
    }
    Ladder {
        sel: prev,
        ands,
        flips,
    }
}

fn ladder_down(b: &mut Builder, l: Ladder) {
    for &(c1, c2, a) in l.ands.iter().rev() {
        b.ccx(c1, c2, a);
        b.free(a);
    }
    l.flips.iter().for_each(|&q| b.x(q));
}

/// `V (op) V^dagger` on spin `spin`, every Givens controlled by `sel` through its angle register.
fn rotated(
    b: &mut Builder,
    sel: Qubit,
    reg: Reg,
    bits: &[Qubit],
    net: &Network,
    spin: u8,
    op: impl FnOnce(&mut Builder),
) {
    let mode = |p: u16| 2 * usize::from(p) + usize::from(spin);
    let unit = 1u32 << BETA;
    let givens = |b: &mut Builder, p: usize, q: usize, a: u32| {
        let set: Vec<Qubit> = bits
            .iter()
            .enumerate()
            .filter(|(j, _)| a >> j & 1 == 1)
            .map(|(_, &x)| x)
            .collect();
        set.iter().for_each(|&x| b.cx(sel, x));
        b.givens_modes(p, q, reg);
        set.iter().for_each(|&x| b.cx(sel, x));
    };
    for &(p, q, a) in net.rotations.iter().rev() {
        givens(b, mode(p), mode(q), (unit - a) % unit);
    }
    op(b);
    for &(p, q, a) in &net.rotations {
        givens(b, mode(p), mode(q), a);
    }
}

/// Controlled `gamma_{2m + x}` on spin orbital `m` (Jordan-Wigner, spec/DESIGN.md section 3):
/// `Z_0 ... Z_{m-1} X_m` or `Z_0 ... Z_{m-1} Y_m`, with `Y = i X Z` (the `i` on `sel`).
fn majorana(b: &mut Builder, sel: Qubit, m: usize, x: usize) {
    for j in 0..m {
        let q = b.system(j);
        b.cz(sel, q);
    }
    let q = b.system(m);
    if x == 1 {
        b.cz(sel, q);
    }
    b.cx(sel, q);
    if x == 1 {
        b.s(sel);
    }
}

/// The naive circuit: an outer unary pass sets one flag per generator (one-body `(r, s1)` or
/// square `q`), the inner copy is a unary pass over each flagged item's inner values, and
/// `nested_inner` emits it twice around the Reflect. The copy toggles a pass qubit so the one-body
/// `+-i gamma` term becomes its adjoint in the second pass.
fn build(s: &SaSpec, map: &SaNestedMap, m: Mutation) -> Vec<Op> {
    let mut b = Builder::new(2 * N);
    b.declare_uniform(map.uniform_bits());
    let (u_o, w) = (map.outer_bits(), map.inner_width());
    let bits = b.alloc_n(BETA as usize);
    let reg = b.register(&bits);
    let inner_reg = b.inner_register(u_o, w);
    let ob_flags: Vec<[Qubit; 2]> = (0..N).map(|_| [b.alloc(), b.alloc()]).collect();
    let sq_flags = b.alloc_n(s.r * s.c);
    let pass = b.alloc();
    let outer_bits: Vec<Qubit> = (0..u_o).map(|i| b.uniform(i)).collect();
    let inner_bits: Vec<Qubit> = (u_o..u_o + w).map(|i| b.uniform(i)).collect();
    b.segment(SEG_PREPARE);
    b.segment(SEG_SELECT);
    let outer_pass = |b: &mut Builder| {
        for v in 0..1u64 << u_o {
            let control = b.control();
            let l = ladder_up(b, control, &outer_bits, v);
            match map.decode_outer(v) {
                Generator::OneBody { r, spin } => b.cx(l.sel, ob_flags[r][usize::from(spin)]),
                Generator::Square { r, c } => b.cx(l.sel, sq_flags[r * s.c + c]),
            }
            ladder_down(b, l);
        }
    };
    outer_pass(&mut b);
    b.nested_inner(inner_reg, |b| {
        for (r, flags) in ob_flags.iter().enumerate() {
            for (spin, &flag) in flags.iter().enumerate() {
                let spin = if m == Mutation::WrongSpin && r == 1 {
                    1 - spin as u8
                } else {
                    spin as u8
                };
                for a in 0..1u64 << w {
                    let (mut x, _) = map.inner_item(r, a);
                    if m == Mutation::WrongInnerTerm && r == 0 {
                        x = 1;
                    }
                    let l = ladder_up(b, flag, &inner_bits, a);
                    let sel = l.sel;
                    rotated(b, sel, reg, &bits, &s.e_nets[r], spin, |b| {
                        majorana(b, sel, usize::from(spin), x);
                        if x == 1 {
                            // M_1 = i^p gamma_1 (p = 1 for e >= 0, 3 otherwise): S or Sdg on sel.
                            // The second pass applies M_1^dagger = i^-p gamma_1, which differs by
                            // i^-2p = -1: CZ(sel, pass).
                            if s.e[r] >= 0.0 {
                                b.s(sel);
                            } else {
                                b.sdg(sel);
                            }
                            if m != Mutation::NoAdjoint {
                                b.cz(sel, pass);
                            }
                        }
                    });
                    ladder_down(b, l);
                }
            }
        }
        for (q, &flag) in sq_flags.iter().enumerate() {
            let (r, c) = (q / s.c, q % s.c);
            for a in 0..1u64 << w {
                let (j, s0) = map.inner_item(N + q, a);
                let l = ladder_up(b, flag, &inner_bits, a);
                let sel = l.sel;
                if j == s.b {
                    let neg = (s.wb[q] < 0.0) != (m == Mutation::FlipIdentitySign && q == 1);
                    if neg {
                        b.z(sel);
                    }
                } else {
                    // M = sign(w) i gamma gamma = -sign(w) Z(u, s0).
                    let neg = (s.w[q * s.b + j] > 0.0)
                        != (m == Mutation::FlipSquareSign && q == 0 && j == 0);
                    rotated(b, sel, reg, &bits, &s.nets[r * s.b + j], s0, |b| {
                        let zq = b.system(usize::from(s0));
                        b.cz(sel, zq);
                    });
                    if neg {
                        b.z(sel);
                    }
                }
                let _ = c;
                ladder_down(b, l);
            }
        }
        b.x(pass);
        if m == Mutation::InnerNotRestored {
            b.x(inner_bits[0]);
        }
    });
    outer_pass(&mut b);
    b.segment(SEG_UNPREPARE);
    for q in ob_flags
        .into_iter()
        .flatten()
        .chain(sq_flags)
        .chain([pass])
        .chain(bits)
    {
        b.free(q);
    }
    b.finish()
}

static TMP: AtomicU64 = AtomicU64::new(0);

fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-sa-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    f
}

const AXES: [(&str, &str); 7] = [
    ("encoding", "sos-sa"),
    ("lane_map", "sa-nested-alias-v1"),
    ("lookup", "unary-iteration"),
    ("select", "givens-sa-nested"),
    ("uncompute", "unitary"),
    ("reuse", "serial"),
    ("rotation", "phase-gradient-givens"),
];

fn eval_ops(s: &SaSpec, lanemap: &[u8], ops: &[Op], samples: usize) -> Result<Evaluation, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    let tax = taxonomy::load_taxonomy(&path).unwrap();
    let check = |f: &Family, facts: &femoco_walk::facts::CircuitFacts| -> Vec<AxisVerdict> {
        taxonomy::check(&tax, f, facts)
            .into_iter()
            .filter(|v| v.axis != "uncompute")
            .collect()
    };
    let axes: BTreeMap<String, String> = AXES
        .iter()
        .map(|(a, v)| (a.to_string(), v.to_string()))
        .collect();
    let fam = serde_json::to_vec(&FamilyOut {
        family: Family {
            taxonomy_version: "1.3.0".into(),
            name: "test-sa-naive".into(),
            parent: None,
            axes,
        },
        spec: s.id.clone(),
    })
    .unwrap();
    let file = ops_file(ops);
    evaluate(&Inputs {
        spec: s,
        lanemap,
        family: &fam,
        ops: &file,
        samples,
        tracker: givens_tracker(s),
        check: &check,
    })
}

fn eval_mutant(m: Mutation) -> Result<Evaluation, String> {
    let s = spec();
    let map = exact_map(&s);
    eval_ops(&s, &map.to_bytes(), &build(&s, &map, m), K)
}

#[test]
fn spec_values_are_exact() {
    let s = spec();
    assert_eq!(s.lambda(), Exact::from_int(16));
    // identity = sos + sum (wB^2/2 + sum w^2/4) = -2.5 + (2 + 0.5) + (0.5 + 1.25) = 1.75.
    assert_eq!(s.identity(), Exact::from_f64(1.75).unwrap());
    // E_SOS = sos - sum|e| = -2.5 - 8.
    assert_eq!(s.e_sos, Exact::from_f64(-10.5).unwrap());
    // lambda_flat = 2 Lambda - (identity - E_SOS) = 32 - 12.25.
    assert_eq!(s.lambda_flat, Exact::from_f64(19.75).unwrap());
}

/// `sum_t |c_t - c^_t|` by enumerating every lane triple `(s_o, a, b)` of the map.
fn brute_force_error(s: &SaSpec, map: &SaNestedMap) -> Exact {
    let (u_o, w) = (map.outer_bits(), map.inner_width());
    // key: (outer item, s1 or 0, x_full, y_full) -> lane count; x_full: one-body x, square j*2+s0
    // for j < B and 2B for the identity.
    let mut counts: HashMap<(usize, u8, usize, usize), u64> = HashMap::new();
    for so in 0..1u64 << u_o {
        let o = map.outer_item(so);
        let s1 = if o < N {
            ((so >> (u_o - 1)) & 1) as u8
        } else {
            0
        };
        let full = |a: u64| {
            let (j, s0) = map.inner_item(o, a);
            if o < N {
                j
            } else if j == s.b {
                2 * s.b
            } else {
                2 * j + usize::from(s0)
            }
        };
        for a in 0..1u64 << w {
            for bb in 0..1u64 << w {
                let (x, y) = (full(a), full(bb));
                if x != y {
                    *counts.entry((o, s1, x, y)).or_default() += 1;
                }
            }
        }
    }
    let lam = map.lambda_decl.clone();
    let mut err = Exact::zero();
    let mut seen = 0;
    let ex = |x: f64| Exact::from_f64(x).unwrap().abs();
    let mut add = |key: (usize, u8, usize, usize), c: Exact| {
        let n = counts.get(&key).copied().unwrap_or(0);
        seen += 1;
        // c^ = 2 lambda n / 2^(u_o + 2w)
        let got = lam
            .mul(&Exact::from_int(2 * n as i64))
            .mul(&Exact::dyadic(BigInt::from(1), u_o + 2 * w));
        err = err.add(&c.sub(&got).abs());
    };
    for r in 0..N {
        for s1 in 0..2u8 {
            for (x, y) in [(0, 1), (1, 0)] {
                add((r, s1, x, y), ex(s.e[r]).mul(&Exact::dyadic(1.into(), 2)));
            }
        }
    }
    for q in 0..s.r * s.c {
        let o = N + q;
        let items = 2 * s.b + 1;
        for x in 0..items {
            for y in 0..items {
                if x == y {
                    continue;
                }
                let wt = |i: usize| {
                    if i == 2 * s.b {
                        ex(s.wb[q])
                    } else {
                        ex(s.w[q * s.b + i / 2])
                    }
                };
                let div = if x == 2 * s.b || y == 2 * s.b { 2 } else { 3 };
                add(
                    (o, 0, x, y),
                    wt(x).mul(&wt(y)).mul(&Exact::dyadic(1.into(), div)),
                );
            }
        }
    }
    assert_eq!(
        seen,
        counts.len(),
        "every enumerated product is a spec term"
    );
    err
}

#[test]
fn rounding_error_matches_brute_force() {
    let s = spec();
    let exact = exact_map(&s);
    assert_eq!(exact.rounding_error(&s).unwrap(), Exact::zero());
    assert_eq!(brute_force_error(&s, &exact), Exact::zero());
    // Non-exact tables from the builder at several widths: the formula equals the enumeration.
    for (o, i) in [
        ((3, 1), (2, 1)),
        ((3, 2), (2, 2)),
        ((3, 3), (2, 1)),
        ((4, 2), (3, 1)),
    ] {
        let map = sa_nested::build(&s, o, i).unwrap();
        let bytes = map.to_bytes();
        let back = lanemap::parse(&bytes, &s).unwrap();
        let formula = back.rounding_error(&s).unwrap();
        assert_eq!(formula, brute_force_error(&s, &map), "{o:?} {i:?}");
        println!("{o:?} {i:?}: rounding error {}", formula.to_f64());
    }
    // Non-dyadic weights: every builder map has a nonzero error, and the formula still matches.
    let bytes = payload(&[0.3, -1.7, 2.9], &[0.45, -1.1], &[0.7, 1.3, -2.2, 0.35]);
    let odd = parse_payload("test-sa-odd", &bytes).unwrap();
    for (o, i) in [((3, 2), (2, 2)), ((3, 4), (2, 3)), ((4, 3), (3, 2))] {
        let map = sa_nested::build(&odd, o, i).unwrap();
        let f = map.rounding_error(&odd).unwrap();
        assert!(f > Exact::zero());
        assert_eq!(f, brute_force_error(&odd, &map), "{o:?} {i:?}");
    }
    // A perturbed exact map: one outer lane moved from a square to a one-body item.
    let outer = Table::from_counts(3, 2, &[7, 2, 8, 7, 8]).unwrap();
    let mut inner = exact_map(&s).inner;
    inner[N + 1] = Table::from_counts(2, 0, &[1, 2, 1]).unwrap();
    let map = SaNestedMap::new(Exact::from_int(16), outer, inner, &s).unwrap();
    let f = map.rounding_error(&s).unwrap();
    assert!(f > Exact::zero());
    assert_eq!(f, brute_force_error(&s, &map));
}

#[test]
fn hand_built_circuit_passes() {
    let ev = eval_mutant(Mutation::None).unwrap();
    assert_eq!(ev.rounding_error, Exact::zero());
    assert_eq!(ev.lambda, Exact::from_int(16));
    let n = ev.nested.expect("nested stats");
    assert_eq!(n.inner_width, 3);
    assert!(n.diagonal_samples > K / 4 && n.paired_samples > K / 4);
    assert!(ev.facts.nested_validated);
    for axis in ["encoding", "lane_map", "select", "rotation"] {
        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
        assert_eq!(v.status, AxisStatus::Verified, "{axis}");
    }
    // No certificate for the synthetic spec: the score falls back to the worst-case Lambda.
    assert_eq!(ev.score_lambda(), 16.0);
    let json = score_json(&ev);
    assert_eq!(json["metrics"]["encoding"], "sos-sa");
    assert_eq!(
        json["metrics"]["spectral_amplification"]["available"],
        false
    );
    println!(
        "C_step {} Q_peak {} paired {} diagonal {}",
        ev.toffoli, ev.qubits, n.paired_samples, n.diagonal_samples
    );
}

fn rejects(m: Mutation, want: &str) {
    let e = eval_mutant(m).expect_err(&format!("{m:?} must be rejected"));
    println!("{m:?}: {e}");
    assert!(e.contains(want), "{m:?}: got {e}");
}

#[test]
fn a_missing_adjoint_is_rejected() {
    // Diagonal lanes of a one-body x = 1 item apply (i gamma)^2 = -1 instead of 1.
    rejects(Mutation::NoAdjoint, "sign");
}

#[test]
fn a_wrong_square_sign_is_rejected() {
    rejects(Mutation::FlipSquareSign, "sign");
}

#[test]
fn a_wrong_identity_sign_is_rejected() {
    rejects(Mutation::FlipIdentitySign, "sign");
}

#[test]
fn a_wrong_inner_term_is_rejected() {
    rejects(Mutation::WrongInnerTerm, "differs from the reference");
}

#[test]
fn a_wrong_spin_is_rejected() {
    rejects(Mutation::WrongSpin, "differs from the reference");
}

#[test]
fn an_unrestored_inner_register_is_rejected() {
    rejects(Mutation::InnerNotRestored, "");
}

#[test]
fn a_flat_or_df_lane_map_is_refused_for_sos_sa() {
    let s = spec();
    let map = exact_map(&s);
    // A df lane-map family string with this payload is refused by its own parser.
    let mut bytes = map.to_bytes();
    let fam = b"df-nested-alias-v1";
    let at = 8 + 2;
    let old = sa_nested::FAMILY.len();
    bytes.splice(at..at + old, fam.iter().copied());
    bytes[8..10].copy_from_slice(&(fam.len() as u16).to_le_bytes());
    let e = lanemap::parse(&bytes, &s).err().expect("refused");
    assert!(e.contains("not a df spec"), "{e}");
    // lambda_decl must be dyadic and positive.
    let bad = SaNestedMap::new(
        Exact::from_int(-1),
        map.outer.clone(),
        map.inner.clone(),
        &s,
    );
    assert!(bad.is_err());
    // Too many uniform bits.
    let wide = sa_nested::build(&s, (3, 30), (2, 29));
    assert!(wide.err().unwrap().contains("above 63"));
    let _ = BigInt::from(0).abs();
}
