//! Bit-sliced execution of a lowered op stream on one batch of `64 * W` lanes.
//!
//! Every non-system wire and classical bit is one `u64` word per 64 lanes (bit `l` of word `j`
//! is lane `64 j + l`). Each lane's phase is an exact element of `Z/8` in three planes
//! (`w = exp(i pi / 4)`). The system register is carried as an operator, not a state:
//! `w^phase * D * X^x Z^z` with `D = prod_q S_q^{s_q}`, in planes `x`, `z`, `s0`, `s1` per
//! system qubit. Gates act on the left of that operator:
//! - `X_q D = i^{s_q} S_q^{-s_q} X_q (D without q)`, so `X_q` adds `2 s_q` to the phase,
//!   negates `s_q` and flips `x_q`;
//! - `Z_q` commutes with `D`, and `Z_q X^x Z^z = (-1)^{x_q} X^x Z^{z + e_q}`;
//! - `S_q` adds one to `s_q`.
//!
//! A lane whose final `s` is nonzero did not apply a Pauli and is rejected by the checks.
// Word-indexed loops over the fixed `W` words read most clearly and vectorize well here.
#![allow(clippy::needless_range_loop)]
use super::compile::{Compiled, SimOp};
use super::tracker::{LaneFrame, LaneTracker, TrackerFactory};
use crate::circuit::NONE;
use sha3::digest::XofReader;

/// Words per batch: a batch is `64 * W` lanes.
pub const W: usize = 8;
pub const LANES: usize = 64 * W;
type Words = [u64; W];

/// Executed-gate tallies summed over a batch's active lanes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tally {
    pub ccx: u64,
    pub ccz: u64,
    pub cliffords: u64,
    pub hmr: u64,
    pub resets: u64,
    pub givens: u64,
    /// Executed `Reflect` ops (nested DF).
    pub reflects: u64,
    /// Executed `SpinSwap` ops (sos-sa, spec/SPEC-SA.md section 11), whatever their control.
    pub spin_swaps: u64,
    /// Tapered sos-sa specs only (spec/SPEC-SA.md section 13): the summed per-position charge of
    /// the executed `Givens` (`TrackerFactory::givens_charge`); 0 for every other tracker.
    pub givens_charge: u64,
}

/// Per-lane data for the nested `Reflect` (spec/DESIGN.md section 16): the inner register's
/// qubits in bit order, the value each lane's inner register must hold at the `Reflect` (`a`,
/// bit planes) and the bits to flip there (`a ^ b`).
pub struct ReflectLanes {
    pub qubits: Vec<u32>,
    pub want: Vec<[u64; W]>,
    pub flip: Vec<[u64; W]>,
}

/// A failure inside execution: the batch-local lane and the reason.
#[derive(Debug)]
pub struct ExecError {
    pub lane: usize,
    pub kind: &'static str,
    pub msg: String,
}

pub struct Lanes<'a> {
    pub q: Vec<u64>,
    pub b: Vec<u64>,
    pub p: [Words; 3],
    pub x: Vec<u64>,
    pub z: Vec<u64>,
    pub s0: Vec<u64>,
    pub s1: Vec<u64>,
    pub active: Words,
    pub tally: Tally,
    pub trackers: Vec<Option<Box<dyn LaneTracker>>>,
    pub system: usize,
    /// Set by the validator when the lane map is nested; `None` rejects any `Reflect`.
    pub reflect: Option<ReflectLanes>,
    factory: Option<&'a dyn TrackerFactory>,
    registers: &'a [Vec<u32>],
}

fn first_lane(w: &Words) -> usize {
    w.iter()
        .enumerate()
        .find(|(_, v)| **v != 0)
        .map_or(0, |(j, v)| 64 * j + v.trailing_zeros() as usize)
}

fn pop(w: &Words) -> u64 {
    w.iter().map(|v| u64::from(v.count_ones())).sum()
}

impl<'a> Lanes<'a> {
    #[must_use]
    pub fn new(c: &'a Compiled, system: usize, factory: Option<&'a dyn TrackerFactory>) -> Self {
        let zeros = |n: usize| vec![0u64; n * W];
        Self {
            q: zeros(c.num_qubits as usize),
            b: zeros(c.num_bits as usize),
            p: [[0; W]; 3],
            x: zeros(system),
            z: zeros(system),
            s0: zeros(system),
            s1: zeros(system),
            active: [0; W],
            tally: Tally::default(),
            trackers: (0..LANES).map(|_| None).collect(),
            system,
            reflect: None,
            factory,
            registers: &c.registers,
        }
    }

    #[inline(always)]
    fn w(&self, q: u32) -> &[u64] {
        let i = q as usize * W;
        &self.q[i..i + W]
    }

