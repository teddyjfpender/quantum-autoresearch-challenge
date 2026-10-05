//! Shared thc fixtures: a small synthetic thc spec (3 spatial orbitals, rank 3, random 8-bit
//! networks, so the factors are far from orthogonal; dyadic weights so an exact lane map exists),
//! a naive controlled SELECT that implements every lane with Givens rotations and flips the swap
//! bit, and its mutations.
#![allow(dead_code)]
use femoco_walk::circuit::{read_ops, write_ops, Builder, Op, OpsFile, Qubit, Reg, SEG_SELECT};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::facts::CircuitFacts;
use femoco_walk::lanemap::alias::AliasMap;
use femoco_walk::lanemap::thc_pair::ThcPairMap;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{Evaluation, FamilyOut, Inputs};
use femoco_walk::sim::givens_tracker;
use femoco_walk::spec::thc::{parse_payload, Pair, ThcSpec};
use femoco_walk::spec::{EncodingSpec, Exact, Network};
use femoco_walk::taxonomy::{AxisStatus, AxisVerdict, Family};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub const SPATIAL: usize = 3;
pub const RANK: usize = 3;
pub const BETA: u32 = 8;

/// Deterministic angles in `0..2^BETA`.
pub fn angles(seed: u64, count: usize) -> Vec<u32> {
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

/// A `FEMOTHC1` payload (the THC encoding; no THC spec ships here).
pub fn payload(
    n: usize,
    beta: u32,
    ecore: f64,
    t: &[f64],
    t_ang: &[u32],
    chi_ang: &[u32],
    zeta_tri: &[f64],
) -> Vec<u8> {
    let m = chi_ang.len() / (n - 1);
    let mut out = b"FEMOTHC1".to_vec();
    for v in [1u32, n as u32, beta, m as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&ecore.to_le_bytes());
    t.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    t_ang
        .iter()
        .chain(chi_ang)
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    zeta_tri
        .iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    out
}

/// One-body `t = (2, -1, 2)`; `zeta` in pair order `(00, 01, 11, 02, 12, 22)` =
/// `(2, -2, -2, 2, -4, 2)`. Combination weights `|t|/8`, `|zeta|/8` (`mu < nu`) and `|zeta|/16`
/// (`mu = nu`) are `(2, 1, 2, 1, 2, 1, 2, 4, 1) / 8`, total 2, so 16 buckets hold them exactly.
pub fn spec() -> ThcSpec {
    let r = SPATIAL - 1;
    let bytes = payload(
        SPATIAL,
        BETA,
        0.25,
        &[2.0, -1.0, 2.0],
        &angles(10, SPATIAL * r),
        &angles(20, RANK * r),
        &[2.0, -2.0, -2.0, 2.0, -4.0, 2.0],
    );
    parse_payload("test-thc-v1", &bytes).unwrap()
}

/// The exact lane map: k = 4, mu = 0, counts (2, 1, 2, 1, 2, 1, 2, 4, 1), lambda_decl = 16.
pub fn lanemap(spec: &ThcSpec) -> ThcPairMap {
    let counts = [2, 1, 2, 1, 2, 1, 2, 4, 1];
    let t = AliasMap::from_counts(4, 0, Exact::from_int(16), &counts).unwrap();
    ThcPairMap::new(t, spec).unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    None,
    /// An extra -1 on every lane of this pair.
    FlipSign(u64),
    /// The pair's `mu` factor uses the next factor `(mu + 1) mod M`.
    WrongMu(u64),
    /// The pair's `nu` factor uses the next factor `(nu + 1) mod M`.
    WrongNu(u64),
    /// The pair's first applied rotation angle is one unit off (in `V` only).
    AngleOffByOne(u64),
    /// The pair's last rotation of `V` is skipped.
    MissingRotation(u64),
    /// The swap bit is not flipped at the end (a SELECT that is not a reflection).
    NoSwapFlip,
    /// The swap bit is flipped on control-0 lanes too.
    SwapFlipUncontrolled,
    /// The swap bit is ignored: every lane applies `Z(chi_mu) Z(chi_nu)`, never the reversed
    /// product (not Hermitian when the factors overlap).
    IgnoreOrder,
}

/// One `Z(u, spin)` factor as the circuit implements it.
struct Factor {
    net: Network,
    spin: u8,
}

/// The factors of the lane's operator in product order (the leftmost acts last).
fn factors(spec: &ThcSpec, map: &ThcPairMap, s: u64, m: Mutation) -> Vec<Factor> {
    let l = map.decode(s);
    let net = |n: &std::sync::Arc<Network>| (**n).clone();
    let mut out = match spec.pair(l.pair).unwrap() {
        Pair::OneBody { k } => vec![Factor {
            net: net(&spec.t_nets[k]),
            spin: l.s1,
        }],
        Pair::TwoBody { mu, nu } if mu == nu && l.s1 == l.s2 => Vec::new(),
        Pair::TwoBody { mu, nu } => {
            let mu = if m == Mutation::WrongMu(l.pair) {
                (mu + 1) % RANK
            } else {
                mu
            };
            let nu = if m == Mutation::WrongNu(l.pair) {
                (nu + 1) % RANK
            } else {
                nu
            };
            let a = Factor {
                net: net(&spec.chi_nets[mu]),
                spin: l.s1,
            };
            let b = Factor {
                net: net(&spec.chi_nets[nu]),
                spin: l.s2,
            };
            if l.swap && m != Mutation::IgnoreOrder {
                vec![b, a]
            } else {
                vec![a, b]
            }
        }
    };
    if m == Mutation::AngleOffByOne(l.pair) {
        if let Some(f) = out.last_mut() {
            f.net.rotations[0].2 = (f.net.rotations[0].2 + 1) % (1 << BETA);
        }
    }
    out
}

/// Loads `a` (controlled on `sel`), applies `Givens` on modes `(p, q)`, unloads.
fn givens(b: &mut Builder, sel: Qubit, reg: Reg, bits: &[Qubit], p: usize, q: usize, a: u32) {
    let set: Vec<Qubit> = bits
        .iter()
        .enumerate()
        .filter(|(j, _)| a >> j & 1 == 1)
        .map(|(_, &x)| x)
        .collect();
    set.iter().for_each(|&x| b.cx(sel, x));
    b.givens_modes(p, q, reg);
    set.iter().for_each(|&x| b.cx(sel, x));
}

/// `V Z_spin V^dagger` for the factor's network, all controlled on `sel`.
fn apply_factor(
    b: &mut Builder,
    sel: Qubit,
    reg: Reg,
    bits: &[Qubit],
    f: &Factor,
    skip_last: bool,
) {
    let mode = |p: u16| 2 * usize::from(p) + usize::from(f.spin);
    let unit = 1u32 << BETA;
    for &(p, q, a) in f.net.rotations.iter().rev() {
        givens(b, sel, reg, bits, mode(p), mode(q), (unit - a) % unit);
    }
    let z = b.system(usize::from(f.spin));
    b.cz(sel, z);
    let last = f.net.rotations.len() - 1;
    for (j, &(p, q, a)) in f.net.rotations.iter().enumerate() {
        if !(skip_last && j == last) {
            givens(b, sel, reg, bits, mode(p), mode(q), a);
        }
    }
}

/// Naive controlled SELECT: every uniform value gets an AND ladder with the control, then the
/// lane's factors (the last factor first) and its sign; at the end the swap bit is flipped under
/// the control (Lee et al. 2021 Fig. 6: the `X` on the swap-control qubit).
pub fn build(spec: &ThcSpec, map: &ThcPairMap, m: Mutation) -> Vec<Op> {
    let mut b = Builder::new(2 * SPATIAL);
    let u = map.uniform_bits();
    b.declare_uniform(u);
    let bits = b.alloc_n(BETA as usize);
    let reg = b.register(&bits);
    b.segment(SEG_SELECT);
    for s in 0..1u64 << u {
        let l = map.decode(s);
        let p = spec.pair(l.pair).unwrap();
        let (sel, ladder, flips) = ladder_up(&mut b, s);
        let fs = factors(spec, map, s, m);
        for (i, f) in fs.iter().enumerate().rev() {
            let skip = m == Mutation::MissingRotation(l.pair) && i == 0;
            apply_factor(&mut b, sel, reg, &bits, f, skip);
        }
        if spec.negative(p) != (m == Mutation::FlipSign(l.pair)) {
            b.z(sel);
        }
        ladder_down(&mut b, ladder, &flips);
    }
    let swap = b.uniform(map.swap_bit());
    match m {
        Mutation::NoSwapFlip => {}
        Mutation::SwapFlipUncontrolled => b.x(swap),
        _ => {
            let c = b.control();
            b.cx(c, swap);
        }
    }
    bits.iter().for_each(|&q| b.free(q));
    b.finish()
}

type Ladder = Vec<(Qubit, Qubit, Qubit)>;

fn ladder_up(b: &mut Builder, s: u64) -> (Qubit, Ladder, Vec<Qubit>) {
    let u = b.uniform_bits();
    let flips: Vec<Qubit> = (0..u)
        .filter(|&i| s >> i & 1 == 0)
        .map(|i| b.uniform(i))
        .collect();
    flips.iter().for_each(|&q| b.x(q));
    let mut ladder = Vec::new();
    let mut prev = b.control();
    for i in 0..u {
        let ui = b.uniform(i);
        let a = b.alloc();
        b.ccx(prev, ui, a);
        ladder.push((prev, ui, a));
        prev = a;
    }
    (prev, ladder, flips)
}

fn ladder_down(b: &mut Builder, mut ladder: Ladder, flips: &[Qubit]) {
    while let Some((c1, c2, a)) = ladder.pop() {
        b.ccx(c1, c2, a);
        b.free(a);
    }
    flips.iter().for_each(|&q| b.x(q));
}

static TMP: AtomicU64 = AtomicU64::new(0);

pub fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-thc-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    f
}

