//! Adversarial checks of the `sa-toff` levers, written without the walk's own fault hooks or
//! expected-count ledger. The helpers are a small-spec payload writer, an exhaustive lane
//! enumerator, a structural mutant generator and an exact expected-Toffoli analyzer. Environment:
//! `RT_KEYS` (the `Hmr` outcome keys per exhaustive run, default 2) and `RT_ROWS` (labels of the
//! pinned rows to analyze, default all).
//!
//! 1. `exhaustive_*`: every lane `(c, s, b)` of small sos-sa specs through the trusted simulator
//!    (`validate::run_nested`) for lever `p` (shared inner padding, with `2^inner_a = 2^k_i`
//!    blocks, on specs whose pad plan has d = 0 and d = 1), for the split one-hot `v` / `w`, and
//!    for every lever combined with them in a pinned row (`imchxlgrp`, `imchxgrvd`, ...), at
//!    several QROAM block counts `inner_a`.
//! 2. `mutants_*`: the base mutant classes (`mutants`) plus: every Toffoli into an angle register (the
//!    split one-hot's group corrections) dropped / moved to the next bit, every CNOT or Toffoli
//!    into a correction control (parity scratch, group bits) dropped, and, on the `p` and `v`
//!    bundles, every CCX and every CX of the stream dropped one at a time. Each is judged by the
//!    harness's `score::evaluate`; survivors are re-judged at K = 2^18 and then exhaustively.
//! 3. `p_*`: the padded lane map against the circuit built for the unpadded one (and vice versa)
//!    must be rejected, and on FeMoco the padded tables must have exactly the unpadded counts.
//! 4. `exact_expectation_pinned_rows`: the EXACT expected Toffolis per step of the pinned
//!    outcome-gated rows, digest-checked against tests/sa_digests.rs.
#![cfg(feature = "walk")]
use femoco_walk::circuit::{read_ops, write_ops, Builder, Op, OperationType as K, OpsFile, NONE};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::fiat_shamir::Lane;
use femoco_walk::lanemap::{self, LaneMap};
use femoco_walk::score::{Evaluation, FamilyOut, Inputs};
use femoco_walk::sim::{compile_sa, givens_tracker, validate, Inner, Layout, TrackerFactory};
use femoco_walk::spec::sa::{parse_payload, SaSpec};
use femoco_walk::spec::EncodingSpec;
use femoco_walk::taxonomy;
use femoco_walk::walk::sa_low::{self, pareto::Narrow, Params, Tweaks};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------------------------
// Small specs (own payload writer; same layout as FEMOSAS1 v1).

const BETA: u32 = 8;

fn xorshift(seed: u64, count: usize, bits: u32) -> Vec<u32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..count)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % (1 << bits)) as u32
        })
        .collect()
}

/// `FEMOSAS1` for `n` orbitals, `(R, B, C)`, with angle `k` of network `net` passed through
/// `mask(j, a)` (`j` the chain position).
fn payload(
    n: usize,
    rbc: (usize, usize, usize),
    e: &[f64],
    wb: &[f64],
    w: &[f64],
    seed: u64,
    mask: &dyn Fn(usize, u32) -> u32,
) -> Vec<u8> {
    let (r, bb, c) = rbc;
    let mut out = b"FEMOSAS1".to_vec();
    for v in [1u32, n as u32, r as u32, bb as u32, c as u32, BETA, 3] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(-2.5f64).to_le_bytes());
    e.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    for (i, a) in xorshift(seed, n * (n - 1), BETA).iter().enumerate() {
        out.extend_from_slice(&mask(i % (n - 1), *a).to_le_bytes());
    }
    wb.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    w.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    for (i, a) in xorshift(seed + 1, r * bb * (n - 1), BETA)
        .iter()
        .enumerate()
    {
        out.extend_from_slice(&mask(i % (n - 1), *a).to_le_bytes());
    }
    out
}

/// N = 5, (R, B, C) = (1, 2, 2), dyadic weights (exact lane maps at wide keeps).
fn spec5(seed: u64, mask: &dyn Fn(usize, u32) -> u32) -> SaSpec {
    let bytes = payload(
        5,
        (1, 2, 2),
        &[3.0, -1.0, 2.0, 1.0, -1.0],
        &[2.0, -1.0],
        &[1.0, 1.0, 2.0, -1.0],
        seed,
        mask,
    );
    parse_payload("rt-sa5-v1", &bytes).unwrap()
}

/// N = 4, (R, B, C) = (2, 3, 1): two square rows, three items each (more one-hot leaves, a
/// ragged tree with left-only nodes).
fn spec4(seed: u64) -> SaSpec {
    let bytes = payload(
        4,
        (2, 3, 1),
        &[1.0, -2.0, 1.0, 4.0],
        &[1.0, -4.0],
        &[1.0, -1.0, 1.0, 2.0, -1.0, 1.0],
        seed,
        &|_, a| a,
    );
    parse_payload("rt-sa4-v1", &bytes).unwrap()
}

fn params(outer: (u32, u32), inner: (u32, u32), tw: &str, swap: bool) -> Params {
    Params {
        outer,
        inner,
        outer_a: 1,
        inner_a: 1,
        swap,
        chunks: 1,
        outer_erase: false,
        dense: false,
        tw: Tweaks::parse(tw),
        pareto: false,
        lean: false,
        carries: 0,
        drop_alt: false,
        narrow: Narrow::OFF,
    }
}

fn build(s: &SaSpec, p: Params) -> (Vec<u8>, Vec<Op>) {
    let map = sa_low::lane_map(s, p).unwrap();
    let mut b = Builder::new(2 * s.n);
    b.declare_uniform(map.uniform_bits());
    let _ = sa_low::emit(s, &map, &mut b, p);
    (map.to_bytes(), b.finish())
}

static TMP: AtomicU64 = AtomicU64::new(0);

fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-rt-est-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    f
}

