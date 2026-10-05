//! Narrowing gadgets for the streamed `sa-pareto` points (`README.md` here,
//! `pareto::Narrow`).
//!
//! The main one is an **outcome-gated re-read**. A keep comparison `lt = [draw < keep(x)]` needs
//! `keep(x)` again when it is erased. Holding `keep` from the comparison to the erasure costs
//! `mu` qubits across the peak. Instead, `keep` is measured (X basis) right after the comparison.
//! Its phase `(-1)^(m . keep(x))` depends on the index alone, so the read's final fixup cancels
//! it. `lt` is erased later by an X measurement with outcome `r`, which leaves `(-1)^(r lt)`.
//! Only the lanes with `r = 1` (half of them) need that phase cancelled. There, under
//! `PushCondition(r)`, a second read loads `keep(x)`, the comparison's phase `(-1)^[draw <
//! keep]` is applied with Cliffords on `mu - 1` carries (Gidney 2018, arXiv:1709.06648: `MAJ(x,
//! y, c) = c ^ (x ^ c)(y ^ c)`, so `(-1)^MAJ = Z(c) CZ(x ^ c, y ^ c)`), and the second read is
//! measured. Its outcomes are 0 on the lanes where the block did not run, so a fixup after the
//! block cancels its phases on every lane.
//!
//! Expected cost: half of `read + (mu - 1)`, against `mu - 1` for erasing with `keep` held.
//!
//! `Builder::free` and `Builder::hmr` refuse to run inside a condition block. The liveness rule
//! (`src/sim/liveness.rs`) ends a qubit only at a depth-0 `R` or `Hmr`. So every qubit a gated
//! block touches is measured with a kept `Hmr`, which demolishes it to `|0>` on the lanes that
//! ran it, while the other lanes never touched it. Such a qubit is then reused inside the block
//! ([`Pool`]) and freed by an unconditional `R` after the block. The block's static footprint is
//! the largest number of its qubits live at once.
use crate::circuit::{Bit, Builder, Op, OperationType, Qubit};
use crate::walk::shared::lookup::{set_bits, swap_network, swap_perm, word, xor, Word};

/// Qubits for a condition block: every one is `|0>` on every lane whenever it is in the free
/// list (measured by a kept `Hmr` where the block ran, untouched elsewhere), and all of them are
/// freed after the block.
#[derive(Default)]
pub struct Pool {
    all: Vec<Qubit>,
    free: Vec<Qubit>,
}

impl Pool {
    /// A `|0>` qubit: one measured earlier in the block, or a fresh one.
    pub fn take(&mut self, b: &mut Builder) -> Qubit {
        if let Some(q) = self.free.pop() {
            return q;
        }
        let q = b.alloc();
        self.all.push(q);
        q
    }

    /// `n` qubits from [`Pool::take`].
    pub fn take_n(&mut self, b: &mut Builder, n: usize) -> Vec<Qubit> {
        (0..n).map(|_| self.take(b)).collect()
    }

    /// Measures `q` (X basis, kept allocated) into a new bit and returns it to the pool.
    pub fn measure(&mut self, b: &mut Builder, q: Qubit) -> Bit {
        let bit = b.new_bit();
        hmr_into(b, q, bit);
        self.free.push(q);
        bit
    }

    /// Measures `q` into `bit` and returns it to the pool.
    pub fn measure_into(&mut self, b: &mut Builder, q: Qubit, bit: Bit) {
        hmr_into(b, q, bit);
        self.free.push(q);
    }

    /// Lever `~`: takes over every qubit of `other`, which must all be back (so a
    /// block that needed two pools at once, one inside the other's callback, reuses both after).
    ///
    /// # Panics
    /// If a qubit of `other` is still in use.
    pub fn absorb(&mut self, other: Pool) {
        assert_eq!(
            other.all.len(),
            other.free.len(),
            "a pool qubit is still in use"
        );
        self.all.extend(other.all);
        self.free.extend(other.free);
    }

    /// Returns `q` (handed out by [`Pool::take`]) without measuring it: the caller guarantees it
    /// is `|0>` on every lane again (lever `~`'s in-place comparator's carry-in).
    pub fn give_back(&mut self, q: Qubit) {
        assert!(self.all.contains(&q), "not a pool qubit");
        self.free.push(q);
    }

