//! `SpinSwap` (op kind 32), the sos-sa spec addition of spec/SPEC-SA.md section 11:
//! - it is accepted for sos-sa specs only, and a non-system control is required;
//! - its semantics are the `F` layer `prod_p G_{2p, 2p+1}(pi / 2)` that a df baseline builds from
//!   `N` Givens reading `pi / 2` from a register: the same circuit passes either way and only the
//!   charge differs;
//! - its charge, `N + 1` Toffolis, rests on two dense identities checked here on 2 and 3 spatial
//!   orbitals: `F` is the fermionic swap of the two spin blocks times one parity `Z` per pair, and
//!   in the spin-blocked Jordan-Wigner order that swap is the qubit swap of the blocks times
//!   `(-1)^(N_up N_down)`.
#![cfg(feature = "walk")]
#![allow(clippy::needless_range_loop)]
use femoco_walk::circuit::{Builder, Op, OperationType as K, Qubit};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{spin_swap_toffoli, FamilyOut, Inputs};
use femoco_walk::sim::{compile, compile_nested, compile_sa, givens_tracker, Inner, Layout};
use femoco_walk::spec::sa::{parse_payload, SaSpec};
use femoco_walk::spec::EncodingSpec;
use femoco_walk::taxonomy;
use femoco_walk::walk::sa_low::{emit, family, lane_map, Params, Tweaks};

const BETA: u32 = 8;

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

/// The 3-orbital exact sos-sa spec of `tests/sa_nested.rs`.
fn spec() -> SaSpec {
    let n = 3;
    let mut out = b"FEMOSAS1".to_vec();
    for v in [1u32, n as u32, 1, 2, 2, BETA, 3] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(-2.5f64).to_le_bytes());
    for x in [3.0f64, -1.0, 4.0] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    for a in angles(30, n * (n - 1)) {
        out.extend_from_slice(&a.to_le_bytes());
    }
    for x in [2.0f64, -1.0, 1.0, 1.0, 2.0, -1.0] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    for a in angles(31, 2 * (n - 1)) {
        out.extend_from_slice(&a.to_le_bytes());
    }
    parse_payload("test-sa-v1", &out).unwrap()
}

const P: Params = Params {
    outer: (3, 2),
    inner: (2, 1),
    outer_a: 1,
    inner_a: 1,
    swap: true,
    chunks: 1,
    outer_erase: false,
    dense: false,
    tw: Tweaks::OFF,
    pareto: false,
    lean: false,
    carries: 0,
    drop_alt: false,
    narrow: femoco_walk::walk::sa_low::pareto::Narrow::OFF,
};

fn build(s: &SaSpec, p: Params) -> (Vec<u8>, Vec<Op>) {
    let map = lane_map(s, p).unwrap();
    let mut b = Builder::new(2 * s.n);
    b.declare_uniform(map.uniform_bits());
    let _ = emit(s, &map, &mut b, p);
    (map.to_bytes(), b.finish())
}