/// The harness: `score::evaluate` with the production taxonomy check.
fn harness(
    s: &SaSpec,
    p: Params,
    lm: &[u8],
    ops: &[Op],
    samples: usize,
) -> Result<Evaluation, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    let tax = taxonomy::load_taxonomy(&path).unwrap();
    let check = |f: &_, facts: &_| taxonomy::check(&tax, f, facts);
    let fam = serde_json::to_vec(&FamilyOut {
        family: if p.tw.any_toff() {
            sa_low::family_toff()
        } else {
            sa_low::family(p.swap)
        },
        spec: s.id().to_string(),
    })
    .unwrap();
    let file = ops_file(ops);
    evaluate(&Inputs {
        spec: s,
        lanemap: lm,
        family: &fam,
        ops: &file,
        samples,
        tracker: givens_tracker(s),
        check: &check,
    })
}

// ---------------------------------------------------------------------------------------------
// Exhaustive lanes through the trusted simulator.

/// Every `(c, s, b)` lane, under each of `keys` `Hmr` outcome streams. Skips the rounding rule
/// (the per-lane operator check is what is exhausted here). Returns the number of lanes run.
fn exhaustive(s: &SaSpec, lm_bytes: &[u8], ops: &[Op], keys: usize) -> Result<usize, String> {
    let lm = lanemap::parse(lm_bytes, s)?;
    let layout = Layout {
        system: s.system_qubits(),
        uniform: lm.uniform_bits(),
    };
    let (lo, width) = lm.inner_bits().ok_or("not nested")?;
    let compiled = compile_sa(ops, &layout, givens_tracker(s), Some(Inner { lo, width }))?;
    let u = layout.uniform;
    assert!(u + width < 23, "too many lanes: u {u} w {width}");
    let inner_mask = ((1u64 << width) - 1) << lo;
    let mut lanes = Vec::new();
    let mut after = Vec::new();
    for c in [false, true] {
        for sv in 0..1u64 << u {
            for bv in 0..1u64 << width {
                lanes.push(Lane { c, s: sv });
                after.push((sv & !inner_mask) | (bv << lo));
            }
        }
    }
    let reference = |x: u64| lm.reference_op(s, x);
    let reference_nested = |x: u64, a: u64| lm.reference_nested(s, x, a);
    let uniform_after = |x: u64| lm.uniform_after(x);
    for k in 0..keys {
        let key: [u8; 32] = Sha256::digest(format!("adversarial-sa hmr key {k}").as_bytes()).into();
        let ctx = validate::Context {
            compiled: &compiled,
            layout,
            hmr_key: key,
            reference: &reference,
            uniform_after: &uniform_after,
            tracker: givens_tracker(s),
        };
        let n = validate::Nested {
            lo,
            width,
            after: &after,
            reference: &reference_nested,
        };
        let out = femoco_walk::equiv::run_nested_checked(&ctx, &n, &lanes);
        if let Some(why) = out.rejection() {
            return Err(format!("key {k}: {why}"));
        }
    }
    Ok(lanes.len() * keys)
}

