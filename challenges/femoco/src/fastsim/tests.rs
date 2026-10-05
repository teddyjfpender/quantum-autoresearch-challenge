//! The fast engine against the reference.
//!
//! - `op_*`: every op kind on random states. The reference `Lanes` and the fast `Exec` start from
//!   the same random planes, run the same op (with and without its own condition, under a pushed
//!   condition), and must agree on every plane, the tally and the execution error; at `Givens`
//!   and `SpinSwap` the reference's tracker calls (recorded by a tracker that only logs) must be
//!   exactly the calls the fast engine wrote into each lane's trace.
//! - `runs_*`: whole random circuits through `validate::run` and `fastsim::run`, which must give
//!   the same `Outcome` (`Debug` text: tallies, failure counts, first failures with messages).
//!   The circuits are mirrored (a random forward part, then its inverse), so most lanes pass and
//!   the memoized tracker path is exercised, with `Givens`, `SpinSwap`, frames, conditions and
//!   measurement-based uncomputation; injected faults make some batches fail.
//! - kernel tests: the fast kernels against the reference on random and adversarial vectors.
use super::exec::Exec;
use super::{decode_calls, gauss, kernels};
use crate::circuit::NONE;
use crate::fiat_shamir::{hmr_stream, Lane};
use crate::sim::compile::{Compiled, Layout, SimOp};
use crate::sim::gaussian::{self, Tv};
use crate::sim::lanes::{Lanes, ReflectLanes, LANES, W};
use crate::sim::tracker::{LaneFrame, LaneTracker, TrackerFactory};
use crate::sim::validate::{self, Context};
use crate::spec::{Monomial, SystemOp};
use std::collections::BTreeMap;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick(&mut self, v: &[u32]) -> u32 {
        v[self.below(v.len() as u64) as usize]
    }
    fn coin(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

/// A tracker that only logs its calls; `finish` returns the log as its error text.
struct Log(Vec<(LaneFrame, usize, usize, u64)>);

impl LaneTracker for Log {
    fn givens(&mut self, before: &LaneFrame, p: usize, angle: u64) -> Result<(), String> {
        self.givens_modes(before, p, p + 1, angle)
    }
    fn givens_modes(
        &mut self,
        before: &LaneFrame,
        p: usize,
        q: usize,
        angle: u64,
    ) -> Result<(), String> {
        self.0.push((before.clone(), p, q, angle));
        Ok(())
    }
    fn finish(&mut self, _: &LaneFrame, _: Option<&SystemOp>) -> Result<(), String> {
        Err(format!("{:?}", self.0))
    }
}

struct LogFactory;

impl TrackerFactory for LogFactory {
    fn lane(&self, _: usize) -> Box<dyn LaneTracker> {
        Box::new(Log(Vec::new()))
    }
    fn givens_toffoli_cost(&self) -> f64 {
        1.0
    }
    fn phase_gradient_qubits(&self) -> u64 {
        5
    }
    fn quarter_turn(&self) -> Option<u64> {
        Some(8)
    }
    fn givens_charge(&self, p: usize, q: usize) -> Option<u64> {
        Some((7 * p + q) as u64)
    }
}

const SYS: usize = 6;
const UNI: u32 = 4;
const ANC: u32 = 8;
const BITS: u32 = 6;

fn layout() -> Layout {
    Layout {
        system: SYS,
        uniform: UNI,
    }
}

fn compiled(ops: Vec<SimOp>, num_qubits: u32, num_bits: u32, registers: Vec<Vec<u32>>) -> Compiled {
    Compiled {
        ops,
        num_qubits,
        num_bits,
        registers,
        q_peak: 0,
        segment_ancilla_peak: BTreeMap::new(),
        uses_givens: true,
    }
}

fn nonsys(l: &Layout, num_qubits: u32) -> Vec<u32> {
    std::iter::once(0)
        .chain(1 + l.system as u32..num_qubits)
        .collect()
}

/// A random op of `kind` on the unit-test layout.
fn random_op(r: &mut Rng, kind: usize, cond: u32, num_qubits: u32) -> SimOp {
    let l = layout();
    let ns = nonsys(&l, num_qubits);
    let two = |r: &mut Rng| loop {
        let (a, b) = (r.pick(&ns), r.pick(&ns));
        if a != b {
            return (a, b);
        }
    };
    let sq = r.below(SYS as u64) as u32;
    let ctrl = |r: &mut Rng| match r.below(3) {
        0 => [NONE, NONE],
        1 => [r.pick(&ns), NONE],
        _ => [r.pick(&ns), r.pick(&ns)],
    };
    match kind {
        0 => SimOp::X {
            t: r.pick(&ns),
            cond,
        },
        1 => {
            let (c, t) = two(r);
            SimOp::Cx { c, t, cond }
        }
        2 => {
            let (a, t) = two(r);
            let b = loop {
                let b = r.pick(&ns);
                if b != t {
                    break b;
                }
            };
            SimOp::Ccx { a, b, t, cond }
        }
        3 => {
            let (a, b) = two(r);
            SimOp::Swap { a, b, cond }
        }
        4 => {
            let mut q = [NONE; 3];
            for x in q.iter_mut().take(1 + r.below(3) as usize) {
                *x = r.pick(&ns);
            }
            SimOp::Phase {
                q,
                k: [2u8, 4, 6][r.below(3) as usize],
                cond,
            }
        }
        5 => SimOp::Neg { cond },
        6 => SimOp::Hmr {
            t: r.pick(&ns),
            bit: r.below(u64::from(BITS)) as u32,
            cond,
        },
        7 => SimOp::Reset { t: r.pick(&ns) },
        8 => SimOp::Bit {
            bit: r.below(u64::from(BITS)) as u32,
            kind: r.below(3) as u8,
            cond,
        },
        9 => SimOp::Sys {
            q: sq,
            z: r.coin(1, 2),
            ctrl: ctrl(r),
            cond,
        },
        10 => SimOp::SysS {
            q: sq,
            k: if r.coin(1, 2) { 1 } else { 3 },
            cond,
        },
        11 => {
            let p = r.below(SYS as u64 - 1) as u32;
            let q = p + 1 + r.below(SYS as u64 - 1 - u64::from(p)) as u32;
            SimOp::Givens {
                p,
                q,
                reg: r.below(2) as u32,
                cond,
            }
        }
        12 => SimOp::SpinSwap {
            c: r.pick(&ns),
            cond,
            dagger: r.coin(1, 2),
        },
        _ => SimOp::Reflect,
    }
}

const KINDS: usize = 14;

fn random_words(r: &mut Rng, n: usize) -> Vec<u64> {
    (0..n).map(|_| r.next()).collect()
}

/// One op kind on random states: reference `Lanes` vs fast `Exec`.
fn check_kind(kind: usize, seed: u64) {
    let mut r = Rng(seed);
    let l = layout();
    let num_qubits = l.first_ancilla() + ANC;
    let registers = vec![
        (1 + l.system as u32..1 + l.system as u32 + UNI).collect::<Vec<_>>(),
        vec![l.first_ancilla(), l.first_ancilla() + 3, 0],
    ];
    let cond = match r.below(3) {
        0 => NONE,
        _ => r.below(u64::from(BITS)) as u32,
    };
    let op = random_op(&mut r, kind, cond, num_qubits);
    let pushed = r.coin(1, 3);
    let ops = if pushed {
        vec![
            SimOp::Push {
                bit: r.below(u64::from(BITS)) as u32,
            },
            op,
            SimOp::Pop,
        ]
    } else {
        vec![op]
    };
    let c = compiled(ops.clone(), num_qubits, BITS, registers);
    let f: &dyn TrackerFactory = &LogFactory;
    let mut a = Lanes::new(&c, l.system, Some(f));
    let mut b = Exec::<W>::new(&c, l.system, Some(f));
    a.q = random_words(&mut r, num_qubits as usize * W);
    a.b = random_words(&mut r, BITS as usize * W);
    for plane in &mut a.p {
        plane.copy_from_slice(&random_words(&mut r, W));
    }
    // Frames: sparse random planes, so frames are often but not always the identity.
    let sparse = |r: &mut Rng| -> Vec<u64> {
        (0..SYS * W)
            .map(|_| r.next() & r.next() & r.next())
            .collect()
    };
    a.x = sparse(&mut r);
    a.z = sparse(&mut r);
    a.s0 = sparse(&mut r);
    a.s1 = sparse(&mut r);
    a.active.copy_from_slice(&random_words(&mut r, W));
    if r.coin(1, 4) {
        a.active = [!0; W];
    }
    let reflect = |r: &mut Rng| ReflectLanes {
        qubits: vec![1 + l.system as u32, 2 + l.system as u32],
        want: (0..2).map(|_| std::array::from_fn(|_| r.next())).collect(),
        flip: (0..2).map(|_| std::array::from_fn(|_| r.next())).collect(),
    };
    let rf = reflect(&mut r);
    // A Reflect that can pass: `want` is the lanes' own register value where `active`.
    let rf = if r.coin(1, 2) {
        ReflectLanes {
            want: rf
                .qubits
                .iter()
                .map(|&q| std::array::from_fn(|j| a.q[q as usize * W + j]))
                .collect(),
            ..rf
        }
    } else {
        rf
    };
    a.reflect = Some(ReflectLanes {
        qubits: rf.qubits.clone(),
        want: rf.want.clone(),
        flip: rf.flip.clone(),
    });
    b.reflect = Some(super::exec::WideReflect::new::<W>(&[rf]));
    b.q.clone_from(&a.q);
    b.b.clone_from(&a.b);
    b.p = a.p;
    b.x.clone_from(&a.x);
    b.z.clone_from(&a.z);
    b.s0.clone_from(&a.s0);
    b.s1.clone_from(&a.s1);
    b.active = a.active;
    b.dirty = [!0; W];
    let key = [seed as u8; 32];
    let ra = a.run(&ops, &mut hmr_stream(&key, 3));
    b.run(&ops, &mut [hmr_stream(&key, 3)]);
    let ctx = format!("kind {kind} seed {seed} ops {ops:?}");
    assert_eq!(ra.is_err(), b.failed, "{ctx}: execution error");
    assert_eq!(format!("{:?}", a.tally), format!("{:?}", b.tally), "{ctx}");
    if ra.is_err() {
        return;
    }
    assert_eq!(a.q, b.q, "{ctx}: qubits");
    assert_eq!(a.b, b.b, "{ctx}: bits");
    assert_eq!(a.p, b.p, "{ctx}: phase");
    assert_eq!(a.x, b.x, "{ctx}: x");
    assert_eq!(a.z, b.z, "{ctx}: z");
    assert_eq!(a.s0, b.s0, "{ctx}: s0");
    assert_eq!(a.s1, b.s1, "{ctx}: s1");
    let id = gauss::identity_frame(SYS);
    for lane in 0..LANES {
        let want = a.trackers[lane]
            .as_mut()
            .map(|t| t.finish(&id, None).unwrap_err());
        let got = (!b.traces[lane].is_empty()).then(|| {
            let calls: Vec<(LaneFrame, usize, usize, u64)> = decode_calls(&b.traces[lane], SYS)
                .into_iter()
                .map(|(f, p, q, a)| (f.unwrap_or_else(|| id.clone()), p, q, a))
                .collect();
            format!("{calls:?}")
        });
        assert_eq!(want, got, "{ctx}: tracker calls of lane {lane}");
        assert_eq!(
            a.lane_frame(lane, true),
            b.lane_frame(lane, true),
            "{ctx}: frame of lane {lane}"
        );
    }
}

macro_rules! op_tests {
    ($($name:ident: $kind:expr,)*) => {$(
        #[test]
        fn $name() {
            for seed in 0..300 {
                check_kind($kind, 1000 * $kind + seed);
            }
        }
    )*};
}

op_tests! {
    op_x: 0,
    op_cx: 1,
    op_ccx: 2,
    op_swap: 3,
    op_phase: 4,
    op_neg: 5,
    op_hmr: 6,
    op_reset: 7,
    op_bit: 8,
    op_sys: 9,
    op_sys_s: 10,
    op_givens: 11,
    op_spin_swap: 12,
    op_reflect: 13,
}

#[test]
fn op_sequences() {
    // Random short sequences of every kind (frames carried across Givens and SpinSwap).
    for seed in 0..400 {
        let mut r = Rng(seed);
        let l = layout();
        let num_qubits = l.first_ancilla() + ANC;
        let mut ops = Vec::new();
        for _ in 0..1 + r.below(12) {
            let kind = r.below(KINDS as u64 - 1) as usize;
            let cond = if r.coin(1, 3) {
                r.below(u64::from(BITS)) as u32
            } else {
                NONE
            };
            if kind == 7 {
                continue; // a Reset on random states almost always fails
            }
            ops.push(random_op(&mut r, kind, cond, num_qubits));
        }
        let registers = vec![
            (1 + l.system as u32..1 + l.system as u32 + UNI).collect::<Vec<_>>(),
            vec![l.first_ancilla()],
        ];
        let c = compiled(ops.clone(), num_qubits, BITS, registers);
        let f: &dyn TrackerFactory = &LogFactory;
        let mut a = Lanes::new(&c, l.system, Some(f));
        let mut b = Exec::<W>::new(&c, l.system, Some(f));
        let w = random_words(&mut r, num_qubits as usize * W);
        a.q.clone_from(&w);
        b.q = w;
        let w = random_words(&mut r, BITS as usize * W);
        a.b.clone_from(&w);
        b.b = w;
        a.active = [!0; W];
        b.active = [!0; W];
        let key = [7u8; 32];
        a.run(&ops, &mut hmr_stream(&key, seed)).unwrap();
        b.run(&ops, &mut [hmr_stream(&key, seed)]);
        assert!(!b.failed);
        assert_eq!(format!("{:?}", a.tally), format!("{:?}", b.tally));
        assert_eq!((&a.q, &a.b, a.p), (&b.q, &b.b, b.p), "seed {seed}");
        let id = gauss::identity_frame(SYS);
        for lane in 0..LANES {
            let want = a.trackers[lane]
                .as_mut()
                .map(|t| t.finish(&id, None).unwrap_err());
            let got = (!b.traces[lane].is_empty()).then(|| {
                let calls: Vec<(LaneFrame, usize, usize, u64)> = decode_calls(&b.traces[lane], SYS)
                    .into_iter()
                    .map(|(f, p, q, a)| (f.unwrap_or_else(|| id.clone()), p, q, a))
                    .collect();
                format!("{calls:?}")
            });
            assert_eq!(want, got, "seed {seed} lane {lane}");
            assert_eq!(a.lane_frame(lane, true), b.lane_frame(lane, true));
        }
    }
}

/// A mirrored random circuit on `n` system qubits: forward units, then their inverses in
/// reverse order, then `R` on every ancilla. Every lane's operator is the identity, so with the
/// identity reference every lane passes unless `fault` drops one inverse op.
struct Mirror {
    ops: Vec<SimOp>,
    /// Where the forward part ends (a nested circuit's `Reflect` goes here).
    mid: usize,
    num_qubits: u32,
    num_bits: u32,
    registers: Vec<Vec<u32>>,
}

#[allow(clippy::too_many_lines)]
fn mirror(r: &mut Rng, n: usize, u: u32, beta: u32, units: usize, fault: bool) -> Mirror {
    let l = Layout {
        system: n,
        uniform: u,
    };
    let first = l.first_ancilla();
    let anc = 10u32;
    let one = first + anc; // register 1: a single qubit, X'ed to 1 around an inverse Givens
    let mut fresh_q = one + 1;
    let mut fresh_b = 4u32;
    let uni: Vec<u32> = (1 + n as u32..1 + n as u32 + u).collect();
    let ctrls: Vec<u32> = std::iter::once(0)
        .chain(uni.iter().copied())
        .chain(first..first + anc)
        .collect();
    let ancs: Vec<u32> = (first..first + anc).collect();
    let angle_reg: Vec<u32> = uni.iter().copied().take(beta as usize).collect();
    let registers = vec![angle_reg.clone(), vec![one]];
    let mut fwd: Vec<Vec<SimOp>> = Vec::new();
    let mut inv: Vec<Vec<SimOp>> = Vec::new();
    let mut depth = 0;
    for _ in 0..units {
        let cond = if r.coin(1, 4) {
            r.below(4) as u32
        } else {
            NONE
        };
        let (f, i) = match r.below(16) {
            0 => {
                let t = r.pick(&ancs);
                (vec![SimOp::X { t, cond }], vec![SimOp::X { t, cond }])
            }
            1 => {
                let c = r.pick(&ctrls);
                let t = r.pick(&ancs);
                if c == t {
                    continue;
                }
                let o = SimOp::Cx { c, t, cond };
                (vec![o], vec![o])
            }
            2 | 3 => {
                let (a, b) = (r.pick(&ctrls), r.pick(&ctrls));
                if a == b {
                    continue;
                }
                if r.coin(1, 2) {
                    // Measurement-based uncomputation of a fresh AND.
                    let t = fresh_q;
                    fresh_q += 1;
                    let bit = fresh_b;
                    fresh_b += 1;
                    (
                        vec![SimOp::Ccx { a, b, t, cond }],
                        vec![
                            SimOp::Hmr { t, bit, cond },
                            SimOp::Push { bit },
                            SimOp::Phase {
                                q: [a, b, NONE],
                                k: 4,
                                cond,
                            },
                            SimOp::Pop,
                        ],
                    )
                } else {
                    let t = r.pick(&ancs);
                    if t == a || t == b {
                        continue;
                    }
                    let o = SimOp::Ccx { a, b, t, cond };
                    (vec![o], vec![o])
                }
            }
            4 => {
                let (a, b) = (r.pick(&ancs), r.pick(&ancs));
                if a == b {
                    continue;
                }
                let o = SimOp::Swap { a, b, cond };
                (vec![o], vec![o])
            }
            5 => {
                let mut q = [NONE; 3];
                for x in q.iter_mut().take(1 + r.below(3) as usize) {
                    *x = r.pick(&ctrls);
                }
                let k = [2u8, 4, 6][r.below(3) as usize];
                (
                    vec![SimOp::Phase { q, k, cond }],
                    vec![SimOp::Phase {
                        q,
                        k: (8 - k) % 8,
                        cond,
                    }],
                )
            }
            6 => (vec![SimOp::Neg { cond }], vec![SimOp::Neg { cond }]),
            7..=9 => {
                let ctrl = match r.below(3) {
                    0 => [NONE, NONE],
                    1 => [r.pick(&ctrls), NONE],
                    _ => [r.pick(&ctrls), r.pick(&ctrls)],
                };
                let o = SimOp::Sys {
                    q: r.below(n as u64) as u32,
                    z: r.coin(1, 2),
                    ctrl,
                    cond,
                };
                (vec![o], vec![o])
            }
            10 => {
                // S pairs (a Pauli), rarely a lone S (a tracker rejection at a later Givens).
                let q = r.below(n as u64) as u32;
                let k = if r.coin(1, 2) { 1 } else { 3 };
                let mut f = vec![SimOp::SysS { q, k, cond }];
                let mut i = vec![SimOp::SysS { q, k: 4 - k, cond }];
                if !r.coin(1, 10) {
                    f.push(SimOp::SysS { q, k, cond });
                    i.insert(0, SimOp::SysS { q, k: 4 - k, cond });
                }
                (f, i)
            }
            11..=13 => {
                let p = r.below(n as u64 - 1) as u32;
                let q = p + 1 + r.below(n as u64 - 1 - u64::from(p)) as u32;
                let g = |reg| SimOp::Givens { p, q, reg, cond };
                // G(a)^-1 = G(~a) G(1) (mod 2^beta).
                let mut i: Vec<SimOp> = angle_reg
                    .iter()
                    .map(|&t| SimOp::X { t, cond: NONE })
                    .collect();
                i.push(g(0));
                i.extend(angle_reg.iter().map(|&t| SimOp::X { t, cond: NONE }));
                i.push(SimOp::X { t: one, cond: NONE });
                i.push(g(1));
                i.push(SimOp::X { t: one, cond: NONE });
                (vec![g(0)], i)
            }
            14 => {
                let c = r.pick(&ctrls);
                let d = r.coin(1, 2);
                (
                    vec![SimOp::SpinSwap { c, cond, dagger: d }],
                    vec![SimOp::SpinSwap {
                        c,
                        cond,
                        dagger: !d,
                    }],
                )
            }
            _ => {
                let bit = r.below(4) as u32;
                if depth < 2 && r.coin(1, 2) {
                    depth += 1;
                    (vec![SimOp::Push { bit }], vec![SimOp::Pop])
                } else if depth > 0 {
                    depth -= 1;
                    (vec![SimOp::Pop], vec![SimOp::Push { bit: NONE }])
                } else {
                    let o = SimOp::Bit { bit, kind: 0, cond };
                    (vec![o], vec![o])
                }
            }
        };
        fwd.push(f);
        inv.push(i);
    }
    while depth > 0 {
        depth -= 1;
        fwd.push(vec![SimOp::Pop]);
        inv.push(vec![SimOp::Push { bit: NONE }]);
    }
    // Push/Pop mirror: a forward Push{b} ... Pop becomes Push{b} ... Pop in the inverse, so
    // the inverse of a Pop unit opens with the matching Push's bit. Pair them up.
    let mut stack = Vec::new();
    for (k, f) in fwd.iter().enumerate() {
        match f.first() {
            Some(SimOp::Push { bit }) if f.len() == 1 => stack.push((k, *bit)),
            Some(SimOp::Pop) if f.len() == 1 => {
                if let Some((_, bit)) = stack.pop() {
                    inv[k] = vec![SimOp::Push { bit }];
                }
            }
            _ => {}
        }
    }
    let mut ops: Vec<SimOp> = fwd.into_iter().flatten().collect();
    let mid = ops.len();
    let mut tail: Vec<SimOp> = inv.into_iter().rev().flatten().collect();
    if fault && !tail.is_empty() {
        let k = r.below(tail.len() as u64) as usize;
        if !matches!(tail[k], SimOp::Push { .. } | SimOp::Pop) {
            tail.remove(k);
        }
    }
    ops.extend(tail);
    // A faulty circuit sometimes leaves its ancillas unfreed (the end-of-lane check).
    if !(fault && r.coin(1, 2)) {
        for t in first..fresh_q {
            ops.push(SimOp::Reset { t });
        }
    }
    Mirror {
        ops,
        mid,
        num_qubits: fresh_q,
        num_bits: fresh_b,
        registers,
    }
}

fn identity_ref(_: u64) -> SystemOp {
    SystemOp::Monomial(Monomial {
        phase: 0,
        majoranas: Vec::new(),
    })
}

fn same_after(s: u64) -> u64 {
    s
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Plain,
    /// A nested lane map: a `Reflect` between the two halves, second-pass inner values.
    Nested,
    /// `Rotated` references (`(G gamma_a G^dagger)^2`, the identity up to rounding), with
    /// networks shared between lanes (their `Arc` addresses are part of the memo key).
    Rotated,
}

fn compare_runs(seed: u64, threads: usize, fault: bool, mode: Mode) -> (bool, bool) {
    let mut r = Rng(seed);
    let n = 2 + 2 * r.below(4) as usize;
    let u = 5 + r.below(3) as u32;
    let beta = (3 + r.below(3) as u32).min(u - 2);
    let units = 5 + r.below(60) as usize;
    let mut m = mirror(&mut r, n, u, beta, units, fault);
    if matches!(mode, Mode::Nested) {
        m.ops.insert(m.mid, SimOp::Reflect);
    }
    let c = compiled(m.ops, m.num_qubits, m.num_bits, m.registers);
    let tracker = gaussian::factory(beta);
    let nets: Vec<std::sync::Arc<crate::spec::Network>> = (0..3)
        .map(|_| {
            std::sync::Arc::new(crate::spec::Network {
                beta: 5,
                rotations: (0..1 + r.below(6))
                    .map(|_| {
                        let p = r.below(n as u64 - 1) as u16;
                        let q = p + 1 + r.below(n as u64 - 1 - u64::from(p)) as u16;
                        (p, q, r.below(32) as u32)
                    })
                    .collect(),
            })
        })
        .collect();
    let rotated = |s: u64| {
        if s % 4 == 3 {
            return identity_ref(s);
        }
        let part = crate::spec::RotatedPart {
            network: std::sync::Arc::clone(&nets[(s % 3) as usize]),
            spin: None,
            majoranas: vec![(s % (2 * n as u64)) as u16],
        };
        SystemOp::Rotated(crate::spec::Rotated {
            phase: 0,
            parts: vec![part.clone(), part],
        })
    };
    let reference: &(dyn Fn(u64) -> SystemOp + Sync) = match mode {
        Mode::Rotated => &rotated,
        _ => &identity_ref,
    };
    let ctx = Context {
        compiled: &c,
        layout: Layout {
            system: n,
            uniform: u,
        },
        hmr_key: [seed as u8; 32],
        reference,
        uniform_after: &same_after,
        tracker: tracker.map(|t| t as &dyn TrackerFactory),
    };
    let k = 1 + r.below(10 * LANES as u64) as usize;
    let lanes: Vec<Lane> = (0..k)
        .map(|_| Lane {
            c: r.coin(1, 2),
            s: r.below(1 << u),
        })
        .collect();
    // Nested: inner register = the top two uniform bits; half the lanes are diagonal.
    let (lo, width) = (u - 2, 2u32);
    let after: Vec<u64> = lanes
        .iter()
        .map(|l| {
            if r.coin(1, 2) {
                l.s
            } else {
                (l.s & ((1 << lo) - 1)) | (r.below(4) << lo)
            }
        })
        .collect();
    let nested_ref = |_: u64, _: u64| identity_ref(0);
    let nested = validate::Nested {
        lo,
        width,
        after: &after,
        reference: &nested_ref,
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    let want = pool.install(|| match mode {
        Mode::Nested => validate::run_nested(&ctx, &nested, &lanes),
        _ => validate::run(&ctx, &lanes),
    });
    // Every pass width (batches per pass) gives the reference's outcome.
    for width in [1, 2, 4, 8] {
        let got = pool.install(|| match mode {
            Mode::Nested => super::run_width(&ctx, Some(&nested), &lanes, width, None),
            _ => super::run_width(&ctx, None, &lanes, width, None),
        });
        assert_eq!(
            format!("{want:?}"),
            format!("{got:?}"),
            "seed {seed} threads {threads} fault {fault} {mode:?} width {width}"
        );
    }
    let givens = c
        .ops
        .iter()
        .any(|o| matches!(o, SimOp::Givens { .. } | SimOp::SpinSwap { .. }));
    (want.rejection().is_none(), givens)
}

#[test]
fn runs_nested_and_rotated_agree() {
    let mut passed = [0usize; 2];
    for seed in 2000..2100 {
        for threads in [1, 4] {
            for (i, mode) in [Mode::Nested, Mode::Rotated].into_iter().enumerate() {
                passed[i] += usize::from(compare_runs(seed, threads, seed % 5 == 0, mode).0);
            }
        }
    }
    assert!(passed[0] > 20 && passed[1] > 40, "passing runs: {passed:?}");
}

#[test]
fn runs_mirrored_circuits_pass_and_agree() {
    let (mut passed, mut tracked) = (0, 0);
    for seed in 0..120 {
        for threads in [1, 4] {
            let (ok, givens) = compare_runs(seed, threads, false, Mode::Plain);
            passed += usize::from(ok);
            tracked += usize::from(ok && givens);
        }
    }
    // Most mirrored circuits pass (a lone S before a Givens is the designed exception), most
    // of them through the tracker.
    assert!(passed > 120, "only {passed} of 240 runs passed");
    assert!(
        tracked > 100,
        "only {tracked} passing runs used the tracker"
    );
}

#[test]
fn runs_with_faults_agree() {
    let mut rejected = 0;
    for seed in 1000..1120 {
        for threads in [1, 4] {
            rejected += usize::from(!compare_runs(seed, threads, true, Mode::Plain).0);
        }
    }
    assert!(
        rejected > 60,
        "only {rejected} of 240 faulty runs were rejected"
    );
}

/// Lanes with the same tracker calls but different final phases (or frames) must not share a
/// verdict: a `Givens` on every lane, then a phase (or a system `Z`) on lanes whose uniform bit
/// `k` is 1, never undone.
#[test]
fn equal_traces_with_different_final_frames() {
    let l = Layout {
        system: 4,
        uniform: 4,
    };
    let u = |k: u32| 1 + l.system as u32 + k;
    for (tail, name) in [
        (
            SimOp::Phase {
                q: [u(3), NONE, NONE],
                k: 4,
                cond: NONE,
            },
            "phase 4",
        ),
        (
            SimOp::Phase {
                q: [u(3), NONE, NONE],
                k: 2,
                cond: NONE,
            },
            "phase 2",
        ),
        (
            SimOp::Sys {
                q: 1,
                z: true,
                ctrl: [u(3), NONE],
                cond: NONE,
            },
            "Z",
        ),
        (
            SimOp::Sys {
                q: 2,
                z: false,
                ctrl: [u(3), NONE],
                cond: NONE,
            },
            "X",
        ),
    ] {
        let ops = vec![
            SimOp::Sys {
                q: 0,
                z: true,
                ctrl: [NONE, NONE],
                cond: NONE,
            },
            SimOp::Givens {
                p: 0,
                q: 2,
                reg: 0,
                cond: NONE,
            },
            SimOp::Sys {
                q: 0,
                z: true,
                ctrl: [NONE, NONE],
                cond: NONE,
            },
            tail,
        ];
        // The angle register is uniform bits 0..3 held at 0 by the lanes below.
        let c = compiled(ops, l.first_ancilla(), 1, vec![vec![u(0), u(1), u(2)]]);
        let tracker = gaussian::factory(3).map(|t| t as &dyn TrackerFactory);
        let ctx = Context {
            compiled: &c,
            layout: l,
            hmr_key: [1; 32],
            reference: &identity_ref,
            uniform_after: &same_after,
            tracker,
        };
        let lanes: Vec<Lane> = (0..3 * LANES as u64)
            .map(|i| Lane {
                c: i % 3 == 0,
                s: (i / 5 % 2) << 3,
            })
            .collect();
        let want = validate::run(&ctx, &lanes);
        let got = super::run(&ctx, None, &lanes);
        // Only the lanes with uniform bit 3 set fail.
        let failed: u64 = want.failed.iter().sum();
        assert!(
            failed > 0 && failed < lanes.len() as u64,
            "{name}: {failed} failed"
        );
        assert_eq!(format!("{want:?}"), format!("{got:?}"), "{name}");
    }
}

fn lcg_f(r: &mut Rng) -> f64 {
    (r.next() >> 11) as f64 / (1u64 << 53) as f64 - 0.5
}

fn random_ys(r: &mut Rng, m: usize, n: usize, zeros: u64) -> Vec<Tv> {
    (0..m)
        .map(|_| Tv {
            odd: r.coin(1, 2),
            v: (0..n)
                .map(|_| {
                    if r.coin(zeros, 10) {
                        if r.coin(1, 2) {
                            0.0
                        } else {
                            -0.0
                        }
                    } else {
                        lcg_f(r)
                    }
                })
                .collect(),
        })
        .collect()
}

/// The reference `ad_residual` is private; `GaussianLane::measure`'s `off` is it. This is the
/// same function, copied from `crate::sim::gaussian` as the oracle for the kernels.
fn ad_residual_oracle(u: Option<&[f64]>, ys: &[&Tv], n: usize) -> f64 {
    let mut worst = 0.0f64;
    for block in [false, true] {
        let flip = ys.iter().filter(|y| y.odd != block).count() % 2 == 1;
        let same: Vec<&[f64]> = ys
            .iter()
            .filter(|y| y.odd == block)
            .map(|y| y.v.as_slice())
            .collect();
        let mut y = vec![0.0; n];
        for j in 0..n {
            match u {
                Some(u) => (0..n).for_each(|r| y[r] = u[r * n + j]),
                None => (0..n).for_each(|r| y[r] = f64::from(u8::from(r == j))),
            }
            for w in same.iter().rev() {
                let d = 2.0 * w.iter().zip(&y).map(|(a, b)| a * b).sum::<f64>();
                y.iter_mut()
                    .zip(w.iter())
                    .for_each(|(a, b)| *a = d * b - *a);
            }
            let sign = if flip { -1.0 } else { 1.0 };
            for (r, v) in y.iter().enumerate() {
                let e = (sign * v - f64::from(u8::from(r == j))).abs();
                if e.is_nan() {
                    return f64::INFINITY;
                }
                worst = worst.max(e);
            }
        }
    }
    worst
}

#[test]
fn sum_start_is_the_std_fold() {
    let mut r = Rng(5);
    for _ in 0..200 {
        let v: Vec<f64> = (0..r.below(40))
            .map(|_| if r.coin(1, 4) { -0.0 } else { lcg_f(&mut r) })
            .collect();
        let mut acc = kernels::sum_start();
        for x in &v {
            acc += x;
        }
        assert_eq!(acc.to_bits(), v.iter().sum::<f64>().to_bits());
    }
    assert_eq!(
        std::iter::empty::<f64>().sum::<f64>().to_bits(),
        kernels::sum_start().to_bits()
    );
}

#[test]
fn ad_residual_kernels_match_the_reference() {
    let mut r = Rng(11);
    for case in 0..300 {
        let n = 1 + r.below(20) as usize;
        let m = r.below(30) as usize;
        let z = r.below(9);
        let ys = random_ys(&mut r, m, n, z);
        let refs: Vec<&Tv> = ys.iter().collect();
        let u: Option<Vec<f64>> = r.coin(2, 3).then(|| {
            (0..n * n)
                .map(|_| if r.coin(1, 3) { 0.0 } else { lcg_f(&mut r) })
                .collect()
        });
        let want = ad_residual_oracle(u.as_deref(), &refs, n);
        let exact = kernels::ad_residual_exact(u.as_deref(), &refs, n);
        let fast = kernels::ad_residual(u.as_deref(), &refs, n);
        assert_eq!(want.to_bits(), exact.to_bits(), "case {case}");
        assert_eq!(want.to_bits(), fast.to_bits(), "case {case}");
    }
    // Non-finite inputs go to the exact kernel, whose NaN rule is the reference's.
    let mut ys = random_ys(&mut r, 6, 5, 3);
    ys[2].v[1] = f64::NAN;
    ys[4].v[0] = f64::INFINITY;
    let refs: Vec<&Tv> = ys.iter().collect();
    let want = ad_residual_oracle(None, &refs, 5);
    assert_eq!(
        want.to_bits(),
        kernels::ad_residual(None, &refs, 5).to_bits()
    );
}

#[test]
fn pfaffian_kernel_matches_the_reference_up_to_zero_signs() {
    let mut r = Rng(13);
    let same = |a: f64, b: f64| gauss::same_up_to_zero_sign(a, b);
    for case in 0..400 {
        let n = 1 + r.below(12) as usize;
        let m = 2 * r.below(14) as usize + usize::from(r.coin(1, 8));
        let z = r.below(9);
        let mut ys = random_ys(&mut r, m, n, z);
        // Repeated and opposite vectors: zero heads, exact cancellations, pivot ties.
        if m >= 4 && r.coin(1, 2) {
            let src = ys[r.below(m as u64) as usize].v.clone();
            let k = r.below(m as u64) as usize;
            ys[k].v = if r.coin(1, 2) {
                src
            } else {
                src.iter().map(|x| -x).collect()
            };
        }
        let refs: Vec<&Tv> = ys.iter().collect();
        let want = gaussian::pfaffian::vacuum_expectation(&refs);
        match kernels::vacuum_expectation(&refs) {
            Some(got) => assert!(
                same(got.re, want.re) && same(got.im, want.im),
                "case {case}: {got:?} vs {want:?}"
            ),
            None => panic!("case {case}: the kernel declined finite inputs"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The guarded fold (`fold`, `gauss::fold_probe`): the bound holds on every case, a certificate
// is only issued when the reference's residual is within tolerance, and the verdict is the
// reference `GaussianLane::finish` verdict.

fn pauli_frame(r: &mut Rng, n: usize, kind: u64) -> LaneFrame {
    let words = n.div_ceil(64);
    let mut f = gauss::identity_frame(n);
    match kind {
        // A Z string (pairs only after conversion).
        0 => {
            for q in 0..n {
                if r.coin(1, 2) {
                    f.z[q / 64] |= 1 << (q % 64);
                }
            }
        }
        // A general Pauli frame (unpaired Majoranas too), with S^2 = Z factors.
        1 => {
            for q in 0..n {
                if r.coin(1, 3) {
                    f.x[q / 64] |= 1 << (q % 64);
                }
                if r.coin(1, 2) {
                    f.z[q / 64] |= 1 << (q % 64);
                }
                if r.coin(1, 6) {
                    f.s_pow[q] = 2;
                }
            }
        }
        // A few X's.
        _ => {
            for _ in 0..1 + r.below(2) {
                let q = r.below(n as u64) as usize;
                f.x[q / 64] ^= 1 << (q % 64);
            }
        }
    }
    let _ = words;
    f
}

/// Frame of the Majorana product `gamma_m1 ... gamma_md` (phase dropped): JW bits.
fn majorana_frame(ms: &[u16], n: usize) -> LaneFrame {
    let mut f = gauss::identity_frame(n);
    for &m in ms {
        let j = usize::from(m) / 2;
        f.x[j / 64] ^= 1 << (j % 64);
        for q in 0..j + usize::from(m) % 2 {
            f.z[q / 64] ^= 1 << (q % 64);
        }
    }
    f
}

struct FoldCase {
    calls: Vec<(Option<LaneFrame>, usize, usize, u64)>,
    after: LaneFrame,
    reference: Option<SystemOp>,
}

/// Mirrored calls (`A^-1 A` up to a phase): random rotations with random frames, then the
/// inverse rotations with the frames mirrored. Frames are Z strings, general Paulis or a few X.
fn mirrored_case(r: &mut Rng, n: usize, beta: u8, m: usize, kinds: u64, frames: u64) -> FoldCase {
    let full = 1u64 << beta;
    let fwd: Vec<(Option<LaneFrame>, usize, usize, u64)> = (0..m)
        .map(|_| {
            let p = r.below(n as u64 - 1) as usize;
            let q = p + 1 + r.below((n - p - 1) as u64) as usize;
            // About `frames` hand-offs per lane (the reference refuses more than MAX_FACTORS
            // Majoranas, which would only test the refusal).
            let frame = if r.below(m as u64) < frames {
                let k = r.below(kinds);
                Some(pauli_frame(r, n, k))
            } else {
                None
            };
            (frame, p, q, r.below(full))
        })
        .collect();
    let mut calls = fwd.clone();
    // A^-1 = F_1 G_1^-1 F_2 G_2^-1 ... F_m G_m^-1: calls (I, G_m^-1), (F_m, G_m-1^-1), ...
    for t in (0..m).rev() {
        let frame = if t + 1 < m {
            fwd[t + 1].0.clone()
        } else {
            None
        };
        let (_, p, q, a) = fwd[t];
        calls.push((frame, p, q, (full - a) % full));
    }
    let after = fwd[0].0.clone().unwrap_or_else(|| gauss::identity_frame(n));
    FoldCase {
        calls,
        after,
        reference: None,
    }
}

/// A lane implementing the `Rotated` reference `G m G^dagger` exactly as a network: the inverse
/// rotations, then the frame of `m`, then the rotations.
fn rotated_case(r: &mut Rng, n: usize, beta: u8, m: usize, off_by_one: bool) -> FoldCase {
    let full = 1u64 << beta;
    let rots: Vec<(u16, u16, u32)> = (0..m)
        .map(|_| {
            let p = r.below(n as u64 - 1) as u16;
            let q = p + 1 + r.below(n as u64 - 1 - u64::from(p)) as u16;
            (p, q, r.below(full) as u32)
        })
        .collect();
    let mut ms: Vec<u16> = (0..2 * n as u16).filter(|_| r.coin(1, 4)).collect();
    if ms.is_empty() {
        ms.push(r.below(2 * n as u64) as u16);
    }
    let mut calls = Vec::new();
    for &(p, q, a) in rots.iter().rev() {
        calls.push((
            None,
            usize::from(p),
            usize::from(q),
            (full - u64::from(a)) % full,
        ));
    }
    for (i, &(p, q, a)) in rots.iter().enumerate() {
        let frame = (i == 0).then(|| majorana_frame(&ms, n));
        let a = if off_by_one && i == m / 2 {
            u64::from(a) + 1
        } else {
            u64::from(a)
        };
        calls.push((frame, usize::from(p), usize::from(q), a % full));
    }
    let reference = SystemOp::Rotated(crate::spec::Rotated {
        phase: 0,
        parts: vec![crate::spec::RotatedPart {
            network: std::sync::Arc::new(crate::spec::Network {
                beta,
                rotations: rots,
            }),
            spin: None,
            majoranas: ms,
        }],
    });
    FoldCase {
        calls,
        after: gauss::identity_frame(n),
        reference: Some(reference),
    }
}

fn reference_finish(c: &FoldCase, n: usize, beta: u8) -> bool {
    let mut g = gaussian::GaussianLane::new(n, beta);
    let id = gauss::identity_frame(n);
    for (f, p, q, a) in &c.calls {
        if g.givens_modes(f.as_ref().unwrap_or(&id), *p, *q, *a)
            .is_err()
        {
            return false;
        }
    }
    g.finish(&c.after, c.reference.as_ref()).is_ok()
}

fn probe(c: &FoldCase, n: usize, beta: u8) -> gauss::Probe {
    gauss::fold_probe(
        beta,
        None,
        n,
        c.calls.iter().map(|(f, p, q, a)| gauss::Call {
            frame: f.as_ref(),
            p: *p,
            q: *q,
            angle: *a,
        }),
        &c.after,
        c.reference.as_ref(),
        true,
        &super::fold::GuardCounts::default(),
    )
    .expect("finite kernels")
}

/// Checks one case: the bound, the certificate's meaning, and the verdict.
fn check_fold(c: &FoldCase, n: usize, beta: u8, what: &str) -> gauss::Probe {
    let p = probe(c, n, beta);
    let want = reference_finish(c, n, beta);
    assert_eq!(p.verdict, want, "{what}: verdict");
    if let (Some(g), Some(off)) = (p.g, p.off_ref) {
        assert!(
            (off - p.off_fold).abs() <= g,
            "{what}: |{off:e} - {:e}| > g = {g:e}",
            p.off_fold
        );
        if p.certified {
            assert!(off <= gaussian::AD_TOL, "{what}: certified {off:e}");
        }
    }
    // The engine's own path (no exact residual when certified) gives the same verdict.
    let fast = gauss::verdict(
        Some((beta, None)),
        &super::fold::GuardCounts::default(),
        &|| Box::new(gaussian::GaussianLane::new(n, beta)),
        n,
        c.calls.iter().map(|(f, p, q, a)| gauss::Call {
            frame: f.as_ref(),
            p: *p,
            q: *q,
            angle: *a,
        }),
        &c.after,
        c.reference.as_ref(),
    );
    assert_eq!(fast, want, "{what}: engine verdict");
    p
}

#[test]
fn fold_guard_holds_on_mirrored_lanes() {
    let mut r = Rng(31);
    let (mut cert, mut total) = (0, 0);
    for case in 0..120 {
        let n = 3 + r.below(14) as usize;
        let beta = 6 + r.below(20) as u8;
        let m = 1 + r.below(60) as usize;
        let c = mirrored_case(&mut r, n, beta, m, 3, 4);
        let p = check_fold(
            &c,
            n,
            beta,
            &format!("mirrored {case} (n {n}, beta {beta}, m {m})"),
        );
        total += 1;
        cert += usize::from(p.certified);
    }
    // Most mirrored lanes are the identity up to rounding: the guard certifies them.
    assert!(cert * 2 > total, "only {cert} of {total} certified");
}

#[test]
fn fold_guard_holds_on_z_string_lanes() {
    // The SA shape: Z strings handed off and rotated by many later rotations.
    let mut r = Rng(37);
    for case in 0..60 {
        let n = 8 + r.below(30) as usize;
        let beta = 10 + r.below(16) as u8;
        let m = 20 + r.below(200) as usize;
        let c = mirrored_case(&mut r, n, beta, m, 1, 3);
        let p = check_fold(&c, n, beta, &format!("z-string {case} (n {n}, m {m})"));
        let factors: usize = c
            .calls
            .iter()
            .filter_map(|(f, ..)| f.as_ref())
            .chain([&c.after])
            .map(|f| gaussian::frame_to_majoranas(f, n).map_or(0, |(_, m)| m.len()))
            .sum();
        // Without a refusal (more than MAX_FACTORS Majoranas) there is always a bound.
        assert!(
            p.g.is_some() || factors > gaussian::MAX_FACTORS,
            "z-string {case}: no bound ({factors} factors, verdict {}, off_ref {:?})",
            p.verdict,
            p.off_ref
        );
    }
}

#[test]
fn fold_guard_holds_on_rotated_references() {
    let mut r = Rng(41);
    let mut certified = 0;
    for case in 0..80 {
        let n = 3 + r.below(12) as usize;
        let beta = 8 + r.below(18) as u8;
        let m = 1 + r.below(40) as usize;
        let off = r.coin(1, 3);
        let c = rotated_case(&mut r, n, beta, m, off);
        let p = check_fold(&c, n, beta, &format!("rotated {case} (off by one {off})"));
        certified += usize::from(p.certified);
        if off && p.off_ref.is_some_and(|o| o > gaussian::AD_TOL) {
            assert!(
                !p.certified,
                "rotated {case}: a failing residual was certified"
            );
        }
    }
    assert!(certified > 0);
}

/// Adversarial for the guard: very long lanes, where the a-priori growth of `g` exceeds the
/// margin under the tolerance. The guard must decline (a fallback) and the verdict stays the
/// reference's.
#[test]
fn fold_guard_falls_back_when_the_band_reaches_the_tolerance() {
    let mut r = Rng(43);
    let n = 12;
    let beta = 30;
    let c = mirrored_case(&mut r, n, beta, 60_000, 1, 2);
    let p = check_fold(&c, n, beta, "long lane");
    let g = p.g.unwrap_or(f64::INFINITY);
    assert!(
        p.off_fold + g > gaussian::AD_TOL && !p.certified,
        "long lane: off_fold {:e}, g {g:e}, certified {}",
        p.off_fold,
        p.certified
    );
}

/// The decision rule at the threshold itself, on values a run could produce.
#[test]
fn fold_certificate_threshold() {
    use super::fold::certifies;
    let tol = gaussian::AD_TOL;
    let g = 1e-12;
    assert!(certifies(tol - 2.0 * g, Some(g)));
    assert!(
        !certifies(tol - g, Some(g)),
        "exactly at the tolerance needs the rounding margin"
    );
    assert!(!certifies(tol - g + 1e-25, Some(g)));
    assert!(!certifies(tol, Some(0.0)));
    assert!(!certifies(tol * (1.0 - 1e-17), Some(0.0)));
    assert!(certifies(tol * (1.0 - 1e-14), Some(0.0)));
    assert!(!certifies(0.0, None));
    assert!(!certifies(f64::NAN, Some(g)));
    assert!(!certifies(0.0, Some(f64::INFINITY)));
    assert!(!certifies(0.0, Some(f64::NAN)));
}

/// The Pfaffian kernel on the SA shape: vectors confined to sectors (several components that
/// interleave in logical order), pairs (the same vector in both blocks), repeated vectors
/// (pivot ties), and orthonormal families from rotations.
#[test]
fn pfaffian_kernel_matches_the_reference_on_sector_structured_inputs() {
    let mut r = Rng(17);
    let same = |a: f64, b: f64| gauss::same_up_to_zero_sign(a, b);
    for case in 0..400 {
        let sectors = 1 + r.below(3) as usize;
        let n = sectors * (2 + r.below(6) as usize);
        // A random rotation per sector, applied to basis vectors: orthonormal families.
        let mut fam: Vec<Vec<f64>> = (0..n)
            .map(|q| {
                let mut v = vec![0.0; n];
                v[q] = 1.0;
                v
            })
            .collect();
        for _ in 0..r.below(40) {
            let sec = r.below(sectors as u64) as usize;
            let modes: Vec<usize> = (0..n).filter(|q| q % sectors == sec).collect();
            if modes.len() < 2 {
                continue;
            }
            let p = modes[r.below(modes.len() as u64) as usize];
            let q = modes[r.below(modes.len() as u64) as usize];
            if p == q {
                continue;
            }
            let (c, s) = gaussian::cos_sin(r.below(1 << 12), 12);
            for v in &mut fam {
                gaussian::rotate(v, p.min(q), p.max(q), c, s);
            }
        }
        let mut ys: Vec<Tv> = Vec::new();
        let target = 2 * (1 + r.below(20) as usize);
        while ys.len() < target {
            let q = r.below(n as u64) as usize;
            let v = fam[q].clone();
            match r.below(4) {
                // A pair: even then odd, same vector.
                0 | 1 => {
                    ys.push(Tv {
                        odd: false,
                        v: v.clone(),
                    });
                    ys.push(Tv { odd: true, v });
                }
                // An unpaired factor (sometimes a repeat of an earlier vector).
                2 => ys.push(Tv {
                    odd: r.coin(1, 2),
                    v,
                }),
                _ => {
                    let k = r.below(ys.len().max(1) as u64) as usize;
                    if let Some(y) = ys.get(k).cloned() {
                        ys.push(y);
                    }
                }
            }
        }
        if r.coin(1, 4) {
            ys.pop();
        }
        let refs: Vec<&Tv> = ys.iter().collect();
        let want = gaussian::pfaffian::vacuum_expectation(&refs);
        let got = kernels::vacuum_expectation(&refs).expect("finite");
        assert!(
            same(got.re, want.re) && same(got.im, want.im),
            "case {case}: {got:?} vs {want:?}"
        );
    }
}

/// The upper triangle of the contraction matrix exactly as the reference forms it.
fn reference_upper(ys: &[&Tv]) -> (Vec<f64>, Vec<f64>) {
    let m = ys.len();
    let (mut re, mut im) = (vec![0.0; m * m], vec![0.0; m * m]);
    for i in 0..m {
        for j in i + 1..m {
            let d: f64 = ys[i].v.iter().zip(&ys[j].v).map(|(x, y)| x * y).sum();
            let (r, c) = match (ys[i].odd, ys[j].odd) {
                (false, true) => (0.0, d),
                (true, false) => (0.0, -d),
                _ => (d, 0.0),
            };
            re[i * m + j] = r;
            im[i * m + j] = c;
        }
    }
    (re, im)
}

/// Lane-like inputs: a block of pairs from one orthonormal family (the reference operator's
/// factors), then pairs from a second family close to it (the lane's factors), so the pivot
/// often leaves the pair for a far partner; with unpaired factors, repeats and odd/even order
/// varied.
fn lane_like_ys(r: &mut Rng, n: usize) -> Vec<Tv> {
    let family = |r: &mut Rng, rots: u64, base: Option<&Vec<Vec<f64>>>| -> Vec<Vec<f64>> {
        let mut fam: Vec<Vec<f64>> = match base {
            Some(b) => b.clone(),
            None => (0..n)
                .map(|q| (0..n).map(|x| f64::from(u8::from(x == q))).collect())
                .collect(),
        };
        for _ in 0..rots {
            let p = r.below(n as u64) as usize;
            let q = r.below(n as u64) as usize;
            if p == q {
                continue;
            }
            let (c, s) = gaussian::cos_sin(r.below(1 << 14), 14);
            for v in &mut fam {
                gaussian::rotate(v, p.min(q), p.max(q), c, s);
            }
        }
        fam
    };
    let a = family(r, 6 * n as u64, None);
    // The second family: the first one, rotated a little more (or not at all).
    let extra = r.below(4);
    let b = family(r, extra, Some(&a));
    let mut ys = Vec::new();
    for fam in [&a, &b] {
        let take = 1 + r.below(n as u64) as usize;
        for q in 0..take {
            let v = fam[(q * 7 + 3) % n].clone();
            match r.below(8) {
                0 => ys.push(Tv {
                    odd: r.coin(1, 2),
                    v,
                }),
                1 => {
                    ys.push(Tv {
                        odd: true,
                        v: v.clone(),
                    });
                    ys.push(Tv { odd: false, v });
                }
                2 => {
                    for _ in 0..2 {
                        ys.push(Tv {
                            odd: false,
                            v: v.clone(),
                        });
                        ys.push(Tv {
                            odd: true,
                            v: v.clone(),
                        });
                    }
                }
                _ => {
                    ys.push(Tv {
                        odd: false,
                        v: v.clone(),
                    });
                    ys.push(Tv { odd: true, v });
                }
            }
        }
    }
    if ys.len() % 2 == 1 {
        ys.pop();
    }
    ys
}

/// The delayed-update elimination (`pfblock`) equals the reference's Pfaffian up to zero signs
/// at every panel size, on random, structured and lane-like inputs large enough for several
/// panels.
#[test]
fn blocked_pfaffian_matches_the_reference() {
    let mut r = Rng(29);
    let same = |a: f64, b: f64| gauss::same_up_to_zero_sign(a, b);
    let mut swaps = 0usize;
    let mut counts = super::pfblock::Counts::default();
    for case in 0..300 {
        let ys = match case % 3 {
            0 => {
                let n = 1 + r.below(16) as usize;
                let m = 2 * (1 + r.below(40) as usize);
                let z = r.below(9);
                let mut ys = random_ys(&mut r, m, n, z);
                if r.coin(1, 2) {
                    let src = ys[r.below(m as u64) as usize].v.clone();
                    let k = r.below(m as u64) as usize;
                    ys[k].v = src;
                }
                ys
            }
            _ => {
                let n = 2 + r.below(40) as usize;
                lane_like_ys(&mut r, n)
            }
        };
        let refs: Vec<&Tv> = ys.iter().collect();
        let m = refs.len();
        if m < 2 {
            continue;
        }
        let want = gaussian::pfaffian::vacuum_expectation(&refs);
        let (re0, im0) = reference_upper(&refs);
        swaps += off_pair_pivots(&re0, &im0, m);
        for panel in [1usize, 2, 3, 5, 8, 16] {
            for links in [false, true] {
                let (mut re, mut im) = (re0.clone(), im0.clone());
                let got = super::pfblock::pfaffian_dense_counted(
                    &mut re,
                    &mut im,
                    m,
                    panel,
                    links,
                    Some(&mut counts),
                )
                .unwrap_or_else(|| panic!("case {case}: declined finite inputs"));
                assert!(
                    same(got.re, want.re) && same(got.im, want.im),
                    "case {case} (m {m}, panel {panel}, links {links}): {got:?} vs {want:?}"
                );
            }
        }
    }
    // The inputs must exercise the swap path and the linked columns, often.
    assert!(swaps > 1000, "only {swaps} off-pair pivots");
    assert!(
        counts.links > 1000 && counts.skipped > counts.updates / 20,
        "links barely exercised: {counts:?}"
    );
}

/// The real-coefficient elimination (`pfreal`) equals the reference's vacuum overlap up to zero
/// signs at every panel size, on random (with zeros, repeats and negated repeats: zero heads,
/// cancellations, pivot ties), structured and lane-like inputs large enough for several panels.
#[test]
fn real_pfaffian_matches_the_reference() {
    let mut r = Rng(31);
    let same = |a: f64, b: f64| gauss::same_up_to_zero_sign(a, b);
    let mut swaps = 0usize;
    let mut zero_results = 0usize;
    for case in 0..600 {
        let ys = match case % 3 {
            0 => {
                let n = 1 + r.below(16) as usize;
                let m = 2 * (1 + r.below(40) as usize);
                let z = r.below(9);
                let mut ys = random_ys(&mut r, m, n, z);
                for _ in 0..r.below(3) {
                    let src = ys[r.below(m as u64) as usize].v.clone();
                    let k = r.below(m as u64) as usize;
                    ys[k].v = if r.coin(1, 2) {
                        src
                    } else {
                        src.iter().map(|x| -x).collect()
                    };
                }
                ys
            }
            _ => {
                let n = 2 + r.below(40) as usize;
                lane_like_ys(&mut r, n)
            }
        };
        let refs: Vec<&Tv> = ys.iter().collect();
        let m = refs.len();
        if m < 2 {
            continue;
        }
        let want = gaussian::pfaffian::vacuum_expectation(&refs);
        if want.re == 0.0 && want.im == 0.0 {
            zero_results += 1;
        }
        let (re0, im0) = reference_upper(&refs);
        swaps += off_pair_pivots(&re0, &im0, m);
        let par0: Vec<bool> = refs.iter().map(|y| y.odd).collect();
        let c0 = super::pfreal::coefficients(&re0, &im0, &par0).expect("off-class parts are zero");
        for panel in [1usize, 2, 3, 5, 8, 16, 32] {
            let (mut c, mut par) = (c0.clone(), par0.clone());
            let got = super::pfreal::pfaffian_real(&mut c, &mut par, m, panel)
                .unwrap_or_else(|| panic!("case {case}: declined finite inputs"));
            assert!(
                same(got.re, want.re) && same(got.im, want.im),
                "case {case} (m {m}, panel {panel}): {got:?} vs {want:?}"
            );
        }
        // And through `vacuum_expectation` (coefficients from `gram_coef`).
        let got = kernels::vacuum_expectation(&refs).expect("finite");
        assert!(
            same(got.re, want.re) && same(got.im, want.im),
            "case {case}: {got:?} vs {want:?}"
        );
    }
    assert!(swaps > 1000, "only {swaps} off-pair pivots");
    assert!(zero_results > 10, "only {zero_results} zero overlaps");
}

/// The identity the pivot search relies on: `hypot(x, z) == |x|` bit for bit for a zero `z`
/// (either sign), on normal, subnormal, huge and zero `x`.
#[test]
fn hypot_with_a_zero_part_is_the_absolute_value() {
    let mut r = Rng(37);
    let mut xs: Vec<f64> = vec![
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        f64::MAX,
        -f64::MAX,
        5e-324,
        -5e-324,
        1.0,
        -1.0,
    ];
    for _ in 0..200_000 {
        let bits = (r.below(1 << 32) << 32) | r.below(1 << 32);
        let x = f64::from_bits(bits);
        if x.is_finite() {
            xs.push(x);
        }
        xs.push(lcg_f(&mut r));
    }
    for x in xs {
        for z in [0.0f64, -0.0] {
            assert_eq!(x.hypot(z).to_bits(), x.abs().to_bits(), "hypot({x:e}, {z})");
            assert_eq!(z.hypot(x).to_bits(), x.abs().to_bits(), "hypot({z}, {x:e})");
        }
    }
}

/// How many of the reference elimination's steps pivot away from `k + 1` (the reference's
/// loop, counting).
fn off_pair_pivots(re: &[f64], im: &[f64], m: usize) -> usize {
    let mut a: Vec<(f64, f64)> = vec![(0.0, 0.0); m * m];
    for i in 0..m {
        for j in i + 1..m {
            a[i * m + j] = (re[i * m + j], im[i * m + j]);
            a[j * m + i] = (-re[i * m + j], -im[i * m + j]);
        }
    }
    let norm = |z: (f64, f64)| z.0.hypot(z.1);
    let mut count = 0;
    for k in (0..m.saturating_sub(1)).step_by(2) {
        let piv = (k + 1..m)
            .max_by(|&x, &y| norm(a[k * m + x]).total_cmp(&norm(a[k * m + y])))
            .unwrap_or(k + 1);
        if piv != k + 1 {
            count += 1;
            for j in 0..m {
                a.swap((k + 1) * m + j, piv * m + j);
            }
            for i in 0..m {
                a.swap(i * m + k + 1, i * m + piv);
            }
        }
        let h = a[k * m + k + 1];
        let d = h.0 * h.0 + h.1 * h.1;
        if d == 0.0 {
            break;
        }
        let tau: Vec<(f64, f64)> = (0..m)
            .map(|j| {
                let x = a[k * m + j];
                ((x.0 * h.0 + x.1 * h.1) / d, (x.1 * h.0 - x.0 * h.1) / d)
            })
            .collect();
        let row: Vec<(f64, f64)> = (0..m).map(|j| a[(k + 1) * m + j]).collect();
        for i in k + 2..m {
            for j in k + 2..m {
                let (t, r) = (tau[j], row[i]);
                let (u, w) = (tau[i], row[j]);
                let x = (t.0 * r.0 - t.1 * r.1, t.0 * r.1 + t.1 * r.0);
                let y = (u.0 * w.0 - u.1 * w.1, u.0 * w.1 + u.1 * w.0);
                a[i * m + j].0 += x.0 - y.0;
                a[i * m + j].1 += x.1 - y.1;
            }
        }
    }
    count
}

/// `cargo test --release --lib fastsim::tests::time_elimination -- --ignored --nocapture`
#[test]
#[ignore]
fn time_elimination() {
    let mut r = Rng(5);
    for (m, n) in [(216usize, 108usize), (304, 152)] {
        let ys = random_ys(&mut r, m, n, 0);
        let refs: Vec<&Tv> = ys.iter().collect();
        let (re0, im0) = reference_upper(&refs);
        let updates = (0..m)
            .step_by(2)
            .map(|k| (m - k - 2) * (m - k - 1) / 2)
            .sum::<usize>() as f64;
        for panel in [1usize, 4, 8, 16] {
            let reps = 40;
            let t = std::time::Instant::now();
            let mut h = 0.0;
            for _ in 0..reps {
                let (mut re, mut im) = (re0.clone(), im0.clone());
                h += super::pfblock::pfaffian_dense(&mut re, &mut im, m, panel)
                    .map_or(0.0, |c| c.re);
            }
            let dt = t.elapsed().as_secs_f64() / f64::from(reps);
            eprintln!(
                "m {m} panel {panel}: {:.3} ms, {:.1} GFLOP/s ({h:e})",
                dt * 1e3,
                updates * 16.0 / dt * 1e-9
            );
        }
    }
}

/// Real contraction matrices (`FEMOCO_MATS`, a dump of `(m, re, im)` records): every panel size
/// against the per-component elimination, timed. Tooling.
#[test]
#[ignore]
fn time_elimination_on_dumped_matrices() {
    let Some(path) = std::env::var_os("FEMOCO_MATS") else {
        return;
    };
    let b = std::fs::read(path).expect("read");
    let mut at = 0;
    let mut mats = Vec::new();
    while at + 8 <= b.len() {
        let m = u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) as usize;
        at += 8;
        let v: Vec<f64> = b[at..at + 16 * m * m]
            .chunks(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        at += 16 * m * m;
        mats.push((m, v[..m * m].to_vec(), v[m * m..].to_vec()));
    }
    let same = |a: f64, b: f64| gauss::same_up_to_zero_sign(a, b);
    let panels: Vec<usize> = std::env::var("FEMOCO_PANELS")
        .ok()
        .map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![0, 1, 4, 8, 16]);
    for panel in panels {
        let t = std::time::Instant::now();
        let mut out = Vec::new();
        for (m, re, im) in &mats {
            let (mut re, mut im) = (re.clone(), im.clone());
            out.push(kernels::pfaffian_components(&mut re, &mut im, *m, panel));
        }
        let dt = t.elapsed().as_secs_f64() / mats.len() as f64;
        let mut want = Vec::new();
        for (m, re, im) in &mats {
            let (mut re, mut im) = (re.clone(), im.clone());
            want.push(kernels::pfaffian_components(&mut re, &mut im, *m, 0));
        }
        for (g, w) in out.iter().zip(&want) {
            let (g, w) = (g.expect("finite"), w.expect("finite"));
            assert!(same(g.re, w.re) && same(g.im, w.im));
        }
        eprintln!(
            "{} matrices, panel {panel}: {:.3} ms each",
            mats.len(),
            dt * 1e3
        );
    }
}

fn load_mats(path: &std::ffi::OsStr) -> Vec<(usize, Vec<f64>, Vec<f64>)> {
    let b = std::fs::read(path).expect("read");
    let mut at = 0;
    let mut mats = Vec::new();
    while at + 8 <= b.len() {
        let m = u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) as usize;
        at += 8;
        let v: Vec<f64> = b[at..at + 16 * m * m]
            .chunks(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        at += 16 * m * m;
        mats.push((m, v[..m * m].to_vec(), v[m * m..].to_vec()));
    }
    mats
}

/// Link survival study (tooling): how much of the elimination's update work falls on columns
/// that are exact `w`-multiples (`w` in 1, -1, i, -i) of their left neighbour for every row above.
#[test]
#[ignore]
fn link_study() {
    let Some(path) = std::env::var_os("FEMOCO_MATS") else {
        return;
    };
    let mats = load_mats(&path);
    let eq = |a: (f64, f64), b: (f64, f64)| {
        gauss::same_up_to_zero_sign(a.0, b.0) && gauss::same_up_to_zero_sign(a.1, b.1)
    };
    let mulw = |z: (f64, f64), w: u8| match w {
        0 => z,
        1 => (-z.1, z.0),
        2 => (-z.0, -z.1),
        _ => (z.1, -z.0),
    };
    let (mut total, mut skip, mut linked0) = (0u64, 0u64, 0u64);
    for (m, re, im) in &mats {
        let m = *m;
        let mut a: Vec<(f64, f64)> = vec![(0.0, 0.0); m * m];
        for i in 0..m {
            for j in i + 1..m {
                a[i * m + j] = (re[i * m + j], im[i * m + j]);
                a[j * m + i] = (-re[i * m + j], -im[i * m + j]);
            }
        }
        // link[p] = Some(w): column p + 1 = w * column p for every row above p.
        let mut link: Vec<Option<u8>> = vec![None; m];
        let mut p = 1;
        while p + 1 < m {
            let w = (0..4u8).find(|&w| (0..p).all(|x| eq(a[x * m + p + 1], mulw(a[x * m + p], w))));
            if let Some(w) = w {
                link[p] = Some(w);
                linked0 += 1;
                p += 2;
            } else {
                p += 1;
            }
        }
        let norm = |z: (f64, f64)| z.0.hypot(z.1);
        for k in (0..m.saturating_sub(1)).step_by(2) {
            let piv = (k + 1..m)
                .max_by(|&x, &y| norm(a[k * m + x]).total_cmp(&norm(a[k * m + y])))
                .unwrap_or(k + 1);
            if piv != k + 1 {
                for z in [k + 1, piv] {
                    link[z] = None;
                    if z > 0 {
                        link[z - 1] = None;
                    }
                }
                for j in 0..m {
                    a.swap((k + 1) * m + j, piv * m + j);
                }
                for i in 0..m {
                    a.swap(i * m + k + 1, i * m + piv);
                }
            }
            let h = a[k * m + k + 1];
            let d = h.0 * h.0 + h.1 * h.1;
            if d == 0.0 {
                break;
            }
            let tau: Vec<(f64, f64)> = (0..m)
                .map(|j| {
                    let x = a[k * m + j];
                    ((x.0 * h.0 + x.1 * h.1) / d, (x.1 * h.0 - x.0 * h.1) / d)
                })
                .collect();
            let row: Vec<(f64, f64)> = (0..m).map(|j| a[(k + 1) * m + j]).collect();
            for p in k + 2..m.saturating_sub(1) {
                if let Some(w) = link[p] {
                    if !(eq(tau[p + 1], mulw(tau[p], w)) && eq(row[p + 1], mulw(row[p], w))) {
                        link[p] = None;
                    }
                }
            }
            for i in k + 2..m {
                for j in k + 2..m {
                    let (t, r) = (tau[j], row[i]);
                    let (u, w) = (tau[i], row[j]);
                    let x = (t.0 * r.0 - t.1 * r.1, t.0 * r.1 + t.1 * r.0);
                    let y = (u.0 * w.0 - u.1 * w.1, u.0 * w.1 + u.1 * w.0);
                    a[i * m + j].0 += x.0 - y.0;
                    a[i * m + j].1 += x.1 - y.1;
                }
            }
            for i in k + 2..m {
                for j in i + 1..m {
                    total += 1;
                    if j >= 1 && j - 1 > i && link[j - 1].is_some() {
                        skip += 1;
                    }
                }
            }
            // The invariant, checked on the data.
            for p in k + 2..m.saturating_sub(1) {
                if let Some(w) = link[p] {
                    for x in k + 2..p {
                        assert!(
                            eq(a[x * m + p + 1], mulw(a[x * m + p], w)),
                            "invariant broken"
                        );
                    }
                }
            }
        }
    }
    eprintln!(
        "{} matrices: {linked0} initial links, {:.1}% of upper updates skippable",
        mats.len(),
        100.0 * skip as f64 / total as f64
    );
}

#[test]
#[ignore]
fn link_counts_on_dumped_matrices() {
    let Some(path) = std::env::var_os("FEMOCO_MATS") else {
        return;
    };
    let mats = load_mats(&path);
    for panel in [1usize, 16] {
        let mut c = super::pfblock::Counts::default();
        let t = std::time::Instant::now();
        for (m, re, im) in &mats {
            let (mut re, mut im) = (re.clone(), im.clone());
            let _ = super::pfblock::pfaffian_dense_counted(
                &mut re,
                &mut im,
                *m,
                panel,
                true,
                Some(&mut c),
            );
        }
        eprintln!(
            "panel {panel}: {c:?}, {:.1}% skipped, {:.3} ms",
            100.0 * c.skipped as f64 / c.updates as f64,
            t.elapsed().as_secs_f64() * 1e3 / mats.len() as f64
        );
        let t = std::time::Instant::now();
        for (m, re, im) in &mats {
            let (mut re, mut im) = (re.clone(), im.clone());
            let _ =
                super::pfblock::pfaffian_dense_counted(&mut re, &mut im, *m, panel, false, None);
        }
        eprintln!(
            "panel {panel} no links: {:.3} ms",
            t.elapsed().as_secs_f64() * 1e3 / mats.len() as f64
        );
    }
}

#[test]
#[ignore]
fn spin_elimination_on_dumped_matrices() {
    let Some(path) = std::env::var_os("FEMOCO_MATS") else {
        return;
    };
    let mats = load_mats(&path);
    let reps: usize = std::env::var("FEMOCO_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let panel = super::kernels::elim_panel();
    let t = std::time::Instant::now();
    let mut h = 0.0;
    for _ in 0..reps {
        for (m, re, im) in &mats {
            let (mut re, mut im) = (re.clone(), im.clone());
            h += super::pfblock::pfaffian_dense(&mut re, &mut im, *m, panel).map_or(0.0, |c| c.re);
        }
    }
    eprintln!(
        "panel {panel}: {:.3} ms each ({h:e})",
        t.elapsed().as_secs_f64() * 1e3 / (reps * mats.len()) as f64
    );
}

/// `cargo test --release --lib fastsim::tests::time_gram -- --ignored --nocapture`
#[test]
#[ignore]
fn time_gram() {
    let mut r = Rng(9);
    for (k, n) in [(110usize, 108usize), (152, 152)] {
        let base = random_ys(&mut r, k, n, 0);
        let ys: Vec<Tv> = base
            .iter()
            .flat_map(|y| {
                [
                    Tv {
                        odd: false,
                        v: y.v.clone(),
                    },
                    Tv {
                        odd: true,
                        v: y.v.clone(),
                    },
                ]
            })
            .collect();
        let refs: Vec<&Tv> = ys.iter().collect();
        let m = refs.len();
        let (mut re, mut im) = (vec![0.0; m * m], vec![0.0; m * m]);
        let reps = 200;
        let t = std::time::Instant::now();
        for _ in 0..reps {
            kernels::gram_for_tests(&refs, n, m, &mut re, &mut im);
        }
        let dt = t.elapsed().as_secs_f64() / f64::from(reps);
        let macs = (k * (k + 1) / 2 * n) as f64;
        eprintln!(
            "gram k {k} n {n}: {:.1} us, {:.1} GFLOP/s",
            dt * 1e6,
            2.0 * macs / dt * 1e-9
        );
    }
}

/// Real-coefficient against complex elimination on dumped matrices (`FEMOCO_MATS`; parities
/// inferred from row 0, matrices whose row 0 has a zero skipped). Tooling.
#[test]
#[ignore]
fn real_elimination_on_dumped_matrices() {
    let Some(path) = std::env::var_os("FEMOCO_MATS") else {
        return;
    };
    let mats = load_mats(&path);
    let reps: usize = std::env::var("FEMOCO_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let mut prepared = Vec::new();
    for (m, re, im) in &mats {
        let m = *m;
        // Parities relative to index 0, by a search over the non-zero entries (a real part:
        // same parity; an imaginary part: the other); matrices not joined are skipped.
        let mut par = vec![false; m];
        let mut seen = vec![false; m];
        seen[0] = true;
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            for j in 0..m {
                if seen[j] || i == j {
                    continue;
                }
                let (a, b) = if i < j { (i, j) } else { (j, i) };
                let (x, y) = (re[a * m + b], im[a * m + b]);
                if x != 0.0 || y != 0.0 {
                    seen[j] = true;
                    par[j] = par[i] ^ (x == 0.0);
                    stack.push(j);
                }
            }
        }
        if seen.iter().any(|s| !s) {
            continue;
        }
        let Some(c) = super::pfreal::coefficients(re, im, &par) else {
            continue;
        };
        prepared.push((m, re.clone(), im.clone(), par, c));
    }
    eprintln!("{} of {} matrices joined", prepared.len(), mats.len());
    for panel in [4usize, 8, 16, 32] {
        let t = std::time::Instant::now();
        for _ in 0..reps {
            for (m, re, im, _, _) in &prepared {
                let (mut re, mut im) = (re.clone(), im.clone());
                let _ = super::pfblock::pfaffian_dense(&mut re, &mut im, *m, panel);
            }
        }
        let tc = t.elapsed().as_secs_f64() / (reps * prepared.len()) as f64;
        let t = std::time::Instant::now();
        for _ in 0..reps {
            for (m, _, _, par, c) in &prepared {
                let (mut c, mut par) = (c.clone(), par.clone());
                let _ = super::pfreal::pfaffian_real(&mut c, &mut par, *m, panel);
            }
        }
        let tr = t.elapsed().as_secs_f64() / (reps * prepared.len()) as f64;
        let mut ok = 0;
        for (m, re, im, par, c) in &prepared {
            let (mut re, mut im) = (re.clone(), im.clone());
            let a = super::pfblock::pfaffian_dense(&mut re, &mut im, *m, panel);
            let (mut c, mut par) = (c.clone(), par.clone());
            let b = super::pfreal::pfaffian_real(&mut c, &mut par, *m, panel);
            if let (Some(a), Some(b)) = (a, b) {
                ok += usize::from(
                    gauss::same_up_to_zero_sign(a.re, b.re)
                        && gauss::same_up_to_zero_sign(a.im, b.im),
                );
            }
        }
        eprintln!(
            "panel {panel}: complex {:.3} ms, real {:.3} ms, {ok} equal",
            tc * 1e3,
            tr * 1e3
        );
    }
}
