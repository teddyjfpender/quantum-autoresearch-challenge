//! The reflection check (`eval_circuit --reflection-check`, `sim::reflection`; spec/SAMPLING.md
//! section 4, spec/DESIGN.md section 16.6).
//!
//! - A tiny nested circuit written to the check's discipline is certified, and a dense
//!   simulation of every lane confirms what the certificate says.
//! - One mutant per rule is refused with that rule's reason; where the mutant really breaks
//!   Hermiticity the dense simulation confirms it.
//! - A step that is Hermitian but encodes a different operator is certified: the certificate is
//!   about Hermiticity, not about the operator.
//! - The three operator identities the normal form uses hold for dense matrices.
//! - Soundness fuzz: whenever the check certifies a random circuit, a dense simulation of every
//!   lane finds registers restored, an outcome-independent lane operator, and
//!   `L(g, a, b)^dagger = L(g, b, a)`. The converse is never asserted.
//! - With the `walk` feature: the published construction on a 3-orbital spec is certified; a
//!   rotation whose compute and uncompute disagree on one uniform value passes the sampled
//!   check at a small lane count and is refused by this one; and `score.json` changes only by
//!   the keys the flag adds.
#![allow(clippy::needless_range_loop)]
use femoco_walk::circuit::{
    Bit, Builder, Op, OperationType as K, Qubit, Reg, NONE, SEG_INNER_BEGIN, SEG_INNER_END,
    SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE,
};
use femoco_walk::sim::reflection::{certify, probe, Budget, Report};
use femoco_walk::sim::{compile_sa, gaussian, Inner, Layout, TrackerFactory};
use std::collections::HashMap;

// ---- Dense matrices on the system register (real: X, Z and Givens are real) ----

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

fn dist(a: &M, b: &M) -> f64 {
    a.iter()
        .zip(b)
        .flat_map(|(r, q)| r.iter().zip(q).map(|(x, y)| (x - y).abs()))
        .fold(0.0, f64::max)
}

/// Annihilator of mode `j` in the Jordan-Wigner order over `n` modes (bit `j` of the basis
/// index is mode `j`'s occupation).
fn ann(j: usize, n: usize) -> M {
    let d = 1 << n;
    let mut a = vec![vec![0.0; d]; d];
    for x in 0..d {
        if x >> j & 1 == 1 {
            let odd = (x & ((1 << j) - 1)).count_ones() % 2 == 1;
            a[x ^ (1 << j)][x] = if odd { -1.0 } else { 1.0 };
        }
    }
    a
}

/// The generator `K = a+_q a_p - a+_p a_q` of `G_{p,q}` (spec/DESIGN.md section 15.1).
fn generator(p: usize, q: usize, n: usize) -> M {
    let (ap, aq) = (ann(p, n), ann(q, n));
    add(&mul(&t(&aq), &ap), &mul(&t(&ap), &aq), -1.0)
}

/// `G_{p,q}(theta) = exp(theta K)` in closed form: `K^3 = -K`, so the series sums to
/// `1 + sin(theta) K + (1 - cos(theta)) K^2`.
fn givens(p: usize, q: usize, theta: f64, n: usize) -> M {
    let k = generator(p, q, n);
    let k2 = mul(&k, &k);
    add(&add(&eye(1 << n), &k, theta.sin()), &k2, 1.0 - theta.cos())
}

/// The same by its Taylor series.
fn givens_series(p: usize, q: usize, theta: f64, n: usize) -> M {
    let k = generator(p, q, n);
    let mut out = eye(1 << n);
    let mut term = eye(1 << n);
    for i in 1..60 {
        term = mul(&term, &k);
        for row in &mut term {
            for x in row {
                *x *= theta / f64::from(i);
            }
        }
        out = add(&out, &term, 1.0);
    }
    out
}

fn pauli_z(q: usize, n: usize) -> M {
    let mut z = eye(1 << n);
    for (x, row) in z.iter_mut().enumerate() {
        if x >> q & 1 == 1 {
            row[x] = -1.0;
        }
    }
    z
}

fn pauli_x(q: usize, n: usize) -> M {
    let d = 1 << n;
    let mut m = vec![vec![0.0; d]; d];
    for x in 0..d {
        m[x ^ (1 << q)][x] = 1.0;
    }
    m
}

/// The `SpinSwap` layer `F = prod_p G_{2p, 2p+1}(pi / 2)`.
fn f_layer(n: usize, dagger: bool) -> M {
    let theta = std::f64::consts::FRAC_PI_2 * if dagger { -1.0 } else { 1.0 };
    (0..n / 2).fold(eye(1 << n), |m, p| {
        mul(&givens(2 * p, 2 * p + 1, theta, n), &m)
    })
}

/// (9) The identities of the normal form, on four modes.
#[test]
fn normal_form_identities_hold_for_dense_matrices() {
    let n = 4;
    for (p, q) in [(0, 1), (0, 2), (1, 3), (0, 3), (2, 3)] {
        for theta in [0.3, 1.1, -2.0, std::f64::consts::FRAC_PI_2] {
            let g = givens(p, q, theta, n);
            assert!(dist(&g, &givens_series(p, q, theta, n)) < 1e-12);
            let minus = givens(p, q, -theta, n);
            assert!(dist(&mul(&g, &minus), &eye(1 << n)) < 1e-12);
            assert!(dist(&t(&g), &minus) < 1e-12);
            for k in 0..n {
                let z = pauli_z(k, n);
                let conj = mul(&mul(&z, &g), &z);
                // Z_p G(t) Z_p = G(-t), the same for Z_q; every other Z commutes with G.
                let want = if k == p || k == q { &minus } else { &g };
                assert!(dist(&conj, want) < 1e-12, "({p}, {q}), Z_{k}");
            }
        }
    }
    for dagger in [false, true] {
        let f = f_layer(n, dagger);
        for k in 0..n / 2 {
            let (a, b) = (pauli_z(2 * k, n), pauli_z(2 * k + 1, n));
            // F Z_2k F^dagger = Z_2k+1 and F Z_2k+1 F^dagger = Z_2k, for F and for F^dagger.
            assert!(dist(&mul(&mul(&f, &a), &t(&f)), &b) < 1e-12);
            assert!(dist(&mul(&mul(&f, &b), &t(&f)), &a) < 1e-12);
        }
    }
    // F is not its own inverse, so the two directions are different ops.
    assert!(dist(&f_layer(n, false), &f_layer(n, true)) > 0.5);
    // X Z = -Z X on one qubit.
    let (x, z) = (pauli_x(1, n), pauli_z(1, n));
    let zero = vec![vec![0.0; 1 << n]; 1 << n];
    assert!(dist(&add(&mul(&x, &z), &mul(&z, &x), 1.0), &zero) < 1e-12);
}

/// The operator of a normal form: `(-1)^sign Z^frame (last of list) ... (first of list)`.
fn dense_gates(n: usize, beta: u32, list: &[probe::Gate]) -> M {
    let unit = 2.0 * std::f64::consts::PI / f64::from(1u32 << beta);
    list.iter().fold(eye(1 << n), |m, g| {
        let g = match *g {
            probe::Gate::X(q) => pauli_x(q, n),
            probe::Gate::Z(q) => pauli_z(q, n),
            probe::Gate::G { p, q, angle, minus } => givens(
                p,
                q,
                unit * angle as f64 * if minus { -1.0 } else { 1.0 },
                n,
            ),
            probe::Gate::F { dagger } => f_layer(n, dagger),
        };
        mul(&g, &m)
    })
}

fn dense_normal(n: usize, beta: u32, f: &probe::Normal) -> M {
    let mut m = dense_gates(n, beta, &f.list);
    for q in (0..n).filter(|&q| f.frame[q]) {
        m = mul(&pauli_z(q, n), &m);
    }
    if f.sign {
        m = add(&vec![vec![0.0; 1 << n]; 1 << n], &m, -1.0);
    }
    m
}

/// (9) The normal form itself and the adjoint the check builds, against dense matrices: for
/// random words of system ops, the normal form is the word's operator, it holds no `Z`, and the
/// adjoint's normal form is the operator's transpose.
#[test]
fn normal_form_and_adjoint_equal_the_dense_operator() {
    let (n, beta) = (4, 5);
    let mut r = Rng(0xD1B5_4A32_D192_ED03);
    for round in 0..400 {
        let word: Vec<probe::Gate> = (0..1 + r.below(12))
            .map(|_| match r.below(6) {
                0 => probe::Gate::X(r.below(n)),
                1 | 2 => probe::Gate::Z(r.below(n)),
                3 | 4 => {
                    let p = r.below(n - 1);
                    probe::Gate::G {
                        p,
                        q: p + 1 + r.below(n - 1 - p),
                        angle: r.next() % 32,
                        minus: r.chance(50),
                    }
                }
                _ => probe::Gate::F {
                    dagger: r.chance(50),
                },
            })
            .collect();
        let want = dense_gates(n, beta, &word);
        let (normal, adjoint) = probe::normal(n, &word);
        assert!(normal
            .list
            .iter()
            .chain(&adjoint.list)
            .all(|g| !matches!(g, probe::Gate::Z(_))));
        let got = dense_normal(n, beta, &normal);
        assert!(dist(&got, &want) < 1e-9, "round {round}: {word:?}");
        let got = dense_normal(n, beta, &adjoint);
        assert!(
            dist(&got, &t(&want)) < 1e-9,
            "round {round}: adjoint of {word:?}"
        );
    }
}