/// `Hmr` outcome keys per exhaustive run (`RT_KEYS`, default 2).
fn keys() -> usize {
    std::env::var("RT_KEYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
}

// ---------------------------------------------------------------------------------------------
// Mutants.

/// The qubits of every declared register (the `Givens` angle registers), and per qubit its
/// register's qubit list.
fn registers(ops: &[Op]) -> HashMap<u32, Vec<u32>> {
    let mut regs: HashMap<u32, Vec<u32>> = HashMap::new();
    for o in ops {
        if o.kind == K::AppendToRegister {
            regs.entry(o.r_target).or_default().push(o.q_target);
        }
    }
    let mut by_q = HashMap::new();
    for qs in regs.values() {
        for &q in qs {
            by_q.insert(q, qs.clone());
        }
    }
    by_q
}

struct Mutants {
    list: Vec<(String, Vec<Op>)>,
}

impl Mutants {
    fn drop(&mut self, ops: &[Op], i: usize, why: &str) {
        let mut m = ops.to_vec();
        m.remove(i);
        self.list.push((format!("{why} (op {i} dropped)"), m));
    }
    fn replace(&mut self, ops: &[Op], i: usize, op: Op, why: &str) {
        let mut m = ops.to_vec();
        m[i] = op;
        self.list.push((format!("{why} (op {i})"), m));
    }
}

/// Every mutant of the one-hot RPREP and the gated erasure in `ops`.
fn mutants(ops: &[Op]) -> Mutants {
    let regs = registers(ops);
    let mut m = Mutants { list: Vec::new() };
    // One-hot fan-out: CNOTs into an angle register.
    let fan: Vec<usize> = (0..ops.len())
        .filter(|&i| ops[i].kind == K::CX && regs.contains_key(&ops[i].q_target))
        .collect();
    let hot: BTreeSet<u32> = fan.iter().map(|&i| ops[i].q_control1).collect();
    let hot_v: Vec<u32> = hot.iter().copied().collect();
    for &i in &fan {
        m.drop(ops, i, "fan-out CNOT dropped");
        let o = ops[i];
        let reg = &regs[&o.q_target];
        if reg.len() > 1 {
            let k = reg.iter().position(|&q| q == o.q_target).unwrap();
            let mut r = o;
            r.q_target = reg[(k + 1) % reg.len()];
            m.replace(ops, i, r, "fan-out CNOT into the wrong angle bit");
        }
        if hot_v.len() > 1 {
            let k = hot_v.iter().position(|&q| q == o.q_control1).unwrap();
            let mut r = o;
            r.q_control1 = hot_v[(k + 1) % hot_v.len()];
            if r.q_control1 != r.q_target {
                m.replace(ops, i, r, "fan-out CNOT from the wrong one-hot leaf");
            }
        }
    }
    // One-hot write: CNOTs into a one-hot qubit from an iteration flag.
    for i in 0..ops.len() {
        if ops[i].kind == K::CX && hot.contains(&ops[i].q_target) {
            m.drop(ops, i, "one-hot write CNOT dropped");
        }
    }
    // Gated blocks: every Toffoli and every phase op inside a condition, each dropped; each
    // block's condition inverted.
    let mut depth = 0usize;
    for i in 0..ops.len() {
        match ops[i].kind {
            K::PushCondition => {
                depth += 1;
                let r = ops[i];
                // Invert this block only: X on the bit before, X after its pop.
                let close = (i + 1..ops.len())
                    .scan(1i64, |d, j| {
                        match ops[j].kind {
                            K::PushCondition => *d += 1,
                            K::PopCondition => *d -= 1,
                            _ => {}
                        }
                        Some((j, *d))
                    })
                    .find(|&(_, d)| d == 0)
                    .map(|(j, _)| j)
                    .unwrap();
                let mut inv = Op::new(K::BitInvert);
                inv.c_target = r.c_condition;
                let mut mm = ops[..i].to_vec();
                mm.push(inv);
                mm.extend_from_slice(&ops[i..=close]);
                mm.push(inv);
                mm.extend_from_slice(&ops[close + 1..]);
                m.list
                    .push((format!("condition block at op {i} inverted"), mm));
            }
            K::PopCondition => depth -= 1,
            K::CCX | K::CCZ | K::CZ | K::Z if depth > 0 => {
                m.drop(ops, i, "gated op dropped");
            }
            K::CZ | K::Z if ops[i].c_condition != NONE => {
                m.drop(ops, i, "outcome-conditioned phase dropped");
            }
            _ => {}
        }
    }
    m
}

// ---------------------------------------------------------------------------------------------
// Exact expectation of outcome-gated Toffolis.

type Mono = Vec<u32>;

/// A Boolean polynomial over `Hmr` outcome variables (algebraic normal form): a sorted set of
/// monomials, each a sorted set of variables; the empty monomial is the constant 1.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
struct Poly(Vec<Mono>);

impl Poly {
    fn one() -> Self {
        Poly(vec![vec![]])
    }
    fn var(v: u32) -> Self {
        Poly(vec![vec![v]])
    }
    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
    fn is_one(&self) -> bool {
        self.0.len() == 1 && self.0[0].is_empty()
    }
    fn xor(&self, o: &Self) -> Self {
        let mut s: BTreeSet<Mono> = self.0.iter().cloned().collect();
        for m in &o.0 {
            if !s.remove(m) {
                s.insert(m.clone());
            }
        }
        Poly(s.into_iter().collect())
    }
    fn mul(&self, o: &Self) -> Self {
        let mut s: BTreeSet<Mono> = BTreeSet::new();
        for a in &self.0 {
            for b in &o.0 {
                let m: BTreeSet<u32> = a.iter().chain(b).copied().collect();
                let m: Mono = m.into_iter().collect();
                if !s.remove(&m) {
                    s.insert(m);
                }
            }
        }
        assert!(s.len() < 4096, "polynomial blow-up");
        Poly(s.into_iter().collect())
    }
    fn vars(&self, out: &mut BTreeSet<u32>) {
        self.0.iter().flatten().for_each(|&v| {
            out.insert(v);
        });
    }
    fn eval(&self, val: &dyn Fn(u32) -> bool) -> bool {
        self.0
            .iter()
            .fold(false, |acc, m| acc ^ m.iter().all(|&v| val(v)))
    }
}

/// `P(every factor = 1)` over uniform independent outcomes, by enumeration.
fn prob(factors: &[Poly]) -> f64 {
    let mut vs = BTreeSet::new();
    factors.iter().for_each(|f| f.vars(&mut vs));
    let vs: Vec<u32> = vs.into_iter().collect();
    assert!(
        vs.len() <= 24,
        "{} outcome variables in one event",
        vs.len()
    );
    let pos: HashMap<u32, usize> = vs.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let mut hit = 0u64;
    for a in 0..1u64 << vs.len() {
        let val = |v: u32| a >> pos[&v] & 1 == 1;
        if factors.iter().all(|f| f.eval(&val)) {
            hit += 1;
        }
    }
    hit as f64 / (1u64 << vs.len()) as f64
}

/// Exact `E[C_step]` and `Var[C_step]` per lane of an op stream, with the harness's charges:
/// CCX / CCZ 1, `Givens` `2 (w - 2)` at the tracker's scale (tapered) or `2 (beta - 2)`,
/// `SpinSwap` `N + 1`, inner `Reflect` `w_inner - 2`, plus the constant `u - 2`.
struct Expect {
    mean: f64,
    var: f64,
    /// Charged weight that is not conditioned on any outcome.
    fixed: f64,
    events: usize,
    toffoli_gated_ops: usize,
}

fn exact_expectation(
    ops: &[Op],
    system: usize,
    uniform: u32,
    inner_width: u32,
    tracker: Option<&dyn TrackerFactory>,
) -> Expect {
    let mut bits: Vec<Poly> = Vec::new();
    let mut stack: Vec<Poly> = Vec::new();
    let mut next_var = 0u32;
    let mut events: HashMap<Vec<Poly>, f64> = HashMap::new();
    let mut gated_ops = 0usize;
    let tapered = tracker.is_some_and(|t| t.givens_charge(0, 1).is_some());
    let get = |bits: &mut Vec<Poly>, b: u32| -> Poly {
        let b = b as usize;
        if bits.len() <= b {
            bits.resize(b + 1, Poly::default());
        }
        bits[b].clone()
    };
    for o in ops {
        let cond = if o.c_condition != NONE && o.kind != K::PushCondition {
            Some(get(&mut bits, o.c_condition))
        } else {
            None
        };
        let mut factors: Vec<Poly> = stack.clone();
        factors.extend(cond);
        let charge = match o.kind {
            K::CCX | K::CCZ => 1.0,
            K::Givens => {
                let t = tracker.expect("Givens needs a tracker");
                let (p, q) = ((o.q_control1 - 1) as usize, (o.q_target - 1) as usize);
                if tapered {
                    t.givens_charge(p, q).unwrap() as f64
                } else {
                    t.givens_toffoli_cost()
                }
            }
            K::SpinSwap | K::SpinSwapDg => (system / 2 + 1) as f64,
            K::Reflect => f64::from(inner_width.saturating_sub(2)),
            _ => 0.0,
        };
        let mask = || -> Poly { factors.iter().fold(Poly::one(), |acc, f| acc.mul(f)) };
        match o.kind {
            K::PushCondition => {
                let p = get(&mut bits, o.c_condition);
                stack.push(p);
            }
            K::PopCondition => {
                stack.pop().expect("pop on an empty stack");
            }
            K::Hmr => {
                let r = Poly::var(next_var);
                next_var += 1;
                let b = get(&mut bits, o.c_target);
                let m = mask();
                bits[o.c_target as usize] = if m.is_one() {
                    r
                } else {
                    b.xor(&m.mul(&r.xor(&b)))
                };
            }
            K::BitInvert | K::BitStore0 | K::BitStore1 => {
                let b = get(&mut bits, o.c_target);
                let m = mask();
                bits[o.c_target as usize] = match o.kind {
                    K::BitInvert => b.xor(&m),
                    K::BitStore0 => b.xor(&m.mul(&b)),
                    _ => b.xor(&m).xor(&m.mul(&b)),
                };
            }
            _ if charge > 0.0 => {
                let mut key: Vec<Poly> = factors.into_iter().filter(|f| !f.is_one()).collect();
                if key.iter().any(Poly::is_zero) {
                    continue;
                }
                key.sort();
                key.dedup();
                if !key.is_empty() && o.kind != K::Givens {
                    gated_ops += 1;
                }
                *events.entry(key).or_insert(0.0) += charge;
            }
            _ => {}
        }
    }
    let ev: Vec<(Vec<Poly>, f64)> = events.into_iter().collect();
    let p: Vec<f64> = ev.iter().map(|(k, _)| prob(k)).collect();
    let base = f64::from(uniform.saturating_sub(2));
    let mean = base + ev.iter().zip(&p).map(|((_, w), p)| w * p).sum::<f64>();
    let fixed = base
        + ev.iter()
            .filter(|(k, _)| k.is_empty())
            .map(|(_, w)| w)
            .sum::<f64>();
    // Var = sum_{e,f} w_e w_f (P(e and f) - P(e) P(f)), over the gated events only.
    let gated: Vec<usize> = (0..ev.len()).filter(|&i| !ev[i].0.is_empty()).collect();
    let mut var = 0.0;
    for (a, &i) in gated.iter().enumerate() {
        for &j in &gated[a..] {
            let mut both = ev[i].0.clone();
            both.extend(ev[j].0.iter().cloned());
            both.sort();
            both.dedup();
            let c = ev[i].1 * ev[j].1 * (prob(&both) - p[i] * p[j]);
            var += if i == j { c } else { 2.0 * c };
        }
    }
    Expect {
        mean,
        var,
        fixed,
        events: gated.len(),
        toffoli_gated_ops: gated_ops,
    }
}

fn expect_of(s: &SaSpec, lm_bytes: &[u8], ops: &[Op]) -> Expect {
    let lm = lanemap::parse(lm_bytes, s).unwrap();
    let (_, w) = lm.inner_bits().unwrap();
    exact_expectation(
        ops,
        s.system_qubits(),
        lm.uniform_bits(),
        w,
        givens_tracker(s),
    )
}

fn pinned(id: &str) -> Box<dyn EncodingSpec> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    femoco_walk::spec::load(root, id).unwrap()
}