pub fn family(spec: &str) -> Vec<u8> {
    let axes: BTreeMap<String, String> = [("encoding", "thc"), ("lane_map", "thc-pair-alias-v1")]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    let fam = FamilyOut {
        family: Family {
            taxonomy_version: "1.1.0".into(),
            name: "test-thc-naive".into(),
            parent: None,
            axes,
        },
        spec: spec.to_string(),
    };
    serde_json::to_vec(&fam).unwrap()
}

pub fn declared_only(f: &Family, _facts: &CircuitFacts) -> Vec<AxisVerdict> {
    f.axes
        .iter()
        .map(|(axis, value)| AxisVerdict {
            axis: axis.clone(),
            value: value.clone(),
            status: AxisStatus::DeclaredOnly,
        })
        .collect()
}

/// Builds the naive circuit with mutation `m` and evaluates it with the Gaussian tracker.
pub fn eval_mutant(m: Mutation, samples: usize) -> Result<Evaluation, String> {
    let spec = spec();
    let map = lanemap(&spec);
    let file = ops_file(&build(&spec, &map, m));
    let lm = map.to_bytes();
    let fam = family(spec.id());
    evaluate(&Inputs {
        spec: &spec,
        lanemap: &lm,
        family: &fam,
        ops: &file,
        samples,
        tracker: givens_tracker(&spec),
        check: &declared_only,
    })
}