// ---- A dense simulation of every lane, written from spec/DESIGN.md sections 5, 6 and 16 ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Act {
    X(usize),
    Z(usize),
    G(usize, usize, u64),
    F(bool),
}

struct World<'a> {
    ops: &'a [Op],
    registers: &'a [Vec<u32>],
    l: Layout,
    inner: Inner,
    beta: u32,
    cache: HashMap<Vec<Act>, M>,
    size: std::cell::Cell<Option<(usize, usize)>>,
}

/// One lane's run: its sign and its system ops in time order.
type Lane = (bool, Vec<Act>);

impl World<'_> {
    fn hmr_count(&self) -> usize {
        self.ops.iter().filter(|o| o.kind == K::Hmr).count()
    }

    /// Qubits and classical bits the stream names.
    fn sizes(&self) -> (usize, usize) {
        if let Some(s) = self.size.get() {
            return s;
        }
        let first = self.l.first_ancilla() as usize;
        let nq = self
            .ops
            .iter()
            .flat_map(Op::qubits)
            .chain(self.registers.iter().flatten().copied())
            .max()
            .map_or(first, |q| (q as usize + 1).max(first));
        let nb = self
            .ops
            .iter()
            .flat_map(|o| [o.c_target, o.c_condition])
            .filter(|&x| x != NONE)
            .max()
            .map_or(0, |x| x as usize + 1);
        self.size.set(Some((nq, nb)));
        (nq, nb)
    }

    /// Runs lane `(c, s)` with second-pass inner value `b` and the outcome record `outcomes`
    /// (one bit per `Hmr` op, executed or not). `Err`: a per-lane check of section 6 fails.
    fn run(
        &self,
        c: bool,
        s: u64,
        b: u64,
        outcomes: &dyn Fn(usize) -> bool,
    ) -> Result<Lane, String> {
        let l = &self.l;
        let first = l.first_ancilla() as usize;
        let (nq, nb) = self.sizes();
        let mut q = vec![false; nq];
        let mut bits = vec![false; nb];
        q[0] = c;
        let u0 = 1 + l.system;
        for k in 0..l.uniform as usize {
            q[u0 + k] = s >> k & 1 == 1;
        }
        let inner: Vec<usize> = self.inner.qubits(l).iter().map(|&x| x as usize).collect();
        let (mut sign, mut acts, mut stack, mut ordinal) = (false, Vec::new(), vec![true], 0);
        let sys = |x: u32| l.sys(x).map(|x| x as usize);
        for (i, op) in self.ops.iter().enumerate() {
            let base = stack.last().copied().unwrap_or(true);
            let on = base && (op.c_condition == NONE || bits[op.c_condition as usize]);
            let (tq, c1, c2) = (op.q_target, op.q_control1, op.q_control2);
            let held = |q: &[bool], x: u32| x == NONE || q[x as usize];
            match op.kind {
                K::Register | K::AppendToRegister | K::Segment | K::DebugPrint => {}
                K::PushCondition => stack.push(on),
                K::PopCondition => {
                    stack.pop();
                }
                K::BitInvert if on => bits[op.c_target as usize] ^= true,
                K::BitStore0 if on => bits[op.c_target as usize] = false,
                K::BitStore1 if on => bits[op.c_target as usize] = true,
                K::BitInvert | K::BitStore0 | K::BitStore1 => {}
                K::Neg => sign ^= on,
                K::X | K::CX | K::CCX => {
                    if on && held(&q, c1) && held(&q, c2) {
                        match sys(tq) {
                            Some(x) => acts.push(Act::X(x)),
                            None => q[tq as usize] ^= true,
                        }
                    }
                }
                K::Z | K::CZ | K::CCZ => {
                    let qs: Vec<u32> = op.qubits().collect();
                    let on_sys: Vec<usize> = qs.iter().filter_map(|&x| sys(x)).collect();
                    let all = qs.iter().all(|&x| sys(x).is_some() || q[x as usize]);
                    if on && all {
                        match on_sys[..] {
                            [] => sign ^= true,
                            [x] => acts.push(Act::Z(x)),
                            _ => return Err(format!("op {i}: two system qubits")),
                        }
                    }
                }
                K::Swap => {
                    if on {
                        q.swap(tq as usize, c1 as usize);
                    }
                }
                K::Hmr => {
                    let m = outcomes(ordinal);
                    ordinal += 1;
                    if on {
                        sign ^= m && q[tq as usize];
                        q[tq as usize] = false;
                        bits[op.c_target as usize] = m;
                    }
                }
                K::R => {
                    if base && q[tq as usize] {
                        return Err(format!("op {i}: dirty R"));
                    }
                }
                K::Givens => {
                    if on {
                        let reg = &self.registers[op.r_target as usize];
                        let a = reg
                            .iter()
                            .enumerate()
                            .fold(0u64, |a, (k, &x)| a | u64::from(q[x as usize]) << k);
                        let (Some(p), Some(x)) = (sys(c1), sys(tq)) else {
                            return Err(format!("op {i}: Givens operands"));
                        };
                        acts.push(Act::G(p, x, a % (1 << self.beta)));
                    }
                }
                K::SpinSwap | K::SpinSwapDg => {
                    if on && q[tq as usize] {
                        acts.push(Act::F(op.kind == K::SpinSwapDg));
                    }
                }
                K::Reflect => {
                    for (k, &x) in inner.iter().enumerate() {
                        if q[x] != (s >> (self.inner.lo as usize + k) & 1 == 1) {
                            return Err(format!("op {i}: inner register at the Reflect"));
                        }
                        q[x] = b >> k & 1 == 1;
                    }
                }
                K::S | K::Sdg => return Err(format!("op {i}: S is not simulated here")),
            }
        }
        // Registers: control and outer bits as they began, inner bits at b, ancillas 0.
        let mask = ((1u64 << self.inner.width) - 1) << self.inner.lo;
        let after = (s & !mask) | b << self.inner.lo;
        if q[0] != c || (0..l.uniform as usize).any(|k| q[u0 + k] != (after >> k & 1 == 1)) {
            return Err("registers not restored".into());
        }
        if q[first..].iter().any(|&x| x) {
            return Err("dirty ancilla at the end".into());
        }
        Ok((sign, acts))
    }

    /// The lane operator: `(-1)^sign` times the product of its system ops.
    fn dense(&mut self, lane: &Lane) -> M {
        let n = self.l.system;
        if !self.cache.contains_key(&lane.1) {
            let unit = 2.0 * std::f64::consts::PI / f64::from(1u32 << self.beta);
            let m = lane.1.iter().fold(eye(1 << n), |m, a| {
                let g = match *a {
                    Act::X(q) => pauli_x(q, n),
                    Act::Z(q) => pauli_z(q, n),
                    Act::G(p, q, a) => givens(p, q, unit * a as f64, n),
                    Act::F(dagger) => f_layer(n, dagger),
                };
                mul(&g, &m)
            });
            self.cache.insert(lane.1.clone(), m);
        }
        let m = &self.cache[&lane.1];
        if lane.0 {
            add(&vec![vec![0.0; 1 << n]; 1 << n], m, -1.0)
        } else {
            m.clone()
        }
    }

    /// What the certificate claims, checked lane by lane: for every `(c, s, b)` and every
    /// outcome record in `records`, the per-lane register checks hold, the lane operator is the
    /// same for every record, and `L(g, a, b)^T = L(g, b, a)`. `Err` names the first failure.
    fn check(&mut self, records: &[u64]) -> Result<(), String> {
        let (u, w, lo) = (self.l.uniform, self.inner.width, self.inner.lo);
        let mask = ((1u64 << w) - 1) << lo;
        let mut ops: HashMap<(bool, u64, u64), M> = HashMap::new();
        for c in [false, true] {
            for s in 0..1u64 << u {
                for b in 0..1u64 << w {
                    let mut first: Option<(Lane, M)> = None;
                    for &r in records {
                        let lane = self
                            .run(c, s, b, &|k| r >> (k % 64) & 1 == 1)
                            .map_err(|e| format!("lane ({c}, {s}, {b}), record {r:#x}: {e}"))?;
                        match &first {
                            Some((f, _)) if *f == lane => {}
                            Some((_, m)) => {
                                if dist(m, &self.dense(&lane)) > 1e-9 {
                                    return Err(format!(
                                        "lane ({c}, {s}, {b}): the operator depends on the \
                                         outcomes (record {r:#x})"
                                    ));
                                }
                            }
                            None => {
                                let m = self.dense(&lane);
                                first = Some((lane, m));
                            }
                        }
                    }
                    if let Some((_, m)) = first {
                        ops.insert((c, s, b), m);
                    }
                }
            }
        }
        for (&(c, s, b), m) in &ops {
            // The lane (g, b, a): inner value b before the Reflect, a after it.
            let a = (s & mask) >> lo;
            let mirror = &ops[&(c, (s & !mask) | b << lo, a)];
            if dist(&t(m), mirror) > 1e-9 {
                return Err(format!(
                    "lane ({c}, {s}, {b}): L(g, a, b)^dagger differs from L(g, b, a)"
                ));
            }
        }
        Ok(())
    }
}