fn toff_mu(s: &SaSpec, tw: &str, mu: Option<(u32, u32)>, inner_a: Option<usize>) -> Params {
    let base = Params::for_spec(s);
    let (mo, mi) = mu.unwrap_or((base.outer.1, base.inner.1));
    Params {
        tw: Tweaks::parse(tw),
        outer: (base.outer.0, mo),
        inner: (base.inner.0, mi),
        inner_a: inner_a.unwrap_or(base.inner_a),
        ..base
    }
}

/// N = 5 with generic (non-dyadic) weights: keep values carry every bit, so no comparator bit is
/// constantly zero.
fn spec5_generic() -> SaSpec {
    let bytes = payload(
        5,
        (1, 2, 2),
        &[3.1, -1.37, 2.03, 0.91, -1.13],
        &[2.21, -0.83],
        &[1.07, 0.61, 1.93, -1.19],
        30,
        &|_, a| a,
    );
    parse_payload("rt-sa5g-v1", &bytes).unwrap()
}

// ---------------------------------------------------------------------------------------------
// Small specs for lever `p` (inner tables with padding) and helpers.

/// N = 5, (R, B, C) = (1, 11, 11): 12 inner items in `2^4` buckets, a 4-bucket padding block
/// (s = 2), and 16 outer items (the item layout `x` needs `k_x >= k_i`). Every square has the
/// same inner weights. `dominant`: one item holds more than a quarter of each square's weight, so
/// the pad plan is d = 0; otherwise two items hold more than an eighth each and none a quarter,
/// so d = 1.
fn spec_p3(dominant: bool) -> SaSpec {
    let (wb, pat, id): (f64, [f64; 11], &str) = if dominant {
        (
            5.0,
            [1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0],
            "rt-sap11a-v1",
        )
    } else {
        (
            3.0,
            [3.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0],
            "rt-sap11b-v1",
        )
    };
    // The payload stores w as [R][C][B].
    let w: Vec<f64> = (0..121).map(|i| pat[i % 11]).collect();
    // Square weight S = 16 each (11 x 128), one-body 2 sum|e| = 640: 2048 in all (exact).
    let bytes = payload(
        5,
        (1, 11, 11),
        &[100.0, -60.0, 80.0, 40.0, -40.0],
        &[wb; 11],
        &w,
        50,
        &|_, a| a,
    );
    parse_payload(id, &bytes).unwrap()
}