fn eval(
    s: &SaSpec,
    lm: &[u8],
    ops: &[Op],
    swap: bool,
) -> Result<femoco_walk::score::Evaluation, String> {
    let dir = std::env::temp_dir().join(format!(
        "femoco-sa-swap-{}-{}",
        std::process::id(),
        ops.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    femoco_walk::circuit::write_ops(ops, &path).unwrap();
    let file = femoco_walk::circuit::read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let tp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    let tax = taxonomy::load_taxonomy(&tp).unwrap();
    let check = |f: &_, facts: &_| taxonomy::check(&tax, f, facts);
    let fam = serde_json::to_vec(&FamilyOut {
        family: family(swap),
        spec: s.id().to_string(),
    })
    .unwrap();
    evaluate(&Inputs {
        spec: s,
        lanemap: lm,
        family: &fam,
        ops: &file,
        samples: 1 << 13,
        tracker: givens_tracker(s),
        check: &check,
    })
}

/// Replaces every `SpinSwap(c)` / `SpinSwapDg(c)` with the df baseline's form of the same
/// layer: a `beta`-qubit register holding `c 2^(beta - 2)` (or `3 c 2^(beta - 2)`) and `N`
/// Givens `G_{2p, 2p+1}` reading it.
fn expand(ops: &[Op], n: usize, first_free: u32) -> Vec<Op> {
    // One register over fresh qubits, declared before everything (a copy may not declare one).
    let reg = ops
        .iter()
        .filter(|o| o.kind == K::Register)
        .map(|o| o.r_target + 1)
        .max()
        .unwrap_or(0);
    let regq: Vec<u32> = (0..BETA).map(|i| first_free + i).collect();
    let mut out = Vec::new();
    let mut r = Op::new(K::Register);
    r.r_target = reg;
    out.push(r);
    for &q in &regq {
        let mut a = Op::new(K::AppendToRegister);
        a.q_target = q;
        a.r_target = reg;
        out.push(a);
    }
    for o in ops {
        if !matches!(o.kind, K::SpinSwap | K::SpinSwapDg) {
            out.push(*o);
            continue;
        }
        // pi / 2 is bit beta - 2; -pi / 2 = 3 pi / 2 sets bit beta - 1 as well.
        let mut cx = Op::new(K::CX);
        cx.q_control1 = o.q_target;
        cx.q_target = regq[BETA as usize - 2];
        let mut cx_hi = cx;
        cx_hi.q_target = regq[BETA as usize - 1];
        let dg = o.kind == K::SpinSwapDg;
        out.push(cx);
        if dg {
            out.push(cx_hi);
        }
        for p in 0..n {
            let mut g = Op::new(K::Givens);
            g.q_control1 = 1 + 2 * p as u32;
            g.q_target = 2 + 2 * p as u32;
            g.r_target = reg;
            out.push(g);
        }
        out.push(cx);
        if dg {
            out.push(cx_hi);
        }
    }
    out
}

#[test]
fn spin_swap_equals_the_givens_layer_and_costs_less() {
    let s = spec();
    let (lm, ops) = build(&s, P);
    let ev = eval(&s, &lm, &ops, true).unwrap();
    // Two SpinSwaps per copy, on every lane.
    assert!((ev.spin_swaps - 4.0).abs() < 1e-12);
    assert!((ev.spin_swap_toffoli - 4.0 * 4.0).abs() < 1e-12);
    // The same circuit with each SpinSwap written as N Givens reading pi/2 from a register.
    let top = ops.iter().flat_map(Op::qubits).max().unwrap_or(0);
    let wide = expand(&ops, s.n, top + 1);
    let ev2 = eval(&s, &lm, &wide, true).unwrap();
    assert_eq!(ev2.spin_swaps, 0.0);
    // Toffolis: 4 SpinSwaps (N + 1 = 4 each) become 4 N Givens at 2 (beta - 2) = 12 each.
    let d = ev2.toffoli - ev.toffoli;
    assert!(
        (d - (4.0 * 3.0 * 12.0 - 16.0)).abs() < 1e-9,
        "difference {d}"
    );
    println!(
        "SpinSwap C_step {} vs Givens layers {}",
        ev.toffoli, ev2.toffoli
    );
}

#[test]
fn a_wrong_spin_swap_is_rejected() {
    let s = spec();
    let (lm, ops) = build(&s, P);
    // Drop the second SpinSwap of each copy (F^s after V): the lane is left in the other spin.
    let swaps: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o.kind, K::SpinSwap | K::SpinSwapDg))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(swaps.len(), 4);
    let mut m = ops.clone();
    for &i in [swaps[1], swaps[3]].iter().rev() {
        m.remove(i);
    }
    let e = eval(&s, &lm, &m, true).expect_err("an unbalanced spin swap must be rejected");
    println!("{e}");
}

