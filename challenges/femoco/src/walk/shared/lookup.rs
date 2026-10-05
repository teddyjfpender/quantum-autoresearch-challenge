//! Table lookups: unary iteration, clean QROAM, and measurement-based erasure.
//!
//! Everything here reads a classical table `data: u64 -> Word` at the value `x` of a
//! little-endian `index` register, on every lane at once. Costs are in Toffolis (`CCX`); every
//! AND is erased by measurement, so only computing ANDs costs.
//!
//! | function | effect | Toffolis | extra qubits |
//! | --- | --- | --- | --- |
//! | [`xor_lookup`] | `out ^= data(x)` | about `limit` | a few (the iteration) |
//! | [`qroam_xor`] | `out ^= data(x)` | `ceil(limit / lambda) + 2 (lambda - 1) w + E(ceil(limit / lambda))` | `lambda w` |
//! | [`qroam_load`] | fresh `out = data(x)` | `ceil(limit / lambda) + (lambda - 1) w + E(limit)` | `(lambda - 1) w` |
//! | [`erase_lookup`] | `out = data(x)` back to 0, freed | `E(limit) = 2 2^h + ceil(limit / 2^h)` | `2^h` |
//!
//! `w` is the word width and `lambda = m 2^a` the number of QROAM blocks ([`Qroam`]). When
//! `m > 1` the index is first split as `x = m q + r` by a small reversible division, so the
//! block count need not be a power of two. The block layout is the select-swap construction of
//! Low, Kliuchnikov and Schaeffer 2018 (arXiv:1812.00954, Fig. 1c) and Berry et al. 2019
//! (arXiv:1902.02134, App. B, "clean" QROAM); the junk left in the other blocks is erased by
//! X-basis measurement and a phase fixup (Berry et al. 2019, App. C), not by running the lookup
//! backwards.
use super::arith::{cswap_reg, divmod_const, divmod_const_undo};
use super::unary::iterate;
use crate::circuit::{Bit, Builder, Op, OperationType, Qubit};

/// A data word: bit `j` is `w[j / 64] >> (j % 64) & 1`. (`Word`, `word`, `put`, `xor` and
/// `xor_lookup` are verbatim copies of the baselines' helpers.)
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

/// A data table: the word at each index value.
pub type Data<'a> = dyn Fn(u64) -> Word + 'a;

/// How a QROAM read is built: `lambda = m 2^a` blocks. `a = 0, m = 1` is plain unary iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Qroam {
    /// log2 of the power-of-two part of the block count.
    pub a: usize,
    /// Odd (or any) multiplier of the block count; `x mod m` picks a block class.
    pub m: u64,
    /// One-hot bits used when the junk blocks are erased by measurement.
    pub junk_h: usize,
}

impl Qroam {
    /// Plain unary iteration.
    pub const UNARY: Self = Self {
        a: 0,
        m: 1,
        junk_h: 0,
    };

    /// Number of blocks `lambda`.
    #[must_use]
    pub fn blocks(&self) -> u64 {
        self.m << self.a
    }
}

/// Indices of the set bits of `w` below `limit`.
pub fn set_bits(w: &[u64], limit: usize) -> impl Iterator<Item = usize> + '_ {
    (0..limit).filter(move |&j| w[j / 64] >> (j % 64) & 1 == 1)
}

/// The content permutation of [`swap_network`] for low value `low` over `2^a` blocks:
/// `perm[p]` is the block whose data ends at position `p`. `perm[0] == low`.
#[must_use]
pub fn swap_perm(a: usize, low: u64) -> Vec<usize> {
    let mut perm: Vec<usize> = (0..1 << a).collect();
    for t in (0..a).rev() {
        if low >> t & 1 == 1 {
            for j in 0..1 << t {
                perm.swap(j, j + (1 << t));
            }
        }
    }
    perm
}

/// Moves block `low` to position 0 (`reverse = false`), or undoes that (`reverse = true`).
/// `(2^a - 1) w` controlled swaps, one Toffoli each.
pub fn swap_network(b: &mut Builder, low: &[Qubit], blocks: &[Vec<Qubit>], reverse: bool) {
    let a = low.len();
    assert_eq!(blocks.len(), 1 << a, "swap network needs 2^a blocks");
    let mut steps: Vec<(usize, usize)> = Vec::new();
    for t in (0..a).rev() {
        for j in 0..1 << t {
            steps.push((t, j));
        }
    }
    if reverse {
        steps.reverse();
    }
    for (t, j) in steps {
        cswap_reg(b, low[t], &blocks[j], &blocks[j + (1 << t)]);
    }
}

/// The blocks of one QROAM read and how index values map onto them.
struct Layout<'a> {
    lam: u64,
    w: usize,
    limit: u64,
    data: &'a Data<'a>,
}

