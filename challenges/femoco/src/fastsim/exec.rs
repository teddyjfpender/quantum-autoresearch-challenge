//! Bit-sliced execution of one 512-lane batch, recording each lane's tracker inputs instead of
//! running a tracker.
//!
//! The state layout, every classical op, the phase arithmetic, the `Sys` / `SysS` frame rules,
//! `Hmr` (64 outcome bytes per op reached, in op order), strict `R`, `Reflect` and every tally
//! are those of `crate::sim::lanes` (the reference); `tests` check each op kind against it on
//! random states. Two things differ, neither observable:
//!
//! - `Givens` and `SpinSwap` do not call a tracker. For each executing lane they append the
//!   exact tracker call the reference would make (`before` frame, `p`, `q`, angle) to the lane's
//!   trace, then clear the lane's frame with one masked pass instead of one lane at a time. The
//!   frame is extracted only for lanes whose frame planes changed since their last hand-off
//!   (`dirty`); every other lane's frame is the identity, which is what extraction would give.
//! - An execution error (dirty `R`, `Reflect` mismatch, tracker refusal) only marks the batch as
//!   failed: the caller re-runs a failed batch on the reference engine, which reports it.
#![allow(clippy::needless_range_loop)]
use crate::circuit::NONE;
use crate::sim::compile::{Compiled, SimOp};
use crate::sim::lanes::{ReflectLanes, Tally, W};
use crate::sim::tracker::{LaneFrame, TrackerFactory};
use sha3::digest::XofReader;

/// One plane of a pass: `WW` words, 64 lanes each. A pass is `WW / W` consecutive reference
/// batches (512 lanes each, `W` words); batch `i` of the pass owns words `i W .. (i + 1) W`.
pub type Words<const WW: usize> = [u64; WW];

#[inline(always)]
fn pop<const WW: usize>(w: &Words<WW>) -> u64 {
    w.iter().map(|v| u64::from(v.count_ones())).sum()
}

#[inline(always)]
fn any<const WW: usize>(w: &Words<WW>) -> bool {
    w.iter().any(|&v| v != 0)
}

/// Trace record tags (see `put_frame`, `Exec::record`).
pub const TAG_CALL_ID: u8 = 1;
pub const TAG_CALL_FRAME: u8 = 2;

/// LEB128.
#[inline]
pub fn put_var(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Canonical bytes of a frame's `x`, `z` and `s_pow` (phase separate): `words` little-endian
/// words of `x`, then of `z`, then `s_pow` two bits per qubit.
pub fn put_frame(out: &mut Vec<u8>, f: &LaneFrame) {
    for w in f.x.iter().chain(&f.z) {
        out.extend_from_slice(&w.to_le_bytes());
    }
    for c in f.s_pow.chunks(4) {
        let mut b = 0u8;
        for (i, &s) in c.iter().enumerate() {
            b |= (s & 3) << (2 * i);
        }
        out.push(b);
    }
}

/// `Reflect`'s data for a pass: per register qubit, the wanted value and the flip, `WW` words
/// each (the reference's `ReflectLanes` of each batch, side by side).
pub struct WideReflect {
    pub qubits: Vec<u32>,
    pub want: Vec<u64>,
    pub flip: Vec<u64>,
}

impl WideReflect {
    /// The pass's data from each batch's `ReflectLanes` (same register in every batch).
    #[must_use]
    pub fn new<const WW: usize>(batches: &[ReflectLanes]) -> Self {
        let qubits = batches
            .first()
            .map(|b| b.qubits.clone())
            .unwrap_or_default();
        let nq = qubits.len();
        let mut want = vec![0u64; nq * WW];
        let mut flip = vec![0u64; nq * WW];
        for (i, b) in batches.iter().enumerate() {
            for j in 0..nq {
                for w in 0..W {
                    want[j * WW + i * W + w] = b.want[j][w];
                    flip[j * WW + i * W + w] = b.flip[j][w];
                }
            }
        }
        Self { qubits, want, flip }
    }
}

pub struct Exec<'a, const WW: usize> {
    pub q: Vec<u64>,
    pub b: Vec<u64>,
    pub p: [Words<WW>; 3],
    pub x: Vec<u64>,
    pub z: Vec<u64>,
    pub s0: Vec<u64>,
    pub s1: Vec<u64>,
    pub active: Words<WW>,
    pub tally: Tally,
    /// Lanes whose frame planes may be non-zero (set by `Sys`/`SysS`, cleared at a hand-off).
    pub dirty: Words<WW>,
    /// Per lane: the tracker calls the reference would make, in order (empty: no tracker).
    pub traces: Vec<Vec<u8>>,
    pub system: usize,
    /// `Reflect`'s per-lane data for the pass (`set_reflect`).
    pub reflect: Option<WideReflect>,
    /// Set by any execution error; the batch is then re-run on the reference engine.
    pub failed: bool,
    factory: Option<&'a dyn TrackerFactory>,
    registers: &'a [Vec<u32>],
}