#[test]
fn spin_swap_is_for_sos_sa_only() {
    let op = |c: u32| {
        let mut o = Op::new(K::SpinSwap);
        o.q_target = c;
        o
    };
    let layout = Layout {
        system: 6,
        uniform: 2,
    };
    let s = spec();
    let tr = givens_tracker(&s);
    // An ancilla control (qubit 9 = first ancilla).
    let ok = [op(9)];
    assert!(compile_sa(&ok, &layout, tr, None).is_ok());
    for e in [
        compile(&ok, &layout, tr).err(),
        compile_nested(&ok, &layout, tr, Some(Inner { lo: 0, width: 1 })).err(),
    ] {
        assert!(e.unwrap().contains("sos-sa specs only"));
    }
    // A system control, and no tracker, are refused.
    assert!(compile_sa(&[op(2)], &layout, tr, None)
        .err()
        .unwrap()
        .contains("non-system"));
    assert!(compile_sa(&ok, &layout, None, None)
        .err()
        .unwrap()
        .contains("tracker"));
    // Shape: only q_target.
    let mut bad = op(9);
    bad.q_control1 = 10;
    assert!(bad.validate().is_err());
    assert_eq!(spin_swap_toffoli(108), 55.0);
    assert_eq!(spin_swap_toffoli(152), 77.0);
    let _ = Qubit(0);
}

// ---- Dense identities behind the charge ----

type M = Vec<Vec<f64>>;

fn eye(d: usize) -> M {
    (0..d)
        .map(|i| (0..d).map(|j| f64::from(u8::from(i == j))).collect())
        .collect()
}

fn mul(a: &M, b: &M) -> M {
    let d = a.len();
    let mut c = vec![vec![0.0; d]; d];
    for i in 0..d {
        for k in 0..d {
            if a[i][k] != 0.0 {
                for j in 0..d {
                    c[i][j] += a[i][k] * b[k][j];
                }
            }
        }
    }
    c
}

fn t(a: &M) -> M {
    let d = a.len();
    (0..d).map(|i| (0..d).map(|j| a[j][i]).collect()).collect()
}

fn add(a: &M, b: &M, s: f64) -> M {
    a.iter()
        .zip(b)
        .map(|(r, q)| r.iter().zip(q).map(|(x, y)| x + s * y).collect())
        .collect()
}

fn close(a: &M, b: &M) -> bool {
    a.iter()
        .zip(b)
        .all(|(r, q)| r.iter().zip(q).all(|(x, y)| (x - y).abs() < 1e-12))
}

/// Annihilator of mode `j` in a Jordan-Wigner order over `n` modes (bit `j` of the basis index is
/// mode `j`'s occupation; the string counts occupied modes before `j`).
fn ann(j: usize, n: usize) -> M {
    let d = 1 << n;
    let mut a = vec![vec![0.0; d]; d];
    for x in 0..d {
        if x >> j & 1 == 1 {
            let sign = if (x & ((1 << j) - 1)).count_ones() % 2 == 1 {
                -1.0
            } else {
                1.0
            };
            a[x ^ (1 << j)][x] = sign;
        }
    }
    a
}

/// `exp(theta (a_q^+ a_p - a_p^+ a_q))` by its Taylor series (real antisymmetric generator).
fn givens(p: usize, q: usize, theta: f64, n: usize) -> M {
    let (ap, aq) = (ann(p, n), ann(q, n));
    let k = add(&mul(&t(&aq), &ap), &mul(&t(&ap), &aq), -1.0);
    let d = 1 << n;
    let mut out = eye(d);
    let mut term = eye(d);
    for i in 1..60 {
        term = mul(&term, &k);
        term = term
            .iter()
            .map(|r| r.iter().map(|x| x * theta / f64::from(i)).collect())
            .collect();
        out = add(&out, &term, 1.0);
    }
    out
}

fn parity(j: usize, n: usize) -> M {
    let d = 1 << n;
    let mut z = vec![vec![0.0; d]; d];
    for (x, row) in z.iter_mut().enumerate() {
        row[x] = if x >> j & 1 == 1 { -1.0 } else { 1.0 };
    }
    z
}