    /// Frees every qubit the pool handed out (after the condition block, at depth 0).
    ///
    /// # Panics
    /// If a qubit is still in use (not returned by a measurement).
    pub fn close(self, b: &mut Builder) {
        assert_eq!(
            self.all.len(),
            self.free.len(),
            "a pool qubit is still in use"
        );
        for q in self.all {
            b.free(q);
        }
    }
}

/// A classical bit set to 0 on every lane, for an outcome that a gated block may or may not
/// write.
pub fn zero_bit(b: &mut Builder) -> Bit {
    let p = b.new_bit();
    let mut op = Op::new(OperationType::BitStore0);
    op.c_target = p.0;
    b.emit(op);
    p
}

/// X-basis measurement of `q` into `bit`, leaving `q` allocated (in `|0>`).
fn hmr_into(b: &mut Builder, q: Qubit, bit: Bit) {
    let mut op = Op::new(OperationType::Hmr);
    op.q_target = q.0;
    op.c_target = bit.0;
    b.emit(op);
}

/// Erases `t = a AND (NOT) c` by measurement (`common::unary::erase_and`), inside a block.
fn and_erase(b: &mut Builder, a: Qubit, c: Qubit, t: Qubit, negated: bool, pool: &mut Pool) {
    let m = pool.measure(b, t);
    if negated {
        b.x(c);
    }
    b.cz_if(a, c, m);
    if negated {
        b.x(c);
    }
}

/// Unary iteration (`common::unary::iterate`: one AND per internal node, erased by
/// measurement) whose ANDs come from and return to `pool`.
pub fn iterate(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    limit: u64,
    leaf: &mut dyn FnMut(&mut Builder, u64, Qubit),
    pool: &mut Pool,
) {
    assert!(ctl.is_some() || !index.is_empty(), "nothing to iterate on");
    node(b, ctl, index, 0, limit, leaf, pool);
}

fn node(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    base: u64,
    limit: u64,
    leaf: &mut dyn FnMut(&mut Builder, u64, Qubit),
    pool: &mut Pool,
) {
    let Some((&bit, low)) = index.split_last() else {
        leaf(b, base, ctl.expect("a leaf needs an indicator"));
        return;
    };
    let half = 1u64 << low.len();
    let right = base + half < limit;
    let Some(c) = ctl else {
        b.x(bit);
        node(b, Some(bit), low, base, limit, leaf, pool);
        b.x(bit);
        if right {
            node(b, Some(bit), low, base + half, limit, leaf, pool);
        }
        return;
    };
    let child = pool.take(b);
    b.x(bit);
    b.ccx(c, bit, child);
    b.x(bit);
    node(b, Some(child), low, base, limit, leaf, pool);
    if right {
        b.cx(c, child);
        node(b, Some(child), low, base + half, limit, leaf, pool);
        and_erase(b, c, bit, child, false, pool);
    } else {
        and_erase(b, c, bit, child, true, pool);
    }
}

/// A clean-QROAM read inside a block: `2^a` blocks of `w` qubits over the low index bits, the
/// wanted word in `out` (block 0 after the swap network), the other blocks in `junk` (both
/// still to be measured by the caller). Toffolis `ceil(limit / 2^a) + (2^a - 1) w` (the
/// iteration over the high bits, then the swap network), as `qroam::load`.
pub struct Read {
    pub out: Vec<Qubit>,
    pub junk: Vec<Qubit>,
}

/// Loads `data(x)` for the lane's `index` value `x < limit` (0 past it); see [`Read`].
pub fn load(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    w: usize,
    data: &dyn Fn(u64) -> Word,
    a: usize,
    pool: &mut Pool,
) -> Read {
    let a = a.min(index.len());
    let (low, high) = index.split_at(a);
    let lam = 1u64 << a;
    let hi_limit = limit.div_ceil(lam);
    let blocks: Vec<Vec<Qubit>> = (0..lam).map(|_| pool.take_n(b, w)).collect();
    let flat: Vec<Qubit> = blocks.concat();
    let wide = |hv: u64| -> Word {
        let mut out = word(lam as usize * w);
        for j in 0..lam {
            let x = hv * lam + j;
            if x < limit {
                for bit in set_bits(&data(x), w) {
                    let at = j as usize * w + bit;
                    out[at / 64] |= 1 << (at % 64);
                }
            }
        }
        out
    };
    if high.is_empty() {
        for bit in set_bits(&wide(0), flat.len()) {
            b.x(flat[bit]);
        }
    } else {
        let mut leaf = |b: &mut Builder, hv: u64, flag: Qubit| {
            for j in set_bits(&wide(hv), flat.len()) {
                b.cx(flag, flat[j]);
            }
        };
        iterate(b, None, high, hi_limit, &mut leaf, pool);
    }
    swap_network(b, low, &blocks, false);
    let mut it = blocks.into_iter();
    let out = it.next().expect("at least one block");
    Read {
        out,
        junk: it.flatten().collect(),
    }
}

