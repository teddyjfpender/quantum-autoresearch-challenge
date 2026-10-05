//! Lever-agnostic adversarial checks of `sa-toff` bundles. The file uses only the walk API
//! (`sa_low::lane_map`, `sa_low::emit`, `Params`, `Tweaks::parse`) and knows no lever: the
//! bundles under test come from the environment (`RT_BUNDLES` etc.), so the same checks apply to
//! any lever string. The helpers (small-spec payload writer, exhaustive lane enumerator,
//! structural mutant generator, exact expected-Toffoli analyzer) are those of
//! tests/adversarial_sa.rs, with a few more small specs.
//!
//! 1. `rt_exact_expectation` (`RT_RUNS=label=dir,...`): exact expected Toffolis per step of a run
//!    directory's `ops.bin` (outcome-gated ops weighted by their exact probability), fixed part,
//!    worst case.
//! 2. `rt_exhaustive` (`RT_BUNDLES=tw@ia@oa,...`, `RT_SPECS`, `RT_LEVERS`): every lane of several
//!    small generic specs at narrow keeps, both SELECT variants, `RT_KEYS` `Hmr` keys, through the
//!    trusted simulator; lever liveness reported.
//! 3. `rt_mutants` (`RT_BUNDLES`, `RT_MUT_SPECS`): structural mutants (every CCX / CX dropped,
//!    outcome-conditioned ops dropped, condition blocks inverted, ...), each judged exhaustively.
//!    `RT_ALL_GATES=0` leaves out the per-gate drops; `RT_MUT_SAMPLE=n` judges a sample.
//! 4. `rt_dump` (`RT_DUMP=tw@ia@oa:spec:lo:hi`): prints a range of a bundle's ops (diagnostic).
#![cfg(feature = "walk")]
#![allow(dead_code, clippy::too_many_lines, clippy::type_complexity)]
use femoco_walk::circuit::{read_ops, write_ops, Builder, Op, OperationType as K, OpsFile, NONE};
use femoco_walk::fiat_shamir::Lane;
use femoco_walk::lanemap::{self, LaneMap};
use femoco_walk::score::{evaluate, Evaluation, FamilyOut, Inputs};
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
    let dir = std::env::temp_dir().join(format!("femoco-rt-levers-{}-{n}", std::process::id()));
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
        let key: [u8; 32] =
            Sha256::digest(format!("adversarial-levers hmr key {k}").as_bytes()).into();
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
        let out = validate::run_nested(&ctx, &n, &lanes);
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
    /// Charged weight of every outcome-gated event (the worst case is `fixed + gated_weight`).
    gated_weight: f64,
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
        gated_weight: gated.iter().map(|&i| ev[i].1).sum(),
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
/// The base mutants (`mutants`) plus the classes the split one-hot and `p` add (tests/adversarial_sa.rs). With
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

// ---------------------------------------------------------------------------------------------
// More small specs (generic weights).

/// N = 4, (R, B, C) = (2, 3, 1), generic weights.
fn spec4_generic() -> SaSpec {
    let bytes = payload(
        4,
        (2, 3, 1),
        &[1.13, -2.07, 0.93, 3.71],
        &[1.31, -3.77],
        &[1.09, -0.71, 1.37, 2.11, -0.83, 0.57],
        41,
        &|_, a| a,
    );
    parse_payload("rt-mq4g-v1", &bytes).unwrap()
}

/// N = 4, (R, B, C) = (6, 3, 1), generic weights: six square rows, so the folded split one-hot
/// (lever `f`) has whole-row classes at G = 2 and 3.
fn spec6_generic() -> SaSpec {
    let wb = [1.31, -2.17, 0.77, 1.93, -1.11, 0.59];
    let pat = [1.07, -0.61, 0.83];
    let w: Vec<f64> = (0..18)
        .map(|i| pat[i % 3] * (1.0 + 0.11 * (i / 3) as f64))
        .collect();
    let bytes = payload(4, (6, 3, 1), &[2.3, -1.7, 0.9, 1.4], &wb, &w, 61, &|_, a| a);
    parse_payload("rt-mq6g-v1", &bytes).unwrap()
}