/// As [`spec_p3`] with generic (non-dyadic) weights, for the narrow-keep exhaustive runs.
fn spec_p3_generic() -> SaSpec {
    let pat = [
        2.1, 1.1, -0.7, 0.45, 0.6, -0.52, 0.55, 0.93, -1.2, 0.8, 0.66,
    ];
    let w: Vec<f64> = (0..121)
        .map(|i| pat[i % 11] * (1.0 + 0.03 * (i / 11) as f64))
        .collect();
    let wb: Vec<f64> = (0..11).map(|c| 2.3 - 0.05 * c as f64).collect();
    let bytes = payload(
        5,
        (1, 11, 11),
        &[3.1, -1.37, 2.03, 0.91, -1.13],
        &wb,
        &w,
        52,
        &|_, a| a,
    );
    parse_payload("rt-sap11g-v1", &bytes).unwrap()
}

/// `k_o = ceil(log2(N + R C))`.
fn ko(s: &SaSpec) -> u32 {
    let mut k = 0;
    while (1usize << k) < s.n + s.r * s.c {
        k += 1;
    }
    k
}

/// `k_i = ceil(log2(B + 1))`.
fn ki(s: &SaSpec) -> u32 {
    let mut k = 0;
    while (1usize << k) < s.b + 1 {
        k += 1;
    }
    k
}

/// N = 5, (R, B, C) = (1, 5, 1): 6 inner items in `2^3` buckets (a 2-bucket padding block, d = 0).
fn spec_p5() -> SaSpec {
    let bytes = payload(
        5,
        (1, 5, 1),
        &[6.0, -2.0, 4.0, 2.0, -2.0],
        &[3.0],
        &[1.0, -1.0, 1.0, 1.0, -1.0],
        53,
        &|_, a| a,
    );
    parse_payload("rt-sap5-v1", &bytes).unwrap()
}

fn params_a(outer: (u32, u32), inner: (u32, u32), tw: &str, swap: bool, inner_a: usize) -> Params {
    Params {
        inner_a,
        ..params(outer, inner, tw, swap)
    }
}

fn plan_of(s: &SaSpec, p: Params) -> Option<(usize, usize)> {
    let map = sa_low::lane_map(s, p).unwrap();
    sa_low::pad_plan_of(s, p, &map.inner).map(|pl| (pl.s, pl.d))
}

/// A mutant case: name, spec, outer and inner `(k, mu)`, bundle, `inner_a`.
type Case = (
    &'static str,
    SaSpec,
    (u32, u32),
    (u32, u32),
    &'static str,
    usize,
);

const P_BUNDLES: [&str; 11] = [
    "p",
    "xp",
    "imchxp",
    "imchxLp",
    "imchxlp",
    "imchxgrp",
    "imchxlgrp",
    "imchxlgrdp",
    "imchxgrdp",
    "imchxgrvp",
    "imchxlgrvdp",
];

const V_BUNDLES: [&str; 10] = [
    "uv",
    "rv",
    "rw",
    "imchxgrv",
    "imchxgrvd",
    "imchxgrw",
    "imchxgrwd",
    "imchxlgrvd",
    "imchxlgrwd",
    "imchxgv",
];

// ---------------------------------------------------------------------------------------------
// 1. Exhaustive lanes.

/// Lever `p` (and every lever it is combined with in a measured row) on every lane of the p
/// specs, with `2^inner_a = 2^k_i` blocks so the pad plan is live (asserted), both erase paths
/// (SpinSwap on / off), two narrow keep splits, `RT_KEYS` outcome keys.
#[test]
#[ignore = "slow; run in release through heavy.sh"]
fn exhaustive_p_small_specs() {
    let specs = [
        ("N=5 B=11 d0", spec_p3(true)),
        ("N=5 B=11 d1", spec_p3(false)),
        ("N=5 B=11 generic", spec_p3_generic()),
        ("N=5 B=5", spec_p5()),
    ];
    let keeps: [(u32, u32); 2] = [(2, 2), (3, 1)];
    let mut total = 0;
    for (name, s) in &specs {
        for &(mu_o, mu_i) in &keeps {
            let outer = (ko(s), mu_o);
            let inner = (ki(s), mu_i);
            for tw in P_BUNDLES {
                for swap in [true, false] {
                    let p = params_a(outer, inner, tw, swap, ki(s) as usize);
                    let plan = plan_of(s, p);
                    assert!(
                        plan.is_some() || !tw.contains('p'),
                        "{name} {tw}: no pad plan"
                    );
                    let (lm, ops) = build(s, p);
                    let t0 = std::time::Instant::now();
                    let n = exhaustive(s, &lm, &ops, keys()).unwrap_or_else(|e| {
                        panic!("{name} {tw} swap {swap} {outer:?} {inner:?}: {e}")
                    });
                    println!(
                        "  {name} {tw} swap {swap} {outer:?} {inner:?} plan {plan:?}: {n} lane runs pass ({:.1} s)",
                        t0.elapsed().as_secs_f64()
                    );
                    total += n;
                }
            }
        }
    }
    println!("exhaustive p: {total} lane runs, 0 failures");
}

/// The split one-hot `v` / `w` and their combinations, on every lane, at QROAM block counts
/// `inner_a` 1 and 2, on the N = 5, the ragged N = 4 (R = 2, B = 3) and the B = 5 specs.
#[test]
#[ignore = "slow; run in release through heavy.sh"]
fn exhaustive_split_onehot_small_specs() {
    let specs = [
        ("N=5", spec5(30, &|_, a| a), (2u32, 3u32)),
        ("N=5 generic", spec5_generic(), (2, 3)),
        ("N=4 R=2 B=3", spec4(40), (2, 3)),
        ("N=5 B=5", spec_p5(), (3, 2)),
    ];
    let mut total = 0;
    for (name, s, inner) in &specs {
        for tw in V_BUNDLES {
            for ia in [1usize, 2] {
                for swap in [true, false] {
                    let outer = (3, 3);
                    let p = params_a(outer, *inner, tw, swap, ia);
                    let (lm, ops) = build(s, p);
                    let t0 = std::time::Instant::now();
                    let n = exhaustive(s, &lm, &ops, keys())
                        .unwrap_or_else(|e| panic!("{name} {tw} ia {ia} swap {swap}: {e}"));
                    println!(
                        "  {name} {tw} ia {ia} swap {swap} {outer:?} {inner:?}: {n} lane runs pass ({:.1} s)",
                        t0.elapsed().as_secs_f64()
                    );
                    total += n;
                }
            }
        }
    }
    println!("exhaustive v/w: {total} lane runs, 0 failures");
}

