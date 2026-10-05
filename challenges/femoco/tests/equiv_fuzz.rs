//! Property fuzzer for lane-engine equivalence (spec/FAST-EVALUATOR.md section 10): random small
//! op streams, lane maps (reference tables), seeds and lane samples, run through the reference
//! lane engine and the candidate (`FEMOCO_EQUIV_ENGINE`, default the reference itself) at each
//! `FEMOCO_EQUIV_THREADS` count, compared on the whole `Outcome` and every lane's verdict.
//!
//! Each case starts from a valid circuit, built to pass: a controlled select of two random
//! Hermitian Majorana monomials on uniform bit 0 (uncomputed by measurement with a conditioned
//! `CZ` fix-up, or by `CCX` and `R`), wrapped in mirrored noise (`G` then `G^-1`) that uses every
//! op kind the lowered stream has: conditions, `S`/`Sdg` and Paulis on the system, `Givens` with
//! a lane-dependent angle undone by the negated angle, `SpinSwap` / `SpinSwapDg`, `Swap`, and
//! optionally a nested `Reflect` between two identical inner copies and a swap bit flipped on
//! control-1 lanes. Most cases are then broken by one to three op-stream mutations
//! (tests/equiv_common), and some are replaced by unconstrained random streams. Cases the static
//! pass rejects never reach an engine and are only counted.
//!
//! `FEMOCO_EQUIV_FUZZ_CASES` (default 300) and `FEMOCO_EQUIV_FUZZ_SEED` (default 1) set the run.
#![cfg(feature = "walk")]
mod equiv_common;

use equiv_common::{all_mutations, kinds_in, mutate, random_monomial, Rng, Shape};
use femoco_walk::circuit::{Op, OperationType as K, NONE};
use femoco_walk::equiv;
use femoco_walk::fiat_shamir::Lane;
use femoco_walk::sim::compile::{compile_nested, compile_sa, Compiled, Layout};
use femoco_walk::sim::validate::{self, Context, Nested};
use femoco_walk::sim::{gaussian, Frame, Inner, TrackerFactory};
use femoco_walk::spec::{Monomial, SystemOp};
use serde_json::json;
use std::collections::BTreeMap;

fn op(kind: K, t: u32, c1: u32, c2: u32) -> Op {
    let mut o = Op::new(kind);
    (o.q_target, o.q_control1, o.q_control2) = (t, c1, c2);
    o
}

fn cond(mut o: Op, b: u32) -> Op {
    o.c_condition = b;
    o
}

/// One generated case.
struct Case {
    n: usize,
    u: u32,
    sa: bool,
    beta: Option<u32>,
    inner: Option<(u32, u32)>,
    swap_bit: Option<u32>,
    refs: [Monomial; 2],
    ops: Vec<Op>,
    what: String,
}

/// Builder state for a valid case.
struct Gen {
    n: u32,
    u: u32,
    next_q: u32,
    next_b: u32,
    ops: Vec<Op>,
}

impl Gen {
    fn sys(&self, j: u32) -> u32 {
        1 + j
    }
    fn uni(&self, i: u32) -> u32 {
        1 + self.n + i
    }
    fn alloc(&mut self) -> u32 {
        self.next_q += 1;
        self.next_q - 1
    }
    fn bit(&mut self) -> u32 {
        self.next_b += 1;
        self.next_b - 1
    }
    fn hmr(&mut self, q: u32) -> u32 {
        let b = self.bit();
        let mut o = op(K::Hmr, q, NONE, NONE);
        o.c_target = b;
        self.ops.push(o);
        b
    }
    fn register(&mut self, r: u32, qs: &[u32]) {
        let mut o = Op::new(K::Register);
        o.r_target = r;
        self.ops.push(o);
        for &q in qs {
            let mut a = Op::new(K::AppendToRegister);
            a.q_target = q;
            a.r_target = r;
            self.ops.push(a);
        }
    }

    /// `f` controlled on `a`, as tests/harness_common's `apply_frame`.
    fn apply_frame(&mut self, a: u32, f: &Frame) {
        for q in (0..self.n as usize).filter(|&q| f.z_bit(q)) {
            let t = self.sys(q as u32);
            self.ops.push(op(K::CZ, t, a, NONE));
        }
        for q in (0..self.n as usize).filter(|&q| f.x_bit(q)) {
            let t = self.sys(q as u32);
            self.ops.push(op(K::CX, t, a, NONE));
        }
        match f.phase {
            2 => self.ops.push(op(K::S, a, NONE, NONE)),
            4 => self.ops.push(op(K::Z, a, NONE, NONE)),
            6 => self.ops.push(op(K::Sdg, a, NONE, NONE)),
            _ => {}
        }
    }

