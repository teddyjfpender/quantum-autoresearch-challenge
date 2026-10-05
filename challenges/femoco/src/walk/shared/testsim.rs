//! A one-lane basis-state simulator for unit tests of the shared components (not the harness's
//! simulator, which is the authority). Non-system qubits carry bits, the lane carries a phase in
//! `Z/8`, measurement outcomes come from a seeded generator, and system wires are ignored.
//! `assert_clean` checks what the harness checks for ancillas: everything freed or `|0>`, and
//! phase `+1`.
use crate::circuit::{Builder, Op, OperationType as K, NONE};

pub struct Sim {
    q: Vec<bool>,
    bits: Vec<bool>,
    conds: Vec<u32>,
    phase: u8,
    rng: u64,
    first_ancilla: usize,
    uniform: (usize, usize),
    pub toffolis: u64,
}

impl Sim {
    #[must_use]
    pub fn new(b: &Builder, seed: u64) -> Self {
        let sys = b.system_qubits();
        let u = b.uniform_bits() as usize;
        Self {
            q: vec![false; 1 << 20],
            bits: vec![false; 1 << 20],
            conds: Vec::new(),
            phase: 0,
            rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            first_ancilla: 1 + sys + u,
            uniform: (1 + sys, u),
            toffolis: 0,
        }
    }

    pub fn set_uniform(&mut self, v: u64) {
        let (at, u) = self.uniform;
        for i in 0..u {
            self.q[at + i] = v >> i & 1 == 1;
        }
    }

    pub fn set(&mut self, q: crate::circuit::Qubit, v: bool) {
        self.q[q.0 as usize] = v;
    }

    #[must_use]
    pub fn read(&self, qs: &[crate::circuit::Qubit]) -> u64 {
        qs.iter()
            .enumerate()
            .map(|(i, q)| u64::from(self.q[q.0 as usize]) << i)
            .sum()
    }

    fn coin(&mut self) -> bool {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng & 1 == 1
    }

    fn on(&self, op: &Op) -> bool {
        (op.c_condition == NONE || self.bits[op.c_condition as usize])
            && self.conds.iter().all(|&c| self.bits[c as usize])
    }

    pub fn run(&mut self, ops: &[Op]) {
        for op in ops {
            if op.kind == K::PopCondition {
                self.conds.pop();
                continue;
            }
            if op.kind == K::PushCondition {
                self.conds.push(op.c_condition);
                continue;
            }
            if !self.on(op) {
                continue;
            }
            let (t, c1, c2) = (
                op.q_target as usize,
                op.q_control1 as usize,
                op.q_control2 as usize,
            );
            match op.kind {
                K::X => self.q[t] ^= true,
                K::CX => self.q[t] ^= self.q[c1],
                K::CCX => {
                    self.toffolis += 1;
                    self.q[t] ^= self.q[c1] && self.q[c2];
                }
                K::Swap => self.q.swap(t, c1),
                K::Z if self.q[t] => self.phase = (self.phase + 4) % 8,
                K::CZ if self.q[t] && self.q[c1] => self.phase = (self.phase + 4) % 8,
                K::CCZ => {
                    self.toffolis += 1;
                    if self.q[t] && self.q[c1] && self.q[c2] {
                        self.phase = (self.phase + 4) % 8;
                    }
                }
                K::S if self.q[t] => self.phase = (self.phase + 2) % 8,
                K::Sdg if self.q[t] => self.phase = (self.phase + 6) % 8,
                K::Neg => self.phase = (self.phase + 4) % 8,
                K::Hmr => {
                    let m = self.coin();
                    if m && self.q[t] {
                        self.phase = (self.phase + 4) % 8;
                    }
                    self.q[t] = false;
                    self.bits[op.c_target as usize] = m;
                }
                K::R => assert!(!self.q[t], "R on a qubit holding 1 (q{t})"),
                K::BitInvert => self.bits[op.c_target as usize] ^= true,
                K::BitStore0 => self.bits[op.c_target as usize] = false,
                K::BitStore1 => self.bits[op.c_target as usize] = true,
                _ => {}
            }
        }
    }

    /// Every ancilla is 0 and the phase is +1.
    pub fn assert_clean(&self) {
        let dirty: Vec<usize> = (self.first_ancilla..self.q.len())
            .filter(|&i| self.q[i])
            .collect();
        assert!(dirty.is_empty(), "dirty ancillas {dirty:?}");
        assert_eq!(self.phase, 0, "phase garbage");
    }
}