// ---------------------------------------------------------------------------------------------
// 2. Mutants.

/// The base mutants (`mutants`) plus the classes the split one-hot and `p` add (module docs). With
/// `all_gates`, also every CCX and every CX of the stream dropped one at a time.
fn mutants_ext(ops: &[Op], all_gates: bool) -> Mutants {
    let mut m = mutants(ops);
    let regs = registers(ops);
    // Toffolis into an angle register: the split one-hot's group corrections.
    let corr: Vec<usize> = (0..ops.len())
        .filter(|&i| ops[i].kind == K::CCX && regs.contains_key(&ops[i].q_target))
        .collect();
    let mut ctl: BTreeSet<u32> = BTreeSet::new();
    for &i in &corr {
        let o = ops[i];
        ctl.insert(o.q_control1);
        ctl.insert(o.q_control2);
        m.drop(ops, i, "group correction Toffoli dropped");
        let reg = &regs[&o.q_target];
        if reg.len() > 1 {
            let k = reg.iter().position(|&q| q == o.q_target).unwrap();
            let mut r = o;
            r.q_target = reg[(k + 1) % reg.len()];
            m.replace(ops, i, r, "group correction into the wrong angle bit");
        }
    }
    // Writes into a correction control (the parity scratch, the group bits).
    for i in 0..ops.len() {
        if matches!(ops[i].kind, K::CX | K::CCX) && ctl.contains(&ops[i].q_target) {
            m.drop(ops, i, "write into a correction control dropped");
        }
    }
    if all_gates {
        for i in 0..ops.len() {
            match ops[i].kind {
                K::CCX => m.drop(ops, i, "any CCX dropped"),
                K::CX => m.drop(ops, i, "any CX dropped"),
                _ => {}
            }
        }
    }
    m
}

fn judge_list(
    s: &SaSpec,
    p: Params,
    lm: &[u8],
    list: &[(String, Vec<Op>)],
    k: usize,
) -> (usize, usize, Vec<(String, Vec<Op>)>) {
    let (mut h, mut x, mut eq) = (0, 0, Vec::new());
    for (why, m) in list {
        match harness(s, p, lm, m, k) {
            Err(_) => h += 1,
            Ok(_) => match harness(s, p, lm, m, 1 << 18) {
                Err(_) => x += 1,
                Ok(_) => eq.push((why.clone(), m.clone())),
            },
        }
    }
    (h, x, eq)
}

/// Mutants of the `p` and `v` / `w` bundles at wide keeps (exact lane maps) judged by the
/// harness; each survivor is then rebuilt at narrow keeps on the generic spec where possible and
/// reported. Survivors are printed with their class.
#[test]
#[ignore = "slow; run in release through heavy.sh"]
fn mutants_p_and_split_onehot() {
    let cases: Vec<Case> = vec![
        (
            "N=5 B=11 d1",
            spec_p3(false),
            (4, 19),
            (4, 20),
            "imchxlgrp",
            4,
        ),
        (
            "N=5 B=11 d0",
            spec_p3(true),
            (4, 19),
            (4, 20),
            "imchxlgrp",
            4,
        ),
        (
            "N=5 B=11 d1",
            spec_p3(false),
            (4, 19),
            (4, 20),
            "imchxlgrdp",
            4,
        ),
        ("N=5 B=5", spec_p5(), (3, 19), (3, 20), "imchxp", 3),
        ("N=5", spec5(30, &|_, a| a), (3, 5), (2, 6), "imchxgrvd", 2),
        ("N=5", spec5(30, &|_, a| a), (3, 5), (2, 6), "imchxgrwd", 1),
        ("N=5", spec5(30, &|_, a| a), (3, 5), (2, 6), "imchxgrv", 1),
        (
            "N=5 B=11 d1",
            spec_p3(false),
            (4, 19),
            (4, 20),
            "imchxgrvd",
            2,
        ),
    ];
    let mut survivors = Vec::new();
    for (name, s, outer, inner, tw, ia) in &cases {
        let p = params_a(*outer, *inner, tw, true, *ia);
        let (lm, ops) = build(s, p);
        let good = harness(s, p, &lm, &ops, 1 << 13);
        assert!(good.is_ok(), "{name} {tw}: {:?}", good.err());
        let ms = mutants_ext(&ops, true);
        let (h, x, eq) = judge_list(s, p, &lm, &ms.list, 1 << 13);
        println!(
            "{name} {tw} ia {ia} plan {:?}: {} ops, {} mutants, {h} rejected by the harness (K = 8192), {x} more at K = 2^18, {} surviving",
            plan_of(s, p),
            ops.len(),
            ms.list.len(),
            eq.len()
        );
        let mut classes: HashMap<String, usize> = HashMap::new();
        for (why, _) in &eq {
            let c = why.split(" (op").next().unwrap().to_string();
            *classes.entry(c).or_default() += 1;
        }
        let mut cl: Vec<_> = classes.into_iter().collect();
        cl.sort();
        for (c, n) in &cl {
            println!("   surviving class: {c}: {n}");
        }
        for (why, _) in &eq {
            survivors.push(format!("{name} {tw}: {why}"));
        }
    }
    println!("{} survivors in total", survivors.len());
    for s in &survivors {
        println!("   surviving: {s}");
    }
}