    /// Control-1 lanes apply `frames[s & 1]`.
    fn select(&mut self, rng: &mut Rng, frames: &[Frame; 2]) {
        let u0 = self.uni(0);
        for (v, frame) in frames.iter().enumerate() {
            if v == 0 {
                self.ops.push(op(K::X, u0, NONE, NONE));
            }
            let a = self.alloc();
            self.ops.push(op(K::CCX, a, 0, u0));
            self.apply_frame(a, frame);
            if rng.chance(0.6) {
                // Measurement-based uncompute: a -1 where the outcome is 1 and a = 1, fixed by
                // CZ(control, u0) under the outcome.
                let b = self.hmr(a);
                self.ops.push(cond(op(K::CZ, u0, 0, NONE), b));
            } else {
                self.ops.push(op(K::CCX, a, 0, u0));
                self.ops.push(op(K::R, a, NONE, NONE));
            }
            if v == 0 {
                self.ops.push(op(K::X, u0, NONE, NONE));
            }
        }
    }
}

/// The inverse of one noise op (Paulis, CX/CCX, Swap, CZ/CCZ are self-inverse).
fn inverse(o: Op) -> Op {
    let mut i = o;
    i.kind = match o.kind {
        K::S => K::Sdg,
        K::Sdg => K::S,
        K::SpinSwap => K::SpinSwapDg,
        K::SpinSwapDg => K::SpinSwap,
        k => k,
    };
    i
}