/// Every record when the stream has at most `ALL_RECORDS` `Hmr` ops, else all-zero, all-one and
/// 62 drawn.
const ALL_RECORDS: usize = 8;

fn records(hmr: usize, seed: u64) -> Vec<u64> {
    if hmr <= ALL_RECORDS {
        return (0..1u64 << hmr).collect();
    }
    let mut r = Rng(seed | 1);
    let mut out = vec![0, !0];
    out.extend((0..62).map(|_| r.next()));
    out
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 33) as usize % n.max(1)
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

// ---- The tiny circuit ----

const SYS: usize = 4;
const U_O: u32 = 1;
const W: u32 = 2;
const BETA: u32 = 3;

fn layout() -> (Layout, Inner) {
    (
        Layout {
            system: SYS,
            uniform: U_O + W,
        },
        Inner { lo: U_O, width: W },
    )
}

fn tracker() -> Option<&'static dyn TrackerFactory> {
    gaussian::factory(BETA).map(|f| f as &dyn TrackerFactory)
}

/// Compiles (which includes the structural check of `sim::nested`) and certifies.
fn check_ops(ops: &[Op], budget: Budget) -> (Report, Vec<Vec<u32>>) {
    let (l, inner) = layout();
    let c = compile_sa(ops, &l, tracker(), Some(inner)).expect("the circuit compiles");
    (
        certify(ops, &l, Some(inner), &c.registers, budget),
        c.registers,
    )
}

fn world<'a>(ops: &'a [Op], registers: &'a [Vec<u32>]) -> World<'a> {
    let (l, inner) = layout();
    World {
        ops,
        registers,
        l,
        inner,
        beta: BETA,
        cache: HashMap::new(),
        size: std::cell::Cell::new(None),
    }
}

/// The dense verdict on the same circuit.
fn dense_check(ops: &[Op], registers: &[Vec<u32>], seed: u64) -> Result<(), String> {
    let mut w = world(ops, registers);
    let r = records(w.hmr_count(), seed);
    w.check(&r)
}

/// A ladder: `sel = ctrl AND [bits = v]`, each AND in its own ancilla.
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

/// Undoes a ladder through the wires that computed it: by Toffoli and `R`, or by measuring each
/// AND and fixing its phase with `CZ` on the AND's two inputs. `skip_fix` omits the fix of the
/// last AND measured (a mutant).
fn ladder_down(b: &mut Builder, l: Ladder, measured: bool, skip_fix: bool) {
    let last = l.ands.len().saturating_sub(1);
    for (k, &(c1, c2, a)) in l.ands.iter().enumerate().rev() {
        if measured {
            let m = b.hmr(a);
            if !(skip_fix && k == last) {
                b.cz_if(c1, c2, m);
            }
        } else {
            b.ccx(c1, c2, a);
            b.free(a);
        }
    }
    l.flips.iter().for_each(|&q| b.x(q));
}

fn hmr_keep(b: &mut Builder, q: Qubit) -> Bit {
    let bit = b.new_bit();
    let mut op = Op::new(K::Hmr);
    (op.q_target, op.c_target) = (q.0, bit.0);
    b.emit(op);
    bit
}

/// A classical bit that is 0 here in both inner copies (the second copy replays the first's
/// ops, so a bit it accumulates into must be cleared first).
fn cleared_bit(b: &mut Builder) -> Bit {
    let bit = b.new_bit();
    let mut op = Op::new(K::BitStore0);
    op.c_target = bit.0;
    b.emit(op);
    bit
}

fn bit_invert_if(b: &mut Builder, target: Bit, cond: Bit) {
    let mut op = Op::new(K::BitInvert);
    (op.c_target, op.c_condition) = (target.0, cond.0);
    b.emit(op);
}

/// One way the tiny circuit can depart from the discipline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mutant {
    None,
    /// One AND erasure without its `CZ` fix.
    NoAndFix,
    /// The lookup erasure with one parity `BitInvert` missing.
    NoParityInvert,
    /// An ancilla left 1 on one inner value and freed with `R`.
    DirtyReset,
    /// No pass-flag sign: the second copy applies the anti-Hermitian term, not its adjoint.
    NoPassSign,
    /// Around the rotation back only: angle bit 0 flipped where the inner value is 3.
    AngleDefect,
    /// The rotation back is missing.
    NoRotationBack,
    /// A system `X` conditioned on a measurement outcome.
    OutcomeSteers,
    /// `S` then `Sdg` on an ancilla (the identity).
    SGate,
    /// A `Givens` under a measurement outcome.
    ConditionedGivens,
    /// `-1` on the lanes with the outer bit set, in the prepare segment: Hermitian, but another
    /// operator.
    SymmetricSign,
    /// The inner copy leaves inner bit 0 flipped on one outer value.
    InnerNotRestored,
}

/// Angle table of the tiny circuit: the rotation's angle for each inner value.
const TABLE: [u64; 4] = [5, 3, 6, 1];

/// A nested step written to the discipline: per copy, an angle lookup (unary iteration, ANDs
/// erased by measurement), `V^dagger` as `Z V Z` from the same angle register, a term `Z_0`
/// (on the control and the outer bit) times `X_1 Z_1` on inner bit 0 (anti-Hermitian there, so
/// the copy toggles a pass flag and the second copy takes the adjoint through `CZ(inner 0,
/// pass)`), all inside a controlled `SpinSwap` pair, then `V`, and the lookup erased by
/// measurement with a parity fix. Not a block encoding of any spec: it is certified, not sampled.
fn tiny(m: Mutant) -> Vec<Op> {
    let mut b = Builder::new(SYS);
    b.declare_uniform(U_O + W);
    let ang = b.alloc_n(BETA as usize);
    let reg = b.register(&ang);
    let inner_reg = b.inner_register(U_O, W);
    let (pass, e) = (b.alloc(), b.alloc());
    let (c, o) = (b.control(), b.uniform(0));
    let inner: Vec<Qubit> = (U_O..U_O + W).map(|i| b.uniform(i)).collect();
    let one = b.new_bit();
    let mut store = Op::new(K::BitStore1);
    store.c_target = one.0;
    b.emit(store);
    b.segment(SEG_PREPARE);
    if m == Mutant::SymmetricSign {
        b.cz(c, o);
    }
    b.ccx(c, o, e);
    b.segment(SEG_SELECT);
    b.nested_inner(inner_reg, |b| {
        // Angle lookup.
        for (a, &word) in TABLE.iter().enumerate() {
            let l = ladder_up(b, c, &inner, a as u64);
            for j in (0..BETA as usize).filter(|j| word >> j & 1 == 1) {
                b.cx(l.sel, ang[j]);
            }
            ladder_down(b, l, true, m == Mutant::NoAndFix && a == 2);
        }
        if m == Mutant::DirtyReset {
            let d = b.alloc();
            let l = ladder_up(b, c, &inner, 3);
            b.cx(l.sel, d);
            ladder_down(b, l, false, false);
            b.free(d);
        }
        // V^dagger = Z_0 V Z_0, the term, V.
        let (s0, s1) = (b.system(0), b.system(1));
        b.z(s0);
        b.givens_modes(0, 2, reg);
        b.z(s0);
        b.spin_swap_dg(e);
        b.cz(e, s0);
        b.cz(inner[0], s1);
        b.cx(inner[0], s1);
        if m != Mutant::NoPassSign {
            b.cz(inner[0], pass);
        }
        b.spin_swap(e);
        let flip = (m == Mutant::AngleDefect).then(|| ladder_up(b, c, &inner, 3));
        if let Some(l) = &flip {
            b.cx(l.sel, ang[0]);
        }
        let mut g = Op::new(K::Givens);
        (g.q_control1, g.q_target, g.r_target) = (s0.0, s0.0 + 2, reg.0);
        // A condition that is the constant 1 is no condition; an outcome is one.
        g.c_condition = one.0;
        if m == Mutant::ConditionedGivens {
            let fresh = b.alloc();
            g.c_condition = b.hmr(fresh).0;
        }
        if m != Mutant::NoRotationBack {
            b.emit(g);
        }
        if let Some(l) = flip {
            b.cx(l.sel, ang[0]);
            ladder_down(b, l, false, false);
        }
        if m == Mutant::SGate {
            b.s(e);
            b.sdg(e);
        }
        // Erase the lookup: measure the angle bits; the phase (-1)^(m . T[a]) is fixed by a Z
        // on each inner value's indicator, conditioned on that value's parity bit.
        let ms: Vec<Bit> = ang.iter().map(|&q| hmr_keep(b, q)).collect();
        if m == Mutant::OutcomeSteers {
            let mut x = Op::new(K::CX);
            (x.q_target, x.q_control1, x.c_condition) = (s1.0, e.0, ms[0].0);
            b.emit(x);
        }
        for (a, &word) in TABLE.iter().enumerate() {
            let parity = cleared_bit(b);
            let skip = m == Mutant::NoParityInvert && a == 1;
            for j in (0..BETA as usize).filter(|j| word >> j & 1 == 1) {
                if !(skip && j == 0) {
                    bit_invert_if(b, parity, ms[j]);
                }
            }
            let l = ladder_up(b, c, &inner, a as u64);
            b.z_if(l.sel, parity);
            ladder_down(b, l, false, false);
        }
        if m == Mutant::InnerNotRestored {
            b.cx(o, inner[0]);
        }
        b.x(pass);
    });
    b.segment(SEG_UNPREPARE);
    b.ccx(c, o, e);
    if m == Mutant::SymmetricSign {
        // Nothing to undo: the sign is its own inverse and was applied once.
    }
    for q in [pass, e].into_iter().chain(ang) {
        b.free(q);
    }
    b.finish()
}