/// N = 5, (R, B, C) = (3, 5, 2), generic weights: 6 square items in 8 inner buckets, 11 outer
/// items, so group counts up to 7 still split.
fn spec35_generic() -> SaSpec {
    let wb: Vec<f64> = (0..6).map(|c| 1.7 - 0.13 * c as f64).collect();
    let pat = [1.9, -0.7, 0.55, 1.15, -0.95];
    let w: Vec<f64> = (0..30)
        .map(|i| pat[i % 5] * (1.0 + 0.07 * (i / 5) as f64))
        .collect();
    let bytes = payload(
        5,
        (3, 5, 2),
        &[2.9, -1.21, 1.73, 0.67, -1.49],
        &wb,
        &w,
        71,
        &|_, a| a,
    );
    parse_payload("rt-mq35g-v1", &bytes).unwrap()
}

/// N = 7 (odd), (R, B, C) = (5, 6, 3), generic weights: 22 outer items (odd item
/// starts for the aligned item one-hot `V`, five square rows so G = 2..5 fold classes have whole
/// rows plus a remainder), 7 inner items in 8 buckets.
fn spec7_generic() -> SaSpec {
    let wb: Vec<f64> = (0..15).map(|c| 1.9 - 0.07 * c as f64).collect();
    let pat = [1.3, -0.77, 0.61, 1.11, -0.93, 0.52];
    let w: Vec<f64> = (0..90)
        .map(|i| pat[i % 6] * (1.0 + 0.05 * (i / 6) as f64))
        .collect();
    let bytes = payload(
        7,
        (5, 6, 3),
        &[2.7, -1.19, 1.61, 0.73, -1.37, 0.97, -0.59],
        &wb,
        &w,
        83,
        &|_, a| a,
    );
    parse_payload("rt-w2s7g-v1", &bytes).unwrap()
}

/// N = 5, (R, B, C) = (4, 3, 2), generic weights: 13 outer items (an odd count, so the
/// aligned layout's last slot has one in-range item), 4 inner items in 4 buckets (no padding).
fn spec43_generic() -> SaSpec {
    let wb: Vec<f64> = (0..8).map(|c| 1.4 + 0.09 * c as f64).collect();
    let pat = [0.83, -1.21, 0.67];
    let w: Vec<f64> = (0..24)
        .map(|i| pat[i % 3] * (1.0 + 0.13 * (i / 3) as f64))
        .collect();
    let bytes = payload(
        5,
        (4, 3, 2),
        &[1.9, -2.3, 0.71, 1.27, -0.89],
        &wb,
        &w,
        97,
        &|_, a| a,
    );
    parse_payload("rt-w2s43g-v1", &bytes).unwrap()
}

/// N = 4, (R, B, C) = (10, 3, 1), generic weights: ten square rows, so G = 5 folds
/// have two whole-row classes (levers that need whole classes at G = 5 are live).
fn spec10_generic() -> SaSpec {
    let wb: Vec<f64> = (0..10).map(|c| 1.2 + 0.11 * c as f64).collect();
    let pat = [0.91, -1.07, 0.63];
    let w: Vec<f64> = (0..30)
        .map(|i| pat[i % 3] * (1.0 + 0.04 * (i / 3) as f64))
        .collect();
    let bytes = payload(
        4,
        (10, 3, 1),
        &[2.1, -1.3, 0.8, 1.6],
        &wb,
        &w,
        101,
        &|_, a| a,
    );
    parse_payload("rt-w2s10g-v1", &bytes).unwrap()
}

fn rt_spec(name: &str) -> SaSpec {
    match name {
        "s5g" => spec5_generic(),
        "s4g" => spec4_generic(),
        "s6g" => spec6_generic(),
        "s35g" => spec35_generic(),
        "s11g" => spec_p3_generic(),
        "s7g" => spec7_generic(),
        "s43g" => spec43_generic(),
        "s10g" => spec10_generic(),
        other => panic!("unknown small spec {other}"),
    }
}