/// The same mutant classes at narrow keeps on generic specs, judged by the exhaustive run alone
/// (every lane, 1 key): any survivor here changes no reachable lane.
#[test]
#[ignore = "slow; run in release through heavy.sh"]
fn mutants_p_and_split_onehot_exhaustive() {
    let cases: Vec<Case> = vec![
        (
            "N=5 B=11 generic",
            spec_p3_generic(),
            (4, 2),
            (4, 1),
            "imchxlgrp",
            4,
        ),
        (
            "N=5 generic",
            spec5_generic(),
            (3, 3),
            (2, 3),
            "imchxgrvd",
            2,
        ),
        (
            "N=5 generic",
            spec5_generic(),
            (3, 3),
            (2, 3),
            "imchxgrw",
            1,
        ),
    ];
    let mut survivors = Vec::new();
    for (name, s, outer, inner, tw, ia) in &cases {
        let p = params_a(*outer, *inner, tw, true, *ia);
        let (lm, ops) = build(s, p);
        exhaustive(s, &lm, &ops, 1).unwrap();
        // Only the classes `mutants_ext` adds to the base classes of `mutants`.
        let ms = mutants_ext(&ops, false);
        let list: Vec<_> = ms
            .list
            .iter()
            .filter(|(w, _)| {
                w.contains("group correction")
                    || w.contains("correction control")
                    || w.contains("fan-out")
                    || w.contains("one-hot write")
            })
            .collect();
        let mut rejected = 0;
        for (why, m) in &list {
            match exhaustive(s, &lm, m, 1) {
                Err(_) => rejected += 1,
                Ok(_) => survivors.push(format!("{name} {tw}: {why}")),
            }
        }
        println!(
            "{name} {tw} ia {ia} plan {:?}: {} mutants, {rejected} rejected exhaustively",
            plan_of(s, p),
            list.len()
        );
    }
    for s in &survivors {
        println!("   surviving: {s}");
    }
    println!("{} survivors", survivors.len());
}

// ---------------------------------------------------------------------------------------------
// 3. Lever p: cross-judging, and the op stream without p.