fn refused(m: Mutant) -> (String, String, Result<(), String>) {
    let ops = tiny(m);
    let (r, registers) = check_ops(&ops, Budget::default());
    assert!(!r.certified && !r.hermitian, "{m:?}: {}", r.summary());
    let o = r.obstruction.clone().expect("an obstruction");
    assert_eq!(r.json()["status"], "not certified");
    assert_eq!(r.json()["obstruction"]["kind"], o.kind);
    (
        o.kind.to_string(),
        r.summary(),
        dense_check(&ops, &registers, 7),
    )
}

/// (1) The tiny circuit is certified, twice with identical reports, and the dense simulation
/// agrees on every lane.
#[test]
fn tiny_circuit_is_certified_and_dense_simulation_agrees() {
    let ops = tiny(Mutant::None);
    let (r, registers) = check_ops(&ops, Budget::default());
    assert!(r.certified, "{}", r.summary());
    assert!(r.registers_restored && r.outcome_independent && r.hermitian);
    assert_eq!(r.obstruction, None);
    assert_eq!(r.summary(), "certified");
    assert_eq!(r, check_ops(&ops, Budget::default()).0);
    // Per copy: two Givens, two SpinSwaps and one X stay in the normal form; the Zs do not.
    assert_eq!((r.counts.givens, r.counts.spin_swaps), (4, 4));
    assert_eq!(r.counts.compared, 10);
    assert!(r.counts.measurements > 0 && r.counts.outcome_phases_proved > 0);
    let j = r.json();
    assert_eq!(j["version"], "reflection-v1");
    assert_eq!(j["status"], "certified");
    assert!(j["obstruction"].is_null());
    assert!(j["statement"]
        .as_str()
        .unwrap()
        .contains("Does not establish"));
    dense_check(&ops, &registers, 1).unwrap();
}

/// (3) An outcome-dependent phase that is not cancelled: a missing AND fix-up `CZ`, and a
/// missing parity `BitInvert` of the lookup erasure.
#[test]
fn uncancelled_outcome_phase_is_refused() {
    for m in [Mutant::NoAndFix, Mutant::NoParityInvert] {
        let (kind, why, dense) = refused(m);
        assert_eq!(kind, "outcome-phase", "{m:?}: {why}");
        assert!(why.contains("HMR"), "{why}");
        let e = dense.unwrap_err();
        assert!(e.contains("depends on the outcomes"), "{m:?}: {e}");
    }
}

/// (4) An ancilla that is 1 on one inner value when freed: refused at that `R`.
#[test]
fn dirty_reset_is_refused_at_the_reset() {
    let (kind, why, dense) = refused(Mutant::DirtyReset);
    assert_eq!(kind, "reset-not-proven", "{why}");
    assert!(
        why.contains("(R)") && why.contains("is 1 for some lane"),
        "{why}"
    );
    assert!(dense.unwrap_err().contains("dirty R"));
}

/// (6) The second copy applies `M` where it must apply `M^dagger`: refused at the comparison of
/// the sign. (2, tiny form) A rotation back with another angle on one inner value: refused at a
/// Givens angle. A missing rotation: refused on shape. Each really is not Hermitian.
#[test]
fn non_hermitian_steps_are_refused_at_the_comparison() {
    for (m, want) in [
        (Mutant::NoPassSign, "phase-mismatch"),
        (Mutant::AngleDefect, "angle-mismatch"),
        (Mutant::NoRotationBack, "shape-mismatch"),
    ] {
        let ops = tiny(m);
        let (r, registers) = check_ops(&ops, Budget::default());
        // Everything before the comparison holds: only Hermiticity fails.
        assert!(r.registers_restored && r.outcome_independent, "{m:?}");
        assert!(!r.hermitian && !r.certified, "{m:?}");
        assert_eq!(
            r.obstruction.as_ref().unwrap().kind,
            want,
            "{}",
            r.summary()
        );
        let e = dense_check(&ops, &registers, 3).unwrap_err();
        assert!(e.contains("differs from L(g, b, a)"), "{m:?}: {e}");
    }
}

/// (8) Ops and settings outside the rule set are obstructions with their reason, not panics.
#[test]
fn what_the_rule_set_does_not_cover_is_refused() {
    for (m, want) in [
        (Mutant::SGate, "unsupported-op"),
        (Mutant::ConditionedGivens, "conditioned-rotation"),
        (Mutant::OutcomeSteers, "outcome-steers-system"),
        (Mutant::InnerNotRestored, "reflect-not-restored"),
    ] {
        let (kind, why, _) = refused(m);
        assert_eq!(kind, want, "{m:?}: {why}");
    }
    let ops = tiny(Mutant::None);
    let (l, inner) = layout();
    let registers = compile_sa(&ops, &l, tracker(), Some(inner))
        .unwrap()
        .registers;
    // A lane map that is not nested.
    let flat = certify(&ops, &l, None, &registers, Budget::default());
    assert!(!flat.certified);
    assert_eq!(flat.obstruction.unwrap().kind, "unsupported-lane-map");
    // An exhausted budget, at every size up to the work the circuit needs.
    let need = check_ops(&ops, Budget::default()).0.work;
    for work in [0, 1, 50, need / 7, need / 2, need - 1] {
        let b = Budget {
            work,
            ..Budget::default()
        };
        let r = certify(&ops, &l, Some(inner), &registers, b);
        assert!(!r.certified && !r.hermitian, "work {work}");
        assert_eq!(r.obstruction.unwrap().kind, "budget", "work {work}");
    }
    let enough = Budget {
        work: need,
        ..Budget::default()
    };
    assert!(certify(&ops, &l, Some(inner), &registers, enough).certified);
    // An inner register outside the uniform register.
    let outside = Inner { lo: 2, width: 5 };
    let r = certify(&ops, &l, Some(outside), &registers, Budget::default());
    assert_eq!(r.obstruction.unwrap().kind, "structure");
    // Streams the compiler would reject are refused here too, without it.
    let mut twice = ops.clone();
    let reflect = *ops.iter().find(|o| o.kind == K::Reflect).unwrap();
    twice.push(reflect);
    let r = certify(&twice, &l, Some(inner), &registers, Budget::default());
    assert_eq!(r.obstruction.unwrap().kind, "structure");
    let none: Vec<Op> = ops
        .iter()
        .copied()
        .filter(|o| o.kind != K::Reflect)
        .collect();
    let r = certify(&none, &l, Some(inner), &registers, Budget::default());
    assert_eq!(r.obstruction.unwrap().kind, "structure");
    let mut bad = ops.clone();
    let mut h = Op::new(K::Hmr);
    (h.q_target, h.c_target) = (0, 0);
    bad.push(h);
    let r = certify(&bad, &l, Some(inner), &registers, Budget::default());
    assert_eq!(r.obstruction.unwrap().kind, "unsupported-op");
    let mut sysctl = ops.clone();
    let mut cx = Op::new(K::CX);
    (cx.q_control1, cx.q_target) = (1, 0);
    sysctl.push(cx);
    let r = certify(&sysctl, &l, Some(inner), &registers, Budget::default());
    assert_eq!(r.obstruction.unwrap().kind, "unsupported-op");
}