/// `tw@ia@oa` items of `RT_BUNDLES`.
fn rt_bundles() -> Vec<(String, usize, usize)> {
    std::env::var("RT_BUNDLES")
        .expect("RT_BUNDLES=tw@ia@oa,...")
        .split(',')
        .filter(|x| !x.is_empty())
        .map(|x| {
            let v: Vec<&str> = x.split('@').collect();
            (
                v[0].to_string(),
                v.get(1).map_or(1, |a| a.parse().unwrap()),
                v.get(2).map_or(1, |a| a.parse().unwrap()),
            )
        })
        .collect()
}

fn rt_params(s: &SaSpec, tw: &str, ia: usize, oa: usize, keep: (u32, u32), swap: bool) -> Params {
    Params {
        inner_a: ia.min(ki(s) as usize).max(1),
        outer_a: oa.min(ko(s) as usize).max(1),
        ..params((ko(s), keep.0), (ki(s), keep.1), tw, swap)
    }
}

/// Build, or `None` when the walk refuses the bundle on this spec (a panic).
fn try_build(s: &SaSpec, p: Params) -> Option<(Vec<u8>, Vec<Op>)> {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(s, p))).ok();
    std::panic::set_hook(prev);
    r
}

/// The bundles of `RT_BUNDLES` on every lane of the small specs.
#[test]
#[ignore = "slow; run in release through heavy.sh with RT_BUNDLES"]
fn rt_exhaustive() {
    let specs: Vec<String> = std::env::var("RT_SPECS")
        .unwrap_or_else(|_| "s5g,s4g,s6g,s35g,s11g".into())
        .split(',')
        .map(str::to_string)
        .collect();
    let keeps: [(u32, u32); 2] = [(2, 1), (1, 2)];
    let (mut total, mut na, mut fails) = (0usize, 0usize, Vec::new());
    for (tw, ia, oa) in rt_bundles() {
        for name in &specs {
            let s = rt_spec(name);
            // Is each lever of `RT_LEVERS` live on this spec? (ops differ with the letter removed)
            let levers = std::env::var("RT_LEVERS").unwrap_or_default();
            for l in levers.chars().filter(|&l| tw.contains(l)) {
                let p = rt_params(&s, &tw, ia, oa, (2, 1), true);
                let without: String = tw.chars().filter(|&c| c != l).collect();
                let q = rt_params(&s, &without, ia, oa, (2, 1), true);
                let live = match (try_build(&s, p), try_build(&s, q)) {
                    (Some((_, a)), Some((_, b))) => {
                        if a == b {
                            "INERT"
                        } else {
                            "live"
                        }
                    }
                    (Some(_), None) => "live (bundle without it refused)",
                    _ => "n/a",
                };
                println!("  lever {l} in {tw} on {name}: {live}");
            }
            for &keep in &keeps {
                for swap in [true, false] {
                    if name == "s11g" && (keep != (2, 1) || !swap) {
                        continue;
                    }
                    let p = rt_params(&s, &tw, ia, oa, keep, swap);
                    let Some((lm, ops)) = try_build(&s, p) else {
                        println!("  {tw} ia {ia} oa {oa} {name} keep {keep:?} swap {swap}: n/a (walk refuses)");
                        na += 1;
                        continue;
                    };
                    match exhaustive(&s, &lm, &ops, keys()) {
                        Ok(n) => {
                            println!(
                                "  {tw} ia {} oa {} {name} keep {keep:?} swap {swap}: {} ops, {n} lane runs pass",
                                p.inner_a,
                                p.outer_a,
                                ops.len()
                            );
                            total += n;
                        }
                        Err(e) => {
                            println!(
                                "  FAIL {tw} ia {ia} oa {oa} {name} keep {keep:?} swap {swap}: {e}"
                            );
                            fails.push(format!("{tw} {name} {keep:?} {swap}: {e}"));
                        }
                    }
                }
            }
        }
    }
    println!(
        "rt_exhaustive: {total} lane runs pass, {} failing builds, {na} n/a",
        fails.len()
    );
    assert!(fails.is_empty(), "{fails:#?}");
}