impl<'a, const WW: usize> Exec<'a, WW> {
    #[must_use]
    pub fn new(c: &'a Compiled, system: usize, factory: Option<&'a dyn TrackerFactory>) -> Self {
        let zeros = |n: usize| vec![0u64; n * WW];
        Self {
            q: zeros(c.num_qubits as usize),
            b: zeros(c.num_bits as usize),
            p: [[0; WW]; 3],
            x: zeros(system),
            z: zeros(system),
            s0: zeros(system),
            s1: zeros(system),
            active: [0; WW],
            tally: Tally::default(),
            dirty: [0; WW],
            traces: (0..64 * WW).map(|_| Vec::new()).collect(),
            system,
            reflect: None,
            failed: false,
            factory,
            registers: &c.registers,
        }
    }

    #[inline(always)]
    fn and(&self, m: &mut Words<WW>, q: u32) {
        if q != NONE {
            let i = q as usize * WW;
            for j in 0..WW {
                m[j] &= self.q[i + j];
            }
        }
    }

    #[inline(always)]
    fn mask(&self, base: &Words<WW>, cond: u32) -> Words<WW> {
        let mut m = *base;
        if cond != NONE {
            let i = cond as usize * WW;
            for j in 0..WW {
                m[j] &= self.b[i + j];
            }
        }
        m
    }

    #[inline(always)]
    fn add_phase(&mut self, m: &Words<WW>, k: u8) {
        if k == 4 {
            for j in 0..WW {
                self.p[2][j] ^= m[j];
            }
            return;
        }
        for j in 0..WW {
            let mut carry = 0u64;
            for (bit, plane) in self.p.iter_mut().enumerate() {
                let a = if k >> bit & 1 == 1 { m[j] } else { 0 };
                let v = plane[j];
                plane[j] = v ^ a ^ carry;
                carry = (v & a) | (carry & (v ^ a));
            }
        }
    }

    /// Runs `ops`; stops at the first execution error (`failed`). `rngs[i]` is the outcome
    /// stream of the pass's batch `i` (`WW / W` of them).
    pub fn run<R: XofReader>(&mut self, ops: &[SimOp], rngs: &mut [R]) {
        assert_eq!(
            rngs.len() * W,
            WW,
            "one outcome stream per batch of the pass"
        );
        let mut stack: Vec<Words<WW>> = Vec::new();
        let mut base = self.active;
        for op in ops {
            match *op {
                SimOp::Push { bit } => {
                    stack.push(base);
                    base = self.mask(&base, bit);
                }
                SimOp::Pop => base = stack.pop().unwrap_or(self.active),
                _ => {
                    self.step(op, &base, rngs);
                    if self.failed {
                        return;
                    }
                }
            }
        }
    }

    #[inline(always)]
    pub fn step<R: XofReader>(&mut self, op: &SimOp, base: &Words<WW>, rngs: &mut [R]) {
        match *op {
            SimOp::X { t, cond } => {
                let m = self.mask(base, cond);
                self.xor_into(t, &m);
            }
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
            SimOp::Swap { a, b, cond } => {
                let m = self.mask(base, cond);
                self.swap(a, b, &m);
            }
            SimOp::Phase { q, k, cond } => {
                let m = self.mask(base, cond);
                self.phase(q, k, &m);
            }
            SimOp::Neg { cond } => {
                let m = self.mask(base, cond);
                self.add_phase(&m, 4);
            }
            SimOp::Hmr { t, bit, cond } => {
                let m = self.mask(base, cond);
                self.hmr(t, bit, &m, rngs);
            }
            SimOp::Reset { t } => self.reset(t, base),
            SimOp::Bit { bit, kind, cond } => {
                let m = self.mask(base, cond);
                self.bit(bit, kind, &m);
            }
            SimOp::Sys { q, z, ctrl, cond } => {
                let m = self.mask(base, cond);
                self.sys(q, z, ctrl, &m);
            }
            SimOp::SysS { q, k, cond } => {
                let m = self.mask(base, cond);
                self.sys_s(q, k, &m);
            }
            SimOp::Givens { p, q, reg, cond } => {
                let m = self.mask(base, cond);
                self.givens(p, q, reg, &m);
            }
            SimOp::Reflect => self.reflect(base),
            SimOp::SpinSwap { c, cond, dagger } => {
                let m = self.mask(base, cond);
                self.spin_swap(c, dagger, &m);
            }
            SimOp::Push { .. } | SimOp::Pop => {}
        }
    }