/// [`load`] over the index values `start..limit` only (`start` block aligned):
/// the iteration over the high bits skips every block below `start` (`range::iterate_range`'s
/// node rule, with the ANDs from `pool`), so it costs `range_cost` instead of `ceil(limit /
/// 2^a)`. A lane outside the range reads 0, which is what `data` must give there. `start = 0`
/// is [`load`] itself.
#[allow(clippy::too_many_arguments)]
pub fn load_from(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    w: usize,
    data: &dyn Fn(u64) -> Word,
    a: usize,
    pool: &mut Pool,
) -> Read {
    if start == 0 {
        return load(b, index, limit, w, data, a, pool);
    }
    let a = a.min(index.len());
    let (low, high) = index.split_at(a);
    assert!(!high.is_empty(), "a range read needs high index bits");
    let lam = 1u64 << a;
    assert!(
        start.is_multiple_of(lam),
        "range start must be block aligned"
    );
    let hi_limit = limit.div_ceil(lam);
    let blocks: Vec<Vec<Qubit>> = (0..lam).map(|_| pool.take_n(b, w)).collect();
    let flat: Vec<Qubit> = blocks.concat();
    let wide = |hv: u64| -> Word {
        let mut out = word(lam as usize * w);
        for j in 0..lam {
            let x = hv * lam + j;
            if (start..limit).contains(&x) {
                for bit in set_bits(&data(x), w) {
                    let at = j as usize * w + bit;
                    out[at / 64] |= 1 << (at % 64);
                }
            }
        }
        out
    };
    let mut leaf = |b: &mut Builder, hv: u64, flag: Qubit| {
        for j in set_bits(&wide(hv), flat.len()) {
            b.cx(flag, flat[j]);
        }
    };
    node_from(b, None, high, 0, start / lam, hi_limit, &mut leaf, pool);
    swap_network(b, low, &blocks, false);
    let mut it = blocks.into_iter();
    let out = it.next().expect("at least one block");
    Read {
        out,
        junk: it.flatten().collect(),
    }
}

/// `range::iterate_range`'s node over `start..limit` with the ANDs from `pool` (erased by
/// measurement inside the block).
#[allow(clippy::too_many_arguments)]
fn node_from(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    base: u64,
    start: u64,
    limit: u64,
    leaf: &mut dyn FnMut(&mut Builder, u64, Qubit),
    pool: &mut Pool,
) {
    let Some((&bit, low)) = index.split_last() else {
        leaf(b, base, ctl.expect("a leaf needs an indicator"));
        return;
    };
    let half = 1u64 << low.len();
    let left = base + half > start;
    let right = base + half < limit;
    let Some(c) = ctl else {
        if left {
            b.x(bit);
            node_from(b, Some(bit), low, base, start, limit, leaf, pool);
            b.x(bit);
        }
        if right {
            node_from(b, Some(bit), low, base + half, start, limit, leaf, pool);
        }
        return;
    };
    let child = pool.take(b);
    if left {
        b.x(bit);
        b.ccx(c, bit, child);
        b.x(bit);
        node_from(b, Some(child), low, base, start, limit, leaf, pool);
        if right {
            b.cx(c, child);
            node_from(b, Some(child), low, base + half, start, limit, leaf, pool);
            and_erase(b, c, bit, child, false, pool);
        } else {
            and_erase(b, c, bit, child, true, pool);
        }
    } else {
        b.ccx(c, bit, child);
        node_from(b, Some(child), low, base + half, start, limit, leaf, pool);
        and_erase(b, c, bit, child, false, pool);
    }
}