/// (5) A `Z` on the walk control behind 30 nested outcome conditions: no sampled lane reaches
/// it; the check refuses it, because the sign's coefficient of that product of outcomes is the
/// control bit.
#[test]
fn block_behind_a_rare_outcome_pattern_is_refused() {
    let mut ops = tiny(Mutant::None);
    let (q0, b0) = (900u32, 900u32);
    for i in 0..30 {
        let mut h = Op::new(K::Hmr);
        (h.q_target, h.c_target) = (q0 + i, b0 + i);
        ops.push(h);
    }
    for i in 0..30 {
        let mut p = Op::new(K::PushCondition);
        p.c_condition = b0 + i;
        ops.push(p);
    }
    let mut z = Op::new(K::Z);
    z.q_target = 0;
    ops.push(z);
    ops.extend((0..30).map(|_| Op::new(K::PopCondition)));
    let (r, _) = check_ops(&ops, Budget::default());
    assert!(r.registers_restored && !r.outcome_independent && !r.certified);
    let o = r.obstruction.unwrap();
    assert_eq!(o.kind, "outcome-phase", "{o}");
    // Two Negs in the block instead: the identity, certified.
    ops.retain(|o| !(o.kind == K::Z && o.q_target == 0));
    let at = ops
        .iter()
        .rposition(|o| o.kind == K::PushCondition)
        .unwrap()
        + 1;
    ops.splice(at..at, [Op::new(K::Neg), Op::new(K::Neg)]);
    assert!(check_ops(&ops, Budget::default()).0.certified);
}

/// (7) Honesty: a sign on half the lanes makes the step another operator and leaves it
/// Hermitian. The check certifies it, and the dense simulation agrees that it is Hermitian. The
/// certificate is not a statement about the encoded operator.
#[test]
fn hermitian_but_wrong_operator_is_certified() {
    let ops = tiny(Mutant::SymmetricSign);
    let (r, registers) = check_ops(&ops, Budget::default());
    assert!(r.certified, "{}", r.summary());
    dense_check(&ops, &registers, 5).unwrap();
    // It is a different operator: some lane's sign differs from the clean circuit's.
    let clean = tiny(Mutant::None);
    let (a, b) = (world(&ops, &registers), world(&clean, &registers));
    let differ = (0..1u64 << (U_O + W)).any(|s| {
        a.run(true, s, 0, &|_| false).unwrap().0 != b.run(true, s, 0, &|_| false).unwrap().0
    });
    assert!(differ);
}

// ---- (10) Soundness fuzz ----

/// A control wire and a system op it steers.
#[derive(Clone, Copy)]
enum Elem {
    X(Qubit, usize),
    Z(Qubit, usize),
    G(usize, usize),
    F(Qubit),
}

fn emit(b: &mut Builder, e: Elem, reg: Reg, adjoint: bool) {
    match e {
        Elem::X(c, q) => {
            let s = b.system(q);
            b.cx(c, s);
        }
        Elem::Z(c, q) => {
            let s = b.system(q);
            b.cz(c, s);
        }
        // G(-t) = Z_p G(t) Z_p = Z_q G(t) Z_q, from the same angle register.
        Elem::G(p, q) => {
            let s = b.system(if (p + q) % 2 == 0 { p } else { q });
            if adjoint {
                b.z(s);
            }
            b.givens_modes(p, q, reg);
            if adjoint {
                b.z(s);
            }
        }
        Elem::F(c) if adjoint => b.spin_swap_dg(c),
        Elem::F(c) => b.spin_swap(c),
    }
}

/// A random nested step that is Hermitian by construction: `W_out^dagger [copy] R [copy]
/// W_out`, each copy `W^dagger P W` with `P` Hermitian (or anti-Hermitian with the pass-flag
/// sign), every `W` a random list of controlled Paulis, Givens and SpinSwaps, controls drawn
/// from the lane's bits and from ANDs of them, the angle register loaded by a random lookup and
/// erased by measurement or by recomputation.
fn random_step(r: &mut Rng) -> Vec<Op> {
    let mut b = Builder::new(SYS);
    b.declare_uniform(U_O + W);
    let ang = b.alloc_n(BETA as usize);
    let reg = b.register(&ang);
    let inner_reg = b.inner_register(U_O, W);
    let (pass, e) = (b.alloc(), b.alloc());
    let (c, o) = (b.control(), b.uniform(0));
    let inner: Vec<Qubit> = (U_O..U_O + W).map(|i| b.uniform(i)).collect();
    let sys_q = |r: &mut Rng| r.below(SYS);
    let pair = |r: &mut Rng| {
        let p = r.below(SYS - 1);
        (p, p + 1 + r.below(SYS - 1 - p))
    };
    b.segment(SEG_PREPARE);
    b.ccx(c, o, e);
    let outer_pool = [c, o, e];
    let w_out: Vec<Elem> = (0..r.below(3))
        .map(|_| {
            let ctl = outer_pool[r.below(3)];
            if r.chance(50) {
                Elem::X(ctl, sys_q(r))
            } else {
                Elem::Z(ctl, sys_q(r))
            }
        })
        .collect();
    for &x in w_out.iter().rev() {
        emit(&mut b, x, reg, true);
    }
    b.segment(SEG_SELECT);
    let table: Vec<u64> = (0..4).map(|_| r.next() % 8).collect();
    let load_ctl = if r.chance(50) { c } else { e };
    // At most three Hmr per copy, so that every outcome record of the step is enumerated.
    let measured_erase = r.chance(50);
    let measured_load = !measured_erase && r.chance(70);
    let with_f = r.chance(60);
    let n_w = 1 + r.below(3);
    let anti = r.chance(40);
    let core_x = r.chance(70).then(|| sys_q(r));
    // A Z on the X's own qubit would make the term anti-Hermitian without the pass-flag sign.
    let core_z: Vec<usize> = (0..SYS)
        .filter(|&q| r.chance(30) && Some(q) != core_x)
        .collect();
    let seed = r.next();
    b.nested_inner(inner_reg, |b| {
        let mut r = Rng(seed | 1);
        let r = &mut r;
        for (a, &word) in table.iter().enumerate() {
            if word == 0 {
                continue;
            }
            let l = ladder_up(b, load_ctl, &inner, a as u64);
            for j in (0..BETA as usize).filter(|j| word >> j & 1 == 1) {
                b.cx(l.sel, ang[j]);
            }
            ladder_down(b, l, measured_load && a == 0, false);
        }
        let f = with_f.then(|| {
            let f = b.alloc();
            b.ccx(e, inner[0], f);
            f
        });
        let mut pool = vec![c, o, e, inner[0], inner[1]];
        pool.extend(f);
        let w: Vec<Elem> = (0..n_w)
            .map(|_| {
                let ctl = pool[r.below(pool.len())];
                match r.below(5) {
                    0 | 1 => {
                        let (p, q) = pair(r);
                        Elem::G(p, q)
                    }
                    2 => Elem::X(ctl, sys_q(r)),
                    3 => Elem::Z(ctl, sys_q(r)),
                    _ => Elem::F(ctl),
                }
            })
            .collect();
        for &x in w.iter().rev() {
            emit(b, x, reg, true);
        }
        let ctl = pool[r.below(pool.len())];
        for &q in &core_z {
            let s = b.system(q);
            b.cz(ctl, s);
        }
        if let Some(q) = core_x {
            let s = b.system(q);
            if anti {
                // X Z on one qubit is anti-Hermitian; the pass flag turns the second copy's
                // into its adjoint.
                b.cz(ctl, s);
                b.cz(ctl, pass);
            }
            b.cx(ctl, s);
        }
        for &x in &w {
            emit(b, x, reg, false);
        }
        if let Some(f) = f {
            if !measured_erase && r.chance(50) {
                let m = b.hmr(f);
                b.cz_if(e, inner[0], m);
            } else {
                b.ccx(e, inner[0], f);
                b.free(f);
            }
        }
        if measured_erase {
            let ms: Vec<Bit> = ang.iter().map(|&q| hmr_keep(b, q)).collect();
            for (a, &word) in table.iter().enumerate() {
                if word == 0 {
                    continue;
                }
                let parity = cleared_bit(b);
                for j in (0..BETA as usize).filter(|j| word >> j & 1 == 1) {
                    bit_invert_if(b, parity, ms[j]);
                }
                let l = ladder_up(b, load_ctl, &inner, a as u64);
                b.z_if(l.sel, parity);
                ladder_down(b, l, false, false);
            }
        } else {
            for (a, &word) in table.iter().enumerate().rev() {
                if word == 0 {
                    continue;
                }
                let l = ladder_up(b, load_ctl, &inner, a as u64);
                for j in (0..BETA as usize).filter(|j| word >> j & 1 == 1) {
                    b.cx(l.sel, ang[j]);
                }
                ladder_down(b, l, false, false);
            }
        }
        b.x(pass);
    });
    b.segment(SEG_UNPREPARE);
    for &x in &w_out {
        emit(&mut b, x, reg, false);
    }
    b.ccx(c, o, e);
    for q in [pass, e].into_iter().chain(ang) {
        b.free(q);
    }
    b.finish()
}