impl Layout<'_> {
    /// Block contents at high value `hv`: block `j` holds `data(hv lambda + j)` (0 past the
    /// table), where block `j = c 2^a + low` for class `c = x mod m`... flattened as one word.
    fn wide(&self, hv: u64, x_of: &dyn Fn(u64, u64) -> u64) -> Word {
        let mut out = word(self.lam as usize * self.w);
        for j in 0..self.lam {
            let x = x_of(hv, j);
            if x < self.limit {
                let d = (self.data)(x);
                for bit in set_bits(&d, self.w) {
                    let at = j as usize * self.w + bit;
                    out[at / 64] |= 1 << (at % 64);
                }
            }
        }
        out
    }
}

/// Writes `wide(hv)` into the flat block register on the lanes whose `high` is `hv`.
fn write_blocks(
    b: &mut Builder,
    high: &[Qubit],
    hi_limit: u64,
    flat: &[Qubit],
    wide: &dyn Fn(u64) -> Word,
) {
    if high.is_empty() {
        for bit in set_bits(&wide(0), flat.len()) {
            b.x(flat[bit]);
        }
        return;
    }
    let mut leaf = |b: &mut Builder, hv: u64, flag: Qubit| {
        for bit in set_bits(&wide(hv), flat.len()) {
            b.cx(flag, flat[bit]);
        }
    };
    iterate(b, None, high, hi_limit, &mut leaf);
}

/// Splits `index` for a QROAM read: returns `(low, high, undo)`, where the `m 2^a` blocks are
/// chosen by `(r, low)` and `high` is iterated. With `m = 1` the split is by bits; otherwise
/// `index = m q + r` is computed into fresh registers `(q, r)` and `low` is `q`'s low `a` bits.
struct Split {
    low: Vec<Qubit>,
    r: Vec<Qubit>,
    high: Vec<Qubit>,
    /// `Some((q, r))` when a division was done (to be undone afterwards).
    div: Option<(Vec<Qubit>, Vec<Qubit>)>,
}

fn split(b: &mut Builder, index: &[Qubit], p: Qroam) -> Split {
    if p.m == 1 {
        let a = p.a.min(index.len());
        return Split {
            low: index[..a].to_vec(),
            r: Vec::new(),
            high: index[a..].to_vec(),
            div: None,
        };
    }
    let (q, r) = divmod_const(b, index, p.m);
    let a = p.a.min(q.len());
    Split {
        low: q[..a].to_vec(),
        r: r.clone(),
        high: q[a..].to_vec(),
        div: Some((q, r)),
    }
}

fn unsplit(b: &mut Builder, index: &[Qubit], p: Qroam, s: Split) {
    if let Some((q, r)) = s.div {
        divmod_const_undo(b, index, p.m, q, r);
    }
}

/// Moves the block of class `r` (value `r < m`, little-endian bits) and low value `low` to
/// position 0. Blocks are grouped as `blocks[c 2^a + j]`. First every class moves its block
/// `low` to its own position 0 (the power-of-two swap network, per class), then class `r`'s
/// position 0 moves to the global position 0 by a swap network over `r`'s bits (classes past `m`
/// are empty and never selected).
fn select_block(
    b: &mut Builder,
    low: &[Qubit],
    r: &[Qubit],
    blocks: &[Vec<Qubit>],
    m: u64,
    reverse: bool,
) {
    let per = 1usize << low.len();
    let heads: Vec<Vec<Qubit>> = (0..m as usize).map(|c| blocks[c * per].clone()).collect();
    let class_net = |b: &mut Builder| {
        if r.is_empty() {
            return;
        }
        // Pad the class list to 2^|r| with zero-width placeholders: swaps with a missing class
        // are skipped, which is correct because `r < m` on every lane.
        let a_r = r.len();
        let mut steps: Vec<(usize, usize)> = Vec::new();
        for t in (0..a_r).rev() {
            for j in 0..1 << t {
                steps.push((t, j));
            }
        }
        if reverse {
            steps.reverse();
        }
        for (t, j) in steps {
            let k = j + (1 << t);
            if k < heads.len() {
                cswap_reg(b, r[t], &heads[j], &heads[k]);
            }
        }
    };
    let low_nets = |b: &mut Builder| {
        for c in 0..m as usize {
            swap_network(b, low, &blocks[c * per..(c + 1) * per], reverse);
        }
    };
    if reverse {
        class_net(b);
        low_nets(b);
    } else {
        low_nets(b);
        class_net(b);
    }
}