/// Index values a fixup must cover after a read of `limit` values over `a` of `bits` index
/// bits (`qroam::Read::full`).
#[must_use]
pub fn read_full(bits: usize, limit: u64, a: usize) -> u64 {
    let a = a.min(bits);
    let lam = 1u64 << a;
    (limit.div_ceil(lam) * lam).min(1u64 << bits.min(63))
}

/// What a read of `data` (`limit` values, `2^a` blocks of `w` bits) left at index value `x` in
/// its junk blocks, in order, then in its output: the word whose outcomes a fixup over the
/// index cancels (`qroam::erase_all`'s layout).
#[must_use]
pub fn read_content(x: u64, a: usize, w: usize, limit: u64, data: &dyn Fn(u64) -> Word) -> Word {
    let lam = 1u64 << a;
    let junk_w = (lam as usize - 1) * w;
    let mut out = word(junk_w + w);
    let (hv, l) = (x / lam, x % lam);
    for (pos, &j) in swap_perm(a, l).iter().enumerate().skip(1) {
        let y = hv * lam + j as u64;
        if y < limit {
            for bit in set_bits(&data(y), w) {
                let at = (pos - 1) * w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
        }
    }
    if x < limit {
        for bit in set_bits(&data(x), w) {
            let at = junk_w + bit;
            out[at / 64] |= 1 << (at % 64);
        }
    }
    out
}

/// Applies `(-1)^[a < bb]` (equal widths, little-endian) inside a block: `n - 1` carries of
/// `NOT a + bb`, the top carry's phase by Cliffords, the carries erased by measurement.
/// `n - 1` Toffolis.
pub fn lt_phase(b: &mut Builder, a: &[Qubit], bb: &[Qubit], pool: &mut Pool) {
    lt_phase_with(b, a, bb, pool, true);
}

pub(super) fn lt_phase_with(
    b: &mut Builder,
    a: &[Qubit],
    bb: &[Qubit],
    pool: &mut Pool,
    top_z: bool,
) {
    let n = a.len();
    assert!(n > 0 && n == bb.len(), "bad comparator widths");
    a.iter().for_each(|&q| b.x(q));
    let mut cs: Vec<Qubit> = Vec::with_capacity(n);
    for i in 0..n - 1 {
        let t = pool.take(b);
        let (x, y) = (a[i], bb[i]);
        match cs.last().copied() {
            None => b.ccx(x, y, t),
            Some(c) => {
                b.cx(c, x);
                b.cx(c, y);
                b.ccx(x, y, t);
                b.cx(c, t);
                b.cx(c, x);
                b.cx(c, y);
            }
        }
        cs.push(t);
    }
    let (x, y) = (a[n - 1], bb[n - 1]);
    match cs.last().copied() {
        None => b.cz(x, y),
        Some(c) => {
            b.cx(c, x);
            b.cx(c, y);
            b.cz(x, y);
            b.cx(c, x);
            b.cx(c, y);
            if top_z {
                b.z(c);
            }
        }
    }
    for i in (0..cs.len()).rev() {
        let t = cs[i];
        let (x, y) = (a[i], bb[i]);
        match i.checked_sub(1).map(|j| cs[j]) {
            None => and_erase(b, x, y, t, false, pool),
            Some(c) => {
                b.cx(c, t);
                b.cx(c, x);
                b.cx(c, y);
                and_erase(b, x, y, t, false, pool);
                b.cx(c, x);
                b.cx(c, y);
            }
        }
    }
    a.iter().for_each(|&q| b.x(q));
}

/// Lever `~`: [`lt_phase`] with one scratch qubit instead of `n - 1`. The carries
/// of `NOT a + bb` are made in place by Cuccaro et al.'s ripple-carry majority gate (Cuccaro,
/// Draper, Kutin and Moulton 2004, arXiv:quant-ph/0410184: `MAJ(c, b, a)` = `CX(a, b)`,
/// `CX(a, c)`, `CCX(c, b, a)` leaves the carry out in `a`), the top carry's phase by Cliffords
/// (`Z(c) CZ(a ^ c, b ^ c)`, as in [`lt_phase`]) and the chain undone in reverse (the carries
/// live in `a`, so they cannot be erased by measurement). `2 (n - 1)` Toffolis; `a` and `bb` are
/// restored, the carry-in returns `|0>` to `pool`.
pub fn lt_phase_inplace(b: &mut Builder, a: &[Qubit], bb: &[Qubit], pool: &mut Pool, top_z: bool) {
    let n = a.len();
    assert!(n > 0 && n == bb.len(), "bad comparator widths");
    a.iter().for_each(|&q| b.x(q));
    let c0 = pool.take(b);
    let cin = |i: usize| if i == 0 { c0 } else { a[i - 1] };
    for i in 0..n - 1 {
        let (c, y, x) = (cin(i), bb[i], a[i]);
        b.cx(x, y);
        b.cx(x, c);
        b.ccx(c, y, x);
    }
    let (c, x, y) = (cin(n - 1), a[n - 1], bb[n - 1]);
    b.cx(c, x);
    b.cx(c, y);
    b.cz(x, y);
    b.cx(c, x);
    b.cx(c, y);
    if top_z {
        b.z(c);
    }
    for i in (0..n - 1).rev() {
        let (c, y, x) = (cin(i), bb[i], a[i]);
        b.ccx(c, y, x);
        b.cx(x, c);
        b.cx(x, y);
    }
    pool.give_back(c0);
    a.iter().for_each(|&q| b.x(q));
}

/// `shared::lookup::phase_fixup` inside a block: applies `(-1)^(m . data(x))` on the lanes whose
/// `index` holds `x < limit`, with a one-hot of the `h` low index bits made and unmade by two
/// iterations. `2 (2^h - 1) + ceil(limit / 2^h)` Toffolis (fewer when `h` covers the index).
pub fn fixup(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    m: &[Bit],
    data: &dyn Fn(u64) -> Word,
    h: usize,
    pool: &mut Pool,
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
    let hot = pool.take_n(b, 1 << h);
    let one_hot = |b: &mut Builder, pool: &mut Pool| {
        if h > 0 {
            let mut leaf = |b: &mut Builder, v: u64, flag: Qubit| b.cx(flag, hot[v as usize]);
            iterate(b, None, low, hot.len() as u64, &mut leaf, pool);
        } else {
            b.x(hot[0]);
        }
    };
    one_hot(b, pool);
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
        let mut leaf = |b: &mut Builder, hi: u64, f: Qubit| fix(b, hi, Some(f));
        iterate(b, None, high, limit.div_ceil(lam), &mut leaf, pool);
    }
    one_hot(b, pool);
    // Every one-hot qubit is |0> again: hand them back without measuring.
    pool.free.extend(hot);
}