    fn reflect(&mut self, m: &Words<WW>) {
        let Some(r) = self.reflect.take() else {
            self.failed = true;
            return;
        };
        self.tally.reflects += pop(m);
        let mut bad = [0u64; WW];
        for (j, &q) in r.qubits.iter().enumerate() {
            let iq = q as usize * WW;
            for w in 0..WW {
                bad[w] |= (self.q[iq + w] ^ r.want[j * WW + w]) & m[w];
            }
        }
        if any(&bad) {
            self.failed = true;
        } else {
            for (j, &q) in r.qubits.iter().enumerate() {
                let iq = q as usize * WW;
                for w in 0..WW {
                    self.q[iq + w] ^= r.flip[j * WW + w] & m[w];
                }
            }
        }
        self.reflect = Some(r);
    }

    #[inline(always)]
    fn xor_into(&mut self, t: u32, m: &Words<WW>) {
        let i = t as usize * WW;
        for j in 0..WW {
            self.q[i + j] ^= m[j];
        }
    }

    fn swap(&mut self, a: u32, b: u32, m: &Words<WW>) {
        self.tally.cliffords += pop(m);
        let (ia, ib) = (a as usize * WW, b as usize * WW);
        for j in 0..WW {
            let d = (self.q[ia + j] ^ self.q[ib + j]) & m[j];
            self.q[ia + j] ^= d;
            self.q[ib + j] ^= d;
        }
    }