/// `out ^= data(x)` for the lane's `index` value `x < limit` with a clean QROAM: the blocks are
/// written by unary iteration over the high index, block `x`'s word is swapped to position 0,
/// copied into `out`, the swaps are undone, and the blocks (which now hold `data` of every `x`
/// with the lane's high index) are erased by measurement with a fixup over the high index only.
/// `out` may hold anything (XOR semantics), so this also moves a register between two tables.
pub fn qroam_xor(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    out: &[Qubit],
    data: &Data<'_>,
    p: Qroam,
) {
    if p.blocks() == 1 || index.is_empty() {
        xor_lookup(b, index, limit, out, data);
        return;
    }
    let s = split(b, index, p);
    let (a, m) = (s.low.len(), p.m);
    let lam = m << a;
    let w = out.len();
    let blocks: Vec<Vec<Qubit>> = (0..lam).map(|_| b.alloc_n(w)).collect();
    let flat: Vec<Qubit> = blocks.concat();
    let per = 1u64 << a;
    // Index value of block j (= c 2^a + low) at high value hv: q = hv 2^a + low, x = m q + c.
    let x_of = move |hv: u64, j: u64| m * (hv * per + j % per) + j / per;
    let hi_limit = limit.div_ceil(lam);
    let lay = Layout {
        lam,
        w,
        limit,
        data,
    };
    let wide = |hv: u64| lay.wide(hv, &x_of);
    write_blocks(b, &s.high, hi_limit, &flat, &wide);
    select_block(b, &s.low, &s.r, &blocks, m, false);
    for (&src, &dst) in blocks[0].iter().zip(out) {
        b.cx(src, dst);
    }
    select_block(b, &s.low, &s.r, &blocks, m, true);
    erase_lookup(b, &s.high, hi_limit, flat, &wide, p.junk_h);
    unsplit(b, index, p, s);
}

/// A fresh register holding `data(x)` for the lane's `index` value `x < limit`, by a clean
/// QROAM whose position-0 block is the output: the other blocks are measured right after the
/// swap network, and the fixup (which depends on the whole index) costs `E(limit)`. Cheaper
/// than [`qroam_xor`] by the undo swaps and one register, dearer by the fixup; best when
/// `limit` is small and `w` is large (angle words). The classical parity it emits costs about
/// `limit (lambda - 1) w / 2` bit ops, so keep `limit` moderate.
pub fn qroam_load(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    w: usize,
    data: &Data<'_>,
    p: Qroam,
) -> Vec<Qubit> {
    if p.blocks() == 1 || index.is_empty() {
        let out = b.alloc_n(w);
        xor_lookup(b, index, limit, &out, data);
        return out;
    }
    assert_eq!(p.m, 1, "qroam_load supports power-of-two block counts only");
    let a = p.a.min(index.len());
    let (low, high) = index.split_at(a);
    let lam = 1u64 << a;
    let blocks: Vec<Vec<Qubit>> = (0..lam).map(|_| b.alloc_n(w)).collect();
    let flat: Vec<Qubit> = blocks.concat();
    let x_of = move |hv: u64, j: u64| hv * lam + j;
    let hi_limit = limit.div_ceil(lam);
    let lay = Layout {
        lam,
        w,
        limit,
        data,
    };
    let wide = |hv: u64| lay.wide(hv, &x_of);
    write_blocks(b, high, hi_limit, &flat, &wide);
    swap_network(b, low, &blocks, false);
    // Position p (>= 1) holds data(hv lambda + perm[p]).
    let perms: Vec<Vec<usize>> = (0..lam).map(|l| swap_perm(a, l)).collect();
    let junk_w = (lam as usize - 1) * w;
    let junk = |x: u64| -> Word {
        let (hv, l) = (x / lam, x % lam);
        let mut out = word(junk_w);
        for (pos, &j) in perms[l as usize].iter().enumerate().skip(1) {
            let y = hv * lam + j as u64;
            if y < limit {
                for bit in set_bits(&data(y), w) {
                    let at = (pos - 1) * w + bit;
                    out[at / 64] |= 1 << (at % 64);
                }
            }
        }
        out
    };
    let mut it = blocks.into_iter();
    let out = it.next().expect("at least one block");
    let rest: Vec<Qubit> = it.flatten().collect();
    let full = (hi_limit * lam).min(1u64 << index.len().min(63));
    erase_lookup(b, index, full, rest, &junk, p.junk_h);
    out
}

/// Measures every qubit of `out` in the X basis (freeing it) and returns the outcomes.
pub fn measure(b: &mut Builder, out: Vec<Qubit>) -> Vec<Bit> {
    out.into_iter().map(|q| b.hmr(q)).collect()
}

/// Returns `out`, which holds `data(x)` for the lane's `index` value `x < limit` (0 past it),
/// to `|0>` and frees it: X-basis measurement, then [`phase_fixup`]. Costs `E(limit)`.
pub fn erase_lookup(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    out: Vec<Qubit>,
    data: &Data<'_>,
    h: usize,
) {
    let m = measure(b, out);
    phase_fixup(b, index, limit, &m, data, h);
}