/// Applies `(-1)^(AND of qs)` inside a block: an AND chain over all but the last qubit (from and
/// back to `pool`, erased by measurement), then a `CZ` with the last. `|qs| - 2` Toffolis.
pub fn and_phase(b: &mut Builder, qs: &[Qubit], pool: &mut Pool) {
    match qs.len() {
        0 => b.neg(),
        1 => b.z(qs[0]),
        2 => b.cz(qs[0], qs[1]),
        n => {
            let mut chain: Vec<(Qubit, Qubit, Qubit)> = Vec::new();
            let mut acc = qs[0];
            for &q in &qs[1..n - 1] {
                let t = pool.take(b);
                b.ccx(acc, q, t);
                chain.push((acc, q, t));
                acc = t;
            }
            b.cz(acc, qs[n - 1]);
            for (a, c, t) in chain.into_iter().rev() {
                and_erase(b, a, c, t, false, pool);
            }
        }
    }
}

/// How [`gated_lt_erase`] reads `keep` again and cancels that read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reread {
    /// log2 of the second read's block count.
    pub a: usize,
    /// `Some(h)`: cancel the second read inside the block, by a fixup with `h` one-hot bits;
    /// `None`: return its outcomes for the caller's own fixup.
    pub fix: Option<usize>,
}

/// Deliberate faults for the mutant tests (never used by a walk).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    None,
    /// Drop the comparison's phase.
    NoPhase,
    /// Drop only the top carry's `Z` in the comparison's phase.
    NoTopZ,
    /// Run the block where the outcome is 0 instead of 1.
    FlipOutcome,
}

