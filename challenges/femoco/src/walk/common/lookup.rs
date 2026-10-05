//! Table lookups: a QROM by unary iteration (Babbush et al. 2018, arXiv:1805.03662, Sec. III.C)
//! and its measurement-based erasure (Berry et al. 2019, arXiv:1902.02134, App. C).
//!
//! Data words are little-endian bit vectors in `u64` limbs. `xor_lookup` XORs `data(x)` into
//! an output register for the lane's index `x`; calling it with `data = old ^ new` moves the
//! register from one table to another in a single pass. `erase_lookup` returns a register that
//! holds `data(x)` to `|0>`: it measures every output qubit in the X basis and cancels the
//! resulting phase `(-1)^(m . data(x))` with a pass over the index that costs
//! `2 * 2^h + limit / 2^h` Toffolis instead of `limit`.
use super::unary::iterate;
use crate::circuit::{Bit, Builder, Op, OperationType, Qubit};

/// A data word: bit `j` is `w[j / 64] >> (j % 64) & 1`.
pub type Word = Vec<u64>;

/// An empty word of `bits` bits.
#[must_use]
pub fn word(bits: usize) -> Word {
    vec![0; bits.div_ceil(64).max(1)]
}

/// Sets bits `at..at + width` of `w` to the low `width` bits of `v`.
pub fn put(w: &mut Word, at: usize, width: usize, v: u64) {
    for j in 0..width {
        if v >> j & 1 == 1 {
            w[(at + j) / 64] |= 1 << ((at + j) % 64);
        }
    }
}

/// `a ^ b`, limb by limb.
#[must_use]
pub fn xor(a: &[u64], b: &[u64]) -> Word {
    a.iter().zip(b).map(|(x, y)| x ^ y).collect()
}

fn set_bits(w: &[u64], limit: usize) -> impl Iterator<Item = usize> + '_ {
    (0..limit).filter(move |&j| w[j / 64] >> (j % 64) & 1 == 1)
}

/// XORs `data(x)` into `out` on every lane whose `index` holds `x < limit`. Costs about `limit`
/// Toffolis (one AND per internal node of the unary-iteration tree, erased by measurement).
pub fn xor_lookup(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    out: &[Qubit],
    data: &dyn Fn(u64) -> Word,
) {
    let mut leaf = |b: &mut Builder, x: u64, flag: Qubit| {
        for j in set_bits(&data(x), out.len()) {
            b.cx(flag, out[j]);
        }
    };
    iterate(b, None, index, limit, &mut leaf);
}

/// Inverts classical bit `p` where classical bit `m` is 1 (a free classical op).
fn bit_xor(b: &mut Builder, p: Bit, m: Bit) {
    let mut op = Op::new(OperationType::BitInvert);
    op.c_target = p.0;
    op.c_condition = m.0;
    b.emit(op);
}

fn bit_clear(b: &mut Builder, p: Bit) {
    let mut op = Op::new(OperationType::BitStore0);
    op.c_target = p.0;
    b.emit(op);
}

/// A classical running parity `p = m . data(x)`, updated incrementally from `data(x - 1)`.
struct Parity<'a> {
    p: Bit,
    m: Vec<Bit>,
    prev: Word,
    data: &'a dyn Fn(u64) -> Word,
}

impl Parity<'_> {
    fn advance(&mut self, b: &mut Builder, x: u64) {
        let d = (self.data)(x);
        for j in set_bits(&xor(&d, &self.prev), self.m.len()) {
            bit_xor(b, self.p, self.m[j]);
        }
        self.prev = d;
    }
}

/// Writes (or, called again, erases) the one-hot encoding of `index` into `hot`.
fn one_hot(b: &mut Builder, index: &[Qubit], hot: &[Qubit]) {
    let mut leaf = |b: &mut Builder, v: u64, flag: Qubit| b.cx(flag, hot[v as usize]);
    iterate(b, None, index, hot.len() as u64, &mut leaf);
}

/// Returns `out`, which holds `data(x)` for the lane's `index` value `x < limit`, to `|0>` and
/// frees it. Every output qubit is measured in the X basis (`Hmr`), which leaves the phase
/// `(-1)^(m . data(x))`; the fixup applies that phase again. The index is split into `h` low
/// bits, written one-hot into `2^h` fresh qubits, and the high bits are iterated: at high value
/// `X` and low value `v` a `CZ` between the high indicator and `hot[v]`, conditioned on the
/// classical parity `m . data(X 2^h + v)`, cancels the phase exactly where `x = X 2^h + v`.
pub fn erase_lookup(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    out: Vec<Qubit>,
    data: &dyn Fn(u64) -> Word,
    h: usize,
) {
    let m: Vec<Bit> = out.into_iter().map(|q| b.hmr(q)).collect();
    fix_phase(b, index, limit, m, data, h);
}

/// X-basis measurement of `q` into a new classical bit, like [`Builder::hmr`], but the builder
/// keeps `q` allocated: it is `|0>` afterwards and can be written again. The harness counts it
/// live only from its next use (`src/sim/liveness.rs`). This is what a nested inner copy needs:
/// its angle registers are declared once, before the copies, over qubits that each copy writes
/// and erases.
pub fn hmr_keep(b: &mut Builder, q: Qubit) -> Bit {
    let bit = b.new_bit();
    let mut op = Op::new(OperationType::Hmr);
    op.q_target = q.0;
    op.c_target = bit.0;
    b.emit(op);
    bit
}

/// [`erase_lookup`] for a register that stays allocated (see [`hmr_keep`]).
pub fn erase_lookup_keep(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    out: &[Qubit],
    data: &dyn Fn(u64) -> Word,
    h: usize,
) {
    let m: Vec<Bit> = out.iter().map(|&q| hmr_keep(b, q)).collect();
    fix_phase(b, index, limit, m, data, h);
}

/// Cancels the phase `(-1)^(m . data(x))` that measuring a lookup's output left behind.
fn fix_phase(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    m: Vec<Bit>,
    data: &dyn Fn(u64) -> Word,
    h: usize,
) {
    let p = b.new_bit();
    bit_clear(b, p);
    let width = m.len();
    let mut par = Parity {
        p,
        m,
        prev: word(width),
        data,
    };
    let h = h.min(index.len());
    let (low, high) = index.split_at(h);
    let hot = b.alloc_n(1 << h);
    if h > 0 {
        one_hot(b, low, &hot);
    } else {
        b.x(hot[0]);
    }
    let lam = 1u64 << h;
    let mut fix = |b: &mut Builder, hi: u64, flag: Option<Qubit>| {
        for v in 0..lam {
            let x = hi * lam + v;
            if x >= limit {
                break;
            }
            par.advance(b, x);
            match flag {
                Some(f) => b.cz_if(f, hot[v as usize], p),
                None => b.z_if(hot[v as usize], p),
            }
        }
    };
    if high.is_empty() {
        fix(b, 0, None);
    } else {
        let leaf = &mut |b: &mut Builder, hi, f| fix(b, hi, Some(f));
        iterate(b, None, high, limit.div_ceil(lam), leaf);
    }
    if h > 0 {
        one_hot(b, low, &hot);
    } else {
        b.x(hot[0]);
    }
    hot.into_iter().for_each(|q| b.free(q));
}