/// Ops executed under an outcome condition (a `PushCondition` block or the op's own condition)
/// that `mutants` does not already drop, each dropped; every `Hmr` dropped; every phase op
/// (`Z`, `S`, `Sdg`, `CZ`) dropped.
fn mutants_cond(ops: &[Op], m: &mut Mutants) {
    let mut depth = 0usize;
    for i in 0..ops.len() {
        let o = ops[i];
        match o.kind {
            K::PushCondition => depth += 1,
            K::PopCondition => depth -= 1,
            K::Segment | K::DebugPrint | K::Register | K::AppendToRegister => {}
            K::Hmr => m.drop(ops, i, "Hmr dropped"),
            k => {
                let covered = (depth > 0 && matches!(k, K::CCX | K::CCZ | K::CZ | K::Z))
                    || (o.c_condition != NONE && matches!(k, K::CZ | K::Z));
                if (depth > 0 || o.c_condition != NONE) && !covered {
                    m.drop(ops, i, "outcome-conditioned op dropped");
                } else if matches!(k, K::S | K::Sdg | K::Z | K::CZ) && !covered {
                    m.drop(ops, i, "phase op dropped");
                }
            }
        }
    }
}

/// Mutants of each bundle at narrow keeps on generic specs, judged exhaustively.
#[test]
#[ignore = "slow; run in release through heavy.sh with RT_BUNDLES"]
fn rt_mutants() {
    let specs: Vec<String> = std::env::var("RT_MUT_SPECS")
        .unwrap_or_else(|_| "s5g,s6g".into())
        .split(',')
        .map(str::to_string)
        .collect();
    let mut all_surv = Vec::new();
    for (tw, ia, oa) in rt_bundles() {
        for name in &specs {
            let s = rt_spec(name);
            let p = rt_params(&s, &tw, ia, oa, (2, 1), true);
            let Some((lm, ops)) = try_build(&s, p) else {
                println!("{tw} {name}: n/a");
                continue;
            };
            exhaustive(&s, &lm, &ops, 2).expect("the unmutated circuit must pass");
            let all_gates = std::env::var("RT_ALL_GATES").map_or(true, |v| v != "0");
            let mut ms = mutants_ext(&ops, all_gates);
            mutants_cond(&ops, &mut ms);
            // `RT_MUT_SAMPLE=n` keeps a deterministic sample of n mutants (every
            // class represented in proportion), to bound a bundle's run time.
            if let Some(n) = std::env::var("RT_MUT_SAMPLE")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
            {
                let total = ms.list.len();
                if total > n {
                    let mut x = 0x9E37_79B9_7F4A_7C15u64;
                    let mut keyed: Vec<(u64, (String, Vec<Op>))> = ms
                        .list
                        .drain(..)
                        .map(|m| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            (x, m)
                        })
                        .collect();
                    keyed.sort_by_key(|k| k.0);
                    ms.list = keyed.into_iter().take(n).map(|k| k.1).collect();
                    println!("{tw} {name}: sampled {n} of {total} mutants");
                }
            }
            let t0 = std::time::Instant::now();
            let mut rejected = 0usize;
            let mut surv: Vec<String> = Vec::new();
            for (why, m) in &ms.list {
                if exhaustive(&s, &lm, m, 1).is_err() || exhaustive(&s, &lm, m, 4).is_err() {
                    rejected += 1;
                } else {
                    surv.push(why.clone());
                }
            }
            let mut classes: HashMap<String, usize> = HashMap::new();
            for why in &surv {
                *classes
                    .entry(why.split(" (op").next().unwrap().to_string())
                    .or_default() += 1;
            }
            let mut cl: Vec<_> = classes.into_iter().collect();
            cl.sort();
            println!(
                "{tw} ia {} oa {} {name}: {} ops, {} mutants, {rejected} rejected exhaustively, {} survive ({:.1} s)",
                p.inner_a,
                p.outer_a,
                ops.len(),
                ms.list.len(),
                surv.len(),
                t0.elapsed().as_secs_f64()
            );
            for (c, n) in &cl {
                println!("   surviving class: {c}: {n}");
            }
            for w in &surv {
                let i: usize = w
                    .rsplit("op ")
                    .next()
                    .and_then(|x| x.split(|c: char| !c.is_ascii_digit()).next())
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(usize::MAX);
                let desc = ops.get(i).map_or(String::new(), |o| {
                    // Place the op in the step (last Segment code before it) and say
                    // whether it is a member of a CX-CCX-CX controlled-swap triplet.
                    let seg = ops[..i]
                        .iter()
                        .rev()
                        .find(|x| x.kind == K::Segment)
                        .map_or(-1, |x| x.r_target as i64);
                    let swap = |j: usize| {
                        j >= 1
                            && j + 1 < ops.len()
                            && ops[j].kind == K::CCX
                            && ops[j - 1].kind == K::CX
                            && ops[j + 1].kind == K::CX
                            && ops[j - 1] == ops[j + 1]
                    };
                    let in_swap = swap(i) || swap(i + 1) || (i >= 1 && swap(i - 1));
                    format!(
                        "{:?} t{} c1 {} c2 {} cond {} seg {seg} swap-triplet {in_swap}",
                        o.kind, o.q_target, o.q_control1, o.q_control2, o.c_condition
                    )
                });
                all_surv.push(format!("{tw} {name}: {w}: {desc}"));
            }
        }
    }
    for s in &all_surv {
        println!("   surviving: {s}");
    }
    println!("rt_mutants: {} survivors in total", all_surv.len());
}