    #[inline(always)]
    fn and(&self, m: &mut Words, q: u32) {
        if q != NONE {
            let w = self.w(q);
            for j in 0..W {
                m[j] &= w[j];
            }
        }
    }

    #[inline(always)]
    fn mask(&self, base: &Words, cond: u32) -> Words {
        let mut m = *base;
        if cond != NONE {
            let i = cond as usize * W;
            for j in 0..W {
                m[j] &= self.b[i + j];
            }
        }
        m
    }

    /// Adds `k` in Z/8 to the phase of every lane in `m` (ripple-carry over the three planes).
    #[inline(always)]
    fn add_phase(&mut self, m: &Words, k: u8) {
        if k == 4 {
            for j in 0..W {
                self.p[2][j] ^= m[j];
            }
            return;
        }
        for j in 0..W {
            let mut carry = 0u64;
            for (bit, plane) in self.p.iter_mut().enumerate() {
                let a = if k >> bit & 1 == 1 { m[j] } else { 0 };
                let v = plane[j];
                plane[j] = v ^ a ^ carry;
                carry = (v & a) | (carry & (v ^ a));
            }
        }
    }

    /// Runs `ops` on this batch. `rng` supplies each `Hmr`'s outcomes.
    ///
    /// # Errors
    /// A dirty reset or a tracker rejection, with the first offending lane.
    pub fn run(&mut self, ops: &[SimOp], rng: &mut impl XofReader) -> Result<(), ExecError> {
        let mut stack: Vec<Words> = Vec::new();
        let mut base = self.active;
        for (i, op) in ops.iter().enumerate() {
            match *op {
                SimOp::Push { bit } => {
                    stack.push(base);
                    base = self.mask(&base, bit);
                }
                SimOp::Pop => base = stack.pop().unwrap_or(self.active),
                _ => self.step(op, &base, i, rng)?,
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn step(
        &mut self,
        op: &SimOp,
        base: &Words,
        i: usize,
        rng: &mut impl XofReader,
    ) -> Result<(), ExecError> {
        match *op {
            SimOp::X { t, cond } => self.xor_into(t, &self.mask(base, cond)),
            SimOp::Cx { c, t, cond } => {
                let mut m = self.mask(base, cond);
                self.tally.cliffords += pop(&m);
                self.and(&mut m, c);
                self.xor_into(t, &m);
            }
            SimOp::Ccx { a, b, t, cond } => {
                let mut m = self.mask(base, cond);
                self.tally.ccx += pop(&m);
                self.and(&mut m, a);
                self.and(&mut m, b);
                self.xor_into(t, &m);
            }
            SimOp::Swap { a, b, cond } => self.swap(a, b, &self.mask(base, cond)),
            SimOp::Phase { q, k, cond } => self.phase(q, k, &self.mask(base, cond)),
            SimOp::Neg { cond } => self.add_phase(&self.mask(base, cond), 4),
            SimOp::Hmr { t, bit, cond } => self.hmr(t, bit, &self.mask(base, cond), rng),
            SimOp::Reset { t } => return self.reset(t, base, i),
            SimOp::Bit { bit, kind, cond } => self.bit(bit, kind, &self.mask(base, cond)),
            SimOp::Sys { q, z, ctrl, cond } => self.sys(q, z, ctrl, &self.mask(base, cond)),
            SimOp::SysS { q, k, cond } => self.sys_s(q, k, &self.mask(base, cond)),
            SimOp::Givens { p, q, reg, cond } => {
                return self.givens([p, q], reg, &self.mask(base, cond))
            }
            SimOp::Reflect => return self.reflect(base, i),
            SimOp::SpinSwap { c, cond, dagger } => {
                return self.spin_swap(c, dagger, &self.mask(base, cond))
            }
            SimOp::Push { .. } | SimOp::Pop => {}
        }
        Ok(())
    }

    /// Nested `Reflect` on the lanes in `m`: each lane's inner register must hold its `a`; it
    /// then holds `b` (spec/DESIGN.md section 16).
    fn reflect(&mut self, m: &Words, i: usize) -> Result<(), ExecError> {
        let Some(r) = self.reflect.take() else {
            return Err(ExecError {
                lane: first_lane(m),
                kind: "reflect",
                msg: format!("op {i}: Reflect without a nested lane map"),
            });
        };
        self.tally.reflects += pop(m);
        let mut bad = [0u64; W];
        for (j, &q) in r.qubits.iter().enumerate() {
            let iq = q as usize * W;
            for w in 0..W {
                bad[w] |= (self.q[iq + w] ^ r.want[j][w]) & m[w];
            }
        }
        if bad.iter().any(|&v| v != 0) {
            let lane = first_lane(&bad);
            self.reflect = Some(r);
            return Err(ExecError {
                lane,
                kind: "reflect",
                msg: format!(
                    "op {i}: at the Reflect the inner register does not hold the lane's inner \
                     value; the first inner copy must restore it"
                ),
            });
        }
        for (j, &q) in r.qubits.iter().enumerate() {
            let iq = q as usize * W;
            for w in 0..W {
                self.q[iq + w] ^= r.flip[j][w] & m[w];
            }
        }
        self.reflect = Some(r);
        Ok(())
    }

    #[inline(always)]
    fn xor_into(&mut self, t: u32, m: &Words) {
        let i = t as usize * W;
        for j in 0..W {
            self.q[i + j] ^= m[j];
        }
    }

    fn swap(&mut self, a: u32, b: u32, m: &Words) {
        self.tally.cliffords += pop(m);
        let (ia, ib) = (a as usize * W, b as usize * W);
        for j in 0..W {
            let d = (self.q[ia + j] ^ self.q[ib + j]) & m[j];
            self.q[ia + j] ^= d;
            self.q[ib + j] ^= d;
        }
    }

    fn phase(&mut self, q: [u32; 3], k: u8, m: &Words) {
        let n = q.iter().filter(|&&x| x != NONE).count();
        match (n, k) {
            (3, _) => self.tally.ccz += pop(m),
            (2, _) | (_, 2 | 6) => self.tally.cliffords += pop(m),
            _ => {}
        }
        let mut m = *m;
        for &x in &q {
            self.and(&mut m, x);
        }
        self.add_phase(&m, k);
    }

    fn hmr(&mut self, t: u32, bit: u32, m: &Words, rng: &mut impl XofReader) {
        self.tally.hmr += pop(m);
        let mut buf = [0u8; 8 * W];
        rng.read(&mut buf);
        let (it, ib) = (t as usize * W, bit as usize * W);
        let mut kick = [0u64; W];
        for j in 0..W {
            let r = u64::from_le_bytes(buf[8 * j..8 * j + 8].try_into().unwrap_or([0; 8]));
            kick[j] = self.q[it + j] & r & m[j];
            self.b[ib + j] = (self.b[ib + j] & !m[j]) | (r & m[j]);
            self.q[it + j] &= !m[j];
        }
        self.add_phase(&kick, 4);
    }

    fn reset(&mut self, t: u32, m: &Words, i: usize) -> Result<(), ExecError> {
        self.tally.resets += pop(m);
        let it = t as usize * W;
        let mut dirty = [0u64; W];
        for j in 0..W {
            dirty[j] = self.q[it + j] & m[j];
        }
        if dirty.iter().any(|&d| d != 0) {
            return Err(ExecError {
                lane: first_lane(&dirty),
                kind: "dirty-ancilla",
                msg: format!("op {i}: ancilla q{t} is 1 when freed (R); freed qubits must be |0>"),
            });
        }
        Ok(())
    }

    fn bit(&mut self, bit: u32, kind: u8, m: &Words) {
        let i = bit as usize * W;
        for j in 0..W {
            let v = &mut self.b[i + j];
            *v = match kind {
                0 => *v ^ m[j],
                1 => *v & !m[j],
                _ => *v | m[j],
            };
        }
    }

    fn sys(&mut self, q: u32, z: bool, ctrl: [u32; 2], m: &Words) {
        let mut m = *m;
        match ctrl.iter().filter(|&&c| c != NONE).count() {
            2 if z => self.tally.ccz += pop(&m),
            2 => self.tally.ccx += pop(&m),
            1 => self.tally.cliffords += pop(&m),
            _ => {}
        }
        for c in ctrl {
            self.and(&mut m, c);
        }
        let i = q as usize * W;
        if z {
            let mut k = [0u64; W];
            for j in 0..W {
                k[j] = m[j] & self.x[i + j];
                self.z[i + j] ^= m[j];
            }
            self.add_phase(&k, 4);
            return;
        }
        // phase += 2 s_q; s_q = -s_q; x_q ^= 1 (masked).
        let (mut d0, mut d1) = ([0u64; W], [0u64; W]);
        for j in 0..W {
            d0[j] = self.s0[i + j] & m[j];
            d1[j] = self.s1[i + j] & m[j];
            self.s1[i + j] ^= d0[j];
            self.x[i + j] ^= m[j];
        }
        self.add_phase(&d0, 2);
        self.add_phase(&d1, 4);
    }

    fn sys_s(&mut self, q: u32, k: u8, m: &Words) {
        self.tally.cliffords += pop(m);
        let i = q as usize * W;
        for j in 0..W {
            let s0 = self.s0[i + j];
            // +1: s1 ^= s0 & m, s0 ^= m.   +3 (= -1): s1 ^= !s0 & m, s0 ^= m.
            let carry = if k == 1 { s0 } else { !s0 };
            self.s1[i + j] ^= carry & m[j];
            self.s0[i + j] = s0 ^ m[j];
        }
    }

    /// The lane's frame `D X^x Z^z` (phase included when `with_phase`).
    #[must_use]
    pub fn lane_frame(&self, lane: usize, with_phase: bool) -> LaneFrame {
        let (j, bit) = (lane / 64, lane % 64);
        let n = self.system;
        let words = n.div_ceil(64);
        let mut f = LaneFrame {
            x: vec![0; words],
            z: vec![0; words],
            s_pow: vec![0; n],
            phase: 0,
        };
        for q in 0..n {
            let g = |v: &[u64]| v[q * W + j] >> bit & 1;
            f.x[q / 64] |= g(&self.x) << (q % 64);
            f.z[q / 64] |= g(&self.z) << (q % 64);
            f.s_pow[q] = u8::try_from(g(&self.s0) | g(&self.s1) << 1).unwrap_or(0);
        }
        if with_phase {
            let ph = self
                .p
                .iter()
                .enumerate()
                .map(|(b, pl)| (pl[j] >> bit & 1) << b)
                .sum::<u64>();
            f.phase = u8::try_from(ph).unwrap_or(0);
        }
        f
    }

    /// `SpinSwap` (`F`, or `F^dagger` when `dagger`) on the lanes in `m`: charged on all of them;
    /// where the control `c` is also 1, the tracker applies `G_{2p, 2p+1}(+-pi / 2)` for every `p`, which is the Givens path with the
    /// angle register replaced by the constant `pi / 2` (spec/SPEC-SA.md section 11).
    fn spin_swap(&mut self, c: u32, dagger: bool, m: &Words) -> Result<(), ExecError> {
        let fail = |lane, msg: String| ExecError {
            lane,
            kind: "spin_swap",
            msg,
        };
        let Some(factory) = self.factory else {
            return Err(fail(0, "no tracker".into()));
        };
        let Some(quarter) = factory.quarter_turn() else {
            return Err(fail(0, "the tracker has no quarter turn".into()));
        };
        // F^dagger: G(-pi / 2) = G(3 pi / 2) on every pair (the angle is read mod 2^beta).
        let angle = if dagger { 3 * quarter } else { quarter };
        self.tally.spin_swaps += pop(m);
        let mut on = *m;
        self.and(&mut on, c);
        let n = self.system;
        for lane in (0..LANES).filter(|&l| on[l / 64] >> (l % 64) & 1 == 1) {
            let (j, bit) = (lane / 64, lane % 64);
            for p in 0..n / 2 {
                let before = self.lane_frame(lane, false);
                let tr = self.trackers[lane].get_or_insert_with(|| factory.lane(n));
                tr.givens_modes(&before, 2 * p, 2 * p + 1, angle)
                    .map_err(|msg| fail(lane, msg))?;
                for q in 0..n {
                    let clear = !(1u64 << bit);
                    for plane in [&mut self.x, &mut self.z, &mut self.s0, &mut self.s1] {
                        plane[q * W + j] &= clear;
                    }
                }
            }
        }
        Ok(())
    }

    fn givens(&mut self, [p, q]: [u32; 2], reg: u32, m: &Words) -> Result<(), ExecError> {
        let Some(factory) = self.factory else {
            return Err(ExecError {
                lane: 0,
                kind: "givens",
                msg: "no tracker".into(),
            });
        };
        let qubits = self.registers[reg as usize].clone();
        let charge = factory.givens_charge(p as usize, q as usize);
        for lane in (0..LANES).filter(|&l| m[l / 64] >> (l % 64) & 1 == 1) {
            self.tally.givens += 1;
            if let Some(c) = charge {
                self.tally.givens_charge += c;
            }
            let (j, bit) = (lane / 64, lane % 64);
            let angle = qubits.iter().enumerate().fold(0u64, |a, (k, &q)| {
                a | (self.q[q as usize * W + j] >> bit & 1) << k
            });
            let before = self.lane_frame(lane, false);
            let n = self.system;
            let tr = self.trackers[lane].get_or_insert_with(|| factory.lane(n));
            tr.givens_modes(&before, p as usize, q as usize, angle)
                .map_err(|msg| ExecError {
                    lane,
                    kind: "givens",
                    msg,
                })?;
            for q in 0..n {
                let clear = !(1u64 << bit);
                for plane in [&mut self.x, &mut self.z, &mut self.s0, &mut self.s1] {
                    plane[q * W + j] &= clear;
                }
            }
        }
        Ok(())
    }
}