/// Erases `lt = [draw < keep(x)]`, `keep` having been measured away (module docs): `lt` is
/// measured, and where the outcome is 1 `keep(x)` is read again (`r.a`), `(-1)^lt` is applied
/// and the read is measured. Returns the second read's outcomes (junk blocks, then output; 0
/// on lanes that did not run the block), which the caller's fixup over `index` must cancel with
/// [`read_content`]'s data, unless `r.fix` cancels them in the block (then it returns nothing).
///
/// Expected Toffolis `(read + mu - 1 [+ fixup]) / 2`.
#[allow(clippy::too_many_arguments)]
pub fn gated_lt_erase(
    b: &mut Builder,
    lt: Qubit,
    draw: &[Qubit],
    index: &[Qubit],
    limit: u64,
    keep: &dyn Fn(u64) -> Word,
    r: Reread,
    fault: Fault,
) -> Vec<Bit> {
    gated_lt_erase_from(b, lt, draw, index, 0, limit, keep, r, fault)
}

/// [`gated_lt_erase`] whose re-read covers the index values `start..limit` only ([`load_from`];
/// `keep` must be 0 below `start`). `start = 0` is [`gated_lt_erase`].
#[allow(clippy::too_many_arguments)]
pub fn gated_lt_erase_from(
    b: &mut Builder,
    lt: Qubit,
    draw: &[Qubit],
    index: &[Qubit],
    start: u64,
    limit: u64,
    keep: &dyn Fn(u64) -> Word,
    r: Reread,
    fault: Fault,
) -> Vec<Bit> {
    gated_lt_erase_m(b, lt, draw, index, start, limit, keep, r, fault).0
}

/// [`gated_lt_erase_from`], also returning `lt`'s X outcome (lever `W`
/// erases a qubit that equals `lt` except on self-aliased buckets, and cancels the rest of that
/// outcome's phase in its own fixup).
#[allow(clippy::too_many_arguments)]
pub fn gated_lt_erase_m(
    b: &mut Builder,
    lt: Qubit,
    draw: &[Qubit],
    index: &[Qubit],
    start: u64,
    limit: u64,
    keep: &dyn Fn(u64) -> Word,
    r: Reread,
    fault: Fault,
) -> (Vec<Bit>, Bit) {
    let w = draw.len();
    let a = r.a.min(index.len());
    let junk_w = ((1usize << a) - 1) * w;
    let bits: Vec<Bit> = (0..junk_w + w).map(|_| zero_bit(b)).collect();
    let m = b.hmr(lt);
    if fault == Fault::FlipOutcome {
        let mut op = Op::new(OperationType::BitInvert);
        op.c_target = m.0;
        b.emit(op);
    }
    let mut pool = Pool::default();
    b.push_condition(m);
    let rd = load_from(b, index, start, limit, w, keep, a, &mut pool);
    if fault != Fault::NoPhase {
        lt_phase_with(b, draw, &rd.out, &mut pool, fault != Fault::NoTopZ);
    }
    for (&q, &bit) in rd.junk.iter().chain(&rd.out).zip(&bits) {
        pool.measure_into(b, q, bit);
    }
    let out = if let Some(h) = r.fix {
        let full = read_full(index.len(), limit, a);
        let content = |x: u64| read_content(x, a, w, limit, keep);
        fixup(b, index, full, &bits, &content, h, &mut pool);
        Vec::new()
    } else {
        bits
    };
    b.pop_condition();
    if fault == Fault::FlipOutcome {
        let mut op = Op::new(OperationType::BitInvert);
        op.c_target = m.0;
        b.emit(op);
    }
    pool.close(b);
    (out, m)
}

/// Keeps the low `w` bits of `data(x)`: the `keep` field that leads every alias word.
#[must_use]
pub fn low_bits(d: &Word, w: usize) -> Word {
    let mut out = word(w);
    for j in set_bits(d, w) {
        out[j / 64] |= 1 << (j % 64);
    }
    out
}

/// The cheapest fixup split (`qroam::best_split` with a measured one-hot allowed) over `bits`
/// index bits and `limit` values whose one-hot register has at most `2^cap` qubits.
#[must_use]
pub fn fixup_bits(bits: usize, limit: u64, cap: usize) -> super::qroam::Split {
    use super::qroam::{fixup_cost, Split};
    let mut best = Split { h: 0, hot: None };
    let mut cost = fixup_cost(bits, limit, best);
    for h in 0..=bits.min(cap) {
        let opts = std::iter::once(None).chain((0..=h).filter(|_| h > 0).map(Some));
        for o in opts {
            let s = Split { h, hot: o };
            let c = fixup_cost(bits, limit, s);
            if c < cost {
                cost = c;
                best = s;
            }
        }
    }
    best
}

