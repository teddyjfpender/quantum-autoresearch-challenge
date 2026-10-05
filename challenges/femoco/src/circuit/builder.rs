//! The builder contestant code emits ops through (UNTRUSTED side; `eval_circuit` re-checks
//! everything it relies on).
//!
//! Qubit layout, fixed by the harness (spec/DESIGN.md section 4): qubit 0 is the walk control,
//! qubits `1..=2N` the system register (system qubit `j` is qubit `1 + j`, interleaved spin
//! orbitals), then the `u` uniform-register qubits (uniform bit `i` is qubit `1 + 2N + i`,
//! little-endian). The walk must call `declare_uniform(u)` before anything else. Ancillas come
//! from `alloc`, which reuses the lowest freed id, and must be returned to `|0>` with `free`
//! (emits `R`) or `hmr` (measurement-based uncomputation; the circuit fixes the phase).
use super::{Op, OperationType as K, NONE};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Qubit(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bit(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Reg(pub u32);

pub struct Builder {
    ops: Vec<Op>,
    system: u32,
    uniform: Option<u32>,
    live: Vec<bool>,
    free_ids: BinaryHeap<Reverse<u32>>,
    next_bit: u32,
    next_reg: u32,
    depth: u32,
}

impl Builder {
    /// A builder for a system register of `system_qubits` (= 2N) qubits.
    #[must_use]
    pub fn new(system_qubits: usize) -> Self {
        let system = u32::try_from(system_qubits).expect("system register too large");
        Self {
            ops: Vec::new(),
            system,
            uniform: None,
            live: vec![true; 1 + system as usize],
            free_ids: BinaryHeap::new(),
            next_bit: 0,
            next_reg: 0,
            depth: 0,
        }
    }

    /// Declares the uniform register width `u` (the lane map's `uniform_bits`). Must be the
    /// first call on the builder.
    ///
    /// # Panics
    /// If called twice or after any op was emitted, or if `u > 63`.
    pub fn declare_uniform(&mut self, u: u32) {
        assert!(self.uniform.is_none(), "declare_uniform called twice");
        assert!(
            self.ops.is_empty(),
            "declare_uniform must come before any op"
        );
        assert!(u <= 63, "uniform register wider than 63 bits");
        self.uniform = Some(u);
        self.live.resize(self.live.len() + u as usize, true);
    }

    fn u(&self) -> u32 {
        self.uniform
            .expect("call declare_uniform(u) before emitting ops")
    }

    #[must_use]
    pub fn control(&self) -> Qubit {
        Qubit(0)
    }

    #[must_use]
    pub fn system_qubits(&self) -> usize {
        self.system as usize
    }

    /// System qubit `j` (spin orbital `j`, Jordan-Wigner position `j`).
    #[must_use]
    pub fn system(&self, j: usize) -> Qubit {
        assert!(j < self.system as usize, "system qubit {j} out of range");
        Qubit(1 + j as u32)
    }

    #[must_use]
    pub fn uniform_bits(&self) -> u32 {
        self.u()
    }

    /// Uniform-register bit `i` (little-endian: lane value `s` has bit `i` here).
    #[must_use]
    pub fn uniform(&self, i: u32) -> Qubit {
        assert!(i < self.u(), "uniform bit {i} out of range");
        Qubit(1 + self.system + i)
    }

    /// A fresh ancilla in `|0>`.
    pub fn alloc(&mut self) -> Qubit {
        let _ = self.u();
        let id = match self.free_ids.pop() {
            Some(Reverse(id)) => id,
            None => u32::try_from(self.live.len()).expect("too many qubits"),
        };
        if id as usize == self.live.len() {
            self.live.push(true);
        } else {
            self.live[id as usize] = true;
        }
        Qubit(id)
    }

    pub fn alloc_n(&mut self, n: usize) -> Vec<Qubit> {
        (0..n).map(|_| self.alloc()).collect()
    }

    fn release(&mut self, q: Qubit) {
        assert!(
            q.0 > self.system + self.u(),
            "only ancillas can be freed (q{})",
            q.0
        );
        self.check(q);
        self.live[q.0 as usize] = false;
        self.free_ids.push(Reverse(q.0));
    }

    /// Frees an ancilla that is `|0>` on every lane (emits `R`; `eval_circuit` rejects the run
    /// if it is not).
    pub fn free(&mut self, q: Qubit) {
        assert_eq!(self.depth, 0, "free inside a condition block");
        self.push(K::R, [q.0, NONE, NONE], NONE, NONE);
        self.release(q);
    }

    /// X-basis measurement and demolition of an ancilla into a new classical bit; the qubit is
    /// freed. A `1` on the qubit leaves phase `-1` on lanes whose outcome is 1, which the circuit
    /// must cancel (e.g. with `cz_if` on the former controls).
    pub fn hmr(&mut self, q: Qubit) -> Bit {
        assert_eq!(self.depth, 0, "hmr inside a condition block");
        let b = self.new_bit();
        let mut op = Op::new(K::Hmr);
        op.q_target = q.0;
        op.c_target = b.0;
        self.emit(op);
        self.release(q);
        b
    }

    pub fn new_bit(&mut self) -> Bit {
        self.next_bit += 1;
        Bit(self.next_bit - 1)
    }

    /// A register over `qubits` (little-endian), e.g. the angle register a `Givens` reads.
    pub fn register(&mut self, qubits: &[Qubit]) -> Reg {
        let r = self.next_reg;
        self.next_reg += 1;
        let mut op = Op::new(K::Register);
        op.r_target = r;
        self.emit(op);
        for q in qubits {
            let mut op = Op::new(K::AppendToRegister);
            op.q_target = q.0;
            op.r_target = r;
            self.emit(op);
        }
        Reg(r)
    }

    fn check(&self, q: Qubit) {
        assert!(
            self.live.get(q.0 as usize).copied().unwrap_or(false),
            "q{} is not allocated",
            q.0
        );
    }

    fn push(&mut self, kind: K, q: [u32; 3], cond: u32, reg: u32) {
        let mut op = Op::new(kind);
        (op.q_target, op.q_control1, op.q_control2) = (q[0], q[1], q[2]);
        op.c_condition = cond;
        op.r_target = reg;
        self.emit(op);
    }

    /// Emits a raw op after shape validation (for kinds without a helper).
    ///
    /// # Panics
    /// On a malformed op or an unallocated qubit.
    pub fn emit(&mut self, op: Op) {
        let _ = self.u();
        if let Err(e) = op.validate() {
            panic!("builder: {e}");
        }
        for q in op.qubits() {
            self.check(Qubit(q));
        }
        self.ops.push(op);
    }

    pub fn neg(&mut self) {
        self.push(K::Neg, [NONE; 3], NONE, NONE);
    }
    pub fn x(&mut self, t: Qubit) {
        self.push(K::X, [t.0, NONE, NONE], NONE, NONE);
    }
    pub fn z(&mut self, t: Qubit) {
        self.push(K::Z, [t.0, NONE, NONE], NONE, NONE);
    }
    pub fn s(&mut self, t: Qubit) {
        self.push(K::S, [t.0, NONE, NONE], NONE, NONE);
    }
    pub fn sdg(&mut self, t: Qubit) {
        self.push(K::Sdg, [t.0, NONE, NONE], NONE, NONE);
    }
    pub fn cx(&mut self, c: Qubit, t: Qubit) {
        self.push(K::CX, [t.0, c.0, NONE], NONE, NONE);
    }
    pub fn cz(&mut self, a: Qubit, b: Qubit) {
        self.push(K::CZ, [b.0, a.0, NONE], NONE, NONE);
    }
    pub fn swap(&mut self, a: Qubit, b: Qubit) {
        self.push(K::Swap, [b.0, a.0, NONE], NONE, NONE);
    }
    pub fn ccx(&mut self, a: Qubit, b: Qubit, t: Qubit) {
        self.push(K::CCX, [t.0, a.0, b.0], NONE, NONE);
    }
    pub fn ccz(&mut self, a: Qubit, b: Qubit, c: Qubit) {
        self.push(K::CCZ, [c.0, a.0, b.0], NONE, NONE);
    }
    pub fn x_if(&mut self, t: Qubit, b: Bit) {
        self.push(K::X, [t.0, NONE, NONE], b.0, NONE);
    }
    pub fn z_if(&mut self, t: Qubit, b: Bit) {
        self.push(K::Z, [t.0, NONE, NONE], b.0, NONE);
    }
    pub fn cx_if(&mut self, c: Qubit, t: Qubit, b: Bit) {
        self.push(K::CX, [t.0, c.0, NONE], b.0, NONE);
    }
    pub fn cz_if(&mut self, a: Qubit, t: Qubit, b: Bit) {
        self.push(K::CZ, [t.0, a.0, NONE], b.0, NONE);
    }
    /// Ops until the matching `pop_condition` execute only on lanes where `b` is 1.
    pub fn push_condition(&mut self, b: Bit) {
        self.depth += 1;
        self.push(K::PushCondition, [NONE; 3], b.0, NONE);
    }
    pub fn pop_condition(&mut self) {
        assert!(self.depth > 0, "pop_condition without push_condition");
        self.depth -= 1;
        self.push(K::PopCondition, [NONE; 3], NONE, NONE);
    }
    /// Fermionic Givens rotation between system qubits `p` and `p + 1`, angle from `angle`
    /// (DF only; semantics in spec/DESIGN.md section 15).
    pub fn givens(&mut self, p: usize, angle: Reg) {
        self.givens_modes(p, p + 1, angle);
    }
    /// Fermionic Givens rotation between system modes `p < q` (df extension; the semantics and
    /// charge are in spec/DESIGN.md section 15).
    pub fn givens_modes(&mut self, p: usize, q: usize, angle: Reg) {
        let (a, b) = (self.system(p), self.system(q));
        self.push(K::Givens, [b.0, a.0, NONE], NONE, angle.0);
    }
    /// `SpinSwap` (op kind 32, sos-sa specs only; spec/SPEC-SA.md section 11): where `c` is 1,
    /// `F = prod_p G_{2p, 2p+1}(pi / 2)` on the system register, the layer that maps every spin-0
    /// mode to its spin-1 partner.
    pub fn spin_swap(&mut self, c: Qubit) {
        self.push(K::SpinSwap, [c.0, NONE, NONE], NONE, NONE);
    }
    /// `SpinSwapDg` (op kind 33): `F^dagger` where `c` is 1 (spec/SPEC-SA.md section 11).
    pub fn spin_swap_dg(&mut self, c: Qubit) {
        self.push(K::SpinSwapDg, [c.0, NONE, NONE], NONE, NONE);
    }
    /// Segment hint (`SEG_PREPARE`, `SEG_SELECT`, ...): no effect on simulation.
    pub fn segment(&mut self, code: u32) {
        self.push(K::Segment, [NONE; 3], NONE, code);
    }

    /// Nested DF: a register over uniform bits `lo..lo + width`, the inner register a
    /// `df-nested-alias-v1` lane map declares (`DfNestedMap::inner_bits`).
    pub fn inner_register(&mut self, lo: u32, width: u32) -> Reg {
        let qs: Vec<Qubit> = (lo..lo + width).map(|i| self.uniform(i)).collect();
        self.register(&qs)
    }

    /// Nested DF: `Reflect` (op kind 31) on `reg`, `2|+><+| - I` on the inner uniform register
    /// (spec/DESIGN.md section 16.1). Unconditioned; not allowed inside a condition block.
    pub fn reflect(&mut self, reg: Reg) {
        assert_eq!(self.depth, 0, "reflect inside a condition block");
        self.push(K::Reflect, [NONE; 3], NONE, reg.0);
    }

    /// Nested DF: emits `Segment 3`, the ops `inner` emits, `Segment 4`, `Reflect(reg)`, and then
    /// the same ops again, byte for byte, between a second `Segment 3` and `Segment 4`
    /// (spec/DESIGN.md section 16.3). `inner` runs once; it must leave the set of live
    /// ancillas as it found it, close every condition block it opens and declare no registers.
    ///
    /// # Panics
    /// If `inner` breaks one of those rules, or a condition block is open.
    pub fn nested_inner(&mut self, reg: Reg, inner: impl FnOnce(&mut Self)) {
        assert_eq!(self.depth, 0, "nested_inner inside a condition block");
        let live: Vec<u32> = self.live_ids();
        self.segment(super::SEG_INNER_BEGIN);
        let start = self.ops.len();
        inner(self);
        assert_eq!(
            self.depth, 0,
            "nested_inner: a condition block is left open"
        );
        assert_eq!(
            self.live_ids(),
            live,
            "nested_inner: the inner block must free every ancilla it allocates"
        );
        let body: Vec<Op> = self.ops[start..].to_vec();
        assert!(
            body.iter().all(|op| !matches!(
                op.kind,
                K::Register | K::AppendToRegister | K::Segment | K::Reflect
            )),
            "nested_inner: declare registers before the inner block"
        );
        self.segment(super::SEG_INNER_END);
        self.reflect(reg);
        self.segment(super::SEG_INNER_BEGIN);
        self.ops.extend_from_slice(&body);
        self.segment(super::SEG_INNER_END);
    }

    fn live_ids(&self) -> Vec<u32> {
        (0..self.live.len() as u32)
            .filter(|&q| self.live[q as usize])
            .collect()
    }

    #[must_use]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// The finished op stream.
    ///
    /// # Panics
    /// If a condition block is still open.
    #[must_use]
    pub fn finish(self) -> Vec<Op> {
        assert_eq!(self.depth, 0, "unclosed push_condition");
        self.ops
    }
}