// ---------------------------------------------------------------------------------------------
// Exact expectation of a built run directory.

/// `RT_RUNS=label=dir,...`: exact `E[C_step]`, fixed part and worst case of each run's ops.
#[test]
#[ignore = "pinned FeMoco specs; run in release with RT_RUNS"]
fn rt_exact_expectation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for item in std::env::var("RT_RUNS")
        .expect("RT_RUNS=label=dir,...")
        .split(',')
        .filter(|x| !x.is_empty())
    {
        let (label, dir) = item.split_once('=').unwrap();
        let dir = std::path::Path::new(dir);
        let fam: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("family.out.json")).unwrap()).unwrap();
        let id = fam["spec"].as_str().unwrap();
        let boxed = femoco_walk::spec::load(root, id).unwrap();
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let lm = std::fs::read(dir.join("lanemap.bin")).unwrap();
        let file = read_ops(&dir.join("ops.bin")).unwrap();
        let e = expect_of(s, &lm, &file.ops);
        let worst = e.fixed + e.gated_weight;
        println!(
            "{label}\t{id}\tops {}\texact {:.4}\tfixed {:.1}\tgated_events {}\tgated_toffoli_ops {}\tworst {:.1}\tsd_lane {:.4}\tops_sha {}",
            file.ops.len(),
            e.mean,
            e.fixed,
            e.events,
            e.toffoli_gated_ops,
            worst,
            e.var.sqrt(),
            hex::encode(file.sha256)
        );
    }
}

/// `RT_DUMP=tw@ia@oa:spec:lo:hi`: print ops `lo..hi` of a bundle's small-spec build, with the
/// last `Segment` before `lo` (to place a surviving mutant in the step).
#[test]
#[ignore = "diagnostic"]
fn rt_dump() {
    let v = std::env::var("RT_DUMP").expect("RT_DUMP=tw@ia@oa:spec:lo:hi");
    let parts: Vec<&str> = v.split(':').collect();
    let b: Vec<&str> = parts[0].split('@').collect();
    let s = rt_spec(parts[1]);
    let (lo, hi): (usize, usize) = (parts[2].parse().unwrap(), parts[3].parse().unwrap());
    let p = rt_params(
        &s,
        b[0],
        b[1].parse().unwrap(),
        b[2].parse().unwrap(),
        (2, 1),
        true,
    );
    let (_, ops) = build(&s, p);
    let seg = ops[..lo]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, o)| o.kind == K::Segment)
        .map(|(i, o)| (i, o.r_target));
    println!("ops {} ; last segment before {lo}: {seg:?}", ops.len());
    for (i, o) in ops.iter().enumerate().take(hi.min(ops.len())).skip(lo) {
        println!(
            "{i}\t{:?}\tt {}\tc1 {}\tc2 {}\tct {}\tcond {}\tr {}",
            o.kind,
            o.q_target as i64,
            o.q_control1 as i64,
            o.q_control2 as i64,
            o.c_target as i64,
            o.c_condition as i64,
            o.r_target as i64
        );
    }
}