/// Applies the phase `(-1)^(m . data(x))` on the lanes whose `index` holds `x < limit`: the
/// phase an X-basis measurement with outcomes `m` left on a register that held `data(x)`
/// (Berry et al. 2019, App. C). The index is split into `h` low bits, written one-hot into
/// `2^h` fresh qubits, and the high bits are iterated; at high value `X` and low value `v` a
/// `CZ` between the high indicator and `hot[v]`, conditioned on the classical running parity
/// `m . data(X 2^h + v)`, applies the phase exactly where `x = X 2^h + v`.
/// `2 (2^h - 1) + ceil(limit / 2^h)` Toffolis and `2^h` qubits. Several fixups with the same
/// outcomes compose: the phases multiply, so a register whose value is `d1(x) ^ d2(y)` is
/// fixed by one fixup over `x` with `d1` and one over `y` with `d2`.
pub fn phase_fixup(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    m: &[Bit],
    data: &Data<'_>,
    h: usize,
) {
    let p = b.new_bit();
    let mut op = Op::new(OperationType::BitStore0);
    op.c_target = p.0;
    b.emit(op);
    let mut prev = word(m.len());
    let mut advance = |b: &mut Builder, x: u64| {
        let d = data(x);
        for j in set_bits(&xor(&d, &prev), m.len()) {
            let mut op = Op::new(OperationType::BitInvert);
            op.c_target = p.0;
            op.c_condition = m[j].0;
            b.emit(op);
        }
        prev = d;
    };
    let h = h.min(index.len());
    let (low, high) = index.split_at(h);
    let hot = b.alloc_n(1 << h);
    let one_hot = |b: &mut Builder| {
        if h > 0 {
            let mut leaf = |b: &mut Builder, v: u64, flag: Qubit| b.cx(flag, hot[v as usize]);
            iterate(b, None, low, hot.len() as u64, &mut leaf);
        } else {
            b.x(hot[0]);
        }
    };
    one_hot(b);
    let lam = 1u64 << h;
    let mut fix = |b: &mut Builder, hi: u64, flag: Option<Qubit>| {
        for v in 0..lam {
            let x = hi * lam + v;
            if x >= limit {
                break;
            }
            advance(b, x);
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
    one_hot(b);
    hot.into_iter().for_each(|q| b.free(q));
}

/// Loads a classical constant into a fresh register (X gates only).
pub fn load_const(b: &mut Builder, w: usize, v: u64) -> Vec<Qubit> {
    let out = b.alloc_n(w);
    for (j, &q) in out.iter().enumerate() {
        if v >> j & 1 == 1 {
            b.x(q);
        }
    }
    out
}

/// Clears a register loaded by [`load_const`] and frees it.
pub fn unload_const(b: &mut Builder, out: Vec<Qubit>, v: u64) {
    for (j, &q) in out.iter().enumerate() {
        if v >> j & 1 == 1 {
            b.x(q);
        }
    }
    out.into_iter().for_each(|q| b.free(q));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::shared::testsim::Sim;

    fn table(x: u64, w: usize) -> Word {
        let mut d = word(w);
        let v = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 7 ^ x;
        put(&mut d, 0, w.min(64), v);
        d
    }

    fn check(p: Qroam, bits: usize, limit: u64, w: usize, load: bool) {
        for x in 0..(1u64 << bits).min(limit) {
            for seed in 0..3 {
                let mut b = Builder::new(0);
                b.declare_uniform(bits as u32);
                let index: Vec<Qubit> = (0..bits as u32).map(|i| b.uniform(i)).collect();
                let data = |y: u64| table(y, w);
                let out = if load {
                    qroam_load(&mut b, &index, limit, w, &data, p)
                } else {
                    let out = b.alloc_n(w);
                    qroam_xor(&mut b, &index, limit, &out, &data, p);
                    out
                };
                let mut sim = Sim::new(&b, seed);
                sim.set_uniform(x);
                sim.run(b.ops());
                assert_eq!(
                    sim.read(&out),
                    table(x, w)[0] & ((1 << w) - 1),
                    "x {x} {p:?}"
                );
                erase_lookup(&mut b, &index, limit, out, &data, 2);
                let mut sim = Sim::new(&b, seed);
                sim.set_uniform(x);
                sim.run(b.ops());
                sim.assert_clean();
            }
        }
    }

    #[test]
    fn qroam_xor_reads_every_entry() {
        for (a, m) in [
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 1),
            (1, 3),
            (2, 3),
            (0, 5),
            (1, 5),
        ] {
            check(Qroam { a, m, junk_h: 1 }, 6, 53, 7, false);
        }
    }

    #[test]
    fn qroam_load_reads_every_entry() {
        for a in 0..4 {
            check(Qroam { a, m: 1, junk_h: 2 }, 6, 45, 5, true);
        }
    }
}