    fn phase(&mut self, q: [u32; 3], k: u8, m: &Words<WW>) {
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

    fn hmr<R: XofReader>(&mut self, t: u32, bit: u32, m: &Words<WW>, rngs: &mut [R]) {
        self.tally.hmr += pop(m);
        // Each batch reads its own 64 bytes per `Hmr` reached, as the reference does.
        let mut words = [0u64; WW];
        let mut buf = [0u8; 8 * W];
        for (i, rng) in rngs.iter_mut().enumerate() {
            rng.read(&mut buf);
            for k in 0..W {
                words[i * W + k] =
                    u64::from_le_bytes(buf[8 * k..8 * k + 8].try_into().unwrap_or([0; 8]));
            }
        }
        let (it, ib) = (t as usize * WW, bit as usize * WW);
        let mut kick = [0u64; WW];
        for j in 0..WW {
            let r = words[j];
            kick[j] = self.q[it + j] & r & m[j];
            self.b[ib + j] = (self.b[ib + j] & !m[j]) | (r & m[j]);
            self.q[it + j] &= !m[j];
        }
        self.add_phase(&kick, 4);
    }

    fn reset(&mut self, t: u32, m: &Words<WW>) {
        self.tally.resets += pop(m);
        let it = t as usize * WW;
        for j in 0..WW {
            if self.q[it + j] & m[j] != 0 {
                self.failed = true;
            }
        }
    }

    fn bit(&mut self, bit: u32, kind: u8, m: &Words<WW>) {
        let i = bit as usize * WW;
        for j in 0..WW {
            let v = &mut self.b[i + j];
            *v = match kind {
                0 => *v ^ m[j],
                1 => *v & !m[j],
                _ => *v | m[j],
            };
        }
    }

    fn sys(&mut self, q: u32, z: bool, ctrl: [u32; 2], m: &Words<WW>) {
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
        for j in 0..WW {
            self.dirty[j] |= m[j];
        }
        let i = q as usize * WW;
        if z {
            let mut k = [0u64; WW];
            for j in 0..WW {
                k[j] = m[j] & self.x[i + j];
                self.z[i + j] ^= m[j];
            }
            self.add_phase(&k, 4);
            return;
        }
        let (mut d0, mut d1) = ([0u64; WW], [0u64; WW]);
        for j in 0..WW {
            d0[j] = self.s0[i + j] & m[j];
            d1[j] = self.s1[i + j] & m[j];
            self.s1[i + j] ^= d0[j];
            self.x[i + j] ^= m[j];
        }
        self.add_phase(&d0, 2);
        self.add_phase(&d1, 4);
    }

    fn sys_s(&mut self, q: u32, k: u8, m: &Words<WW>) {
        self.tally.cliffords += pop(m);
        for j in 0..WW {
            self.dirty[j] |= m[j];
        }
        let i = q as usize * WW;
        for j in 0..WW {
            let s0 = self.s0[i + j];
            let carry = if k == 1 { s0 } else { !s0 };
            self.s1[i + j] ^= carry & m[j];
            self.s0[i + j] = s0 ^ m[j];
        }
    }

    /// The lane's frame `D X^x Z^z` (phase included when `with_phase`): `Lanes::lane_frame`.
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
            let g = |v: &[u64]| v[q * WW + j] >> bit & 1;
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

    /// Appends the tracker call `(before, p, q, angle)` to `lane`'s trace; `before` is the
    /// lane's current frame (phase excluded), or the identity when the lane is not dirty.
    fn record(&mut self, lane: usize, p: u32, q: u32, angle: u64) {
        let (j, bit) = (lane / 64, lane % 64);
        let frame = (self.dirty[j] >> bit & 1 == 1)
            .then(|| self.lane_frame(lane, false))
            .filter(|f| f.x.iter().chain(&f.z).any(|&w| w != 0) || f.s_pow.iter().any(|&s| s != 0));
        let t = &mut self.traces[lane];
        t.push(if frame.is_some() {
            TAG_CALL_FRAME
        } else {
            TAG_CALL_ID
        });
        put_var(t, u64::from(p));
        put_var(t, u64::from(q));
        put_var(t, angle);
        if let Some(f) = frame {
            put_frame(t, &f);
        }
    }

    /// Clears the frame of every lane in `m` (the reference clears them lane by lane).
    fn clear_frames(&mut self, m: &Words<WW>) {
        let live: Words<WW> = std::array::from_fn(|j| m[j] & self.dirty[j]);
        if !any(&live) {
            return;
        }
        for plane in [&mut self.x, &mut self.z, &mut self.s0, &mut self.s1] {
            for q in 0..self.system {
                for j in 0..WW {
                    plane[q * WW + j] &= !m[j];
                }
            }
        }
        for j in 0..WW {
            self.dirty[j] &= !m[j];
        }
    }

    fn spin_swap(&mut self, c: u32, dagger: bool, m: &Words<WW>) {
        let Some(quarter) = self.factory.and_then(TrackerFactory::quarter_turn) else {
            self.failed = true;
            return;
        };
        let angle = if dagger { 3 * quarter } else { quarter };
        self.tally.spin_swaps += pop(m);
        let mut on = *m;
        self.and(&mut on, c);
        let n = self.system;
        for j in 0..WW {
            let mut w = on[j];
            while w != 0 {
                let lane = 64 * j + w.trailing_zeros() as usize;
                w &= w - 1;
                // The reference hands the frame over at p = 0 and clears it; later calls see
                // the identity.
                for p in 0..n / 2 {
                    self.record(lane, 2 * p as u32, 2 * p as u32 + 1, angle);
                    if p == 0 {
                        let bit = [0u64; WW];
                        let mut one = bit;
                        one[j] = 1u64 << (lane % 64);
                        self.clear_frames(&one);
                    }
                }
            }
        }
        self.clear_frames(&on);
    }

    fn givens(&mut self, p: u32, q: u32, reg: u32, m: &Words<WW>) {
        let Some(factory) = self.factory else {
            self.failed = true;
            return;
        };
        let cnt = pop(m);
        self.tally.givens += cnt;
        if let Some(c) = factory.givens_charge(p as usize, q as usize) {
            self.tally.givens_charge += c * cnt;
        }
        let qubits = &self.registers[reg as usize];
        let reg_words: Vec<usize> = qubits.iter().map(|&q| q as usize * WW).collect();
        for j in 0..WW {
            let mut w = m[j];
            while w != 0 {
                let bit = w.trailing_zeros() as usize;
                w &= w - 1;
                let angle = reg_words
                    .iter()
                    .enumerate()
                    .fold(0u64, |a, (k, &iq)| a | (self.q[iq + j] >> bit & 1) << k);
                self.record(64 * j + bit, p, q, angle);
            }
        }
        self.clear_frames(m);
    }
}