/// One random edit, applied to both inner copies alike when it falls inside the first.
fn mutate(ops: &[Op], r: &mut Rng) -> Vec<Op> {
    let seg = |code: u32| -> Vec<usize> {
        (0..ops.len())
            .filter(|&i| ops[i].kind == K::Segment && ops[i].r_target == code)
            .collect()
    };
    let (begin, end) = (seg(SEG_INNER_BEGIN), seg(SEG_INNER_END));
    let editable = |o: &Op| {
        !matches!(
            o.kind,
            K::Segment | K::Register | K::AppendToRegister | K::Reflect
        )
    };
    let at = loop {
        let i = r.below(ops.len());
        if editable(&ops[i]) && !(begin[1]..=end[1]).contains(&i) {
            break i;
        }
    };
    let wires: Vec<u32> = (0..12).collect();
    let mut new = ops[at];
    let kind = r.below(7);
    match kind {
        // Retarget one operand.
        1 => new.q_target = wires[r.below(wires.len())],
        2 if new.q_control1 != NONE => new.q_control1 = wires[r.below(wires.len())],
        // Toggle a condition on a classical bit.
        3 => {
            new.c_condition = if new.c_condition == NONE {
                r.below(4) as u32
            } else {
                NONE
            }
        }
        // Another gate on the same operands.
        4 => {
            new.kind = match new.kind {
                K::CX => K::CZ,
                K::CZ => K::CX,
                K::X => K::Z,
                K::Z => K::X,
                K::CCX => K::CCZ,
                K::SpinSwap => K::SpinSwapDg,
                K::SpinSwapDg => K::SpinSwap,
                k => k,
            }
        }
        _ => {}
    }
    let mirror = (begin[0] < at && at < end[0]).then(|| at - begin[0] + begin[1]);
    // Exchange with the next op (kind 6), when that one may be edited too.
    let swap = kind == 6 && ops.get(at + 1).is_some_and(editable) && at + 1 != begin[1];
    let mut out = Vec::with_capacity(ops.len() + 2);
    let mut i = 0;
    while i < ops.len() {
        if i != at && Some(i) != mirror {
            out.push(ops[i]);
        } else if swap {
            out.extend([ops[i + 1], ops[i]]);
            i += 1;
        } else {
            match kind {
                // Delete.
                0 => {}
                // Insert a copy of the edited op after the original.
                5 => out.extend([ops[i], new]),
                _ => out.push(new),
            }
        }
        i += 1;
    }
    out
}

/// (10) Whenever the check certifies, the dense simulation of every lane must agree. The
/// circuits are Hermitian by construction, half of them then edited at random; an edit that
/// keeps the step Hermitian may or may not stay certified, and nothing is asserted about a
/// circuit the check refuses.
#[test]
fn certified_random_circuits_are_hermitian_on_every_lane() {
    let (l, inner) = layout();
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut clean, mut clean_certified, mut edited, mut edited_certified) = (0, 0, 0, 0);
    let (mut refused_and_wrong, mut refused_checked, mut full) = (0, 0, 0);
    for case in 0..800 {
        let mut ops = random_step(&mut r);
        let is_edited = case % 2 == 1;
        if is_edited {
            ops = mutate(&ops, &mut r);
        }
        if ops.iter().any(|o| o.validate().is_err()) {
            continue;
        }
        let Ok(compiled) = compile_sa(&ops, &l, tracker(), Some(inner)) else {
            continue;
        };
        let report = certify(
            &ops,
            &l,
            Some(inner),
            &compiled.registers,
            Budget::default(),
        );
        let (count, certified) = if is_edited {
            (&mut edited, &mut edited_certified)
        } else {
            (&mut clean, &mut clean_certified)
        };
        *count += 1;
        if report.certified {
            *certified += 1;
            let hmr = ops.iter().filter(|o| o.kind == K::Hmr).count();
            full += usize::from(hmr <= ALL_RECORDS);
            dense_check(&ops, &compiled.registers, r.next()).unwrap_or_else(|e| {
                panic!("case {case}: certified, but the dense simulation disagrees: {e}")
            });
        } else if is_edited
            && refused_checked < 40
            && !ops.iter().any(|o| matches!(o.kind, K::S | K::Sdg))
        {
            // How often a refusal is a real defect (reported; nothing depends on it but the
            // sanity bound below).
            refused_checked += 1;
            refused_and_wrong += usize::from(dense_check(&ops, &compiled.registers, 11).is_err());
        }
    }
    println!(
        "clean {clean_certified}/{clean} certified; edited {edited_certified}/{edited} certified; \
         {refused_and_wrong}/{refused_checked} refused edits fail the dense simulation; every \
         outcome record enumerated for {full} certified circuits"
    );
    assert_eq!(full, clean_certified + edited_certified);
    // The fuzz is not vacuous: most constructed circuits are certified, some edited ones are,
    // and the dense simulation does reject broken circuits.
    assert!(
        clean >= 300 && clean_certified * 10 >= clean * 9,
        "{clean_certified}/{clean}"
    );
    assert!(
        edited >= 150 && edited_certified >= 20,
        "{edited_certified}/{edited}"
    );
    assert!(
        refused_and_wrong >= 10,
        "{refused_and_wrong}/{refused_checked}"
    );
}

// ---- The published construction on a small spec, and the evaluator ----

#[cfg(feature = "walk")]
mod sa {
    use super::*;
    use femoco_walk::circuit::{read_ops, write_ops, OpsFile};
    use femoco_walk::lanemap::sa_nested::SaNestedMap;
    use femoco_walk::lanemap::LaneMap;
    use femoco_walk::score::{
        self, score_json, Evaluation, FamilyOut, Inputs, Options, ReflectionCheck,
    };
    use femoco_walk::sim::givens_tracker;
    use femoco_walk::sim::validate;
    use femoco_walk::spec::sa::{parse_payload, SaSpec};
    use femoco_walk::spec::EncodingSpec;
    use femoco_walk::taxonomy;
    use femoco_walk::walk::sa_low::{self, Params, Tweaks};
    use std::sync::atomic::{AtomicU64, Ordering};

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

    /// `sa-low2025` (Low et al.'s construction, with `SpinSwap`) at this spec's size.
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

    struct Toy {
        spec: SaSpec,
        map: SaNestedMap,
        ops: Vec<Op>,
        l: Layout,
        inner: Inner,
    }

    fn toy() -> Toy {
        let spec = spec();
        let map = sa_low::lane_map(&spec, P).unwrap();
        let mut b = Builder::new(2 * spec.n);
        b.declare_uniform(map.uniform_bits());
        let _ = sa_low::emit(&spec, &map, &mut b, P);
        let (lo, width) = map.inner_bits().unwrap();
        let l = Layout {
            system: 2 * spec.n,
            uniform: map.uniform_bits(),
        };
        Toy {
            spec,
            map,
            ops: b.finish(),
            l,
            inner: Inner { lo, width },
        }
    }

    static TMP: AtomicU64 = AtomicU64::new(0);

