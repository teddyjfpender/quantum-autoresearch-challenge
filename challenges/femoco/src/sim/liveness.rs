//! Liveness for `Q_peak`: control, system and uniform are always live; an ancilla is live
//! from the first op that touches it (or a `Givens` that reads it as an angle) until an
//! unconditional `R` or `Hmr` at condition depth 0 returns it to `|0>`. The peak is taken over
//! the whole stream, so reusing freed ids and allocating fresh ones count the same.
use super::compile::Compiled;
use crate::circuit::{Op, OperationType as K, NONE};
use std::collections::BTreeMap;

pub struct Liveness {
    live: Vec<bool>,
    count: u64,
    base: u64,
    pub peak: u64,
    pub segment: u8,
    pub seg_peak: BTreeMap<u8, u64>,
}

impl Liveness {
    #[must_use]
    pub fn new(base: u64) -> Self {
        Self {
            live: Vec::new(),
            count: 0,
            base,
            peak: base,
            segment: 255,
            seg_peak: BTreeMap::new(),
        }
    }

    pub fn touch(&mut self, q: u32, first_ancilla: u32) {
        if q < first_ancilla {
            return;
        }
        let i = (q - first_ancilla) as usize;
        if i >= self.live.len() {
            self.live.resize(i + 1, false);
        }
        if !self.live[i] {
            self.live[i] = true;
            self.count += 1;
            self.peak = self.peak.max(self.base + self.count);
            let p = self.seg_peak.entry(self.segment).or_insert(0);
            *p = (*p).max(self.count);
        }
    }
    pub fn end(&mut self, q: u32, first_ancilla: u32) {
        if let Some(l) = q
            .checked_sub(first_ancilla)
            .and_then(|i| self.live.get_mut(i as usize))
        {
            if *l {
                *l = false;
                self.count -= 1;
            }
        }
    }
}

pub fn track(op: &Op, depth: usize, first: u32, lv: &mut Liveness, out: &mut Compiled) {
    for q in op.qubits() {
        out.num_qubits = out.num_qubits.max(q + 1);
    }
    for b in [op.c_target, op.c_condition] {
        if b != NONE {
            out.num_bits = out.num_bits.max(b + 1);
        }
    }
    match op.kind {
        K::Segment => lv.segment = u8::try_from(op.r_target).unwrap_or(255),
        K::Register | K::AppendToRegister | K::DebugPrint => {}
        K::R | K::Hmr if depth == 0 && op.c_condition == NONE => lv.end(op.q_target, first),
        // A Givens reads its angle register: those qubits are live even if no gate touched them.
        K::Givens => {
            for &q in out
                .registers
                .get(op.r_target as usize)
                .into_iter()
                .flatten()
            {
                lv.touch(q, first);
            }
        }
        _ => op.qubits().for_each(|q| lv.touch(q, first)),
    }
}