/// The fermionic permutation unitary `U a_m U^+ = a_{pi(m)}` for a mode permutation `pi`, with
/// `U |vac> = |vac>`: occupied modes are re-created in the new order.
fn fermionic_perm(pi: &[usize], n: usize) -> M {
    let d = 1 << n;
    let mut u = vec![vec![0.0; d]; d];
    for x in 0..d {
        // |x> = a+_{m1} ... a+_{mk} |vac> with m1 < ... < mk (this JW convention).
        let occ: Vec<usize> = (0..n).filter(|&m| x >> m & 1 == 1).collect();
        let mut v = vec![0.0; d];
        v[0] = 1.0;
        for &m in occ.iter().rev() {
            let c = t(&ann(pi[m], n));
            v = (0..d)
                .map(|i| (0..d).map(|j| c[i][j] * v[j]).sum())
                .collect();
        }
        for i in 0..d {
            u[i][x] = v[i];
        }
    }
    u
}

#[test]
fn f_layer_is_the_spin_block_swap_times_parities() {
    for n in [2usize, 3] {
        let modes = 2 * n;
        // Interleaved (harness) order: mode (p, s) is 2p + s.
        let mut f = eye(1 << modes);
        for p in 0..n {
            f = mul(
                &givens(2 * p, 2 * p + 1, std::f64::consts::FRAC_PI_2, modes),
                &f,
            );
        }
        let pi: Vec<usize> = (0..modes).map(|m| m ^ 1).collect();
        let swap = fermionic_perm(&pi, modes);
        // F = (fermionic spin swap) x prod_p Z on one mode of each pair (which mode, and on
        // which side, is fixed by the Givens sign convention; one of the four must hold).
        let mut zs0 = eye(1 << modes);
        let mut zs1 = eye(1 << modes);
        for p in 0..n {
            zs0 = mul(&parity(2 * p, modes), &zs0);
            zs1 = mul(&parity(2 * p + 1, modes), &zs1);
        }
        let hits = [
            mul(&swap, &zs0),
            mul(&swap, &zs1),
            mul(&zs0, &swap),
            mul(&zs1, &swap),
        ]
        .iter()
        .filter(|c| close(c, &f))
        .count();
        assert!(hits >= 1, "n = {n}: F is not the spin swap times parities");
    }
}

#[test]
fn blocked_order_spin_swap_is_qubit_swap_times_block_parity_phase() {
    for n in [2usize, 3] {
        let modes = 2 * n;
        // Blocked order: mode (p, s) is s n + p. The fermionic swap (p, 0) <-> (p, 1).
        let pi: Vec<usize> = (0..modes).map(|m| (m + n) % modes).collect();
        let fswap = fermionic_perm(&pi, modes);
        let d = 1 << modes;
        let mut claim = vec![vec![0.0; d]; d];
        for x in 0..d {
            let (up, down) = (x & ((1 << n) - 1), x >> n);
            let y = down | up << n;
            let sign = if up.count_ones() % 2 == 1 && down.count_ones() % 2 == 1 {
                -1.0
            } else {
                1.0
            };
            claim[y][x] = sign;
        }
        assert!(close(&fswap, &claim), "n = {n}");
        // And it maps a_(p,0) to a_(p,1).
        for p in 0..n {
            let lhs = mul(&mul(&fswap, &ann(p, modes)), &t(&fswap));
            assert!(close(&lhs, &ann(p + n, modes)), "n = {n}, p = {p}");
        }
    }
}

#[test]
fn spin_swap_lane_map_counts_are_untouched() {
    // SpinSwap changes nothing in the lane map: the same map passes with and without it.
    let s = spec();
    let a = lane_map(&s, P).unwrap();
    let b = lane_map(&s, Params { swap: false, ..P }).unwrap();
    assert_eq!(a.to_bytes(), b.to_bytes());
}