/// The circuit built for the padded lane map is rejected against the unpadded one, and vice
/// versa; with fewer than `2^k_i` blocks `p` changes nothing.
#[test]
fn p_cross_judged_and_inert_below_k_i() {
    for (name, s) in [
        ("d0", spec_p3(true)),
        ("d1", spec_p3(false)),
        ("B=5", spec_p5()),
    ] {
        for tw in ["imchxlgrp", "imchxp", "imchxgrvp"] {
            let k = ki(&s);
            let p = params_a((ko(&s), 19), (k, 20), tw, true, k as usize);
            assert!(plan_of(&s, p).is_some(), "{name} {tw}: no pad plan");
            let q = Params {
                tw: Tweaks::parse(&tw.replace('p', "")),
                ..p
            };
            let (lm_p, ops_p) = build(&s, p);
            let (lm_q, ops_q) = build(&s, q);
            let gp = harness(&s, p, &lm_p, &ops_p, 1 << 13);
            let gq = harness(&s, q, &lm_q, &ops_q, 1 << 13);
            assert!(gp.is_ok(), "{name} {tw} own p stream: {:?}", gp.err());
            assert!(gq.is_ok(), "{name} {tw} own plain stream: {:?}", gq.err());
            let a = harness(&s, p, &lm_q, &ops_p, 1 << 13);
            let b = harness(&s, q, &lm_p, &ops_q, 1 << 13);
            println!(
                "{name} {tw} plan {:?}: p-stream on the plain map: {}; plain stream on the padded map: {}",
                plan_of(&s, p),
                a.as_ref().err().map_or("ACCEPTED".into(), |e| e.chars().take(90).collect::<String>()),
                b.as_ref().err().map_or("ACCEPTED".into(), |e| e.chars().take(90).collect::<String>())
            );
            let same_map = lm_p == lm_q;
            let same_ops = ops_p == ops_q;
            println!("   lane maps equal: {same_map}; op streams equal: {same_ops}");
            if !same_map {
                assert!(
                    a.is_err() && b.is_err(),
                    "{name} {tw}: a cross-judged stream passed"
                );
            } else {
                assert!(!same_ops, "{name} {tw}: p changed nothing");
            }
            // Below 2^k_i blocks, p is off: the same op stream and lane map.
            let p2 = Params {
                inner_a: k as usize - 1,
                ..p
            };
            let q2 = Params {
                inner_a: k as usize - 1,
                ..q
            };
            assert_eq!(
                build(&s, p2),
                build(&s, q2),
                "{name} {tw}: p changed an ia 2 stream"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 4. FeMoco: pinned rows (release, heavy.sh).

/// On the FeMoco specs: the padded inner tables of lever `p` have exactly the unpadded tables'
/// counts (so the same rounded operator); the outer table is untouched.
#[test]
#[ignore = "pinned FeMoco specs; run in release"]
fn p_same_counts_on_femoco() {
    for id in ["reiher-sa-est-v1", "reiher-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = toff_mu(s, "imchxlgrp", None, Some(5));
        let q = toff_mu(s, "imchxlgr", None, Some(5));
        let mp = sa_low::lane_map(s, p).unwrap();
        let mq = sa_low::lane_map(s, q).unwrap();
        let plan = sa_low::pad_plan_of(s, p, &mp.inner);
        let items = s.b + 1;
        let mut differ = 0;
        for (i, (a, b)) in mp.inner.iter().zip(&mq.inner).enumerate() {
            assert_eq!(
                a.counts(items),
                b.counts(items),
                "{id}: table {i} counts differ"
            );
            if a.keep != b.keep || a.alt != b.alt {
                differ += 1;
            }
        }
        assert_eq!(mp.outer.keep, mq.outer.keep);
        assert_eq!(mp.outer.alt, mq.outer.alt);
        println!(
            "{id}: plan {plan:?}; {} inner tables, all with equal counts; {differ} laid out differently; outer identical",
            mp.inner.len()
        );
    }
}

/// Every pinned outcome-gated row: rebuilt, ops SHA-256 checked against
/// tests/sa_digests.rs, exact E[C_step] and its sd. `RT_ROWS` selects labels.
#[test]
#[ignore = "pinned FeMoco specs; run in release"]
fn exact_expectation_pinned_rows() {
    type Row = (
        &'static str,
        &'static str,
        &'static str,
        f64,
        &'static str,
        Option<usize>,
    );
    let rows: [Row; 6] = [
        (
            "E-R-lgr",
            "reiher-sa-est-v1",
            "47b89d6d008e8f876e994633b8951c6fc3ed046131b023e5b9bf92fa8ccb7934",
            9298.003158569336,
            "imchxlgr",
            None,
        ),
        (
            "E-R-lgrp5",
            "reiher-sa-est-v1",
            "dd08e3c70ad7ba0725408fa35b0ea228177a195e4e140bb86acf2b0ced734d12",
            9206.004516601562,
            "imchxlgrp",
            Some(5),
        ),
        (
            "E-L-vd-a4",
            "li-sa-est-v1",
            "01888055ecc61f100bf3f8c1d7b5f252c13d0ba7f3b7808f7b06c26de4c84d71",
            18205.000244140625,
            "imchxgrvd",
            Some(4),
        ),
        (
            "E-L-gr-a5",
            "li-sa-est-v1",
            "da11811ca8b5caf1aa223d12e3e5c8b1d642211c1c51ab03a8e67b6e601568ee",
            13340.99250793457,
            "imchxgr",
            Some(5),
        ),
        (
            "sR-vd-a3",
            "reiher-sa-v1",
            "9742e4deb0502aab31967881ddce8acd36918bbe21ca8e5c99a6928dcaf7013e",
            13623.020050048828,
            "imchxgrvd",
            Some(3),
        ),
        (
            "sL-vd-a4",
            "li-sa-v1",
            "05357cb9e216edc7af16a5b0db281e59ce319cc0e323f7459bd3fd82c71a2be3",
            18685.496,
            "imchxgrvd",
            Some(4),
        ),
    ];
    let want: Option<Vec<String>> = std::env::var("RT_ROWS")
        .ok()
        .map(|v| v.split(',').map(str::to_string).collect());
    let k = 524_288f64;
    for (label, id, sha, measured, tw, ia) in rows {
        if want.as_ref().is_some_and(|w| !w.iter().any(|x| x == label)) {
            continue;
        }
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let pp = toff_mu(s, tw, None, ia);
        let map = sa_low::lane_map(s, pp).unwrap();
        let mut b = Builder::new(s.system_qubits());
        b.declare_uniform(map.uniform_bits());
        let _ = sa_low::emit(s, &map, &mut b, pp);
        let ops = b.finish();
        let file = ops_file(&ops);
        let got = hex::encode(file.sha256);
        assert_eq!(got, sha, "{label}: not the measured circuit");
        let e = expect_of(s, &map.to_bytes(), &ops);
        let sd = (e.var / k).sqrt();
        println!(
            "{label}\t{id}\texact {:.4}\tfixed {:.1}\tgated events {}\tgated Toffoli ops {}\tsd/lane {:.3}\tsd(mean) {:.4}\tmeasured {measured:.3}\tdiff {:+.4}\tz {:+.2}",
            e.mean,
            e.fixed,
            e.events,
            e.toffoli_gated_ops,
            e.var.sqrt(),
            sd,
            measured - e.mean,
            if sd > 0.0 { (measured - e.mean) / sd } else { 0.0 }
        );
    }
}

/// The `mutants_p_and_split_onehot` classes on GENERIC (non-dyadic) specs at wide keeps, where
/// keep values and data words carry live bits everywhere: dyadic test data leaves many bits
/// constantly 0, which makes gates that act only on them dead. Survivors
/// are listed with the op they removed.
#[test]
#[ignore = "slow; run in release through heavy.sh"]
fn mutants_generic_wide() {
    let cases: Vec<Case> = vec![
        (
            "N=5 B=11 generic",
            spec_p3_generic(),
            (4, 24),
            (4, 24),
            "imchxlgrp",
            4,
        ),
        (
            "N=5 generic",
            spec5_generic(),
            (3, 24),
            (2, 24),
            "imchxgrvd",
            2,
        ),
        (
            "N=5 generic",
            spec5_generic(),
            (3, 24),
            (2, 24),
            "imchxgrwd",
            1,
        ),
    ];
    let mut survivors = Vec::new();
    for (name, s, outer, inner, tw, ia) in &cases {
        let p = params_a(*outer, *inner, tw, true, *ia);
        let (lm, ops) = build(s, p);
        let good = harness(s, p, &lm, &ops, 1 << 13);
        assert!(good.is_ok(), "{name} {tw}: {:?}", good.err());
        let ms = mutants_ext(&ops, true);
        let (h, x, eq) = judge_list(s, p, &lm, &ms.list, 1 << 13);
        println!(
            "{name} {tw} ia {ia} plan {:?}: {} ops, {} mutants, {h} rejected by the harness (K = 8192), {x} more at K = 2^18, {} surviving",
            plan_of(s, p),
            ops.len(),
            ms.list.len(),
            eq.len()
        );
        let mut classes: HashMap<String, usize> = HashMap::new();
        for (why, _) in &eq {
            let c = why.split(" (op").next().unwrap().to_string();
            *classes.entry(c).or_default() += 1;
        }
        let mut cl: Vec<_> = classes.into_iter().collect();
        cl.sort();
        for (c, n) in &cl {
            println!("   surviving class: {c}: {n}");
        }
        for (why, _) in &eq {
            let idx: Option<usize> = why
                .rsplit("op ")
                .next()
                .and_then(|t| {
                    t.trim_end_matches(|c: char| !c.is_ascii_digit())
                        .split(' ')
                        .next()
                })
                .and_then(|t| t.parse().ok());
            let desc = idx.map_or(String::new(), |i| {
                let o = ops[i];
                format!(
                    "{:?} c2 {} c1 {} t {} cond {}",
                    o.kind, o.q_control2, o.q_control1, o.q_target, o.c_condition
                )
            });
            survivors.push(format!("{name} {tw}: {why}: {desc}"));
        }
    }
    println!("{} survivors in total", survivors.len());
    for s in &survivors {
        println!("   surviving: {s}");
    }
}