    fn ops_file(ops: &[Op]) -> OpsFile {
        let n = TMP.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("femoco-refl-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ops.bin");
        write_ops(ops, &path).unwrap();
        let f = read_ops(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        f
    }

    impl Toy {
        /// The trusted evaluation of `ops` at `samples` lanes.
        fn eval(
            &self,
            ops: &[Op],
            samples: usize,
            mode: ReflectionCheck,
        ) -> Result<Evaluation, String> {
            let tp =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
            let tax = taxonomy::load_taxonomy(&tp).unwrap();
            let check = |f: &_, facts: &_| taxonomy::check(&tax, f, facts);
            let fam = serde_json::to_vec(&FamilyOut {
                family: sa_low::family(true),
                spec: self.spec.id().to_string(),
            })
            .unwrap();
            let file = ops_file(ops);
            let lm = self.map.to_bytes();
            let inp = Inputs {
                spec: &self.spec,
                lanemap: &lm,
                family: &fam,
                ops: &file,
                samples,
                tracker: givens_tracker(&self.spec),
                check: &check,
            };
            let opts = Options {
                reflection: mode,
                ..Options::default()
            };
            score::evaluate_opts(&inp, validate::reference, None, opts)
        }

        fn certify(&self, ops: &[Op]) -> Report {
            let c = compile_sa(ops, &self.l, givens_tracker(&self.spec), Some(self.inner))
                .expect("compiles");
            certify(
                ops,
                &self.l,
                Some(self.inner),
                &c.registers,
                Budget::default(),
            )
        }

        /// Indices of `Segment 3`, `Segment 4` (first copy) and `Segment 3` (second copy).
        fn copies(&self, ops: &[Op]) -> (usize, usize, usize) {
            let at = |code: u32| -> Vec<usize> {
                (0..ops.len())
                    .filter(|&i| ops[i].kind == K::Segment && ops[i].r_target == code)
                    .collect()
            };
            let (b, e) = (at(SEG_INNER_BEGIN), at(SEG_INNER_END));
            (b[0], e[0], b[1])
        }

        fn fresh(&self, ops: &[Op]) -> u32 {
            ops.iter().flat_map(Op::qubits).max().unwrap() + 1
        }
    }

    fn gate(kind: K, t: u32, c1: u32, c2: u32) -> Op {
        let mut o = Op::new(kind);
        (o.q_target, o.q_control1, o.q_control2) = (t, c1, c2);
        o
    }

    /// `out = ctrl AND [bits = v]` in the fresh ancillas `anc` (one per bit; the last is `out`),
    /// and the ops that undo it, freeing every ancilla with `R`. Each half leaves `bits` as it
    /// found them.
    fn and_ladder(ctrl: u32, bits: &[u32], v: u64, anc: &[u32]) -> (Vec<Op>, Vec<Op>) {
        let flips: Vec<Op> = bits
            .iter()
            .enumerate()
            .filter(|(i, _)| v >> i & 1 == 0)
            .map(|(_, &q)| gate(K::X, q, NONE, NONE))
            .collect();
        let mut up = flips.clone();
        let mut prev = ctrl;
        let mut ands = Vec::new();
        for (&q, &a) in bits.iter().zip(anc) {
            ands.push(gate(K::CCX, a, prev, q));
            prev = a;
        }
        up.extend(ands.iter().copied());
        up.extend(flips.iter().copied());
        let mut down = flips.clone();
        for g in ands.iter().rev() {
            down.push(*g);
            down.push(gate(K::R, g.q_target, NONE, NONE));
        }
        down.extend(flips);
        (up, down)
    }

    /// `score.json` without the wall clock.
    fn body(ev: &Evaluation) -> serde_json::Value {
        let mut v = score_json(ev);
        v["metrics"].as_object_mut().unwrap().remove("eval_seconds");
        v
    }

    /// (1) and (11) The published construction is certified, deterministically; with `report`
    /// the run's `score.json` differs from the plain run's by the new keys only, and with the
    /// flag absent it is the plain run's.
    #[test]
    fn published_construction_is_certified_and_changes_only_its_own_keys() {
        let toy = toy();
        let r = toy.certify(&toy.ops);
        assert!(r.certified, "{}", r.summary());
        assert_eq!(r, toy.certify(&toy.ops));
        let k = 1 << 12;
        let plain = toy.eval(&toy.ops, k, ReflectionCheck::Off).unwrap();
        assert!(plain.reflection.is_none());
        assert!(body(&plain)["metrics"].get("reflection").is_none());
        let limits = &body(&plain)["metrics"]["sampling_limits"];
        assert!(limits.get("hermitian_on_every_lane").is_none());
        assert!(limits["implied_error_bound_holds_if"]
            .as_str()
            .unwrap()
            .contains("sampling does not establish this"));
        for mode in [ReflectionCheck::Report, ReflectionCheck::Require] {
            let ev = toy.eval(&toy.ops, k, mode).unwrap();
            assert_eq!(ev.reflection.as_ref(), Some(&r));
            assert_eq!(
                (ev.toffoli, ev.qubits, ev.samples),
                (plain.toffoli, plain.qubits, plain.samples)
            );
            let mut j = body(&ev);
            let m = j["metrics"].as_object_mut().unwrap();
            let refl = m.remove("reflection").unwrap();
            assert_eq!(refl["status"], "certified");
            assert_eq!(refl["version"], "reflection-v1");
            let s = m["sampling_limits"].as_object_mut().unwrap();
            assert_eq!(s.remove("hermitian_on_every_lane").unwrap(), "certified");
            assert_eq!(
                s.remove("non_hermitian_phase_scale_applies").unwrap(),
                false
            );
            assert_eq!(s["certifies_energy_error"], false);
            let cond = s.insert(
                "implied_error_bound_holds_if".into(),
                limits["implied_error_bound_holds_if"].clone(),
            );
            assert!(cond
                .unwrap()
                .as_str()
                .unwrap()
                .contains("metrics.reflection"));
            let outcomes = s.insert(
                "measurement_outcomes".into(),
                limits["measurement_outcomes"].clone(),
            );
            assert!(outcomes
                .unwrap()
                .as_str()
                .unwrap()
                .contains("metrics.reflection"));
            assert_eq!(j, body(&plain));
        }
    }

    /// (2) The rare-lane non-Hermitian rotation of spec/SAMPLING.md section 2.2: on one uniform
    /// value the last rotation of each copy reads its angle with bit 0 flipped, so the rotation
    /// back disagrees with the rotation there. The sampled check at 64 lanes accepts some such
    /// circuit; this check refuses every one of them at a Givens angle; and a dense simulation
    /// of the affected lanes confirms the step is not Hermitian there.
    #[test]
    fn rare_lane_rotation_defect_passes_sampling_and_is_refused() {
        let toy = toy();
        let (b1, e1, b2) = toy.copies(&toy.ops);
        let (u_o, w) = (toy.inner.lo, toy.inner.width);
        let u0 = 1 + toy.l.system as u32;
        let outer: Vec<u32> = (0..u_o).map(|i| u0 + i).collect();
        let inner: Vec<u32> = (0..w).map(|i| u0 + u_o + i).collect();
        // The last Givens of the copy and the low bit of its angle register.
        let d = (b1 + 1..e1)
            .rev()
            .find(|&i| toy.ops[i].kind == K::Givens)
            .unwrap()
            - b1;
        let registers = compile_sa(&toy.ops, &toy.l, givens_tracker(&toy.spec), Some(toy.inner))
            .unwrap()
            .registers;
        let x0 = registers[toy.ops[b1 + d].r_target as usize][0];
        let fresh = toy.fresh(&toy.ops);
        let build = |v_o: u64, v_i: u64| -> Vec<Op> {
            let anc_o: Vec<u32> = (0..u_o).map(|i| fresh + i).collect();
            let anc_i: Vec<u32> = (0..w).map(|i| fresh + u_o + i).collect();
            let (f, g) = (anc_o[u_o as usize - 1], anc_i[w as usize - 1]);
            let (up_o, down_o) = and_ladder(0, &outer, v_o, &anc_o);
            let (up_i, down_i) = and_ladder(f, &inner, v_i, &anc_i);
            let flip = gate(K::CX, x0, g, NONE);
            let mut out = up_o;
            for (i, &op) in toy.ops.iter().enumerate() {
                for start in [b1, b2] {
                    if i == start + d {
                        out.push(flip);
                    }
                }
                out.push(op);
                for start in [b1, b2] {
                    if i == start {
                        out.extend(up_i.iter().copied());
                    }
                    if i == start + d {
                        out.push(flip);
                    }
                    if i == start + (e1 - b1) - 1 {
                        out.extend(down_i.iter().copied());
                    }
                }
            }
            out.extend(down_o);
            out
        };
        let mut accepted = None;
        for v in 0..12u64 {
            let (v_o, v_i) = (v * 5 % (1 << u_o), v * 3 % (1 << w));
            let ops = build(v_o, v_i);
            let r = toy.certify(&ops);
            assert!(!r.certified && r.registers_restored && r.outcome_independent);
            let o = r.obstruction.unwrap();
            assert_eq!(o.kind, "angle-mismatch", "{o}");
            // The same circuit under `require` is a rejection with that reason.
            if toy.eval(&ops, 64, ReflectionCheck::Off).is_ok() {
                let e = toy.eval(&ops, 64, ReflectionCheck::Require).unwrap_err();
                assert!(
                    e.starts_with("reflection-check: not certified: op ") && e.contains("angle"),
                    "{e}"
                );
                let ev = toy.eval(&ops, 64, ReflectionCheck::Report).unwrap();
                let j = score_json(&ev);
                assert_eq!(j["metrics"]["reflection"]["status"], "not certified");
                let s = &j["metrics"]["sampling_limits"];
                assert_eq!(s["hermitian_on_every_lane"], "not established");
                assert!(s["implied_error_bound_holds_if"]
                    .as_str()
                    .unwrap()
                    .contains("sampling does not establish this"));
                accepted.get_or_insert((v_o, v_i, ops));
            }
        }
        let (v_o, v_i, ops) = accepted.expect("the sampled check accepts one of the candidates");
        // The full sample finds it: the defect is real.
        let e = toy.eval(&ops, 1 << 14, ReflectionCheck::Off).unwrap_err();
        assert!(e.starts_with("tracker"), "{e}");
        // Dense: on the lanes with the outer value v_o and inner value v_i before or after the
        // Reflect, L(g, a, b)^dagger is not L(g, b, a).
        let registers = compile_sa(&ops, &toy.l, givens_tracker(&toy.spec), Some(toy.inner))
            .unwrap()
            .registers;
        let mut world = World {
            ops: &ops,
            registers: &registers,
            l: toy.l,
            inner: toy.inner,
            beta: BETA,
            cache: HashMap::new(),
            size: std::cell::Cell::new(None),
        };
        let (mut worst, mut clean) = (0.0f64, 0.0f64);
        for b in (0..1u64 << w).filter(|&b| b != v_i).take(3) {
            for (outer_value, into) in [(v_o, &mut worst), (v_o ^ 1, &mut clean)] {
                let s = |a: u64| outer_value | a << u_o;
                let fwd = world.run(true, s(v_i), b, &|k| k % 3 == 0).unwrap();
                let back = world.run(true, s(b), v_i, &|k| k % 5 == 0).unwrap();
                let (fwd, back) = (world.dense(&fwd), world.dense(&back));
                *into = into.max(dist(&t(&fwd), &back));
            }
        }
        assert!(worst > 1e-3, "no affected lane is non-Hermitian: {worst}");
        assert!(
            clean < 1e-9,
            "a lane beside the defect is not Hermitian: {clean}"
        );
    }

    /// (5) A wrong block behind 30 outcome conditions passes the sampled check and is refused.
    #[test]
    fn rare_outcome_block_passes_sampling_and_is_refused() {
        let toy = toy();
        let mut ops = toy.ops.clone();
        let (q0, b0) = (toy.fresh(&ops), 1 << 20);
        for i in 0..30 {
            let mut h = Op::new(K::Hmr);
            (h.q_target, h.c_target) = (q0 + i, b0 + i);
            ops.push(h);
        }
        for i in 0..30 {
            let mut p = Op::new(K::PushCondition);
            p.c_condition = b0 + i;
            ops.push(p);
        }
        ops.push(gate(K::Z, 0, NONE, NONE));
        ops.extend((0..30).map(|_| Op::new(K::PopCondition)));
        toy.eval(&ops, 1 << 12, ReflectionCheck::Off).unwrap();
        let e = toy
            .eval(&ops, 1 << 12, ReflectionCheck::Require)
            .unwrap_err();
        assert!(e.contains("[outcome-phase]"), "{e}");
    }

    /// (6) Both copies apply `M`: the pass-flag sign that turns the second copy's one-body term
    /// into its adjoint is removed. Refused at the comparison of the sign.
    #[test]
    fn missing_adjoint_in_the_second_copy_is_refused() {
        let toy = toy();
        let (b1, e1, b2) = toy.copies(&toy.ops);
        // The pass flag: the copy's last unconditioned X on an ancilla.
        let first = toy.l.first_ancilla();
        let pass = (b1 + 1..e1)
            .rev()
            .map(|i| toy.ops[i])
            .find(|o| o.kind == K::X && o.c_condition == NONE && o.q_target >= first)
            .unwrap()
            .q_target;
        let on_pass = |o: &Op| o.kind == K::CZ && (o.q_target == pass || o.q_control1 == pass);
        let drop: Vec<usize> = (b1 + 1..e1).filter(|&i| on_pass(&toy.ops[i])).collect();
        assert!(!drop.is_empty());
        let ops: Vec<Op> = (0..toy.ops.len())
            .filter(|i| !drop.contains(i) && !drop.contains(&(i.wrapping_sub(b2 - b1))))
            .map(|i| toy.ops[i])
            .collect();
        assert_eq!(ops.len(), toy.ops.len() - 2 * drop.len());
        let r = toy.certify(&ops);
        assert!(r.outcome_independent && !r.certified);
        assert_eq!(r.obstruction.unwrap().kind, "phase-mismatch");
        let e = toy.eval(&ops, 1 << 12, ReflectionCheck::Off).unwrap_err();
        assert!(e.contains("sign flipped"), "{e}");
    }

    /// (7) Honesty: `-1` on one outer value. The step is still Hermitian and the check
    /// certifies it; it encodes another operator, which is the sampled check's to find.
    #[test]
    fn symmetric_sign_error_is_certified_and_left_to_sampling() {
        let toy = toy();
        let u0 = 1 + toy.l.system as u32;
        let outer: Vec<u32> = (0..toy.inner.lo).map(|i| u0 + i).collect();
        let fresh = toy.fresh(&toy.ops);
        let anc: Vec<u32> = (0..toy.inner.lo).map(|i| fresh + i).collect();
        let (up, down) = and_ladder(0, &outer, 5, &anc);
        let mut ops = up;
        ops.push(gate(K::Z, *anc.last().unwrap(), NONE, NONE));
        ops.extend(down);
        ops.extend(toy.ops.iter().copied());
        assert!(toy.certify(&ops).certified);
        let e = toy
            .eval(&ops, 1 << 12, ReflectionCheck::Require)
            .unwrap_err();
        assert!(e.contains("sign flipped"), "{e}");
    }

    /// The ledger's circuits. Ignored: heavy, and the artifacts are built outside the test. Set
    /// `FEMOCO_REFLECTION_ROOTS` to `:`-separated directories, each holding one built circuit
    /// (`ops.bin`, `lanemap.bin`, `family.out.json`), and run through tools/heavy.sh. The check
    /// alone is run (no lanes). The Low et al. baseline must be certified; for the others the
    /// result is printed, and the first obstruction recorded here must not move silently.
    #[test]
    #[ignore = "heavy: needs built circuits (FEMOCO_REFLECTION_ROOTS); run through tools/heavy.sh"]
    fn ledger_circuits() {
        use femoco_walk::{lanemap, spec as specs};
        // (ops.bin SHA-256 prefix, certified, first obstruction's op and kind).
        type First = Option<(usize, &'static str)>;
        let pinned: [(&str, bool, First); 3] = [
            ("95e8fe89", true, None),
            ("a939d204", false, Some((763_394, "reset-not-proven"))),
            ("22ed4a76", false, Some((2_543_511, "reflect-not-restored"))),
        ];
        let roots = std::env::var("FEMOCO_REFLECTION_ROOTS").expect("FEMOCO_REFLECTION_ROOTS");
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for dir in roots.split(':').map(std::path::Path::new) {
            let fam = std::fs::read(dir.join("family.out.json")).unwrap();
            let out: FamilyOut = serde_json::from_slice(&fam).unwrap();
            let spec = specs::load(here, &out.spec).unwrap();
            let lm = lanemap::parse(
                &std::fs::read(dir.join("lanemap.bin")).unwrap(),
                spec.as_ref(),
            )
            .unwrap();
            let ops = read_ops(&dir.join("ops.bin")).unwrap();
            let l = Layout {
                system: spec.system_qubits(),
                uniform: lm.uniform_bits(),
            };
            let inner = lm.inner_bits().map(|(lo, width)| Inner { lo, width });
            let c = compile_sa(&ops.ops, &l, givens_tracker(spec.as_ref()), inner).unwrap();
            let t0 = std::time::Instant::now();
            let r = certify(&ops.ops, &l, inner, &c.registers, Budget::default());
            let digest = hex::encode(ops.sha256);
            println!(
                "{}: ops {digest}: {} ({} ops read, {} symbols, {} sealed, work {}, {:.1} s)",
                dir.display(),
                r.summary(),
                r.counts.ops,
                r.symbols,
                r.sealed,
                r.work,
                t0.elapsed().as_secs_f64()
            );
            if let Some((_, certified, first)) = pinned.iter().find(|(d, ..)| digest.starts_with(d))
            {
                assert_eq!(r.certified, *certified, "{}", dir.display());
                let got = r
                    .obstruction
                    .as_ref()
                    .map(|o| (o.op.unwrap_or(usize::MAX), o.kind));
                assert_eq!(got, *first, "{}", dir.display());
            }
        }
    }
}