/// One group of outcomes for [`fixup_parts`]: the bits and `data(x)`, one bit per outcome.
pub type Part<'a> = (&'a [Bit], &'a dyn Fn(u64) -> Word);

/// One unconditional fixup over `index` (values `0..full`) for several groups of outcomes, each
/// with its own data (`(bits, data)`: `data(x)` gives one bit per outcome of the group).
pub fn fixup_parts(
    b: &mut Builder,
    index: &[Qubit],
    full: u64,
    parts: &[Part<'_>],
    s: super::qroam::Split,
) {
    let bits: Vec<Bit> = parts.iter().flat_map(|(m, _)| m.iter().copied()).collect();
    let all = |x: u64| -> Word {
        let mut out = word(bits.len());
        let mut at = 0;
        for (m, data) in parts {
            if !m.is_empty() {
                for j in set_bits(&data(x), m.len()) {
                    let k = at + j;
                    out[k / 64] |= 1 << (k % 64);
                }
            }
            at += m.len();
        }
        out
    };
    super::qroam::fixup(b, index, full, &bits, &all, s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::common::arith::less_than;
    use crate::walk::shared::lookup::{measure, phase_fixup, put};
    use crate::walk::shared::testsim::Sim;

    /// A small table: `keep(x)` for `x < limit`, pseudo-random.
    fn table(x: u64, w: usize) -> Word {
        let mut v = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5bd1_e995;
        v ^= v >> 29;
        let mut out = word(w);
        put(&mut out, 0, w, v & ((1 << w) - 1));
        out
    }

    /// Index `k` bits, draw `w` bits: read keep, compare, measure keep (fixup at the end),
    /// then the gated erasure; every lane and several outcome seeds must end clean.
    fn run(k: usize, w: usize, limit: u64, r: Reread, f: Fault, seed: u64) -> Result<u64, String> {
        run_from(k, w, 0, limit, r, f, seed)
    }

    /// [`run`] with the re-read over `start..limit` only (`gated_lt_erase_from`); the table is
    /// 0 below `start`, and every lane, those below `start` included, must end clean.
    fn run_from(
        k: usize,
        w: usize,
        start: u64,
        limit: u64,
        r: Reread,
        f: Fault,
        seed: u64,
    ) -> Result<u64, String> {
        let mut b = Builder::new(0);
        b.declare_uniform((k + w) as u32);
        let index: Vec<Qubit> = (0..k as u32).map(|i| b.uniform(i)).collect();
        let draw: Vec<Qubit> = (k as u32..(k + w) as u32).map(|i| b.uniform(i)).collect();
        let data = |x: u64| if x < start { word(w) } else { table(x, w) };
        let rd = crate::walk::sa_low::qroam::load(&mut b, &index, limit, w, &data, 1);
        let keep = rd.out.clone();
        let lt = less_than(&mut b, &draw, &keep);
        let mk = measure(&mut b, keep);
        let extra = gated_lt_erase_from(&mut b, lt, &draw, &index, start, limit, &data, r, f);
        // One fixup for the first read (junk + out) and the second read's outcomes.
        let junk = rd.into_junk();
        let a1 = 1usize.min(k);
        let full1 = read_full(k, limit, a1);
        let full2 = read_full(k, limit, r.a.min(k));
        let full = full1.max(full2);
        let mut bits = junk;
        bits.extend(mk);
        let n1 = bits.len();
        bits.extend(extra.iter().copied());
        let both = |x: u64| {
            let c1 = read_content(x, a1, w, limit, &data);
            let mut out = word(bits.len());
            for j in set_bits(&c1, n1) {
                out[j / 64] |= 1 << (j % 64);
            }
            if !extra.is_empty() {
                let c2 = read_content(x, r.a.min(k), w, limit, &data);
                for j in set_bits(&c2, extra.len()) {
                    let at = n1 + j;
                    out[at / 64] |= 1 << (at % 64);
                }
            }
            out
        };
        phase_fixup(&mut b, &index, full, &bits, &both, 1.min(k));
        let ops = b.ops().to_vec();
        let mut tof = 0;
        for x in 0..1u64 << (k + w) {
            if x & ((1 << k) - 1) >= limit {
                continue;
            }
            let mut s = Sim::new(&b, seed ^ x.wrapping_mul(0x2545_F491_4F6C_DD1D));
            s.set_uniform(x);
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                s.run(&ops);
                s.assert_clean();
            }));
            if res.is_err() {
                return Err(format!("garbage at x = {x}"));
            }
            tof += s.toffolis;
        }
        Ok(tof)
    }

    /// Exhaustive over index and draw values at small widths, block counts and both fixup
    /// placements: clean and phase-free on every lane.
    #[test]
    fn gated_reread_is_exact() {
        for (k, w, limit) in [(2, 2, 4), (3, 2, 7), (3, 3, 8), (4, 2, 11), (2, 1, 3)] {
            for a in 0..=2 {
                for fix in [None, Some(0), Some(1), Some(2)] {
                    for seed in 0..3u64 {
                        run(k, w, limit, Reread { a, fix }, Fault::None, seed * 977 + 3)
                            .unwrap_or_else(|e| panic!("k {k} w {w} a {a} fix {fix:?}: {e}"));
                    }
                }
            }
        }
    }

    /// The range re-read (`sa-toff` levers `k` / `n`): exhaustive over index and
    /// draw values with block-aligned starts, every block count and both fixup placements; the
    /// range iteration costs fewer Toffolis than the full one; mutants are caught.
    #[test]
    fn gated_reread_from_is_exact() {
        for (k, w, limit) in [(3, 2, 8), (3, 2, 7), (4, 2, 13), (4, 3, 16), (5, 2, 27)] {
            for a in 0..=2usize {
                let lam = 1u64 << a.min(k);
                for start in (lam..limit).step_by(lam as usize) {
                    for fix in [None, Some(1)] {
                        for seed in 0..2u64 {
                            let r = Reread { a, fix };
                            run_from(k, w, start, limit, r, Fault::None, seed * 131 + 5)
                                .unwrap_or_else(|e| {
                                    panic!("k {k} w {w} start {start} a {a} fix {fix:?}: {e}")
                                });
                        }
                    }
                }
            }
        }
        // The range iteration skips the blocks below start.
        let r = Reread { a: 1, fix: None };
        let full = run_from(5, 2, 0, 32, r, Fault::None, 1).unwrap();
        let part = run_from(5, 2, 24, 32, r, Fault::None, 1).unwrap();
        assert!(part < full, "range re-read {part} vs full {full}");
        for f in [Fault::NoPhase, Fault::FlipOutcome, Fault::NoTopZ] {
            let caught = (0..4u64).any(|seed| {
                run_from(4, 3, 8, 16, Reread { a: 1, fix: None }, f, seed * 31 + 7).is_err()
            });
            assert!(caught, "{f:?} with a range re-read must be caught");
        }
    }

    /// Mutants: without the comparison's phase, or gated on the wrong outcome, some lane keeps
    /// phase garbage.
    #[test]
    fn gated_reread_mutants_are_caught() {
        for f in [Fault::NoPhase, Fault::FlipOutcome, Fault::NoTopZ] {
            for (k, w) in [(2, 2), (3, 3)] {
                let caught = (0..4u64).any(|seed| {
                    run(k, w, 1 << k, Reread { a: 1, fix: None }, f, seed * 31 + 7).is_err()
                });
                assert!(caught, "{f:?} at k {k} w {w} must be caught");
            }
        }
    }

    /// `lt_phase` alone, exhaustively: `(-1)^[a < b]` exactly, carries clean.
    #[test]
    fn lt_phase_is_the_comparison() {
        for n in 1..=4usize {
            let mut b = Builder::new(0);
            b.declare_uniform(2 * n as u32);
            let a: Vec<Qubit> = (0..n as u32).map(|i| b.uniform(i)).collect();
            let bb: Vec<Qubit> = (n as u32..2 * n as u32).map(|i| b.uniform(i)).collect();
            let mut pool = Pool::default();
            lt_phase(&mut b, &a, &bb, &mut pool);
            pool.close(&mut b);
            // Undo the expected phase with a reference comparator.
            let lt = less_than(&mut b, &a, &bb);
            b.z(lt);
            crate::walk::common::arith::unless_than(&mut b, &a, &bb, lt);
            let ops = b.ops().to_vec();
            for x in 0..1u64 << (2 * n) {
                for seed in 0..3 {
                    let mut s = Sim::new(&b, seed);
                    s.set_uniform(x);
                    s.run(&ops);
                    s.assert_clean();
                }
            }
        }
    }
}