/// Mirrored noise: a random reversible block `G` and then `G^-1`, with each `Givens` undone by
/// a `Givens` on the negated angle register. `angle` is the register `(id, qubits)` (width
/// `beta = 3`), `bits` the classical bits that are fixed for the rest of the stream, `inner`
/// the inner uniform qubits this block may not touch (outside the copies).
#[allow(clippy::too_many_arguments)]
fn noise(
    g: &mut Gen,
    rng: &mut Rng,
    len: usize,
    angle: Option<&(u32, Vec<u32>)>,
    spin: bool,
    bits: &[u32],
    avoid: &[u32],
    ancillas: &[u32],
) {
    let nonsys: Vec<u32> = std::iter::once(0)
        .chain((0..g.u).map(|i| g.uni(i)))
        .chain(ancillas.iter().copied())
        .filter(|q| !avoid.contains(q) && angle.is_none_or(|(_, r)| !r.contains(q)))
        .collect();
    let mut block: Vec<Op> = Vec::new();
    let mut pending_pops = 0usize;
    for _ in 0..len {
        let pick2 = |rng: &mut Rng| {
            let a = rng.pick(&nonsys);
            let mut b = rng.pick(&nonsys);
            while b == a && nonsys.len() > 1 {
                b = rng.pick(&nonsys);
            }
            (a, b)
        };
        let mut o = match rng.below(14) {
            0 => op(K::X, rng.pick(&nonsys), NONE, NONE),
            1 => {
                let (a, b) = pick2(rng);
                if a == b {
                    continue;
                }
                op(K::CX, a, b, NONE)
            }
            2 => {
                let (a, b) = pick2(rng);
                let c = rng.pick(&nonsys);
                if a == b || c == a || c == b {
                    continue;
                }
                op(K::CCX, a, b, c)
            }
            3 => {
                let (a, b) = pick2(rng);
                if a == b {
                    continue;
                }
                op(K::Swap, a, b, NONE)
            }
            4 => op(
                rng.pick(&[K::Z, K::S, K::Sdg]),
                rng.pick(&nonsys),
                NONE,
                NONE,
            ),
            5 => {
                let (a, b) = pick2(rng);
                if a == b {
                    continue;
                }
                op(K::CZ, a, b, NONE)
            }
            6 => {
                let (a, b) = pick2(rng);
                let c = rng.pick(&nonsys);
                if a == b || c == a || c == b {
                    continue;
                }
                op(K::CCZ, a, b, c)
            }
            7 | 8 => {
                let t = g.sys(rng.below(u64::from(g.n)) as u32);
                // S on the system is not a Pauli frame: a later Givens or SpinSwap hand-off
                // would reject it (that path is left to the mutants).
                let kinds: &[K] = if angle.is_some() || spin {
                    &[K::X, K::Z]
                } else {
                    &[K::X, K::Z, K::S, K::Sdg]
                };
                op(rng.pick(kinds), t, NONE, NONE)
            }
            9 => {
                let t = g.sys(rng.below(u64::from(g.n)) as u32);
                let c = rng.pick(&nonsys);
                op(rng.pick(&[K::CX, K::CZ]), t, c, NONE)
            }
            10 => {
                let t = g.sys(rng.below(u64::from(g.n)) as u32);
                let (a, b) = pick2(rng);
                if a == b {
                    continue;
                }
                op(rng.pick(&[K::CCX, K::CCZ]), t, a, b)
            }
            11 if angle.is_some() => {
                let (r, qs) = angle.unwrap();
                // Load a lane-dependent angle, rotate, and leave the load to the mirror.
                for &q in qs {
                    if rng.chance(0.5) {
                        block.push(op(K::CX, q, rng.pick(&nonsys), NONE));
                    }
                }
                let p = rng.below(u64::from(g.n - 1)) as u32;
                let q = p + 1 + rng.below(u64::from(g.n - 1 - p)) as u32;
                let mut o = op(K::Givens, g.sys(q), g.sys(p), NONE);
                o.r_target = *r;
                o
            }
            12 if spin => op(
                rng.pick(&[K::SpinSwap, K::SpinSwapDg]),
                rng.pick(&nonsys),
                NONE,
                NONE,
            ),
            13 if !bits.is_empty() && pending_pops < 3 => {
                let mut p = Op::new(K::PushCondition);
                p.c_condition = rng.pick(bits);
                pending_pops += 1;
                block.push(p);
                continue;
            }
            _ => {
                if pending_pops > 0 {
                    pending_pops -= 1;
                    block.push(Op::new(K::PopCondition));
                }
                continue;
            }
        };
        if !bits.is_empty() && o.kind != K::Givens && rng.chance(0.2) {
            o.c_condition = rng.pick(bits);
        }
        block.push(o);
    }
    block.extend((0..pending_pops).map(|_| Op::new(K::PopCondition)));
    g.ops.extend(block.iter().copied());
    // The mirror: reversed, each op inverted; a push and its pop swap roles.
    let mut stack: Vec<Op> = Vec::new();
    let mut pushes: Vec<Op> = Vec::new();
    for o in &block {
        match o.kind {
            K::PushCondition => pushes.push(*o),
            K::PopCondition => stack.push(pushes.pop().expect("balanced")),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for o in block.iter().rev() {
        match o.kind {
            K::PopCondition => out.push(stack.pop().expect("balanced")),
            K::PushCondition => out.push(Op::new(K::PopCondition)),
            K::Givens => {
                let (_, qs) = angle.unwrap();
                let cnd = o.c_condition;
                let mut neg = Vec::new();
                for &q in qs {
                    neg.push(op(K::X, q, NONE, NONE));
                }
                // +1 on the 3-bit register (little-endian): b2 ^= b0 b1, b1 ^= b0, b0 ^= 1.
                neg.push(op(K::CCX, qs[2], qs[0], qs[1]));
                neg.push(op(K::CX, qs[1], qs[0], NONE));
                neg.push(op(K::X, qs[0], NONE, NONE));
                out.extend(neg.iter().copied());
                out.push(cond(*o, cnd));
                out.extend(neg.iter().rev().copied());
            }
            _ => out.push(inverse(*o)),
        }
    }
    g.ops.extend(out);
}

/// A valid case, built to pass on every lane.
fn valid(rng: &mut Rng) -> Case {
    let n = 2 * (1 + rng.below(4) as usize); // 2, 4, 6, 8 system qubits
    let sa = rng.chance(0.3);
    let tracker = sa || rng.chance(0.5);
    let beta = tracker.then_some(3);
    let nested = rng.chance(0.25);
    let u = 1 + rng.below(4) as u32 + u32::from(nested) * 2;
    let inner = nested.then(|| {
        let w = 1 + rng.below(u64::from(u - 1).min(3)) as u32;
        let lo = 1 + rng.below(u64::from(u - w)) as u32;
        (lo, w)
    });
    let mut g = Gen {
        n: n as u32,
        u,
        next_q: 1 + n as u32 + u,
        next_b: 0,
        ops: Vec::new(),
    };
    let anc = 1 + rng.below(6) as usize;
    let ancillas: Vec<u32> = (0..anc).map(|_| g.alloc()).collect();
    let angle = tracker.then(|| {
        let qs: Vec<u32> = (0..3).map(|_| g.alloc()).collect();
        (0u32, qs)
    });
    if let Some((r, qs)) = &angle {
        g.register(*r, qs);
    }
    let inner_qs: Vec<u32> = inner
        .map(|(lo, w)| (lo..lo + w).map(|j| g.uni(j)).collect())
        .unwrap_or_default();
    if inner.is_some() {
        g.register(1, &inner_qs);
    }
    let (m0, f0) = random_monomial(rng, n);
    let (m1, f1) = random_monomial(rng, n);
    // Random bits: fresh |0> ancillas measured (no phase).
    let nbits = rng.below(3) as usize;
    let mut bits = Vec::new();
    for _ in 0..nbits {
        let q = g.alloc();
        bits.push(g.hmr(q));
    }
    let pre = rng.below(25) as usize;
    noise(
        &mut g,
        rng,
        pre,
        angle.as_ref(),
        sa,
        &bits,
        &inner_qs,
        &ancillas,
    );
    g.select(rng, &[f0, f1]);
    let mid = rng.below(25) as usize;
    noise(
        &mut g,
        rng,
        mid,
        angle.as_ref(),
        sa,
        &bits,
        &inner_qs,
        &ancillas,
    );
    if inner.is_some() {
        let len = rng.below(20) as usize;
        let start = g.ops.len();
        noise(&mut g, rng, len, angle.as_ref(), sa, &bits, &[], &ancillas);
        let body: Vec<Op> = g.ops.split_off(start);
        let seg = |c| {
            let mut s = Op::new(K::Segment);
            s.r_target = c;
            s
        };
        let mut r = Op::new(K::Reflect);
        r.r_target = 1;
        g.ops.push(seg(3));
        g.ops.extend(body.iter().copied());
        g.ops.push(seg(4));
        g.ops.push(r);
        g.ops.push(seg(3));
        g.ops.extend(body);
        g.ops.push(seg(4));
    }
    let post = rng.below(15) as usize;
    noise(
        &mut g,
        rng,
        post,
        angle.as_ref(),
        sa,
        &bits,
        &inner_qs,
        &ancillas,
    );
    // Bit ops after their last use as conditions, and debug hints, change nothing.
    if !bits.is_empty() && rng.chance(0.5) {
        let mut b = Op::new(rng.pick(&[K::BitInvert, K::BitStore0, K::BitStore1]));
        b.c_target = rng.pick(&bits);
        g.ops.push(b);
    }
    let swap_bit = (rng.chance(0.2)).then(|| {
        let outer: Vec<u32> = (0..u).filter(|k| !inner_qs.contains(&g.uni(*k))).collect();
        rng.pick(&outer)
    });
    if let Some(k) = swap_bit {
        let t = g.uni(k);
        g.ops.push(op(K::CX, t, 0, NONE));
    }
    Case {
        n,
        u,
        sa,
        beta,
        inner,
        swap_bit,
        refs: [m0, m1],
        ops: g.ops,
        what: "valid".into(),
    }
}

/// An unconstrained random stream over the same kinds (mostly rejected statically).
fn chaos(rng: &mut Rng) -> Case {
    let mut c = valid(rng);
    let nq = (1 + c.n as u32 + c.u + 6) as u64;
    let kinds = [
        K::X,
        K::Z,
        K::CX,
        K::CZ,
        K::CCX,
        K::CCZ,
        K::Swap,
        K::S,
        K::Sdg,
        K::Hmr,
        K::R,
        K::Neg,
        K::BitInvert,
        K::BitStore0,
        K::BitStore1,
        K::PushCondition,
        K::PopCondition,
        K::Givens,
        K::SpinSwap,
        K::SpinSwapDg,
        K::DebugPrint,
    ];
    let len = 5 + rng.below(60) as usize;
    let mut ops: Vec<Op> = c
        .ops
        .iter()
        .copied()
        .filter(|o| matches!(o.kind, K::Register | K::AppendToRegister))
        .collect();
    for _ in 0..len {
        let k = rng.pick(&kinds);
        let mut o = Op::new(k);
        let q = |rng: &mut Rng| rng.below(nq) as u32;
        match k {
            K::X | K::Z | K::S | K::Sdg | K::R | K::SpinSwap | K::SpinSwapDg => o.q_target = q(rng),
            K::CX | K::CZ | K::Swap => {
                (o.q_target, o.q_control1) = (q(rng), q(rng));
            }
            K::CCX | K::CCZ => {
                (o.q_target, o.q_control1, o.q_control2) = (q(rng), q(rng), q(rng));
            }
            K::Hmr => {
                o.q_target = q(rng);
                o.c_target = rng.below(4) as u32;
            }
            K::BitInvert | K::BitStore0 | K::BitStore1 => o.c_target = rng.below(4) as u32,
            K::PushCondition => o.c_condition = rng.below(4) as u32,
            K::Givens => {
                (o.q_target, o.q_control1) = (q(rng), q(rng));
                o.r_target = 0;
            }
            _ => {}
        }
        if !matches!(k, K::R | K::PushCondition | K::PopCondition | K::DebugPrint)
            && rng.chance(0.2)
        {
            o.c_condition = rng.below(4) as u32;
        }
        if o.validate().is_ok() {
            ops.push(o);
        }
    }
    let depth: i64 = ops
        .iter()
        .map(|o| match o.kind {
            K::PushCondition => 1,
            K::PopCondition => -1,
            _ => 0,
        })
        .sum();
    ops.extend((0..depth.max(0)).map(|_| Op::new(K::PopCondition)));
    c.ops = ops;
    c.what = "chaos".into();
    c
}

#[derive(Default)]
struct Stats {
    cases: usize,
    statically_rejected: usize,
    engine_runs: usize,
    passing: usize,
    with_exec_error: usize,
    lanes: usize,
    verdicts: BTreeMap<String, u64>,
    kinds: BTreeMap<String, u64>,
    mutations: BTreeMap<String, u64>,
    differences: Vec<String>,
    last_rejection: Option<String>,
}

fn run_case(
    i: u64,
    c: &Case,
    rng: &mut Rng,
    cand: &(String, validate::Engine),
    threads: &[usize],
    st: &mut Stats,
) {
    st.cases += 1;
    let layout = Layout {
        system: c.n,
        uniform: c.u,
    };
    let tracker: Option<&dyn TrackerFactory> = c
        .beta
        .and_then(gaussian::factory)
        .map(|f| f as &dyn TrackerFactory);
    let inner = c.inner.map(|(lo, width)| Inner { lo, width });
    let compiled: Compiled = match if c.sa {
        compile_sa(&c.ops, &layout, tracker, inner)
    } else {
        compile_nested(&c.ops, &layout, tracker, inner)
    } {
        Ok(x) => x,
        Err(_) => {
            st.statically_rejected += 1;
            return;
        }
    };
    for k in kinds_in(&c.ops) {
        *st.kinds.entry(k.name().to_string()).or_default() += 1;
    }
    let cap = if rng.chance(0.3) { 1600 } else { 600 };
    let k = 1 + rng.below(cap) as usize;
    let umask = if c.u == 64 {
        u64::MAX
    } else {
        (1u64 << c.u) - 1
    };
    let lanes: Vec<Lane> = (0..k)
        .map(|_| Lane {
            c: rng.chance(0.5),
            s: rng.next() & umask,
        })
        .collect();
    let after: Vec<u64> = lanes
        .iter()
        .map(|l| match c.inner {
            None => l.s,
            Some((lo, w)) => {
                let m = ((1u64 << w) - 1) << lo;
                (l.s & !m) | (rng.next() & m)
            }
        })
        .collect();
    let mut key = [0u8; 32];
    for chunk in key.chunks_mut(8) {
        chunk.copy_from_slice(&rng.next().to_le_bytes());
    }
    let refs = c.refs.clone();
    let reference = move |s: u64| SystemOp::Monomial(refs[(s & 1) as usize].clone());
    let refs2 = c.refs.clone();
    let reference_nested =
        move |s: u64, _after: u64| SystemOp::Monomial(refs2[(s & 1) as usize].clone());
    let sb = c.swap_bit;
    let uniform_after = move |s: u64| sb.map_or(s, |k| s ^ (1 << k));
    let ctx = Context {
        compiled: &compiled,
        layout,
        hmr_key: key,
        reference: &reference,
        uniform_after: &uniform_after,
        tracker,
    };
    let nested = c.inner.map(|(lo, width)| Nested {
        lo,
        width,
        after: &after,
        reference: &reference_nested,
    });
    let (o, p, diffs) = equiv::compare_engines(cand.1, threads, &ctx, nested.as_ref(), &lanes);
    st.engine_runs += 1;
    st.lanes += lanes.len();
    st.last_rejection = o.rejection();
    if st.last_rejection.is_none() {
        st.passing += 1;
    }
    if p.verdicts.contains(&validate::VERDICT_UNCHECKED) {
        st.with_exec_error += 1;
    }
    for &v in &p.verdicts {
        *st.verdicts
            .entry(equiv::verdict_name(v).to_string())
            .or_default() += 1;
    }
    for (t, d) in diffs {
        for x in d {
            st.differences
                .push(format!("case {i} ({}) @{t}: {x}", c.what));
        }
    }
}

#[test]
fn fuzz_engines_agree() {
    let cases: u64 = std::env::var("FEMOCO_EQUIV_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let seed: u64 = std::env::var("FEMOCO_EQUIV_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let cand = equiv_common::candidate();
    let threads = equiv::env_threads();
    let mut st = Stats::default();
    let mut valid_passed = 0usize;
    let mut valid_run = 0usize;
    let mut valid_failures: Vec<String> = Vec::new();
    for i in 0..cases {
        let mut rng = Rng::new(seed.wrapping_mul(1_000_003) ^ i);
        let mut c = if rng.chance(0.08) {
            chaos(&mut rng)
        } else {
            valid(&mut rng)
        };
        if c.what == "valid" && rng.chance(0.65) {
            let sh = Shape::of(&c.ops, c.n, c.u, c.inner);
            let muts = all_mutations(&kinds_in(&c.ops), &[1, 2, 3]);
            let mut names = Vec::new();
            for _ in 0..1 + rng.below(3) {
                let m = rng.pick(&muts);
                if let Some((ops, desc)) = mutate(&c.ops, &sh, m, &mut rng) {
                    c.ops = ops;
                    let name = format!("{m:?}");
                    let short = name.split('(').next().unwrap_or(&name).to_string();
                    *st.mutations.entry(short).or_default() += 1;
                    names.push(desc);
                }
            }
            if !names.is_empty() {
                c.what = format!("mutant: {}", names.join("; "));
            }
        }
        let before = (st.engine_runs, st.passing);
        run_case(i, &c, &mut rng, &cand, &threads, &mut st);
        if c.what == "valid" && st.engine_runs > before.0 {
            valid_run += 1;
            valid_passed += st.passing - before.1;
            if let Some(r) = st.last_rejection.take() {
                valid_failures.push(format!(
                    "case {i} (n {}, u {}, sa {}, beta {:?}, inner {:?}, swap {:?}): {r}",
                    c.n, c.u, c.sa, c.beta, c.inner, c.swap_bit
                ));
            }
        }
        if st.differences.len() > 20 {
            break;
        }
    }
    let summary = json!({
        "kind": "fuzz",
        "engine": cand.0,
        "threads": threads,
        "seed": seed,
        "cases": st.cases,
        "statically_rejected": st.statically_rejected,
        "engine_runs": st.engine_runs,
        "passing_runs": st.passing,
        "valid_runs": valid_run,
        "valid_runs_passing": valid_passed,
        "runs_with_execution_error": st.with_exec_error,
        "lanes": st.lanes,
        "verdicts": st.verdicts,
        "op_kinds_in_runs": st.kinds,
        "mutations_applied": st.mutations,
        "equal": st.differences.is_empty(),
        "differences": st.differences.iter().take(20).collect::<Vec<_>>(),
    });
    equiv::log(&summary);
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    assert!(
        st.differences.is_empty(),
        "{} differences:\n{}",
        st.differences.len(),
        st.differences.join("\n")
    );
    // The generator must keep producing what it claims: valid circuits pass.
    assert_eq!(
        valid_passed,
        valid_run,
        "generated valid circuits failed:\n{}",
        valid_failures
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Tracker edges the generator avoids in valid cases, each built on purpose: more than
/// `MAX_FACTORS` Majorana factors on part of the lanes (deterministic and outcome-selected), an
/// `S` power at a `Givens` hand-off on outcome-selected lanes, and a `SpinSwap` after a system
/// `S`. Each must reach the tracker's rejection on some lanes, identically in both engines.
#[test]
fn tracker_edges_agree() {
    let cand = equiv_common::candidate();
    let threads = equiv::env_threads();
    let n = 8usize;
    let u = 3u32;
    let sys = |j: u32| 1 + j;
    let uni = |i: u32| 1 + n as u32 + i;
    let first = 1 + n as u32 + u;
    let angle: Vec<u32> = (first..first + 3).collect();
    let fresh = first + 3;
    let mut base = Vec::new();
    let mut r = Op::new(K::Register);
    r.r_target = 0;
    base.push(r);
    for &q in &angle {
        let mut a = Op::new(K::AppendToRegister);
        a.q_target = q;
        a.r_target = 0;
        base.push(a);
    }
    base.push(op(K::X, angle[0], NONE, NONE)); // a nonzero angle on every lane
    let givens = {
        let mut g = op(K::Givens, sys(5), sys(2), NONE);
        g.r_target = 0;
        g
    };
    let mut cases: Vec<(&str, Vec<Op>, bool)> = Vec::new();
    // 1. Overflow on lanes with uniform bit 0 set: 80 hand-offs of an 8-qubit X string.
    let mut ops = base.clone();
    for _ in 0..80 {
        for j in 0..n as u32 {
            ops.push(op(K::CX, sys(j), uni(0), NONE));
        }
        ops.push(givens);
    }
    cases.push(("overflow-u0", ops, false));
    // 2. Overflow on outcome-selected lanes: three random bits nested as conditions.
    let mut ops = base.clone();
    for j in 0..3 {
        let mut h = op(K::Hmr, fresh + j, NONE, NONE);
        h.c_target = j;
        ops.push(h);
    }
    for _ in 0..80 {
        for j in 0..3 {
            let mut p = Op::new(K::PushCondition);
            p.c_condition = j;
            ops.push(p);
        }
        for j in 0..n as u32 {
            ops.push(op(K::X, sys(j), NONE, NONE));
        }
        ops.extend((0..3).map(|_| Op::new(K::PopCondition)));
        ops.push(givens);
    }
    cases.push(("overflow-random", ops, false));
    // 3. An S power at a hand-off on outcome-selected lanes.
    let mut ops = base.clone();
    let mut h = op(K::Hmr, fresh, NONE, NONE);
    h.c_target = 0;
    ops.push(h);
    ops.push(cond(op(K::S, sys(3), NONE, NONE), 0));
    ops.push(givens);
    cases.push(("s-at-handoff", ops, false));
    // 4. A SpinSwap after a system S on lanes with uniform bit 1 set (sos-sa compile).
    let mut ops = base.clone();
    let mut h = op(K::Hmr, fresh, NONE, NONE);
    h.c_target = 0;
    ops.push(h);
    ops.push(cond(op(K::Sdg, sys(1), NONE, NONE), 0));
    ops.push(op(K::SpinSwap, uni(1), NONE, NONE));
    cases.push(("spinswap-after-s", ops, true));
    let refs = [
        equiv_common::hermitian(&[], n, false).0,
        equiv_common::hermitian(&[0, 1], n, false).0,
    ];
    let mut st = Stats::default();
    for (i, (name, ops, sa)) in cases.into_iter().enumerate() {
        let c = Case {
            n,
            u,
            sa,
            beta: Some(3),
            inner: None,
            swap_bit: None,
            refs: refs.clone(),
            ops,
            what: name.into(),
        };
        let before = st.verdicts.get("tracker").copied().unwrap_or(0);
        let mut rng = Rng::new(77 + i as u64);
        run_case(i as u64, &c, &mut rng, &cand, &threads, &mut st);
        let after = st.verdicts.get("tracker").copied().unwrap_or(0);
        assert!(
            after > before,
            "{name}: no lane reached the tracker rejection"
        );
        assert!(st.with_exec_error > 0, "{name}: no execution error");
    }
    equiv::log(
        &json!({"kind": "tracker-edges", "engine": cand.0, "equal": st.differences.is_empty(), "verdicts": st.verdicts}),
    );
    assert!(st.differences.is_empty(), "{}", st.differences.join("\n"));
}
